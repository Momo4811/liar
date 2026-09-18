//! C1 — the `await` that isn't there.
//!
//! ```python
//! async def handle(req):
//!     save_user(req.user)     # nothing happens. at all.
//!     return {"ok": True}
//! ```
//!
//! Calling an async function without awaiting it builds a coroutine and throws
//! it away. The body never runs, nothing raises, and the caller returns
//! success. There is no argument to be had about it, which is why this check
//! leads.

use crate::ast::{Ast, Expr, ExprId, Stmt};
use crate::check::CheckId;
use crate::checks::{Check, Ctx};
use crate::finding::Finding;
use crate::ids::FileId;
use crate::index::{ScopeId, ScopeKind};

pub struct UnawaitedCall;

impl Check for UnawaitedCall {
    fn id(&self) -> CheckId {
        CheckId::C1
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let mut findings = Vec::new();

        for (&file, ast) in ctx.asts {
            let Some(file_index) = ctx.index.file(file) else {
                continue;
            };

            for (&stmt, &scope) in &file_index.stmt_scope {
                match ast.stmt(stmt) {
                    // A statement that is nothing but a call. Only the top
                    // level is examined: `gather(f(), g())` is a call to
                    // gather, which is not async, so nothing is reported and
                    // the inner calls are never looked at. Every asyncio
                    // wrapper is therefore suppressed by construction, rather
                    // than by a list of names that would need maintaining.
                    Stmt::Expr { value, .. } => {
                        findings.extend(report_if_async(ctx, file, scope, ast, *value));
                    }

                    // `x = f()` where x is never mentioned again. Deliberately
                    // conservative: if the name is used at all, for anything,
                    // this says nothing, because it cannot know whether that
                    // use awaits it.
                    Stmt::Assign { targets, value, .. } if targets.len() == 1 => {
                        let Expr::Name { name, .. } = ast.expr(targets[0]) else {
                            continue;
                        };
                        if mentions(ast, name) > 1 {
                            continue;
                        }
                        findings.extend(report_if_async(ctx, file, scope, ast, *value));
                    }

                    _ => {}
                }
            }
        }

        findings
    }
}

/// How many times `name` appears as a name anywhere in the file.
///
/// The assignment target itself counts as one, so a genuinely unused binding
/// scores exactly one. Counting file-wide rather than per-function
/// over-counts, which suppresses findings rather than inventing them — the
/// safe direction.
fn mentions(ast: &Ast, name: &str) -> usize {
    ast.exprs()
        .filter(|(_, expr)| matches!(expr, Expr::Name { name: other, .. } if other == name))
        .count()
}

fn report_if_async(
    ctx: &Ctx<'_>,
    file: FileId,
    scope: ScopeId,
    ast: &Ast,
    expr: ExprId,
) -> Option<Finding> {
    // An awaited call is an Await wrapping a Call, not a Call, so it never
    // reaches here.
    let Expr::Call { func, span, .. } = ast.expr(expr) else {
        return None;
    };

    let resolved = resolve_callee(ctx, file, scope, ast, *func)?;
    if !resolved.is_async {
        return None;
    }

    Some(Finding::new(CheckId::C1, file, *span).with_arg("name", resolved.name))
}

struct Callee {
    name: String,
    is_async: bool,
}

/// Works out what is being called.
///
/// Handles a bare name, and `self.method()` inside a class body. A call on any
/// other object would need to know that object's type, which the engine cannot
/// do yet — so it resolves to nothing and the check stays quiet.
fn resolve_callee(
    ctx: &Ctx<'_>,
    file: FileId,
    scope: ScopeId,
    ast: &Ast,
    func: ExprId,
) -> Option<Callee> {
    match ast.expr(func) {
        Expr::Name { name, .. } => {
            let resolved = ctx.index.resolve(file, scope, name)?;
            Some(Callee {
                name: name.clone(),
                is_async: resolved.binding.kind.is_async_function(),
            })
        }

        Expr::Attribute { value, attr, .. } => {
            let Expr::Name { name: base, .. } = ast.expr(*value) else {
                return None;
            };
            if base != "self" {
                return None;
            }

            // `self` only means anything inside a method, so find the class
            // body this scope sits in and look the attribute up there.
            let file_index = ctx.index.file(file)?;
            let class_scope = file_index
                .scopes
                .ancestry(scope)
                .into_iter()
                .find(|&candidate| file_index.scopes.scope(candidate).kind == ScopeKind::Class)?;

            let binding = file_index.scopes.lookup_local(class_scope, attr)?;
            Some(Callee {
                name: format!("self.{attr}"),
                is_async: binding.kind.is_async_function(),
            })
        }

        _ => None,
    }
}
