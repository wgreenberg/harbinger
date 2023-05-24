use anyhow::anyhow;
use anyhow::bail;
use anyhow::Result;
use deno_ast::SourceRange;
use deno_ast::swc::ast::CallExpr;
use deno_ast::swc::ast::PropName;
use deno_ast::SourceRangedForSpanned;
use deno_ast::swc::common::EqIgnoreSpan;
use deno_ast::swc::common::Spanned;
use deno_ast::swc::parser::error::SyntaxError;
use deno_ast::swc::parser::Syntax;
use deno_ast::ParsedSource;
use deno_ast::SourceTextInfo;
use deno_ast::view::AssignOp;
use deno_ast::view::BinaryOp;
use std::path::Path;

fn verify_webpack_chunk_list(call_expr: &CallExpr) -> Option<()> {
  // we're looking for something like:
  //   `(self.webpackChunk = self.webpackChunk || []).push([ ... ])`
  let callee_member = call_expr.callee.as_expr()?
    .as_member()?;

  let called_property = callee_member.prop.as_ident()?;
  if !called_property.sym.eq_str_ignore_ascii_case("push") {
    return None;
  }

  let callee_assignment = callee_member.obj
    .as_paren()?
    .expr.as_assign()?;

  if callee_assignment.op != AssignOp::Assign {
    return None;
  }

  let lhs_member_expr = callee_assignment.left
    .as_expr()?
    .as_member()?;

  let rhs_binary_expr = callee_assignment.right
    .as_bin()?;
  if rhs_binary_expr.op != BinaryOp::LogicalOr {
    return None;
  }
  let rhs_left_member = rhs_binary_expr.left.as_member()?;
  if !rhs_left_member.eq_ignore_span(lhs_member_expr) {
    return None;
  }

  let rhs_array = rhs_binary_expr.right.as_array()?;
  if rhs_array.elems.len() != 0 {
    return None;
  }

  Some(())
}

pub fn unpack_webpack_chunk_list(source: &ParsedSource) -> Option<Vec<(String, String)>> {
  let script = source.script();
  if script.body.len() != 1 {
    return None;
  }

  let call_expr = script.body[0]
    .as_expr()?
    .expr.as_call()?;
  verify_webpack_chunk_list(call_expr)?;
  if call_expr.args.len() != 1 {
    return None;
  }
  let push_arg_arr = call_expr.args[0].expr
    .as_array()?;
  if push_arg_arr.elems.len() != 3 {
    return None;
  }
  let chunk_list = push_arg_arr.elems[1].as_ref()?
    .expr.as_object()?;

  let mut result: Vec<(String, String)> = Vec::new();
  let source_text = source.text_info();
  for maybe_prop in &chunk_list.props {
    let kv = maybe_prop.as_prop()?
      .as_key_value()?;
    let key = kv.key
      .as_num()
      .unwrap()
      .raw.as_ref()?
      .to_string();
    let chunk_range: SourceRange;
    if kv.value.is_arrow() {
      chunk_range = kv.value.range();
    } else if kv.value.is_fn_expr() {
      // for cases like `8551: function (...) { ... }`, a function expr without
      // an identifier is invalid on its own. for now we just chop out the
      // function body, but this isn't great since it leaves behind the arg list
      chunk_range = kv.value.as_fn_expr().as_ref().unwrap()
        .function
        .body.as_ref().unwrap()
        .range();
    } else {
      panic!("unknown chunk list value {:?}", &kv.value);
    }
    result.push((key, source_text.range_text(&chunk_range).to_string()));
  }
  Some(result)
}

// copy/pasted from https://github.com/dprint/dprint-plugin-typescript/blob/31f1d03fb92c9a8d1da26af3ace00286257c488e/src/swc.rs
pub fn parse_swc_ast(file_path: &Path, file_text: &str) -> Result<ParsedSource> {
  let text_info = SourceTextInfo::from_string(file_text.to_string());
  let media_type = deno_ast::MediaType::from_path(file_path);
  let mut syntax = deno_ast::get_syntax(media_type);
  if let Syntax::Es(es) = &mut syntax {
    // support decorators in js
    es.decorators = true;
  }
  let parsed_source = deno_ast::parse_program(deno_ast::ParseParams {
    specifier: file_path.to_string_lossy().to_string(),
    capture_tokens: true,
    maybe_syntax: Some(syntax),
    media_type,
    scope_analysis: false,
    text_info,
  })
  .map_err(|diagnostic| anyhow!("{:#}", &diagnostic))?;
  let diagnostics = parsed_source
    .diagnostics()
    .iter()
    .filter(|e| {
      matches!(
        e.kind,
        // unexpected eof
        SyntaxError::Eof |
        // expected identifier
        SyntaxError::TS1003 |
        SyntaxError::ExpectedIdent |
        // expected semi-colon
        SyntaxError::TS1005 |
        SyntaxError::ExpectedSemi |
        // expected expression
        SyntaxError::TS1109 |
        // expected token
        SyntaxError::Expected(_, _) |
        // various expected
        SyntaxError::ExpectedDigit { .. } |
        SyntaxError::ExpectedSemiForExprStmt { .. } |
        SyntaxError::ExpectedUnicodeEscape |
        // unexpected token
        SyntaxError::Unexpected { .. }
      )
    })
    .collect::<Vec<_>>();

  if !diagnostics.is_empty() {
    let mut final_message = String::new();
    for diagnostic in diagnostics {
      if !final_message.is_empty() {
        final_message.push_str("\n\n");
      }
      final_message.push_str(&format!("{diagnostic}"));
    }
    bail!("{}", final_message)
  }
  Ok(parsed_source)
}
