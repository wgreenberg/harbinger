use anyhow::anyhow;
use anyhow::bail;
use anyhow::Result;
use deno_ast::swc::ast::{self, BlockStmt, BlockStmtOrExpr, CallExpr, Expr, Ident, KeyValueProp};
use deno_ast::swc::common::EqIgnoreSpan;
use deno_ast::swc::common::{FilePathMapping, SourceMap};
use deno_ast::swc::parser::error::SyntaxError;
use deno_ast::swc::parser::Syntax;
use deno_ast::swc::visit::{as_folder, noop_visit_mut_type, FoldWith, VisitMut, VisitMutWith};
use deno_ast::view::AssignOp;
use deno_ast::view::BinaryOp;
use deno_ast::view::EsVersion;
use deno_ast::ParsedSource;
use deno_ast::SourceTextInfo;
use std::{fs::OpenOptions, io::Write, path::Path, sync::Arc};
use swc::config::SourceMapsConfig;
use swc::Compiler;
use swc_core::common::{collections::AHashMap, util::take::Take, Globals, GLOBALS};

fn verify_webpack_chunk_list(call_expr: &CallExpr) -> Option<()> {
    // we're looking for something like:
    //   `(self.webpackChunk = self.webpackChunk || []).push([ ... ])`
    let callee_member = call_expr.callee.as_expr()?.as_member()?;

    let called_property = callee_member.prop.as_ident()?;
    if !called_property.sym.eq_str_ignore_ascii_case("push") {
        return None;
    }

    let callee_assignment = callee_member.obj.as_paren()?.expr.as_assign()?;
    if callee_assignment.op != AssignOp::Assign {
        return None;
    }

    let lhs_member_expr = callee_assignment.left.as_expr()?.as_member()?;
    let rhs_binary_expr = callee_assignment.right.as_bin()?;
    if rhs_binary_expr.op != BinaryOp::LogicalOr {
        return None;
    }
    let rhs_left_member = rhs_binary_expr.left.as_member()?;
    if !rhs_left_member.eq_ignore_span(lhs_member_expr) {
        return None;
    }

    let rhs_array = rhs_binary_expr.right.as_array()?;
    if !rhs_array.elems.is_empty() {
        return None;
    }

    Some(())
}

#[derive(Debug)]
pub struct WebpackChunk {
    pub label: String,
    pub block: BlockStmt,
    module_id: Option<Ident>,
    require_id: Option<Ident>,
    exports_id: Option<Ident>,
}

impl WebpackChunk {
    fn rename_webpack_bits(&mut self) {
        let transformer = WebpackChunkIdentRenameTransformer {
            module_id: self.module_id.as_ref(),
            require_id: self.require_id.as_ref(),
            exports_id: self.exports_id.as_ref(),
        };
        self.block = self.block.take().fold_with(&mut as_folder(transformer));
    }

    pub fn write_to_file(&self, path: &std::path::Path) -> Result<()> {
        let c = Compiler::new(Arc::new(SourceMap::new(FilePathMapping::empty())));
        let mut chunk_path = path.join(&self.label);
        chunk_path.set_extension("js");
        let globals = Globals::new();
        GLOBALS.set(&globals, || {
            let ast_printed = c
                .print(
                    &self.block,
                    None,
                    None,
                    false,
                    EsVersion::Es2022,
                    SourceMapsConfig::Bool(false),
                    &AHashMap::default(),
                    None,
                    false,
                    None,
                    false,
                    false,
                )
                .expect("Failed to print");
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(chunk_path)
                .unwrap();
            file.write_all(ast_printed.code.as_bytes()).unwrap();
        });
        Ok(())
    }
}

struct WebpackChunkIdentRenameTransformer<'a> {
    module_id: Option<&'a Ident>,
    require_id: Option<&'a Ident>,
    exports_id: Option<&'a Ident>,
}

impl<'a> VisitMut for WebpackChunkIdentRenameTransformer<'a> {
    noop_visit_mut_type!(); // omits TypeScript metadata

    fn visit_mut_ident(&mut self, ident: &mut ast::Ident) {
        ident.visit_mut_children_with(self);
        if let Some(module) = &self.module_id {
            if module.to_id() == ident.to_id() {
                ident.sym = "module".into();
                return;
            }
        }
        if let Some(require) = &self.require_id {
            if require.to_id() == ident.to_id() {
                ident.sym = "__webpack_require__".into();
                return;
            }
        }
        if let Some(exports) = &self.exports_id {
            if exports.to_id() == ident.to_id() {
                ident.sym = "exports".into();
                return;
            }
        }
    }
}

impl TryFrom<&KeyValueProp> for WebpackChunk {
    type Error = String;

    fn try_from(kv: &KeyValueProp) -> std::result::Result<Self, Self::Error> {
        let key = kv.key.as_num().unwrap().raw.as_ref().unwrap().to_string();
        let block: BlockStmt;
        let mut params: Vec<Ident> = Vec::new();
        match &*kv.value {
            Expr::Fn(fun) => {
                block = fun.function.body.clone().unwrap();
                for param in &fun.function.params {
                    match &param.pat {
                        deno_ast::swc::ast::Pat::Ident(param_id) => {
                            params.push(param_id.id.clone());
                        }
                        _ => return Err(format!("unexpected param pat {:?}", &param.pat)),
                    }
                }
            }
            Expr::Arrow(arrow) => {
                match &*arrow.body {
                    BlockStmtOrExpr::BlockStmt(blk) => {
                        block = blk.clone();
                    }
                    BlockStmtOrExpr::Expr(_) => {
                        panic!("webpack chunk's arrow function had an expr instead of BlockStmt");
                    }
                }
                for pat in &arrow.params {
                    match pat {
                        deno_ast::swc::ast::Pat::Ident(param_id) => {
                            params.push(param_id.id.clone());
                        }
                        _ => return Err(format!("unexpected param pat {:?}", pat)),
                    }
                }
            }
            _ => return Err(format!("unknown chunk list value {:?}", kv.value)),
        };
        let mut params = params.drain(..);
        Ok(Self {
            label: key,
            block,
            module_id: params.next(),
            exports_id: params.next(),
            require_id: params.next(),
        })
    }
}

pub fn unpack_webpack_chunk_list(source: &ParsedSource) -> Option<Vec<WebpackChunk>> {
    let script = source.script();
    if script.body.len() != 1 {
        return None;
    }

    let call_expr = script.body[0].as_expr()?.expr.as_call()?;
    verify_webpack_chunk_list(call_expr)?;
    if call_expr.args.len() != 1 {
        return None;
    }
    let push_arg_arr = call_expr.args[0].expr.as_array()?;
    if push_arg_arr.elems.len() != 3 {
        return None;
    }
    let chunk_list = push_arg_arr.elems[1].as_ref()?.expr.as_object()?;

    let mut result = Vec::new();
    for maybe_prop in &chunk_list.props {
        let kv = maybe_prop.as_prop()?.as_key_value()?;
        let mut chunk = WebpackChunk::try_from(kv).unwrap();
        chunk.rename_webpack_bits();
        result.push(chunk);
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
