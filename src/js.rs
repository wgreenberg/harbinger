use anyhow::bail;
use anyhow::Result;
use swc::PrintArgs;
use std::{fs::OpenOptions, io::Write, path::Path, sync::Arc};
use swc::Compiler;
use swc_core::{
    common::{
        errors::{ColorConfig, Handler},
        sync::Lrc,
        util::take::Take,
        EqIgnoreSpan, FileName, FilePathMapping, Globals, SourceMap, GLOBALS,
    },
    ecma::{
        ast::{
            self, AssignOp, BinaryOp, BlockStmt, BlockStmtOrExpr, CallExpr, EsVersion, Expr, Ident,
            KeyValueProp, Script
        },
        visit::{as_folder, noop_visit_mut_type, FoldWith, VisitMut, VisitMutWith},
    },
};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};


fn verify_webpack_chunk_list(call_expr: &CallExpr) -> Option<()> {
    // we're looking for something like:
    //   `(self.webpackChunk = self.webpackChunk || []).push([ ... ])`
    let callee_member = call_expr.callee.as_expr()?.as_member()?;

    let called_property = callee_member.prop.as_ident()?;
    if !called_property.sym.eq_ignore_ascii_case("push") {
        return None;
    }

    let callee_assignment = callee_member.obj.as_paren()?.expr.as_assign()?;
    if callee_assignment.op != AssignOp::Assign {
        return None;
    }

    let lhs_member_expr = callee_assignment.left.as_simple()?.as_member()?;
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

    pub fn into_script(self) -> Script {
        Script {
            body: self.block.stmts,
            span: self.block.span,
            shebang: None,
        }
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
                        ast::Pat::Ident(param_id) => {
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
                        ast::Pat::Ident(param_id) => {
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

pub fn unpack_webpack_chunk_list(script: &Script) -> Option<Vec<WebpackChunk>> {
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

pub fn parse_js(file_name: String, file_text: String) -> Result<Script> {
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));
    let fm = cm.new_source_file(Arc::new(FileName::Custom(file_name)), file_text);
    let lexer = Lexer::new(
        Syntax::Es(Default::default()),
        EsVersion::Es2022,
        StringInput::from(&*fm),
        None,
    );

    let mut parser = Parser::new_from(lexer);

    for e in parser.take_errors() {
        e.into_diagnostic(&handler).emit();
    }

    match parser.parse_script() {
        Ok(script) => Ok(script),
        Err(e) => bail!("failed to parse script: {:?}", e),
    }
}

pub fn write_script(script: &Script, path: &Path) -> Result<()> {
    let c = Compiler::new(Arc::new(SourceMap::new(FilePathMapping::empty())));
    let globals = Globals::new();
    GLOBALS.set(&globals, || {
        let print_args = PrintArgs::default();
        let ast_printed = c.print(script, print_args)
            .expect("Failed to print");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(path)
            .unwrap();
        file.write_all(ast_printed.code.as_bytes()).unwrap();
    });
    Ok(())
}
