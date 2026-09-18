//! C2 — the blocking call inside async code.
//!
//! ```python
//! async def fetch_all(urls):
//!     for u in urls:
//!         r = requests.get(u)    # freezes every other task, not just this one
//! ```
//!
//! Async code shares one thread between many tasks. One blocking call and every
//! other task in the process stops until it returns. It is a performance bug
//! that looks completely innocent, which is why it survives review and ships.

use crate::ast::{Ast, Expr, ExprId, Stmt, StmtId};
use crate::check::CheckId;
use crate::checks::{Check, Ctx};
use crate::finding::Finding;
use crate::ids::FileId;
use crate::index::ScopeId;
use crate::span::Span;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const EMBEDDED: &str = include_str!("../../../../data/blocking.toml");

#[derive(Deserialize, Debug)]
struct Table {
    blocking: HashSet<String>,
    offloading: HashSet<String>,
}

impl Table {
    fn embedded() -> &'static Table {
        static TABLE: OnceLock<Table> = OnceLock::new();
        TABLE.get_or_init(|| {
            toml::from_str(EMBEDDED).expect("the embedded blocking table must be valid")
        })
    }
}

/// Identifies one function definition, anywhere in the project.
///
/// The name span is unique per definition within a file, so this needs no
/// separate arena and stays comparable across the whole run.
type FuncKey = (FileId, Span);

struct Function {
    key: FuncKey,
    is_async: bool,
    /// A decorator can wrap the body in a thread, a cache, or nothing at all.
    /// Since the engine does not model decorators, a decorated function's
    /// blocking is not propagated to its callers.
    decorated: bool,
    body_scope: ScopeId,
    file: FileId,
}

pub struct BlockingCall;

impl Check for BlockingCall {
    fn id(&self) -> CheckId {
        CheckId::C2
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let table = Table::embedded();
        let functions = collect_functions(ctx);
        let blocking = propagate(ctx, table, &functions);

        let mut findings = Vec::new();

        for function in &functions {
            if !function.is_async {
                continue;
            }

            let ast = &ctx.asts[&function.file];
            for stmt in statements_in(ctx, function) {
                visit(
                    ctx,
                    table,
                    &blocking,
                    function,
                    ast,
                    &ast.stmt_exprs(stmt),
                    false,
                    &mut findings,
                );
            }
        }

        findings
    }
}

/// Every function definition in the project.
fn collect_functions(ctx: &Ctx<'_>) -> Vec<Function> {
    let mut functions = Vec::new();

    for (&file, ast) in ctx.asts {
        let Some(file_index) = ctx.index.file(file) else {
            continue;
        };

        for &stmt in file_index.stmt_scope.keys() {
            let Stmt::FunctionDef {
                is_async,
                name_span,
                decorators,
                ..
            } = ast.stmt(stmt)
            else {
                continue;
            };
            let Some(&body_scope) = file_index.function_scope.get(&stmt) else {
                continue;
            };

            functions.push(Function {
                key: (file, *name_span),
                is_async: *is_async,
                decorated: !decorators.is_empty(),
                body_scope,
                file,
            });
        }
    }

    functions
}

/// The statements that run when this function runs.
///
/// A statement inside an `if` shares the function's scope, while one inside a
/// nested `def` has its own — so comparing scopes gives exactly the statements
/// belonging to this function and no others.
fn statements_in(ctx: &Ctx<'_>, function: &Function) -> Vec<StmtId> {
    let Some(file_index) = ctx.index.file(function.file) else {
        return Vec::new();
    };
    let mut stmts: Vec<StmtId> = file_index
        .stmt_scope
        .iter()
        .filter_map(|(&stmt, &scope)| (scope == function.body_scope).then_some(stmt))
        .collect();
    stmts.sort_unstable();
    stmts
}

/// Marks every sync function that reaches a blocking call, to a fixpoint.
///
/// Only sync functions propagate. A blocking call inside an `async def` is
/// reported at the call itself, so propagating it to that function's callers
/// would report the same bug a second time. Sync functions are never reported
/// directly — C2 only fires inside async code — so propagation is the only way
/// their blocking reaches the surface.
fn propagate(ctx: &Ctx<'_>, table: &Table, functions: &[Function]) -> HashSet<FuncKey> {
    let mut direct: HashMap<FuncKey, Vec<Callee>> = HashMap::new();

    for function in functions {
        let ast = &ctx.asts[&function.file];
        let mut callees = Vec::new();
        for stmt in statements_in(ctx, function) {
            for &expr in &ast.stmt_exprs(stmt) {
                collect_callees(ctx, function, ast, expr, &mut callees);
            }
        }
        direct.insert(function.key, callees);
    }

    let mut blocking: HashSet<FuncKey> = HashSet::new();
    for function in functions {
        if direct[&function.key]
            .iter()
            .any(|callee| matches!(callee, Callee::Dotted(path) if table.blocking.contains(path)))
        {
            blocking.insert(function.key);
        }
    }

    // Only undecorated sync functions can carry blocking to their callers. A
    // decorator can wrap the body in a thread, a cache, or nothing at all, and
    // the engine does not model them.
    let sync: HashSet<FuncKey> = functions
        .iter()
        .filter(|f| !f.is_async && !f.decorated)
        .map(|f| f.key)
        .collect();

    // Iterating to a fixpoint terminates because the set only grows and is
    // bounded by the number of functions, so mutual recursion is fine.
    loop {
        let mut changed = false;
        for function in functions {
            if blocking.contains(&function.key) {
                continue;
            }
            let reaches = direct[&function.key].iter().any(|callee| {
                matches!(callee, Callee::Function(key) if sync.contains(key) && blocking.contains(key))
            });
            if reaches {
                blocking.insert(function.key);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Async functions never belong in the result. A blocking call inside an
    // async def is reported at the call itself, so keeping the function in this
    // set would report the same bug again at every one of its call sites.
    blocking.retain(|key| sync.contains(key));
    blocking
}

enum Callee {
    Dotted(String),
    Function(FuncKey),
}

fn collect_callees(
    ctx: &Ctx<'_>,
    function: &Function,
    ast: &Ast,
    expr: ExprId,
    out: &mut Vec<Callee>,
) {
    if let Expr::Call { func, .. } = ast.expr(expr)
        && let Some(callee) = classify(ctx, function.file, function.body_scope, ast, *func)
    {
        out.push(callee);
    }
    for child in ast.expr_children(expr) {
        collect_callees(ctx, function, ast, child, out);
    }
}

fn classify(
    ctx: &Ctx<'_>,
    file: FileId,
    scope: ScopeId,
    ast: &Ast,
    func: ExprId,
) -> Option<Callee> {
    if let Some(path) = ctx.index.dotted_path(file, scope, ast, func) {
        return Some(Callee::Dotted(path));
    }

    let Expr::Name { name, .. } = ast.expr(func) else {
        return None;
    };
    let resolved = ctx.index.resolve(file, scope, name)?;
    if !resolved.binding.kind.is_function() {
        return None;
    }
    Some(Callee::Function((
        resolved.file,
        resolved.binding.name_span,
    )))
}

/// Walks expressions looking for blocking calls, carrying whether we are inside
/// an offloading call's arguments.
#[allow(clippy::too_many_arguments)]
fn visit(
    ctx: &Ctx<'_>,
    table: &Table,
    blocking: &HashSet<FuncKey>,
    function: &Function,
    ast: &Ast,
    exprs: &[ExprId],
    offloaded: bool,
    findings: &mut Vec<Finding>,
) {
    for &expr in exprs {
        let mut inside = offloaded;

        if let Expr::Call { func, span, .. } = ast.expr(expr) {
            if is_offloading(table, ast, *func) {
                inside = true;
            } else if !offloaded
                && let Some(name) = blocking_name(ctx, table, blocking, function, ast, *func)
            {
                findings
                    .push(Finding::new(CheckId::C2, function.file, *span).with_arg("name", name));
            }
        }

        visit(
            ctx,
            table,
            blocking,
            function,
            ast,
            &ast.expr_children(expr),
            inside,
            findings,
        );
    }
}

/// Whether this call hands its work to another thread.
///
/// Matched on the called name alone, because `loop.run_in_executor(...)` is a
/// method on a value whose type the engine cannot know. Safe in this direction
/// only: name matching used for suppression can only remove findings, while
/// name matching used for detection is what manufactures false positives.
fn is_offloading(table: &Table, ast: &Ast, func: ExprId) -> bool {
    let name = match ast.expr(func) {
        Expr::Name { name, .. } => name.as_str(),
        Expr::Attribute { attr, .. } => attr.as_str(),
        _ => return false,
    };
    table.offloading.contains(name)
}

fn blocking_name(
    ctx: &Ctx<'_>,
    table: &Table,
    blocking: &HashSet<FuncKey>,
    function: &Function,
    ast: &Ast,
    func: ExprId,
) -> Option<String> {
    match classify(ctx, function.file, function.body_scope, ast, func)? {
        Callee::Dotted(path) if table.blocking.contains(&path) => Some(path),
        Callee::Function(key) if blocking.contains(&key) => {
            let Expr::Name { name, .. } = ast.expr(func) else {
                return None;
            };
            Some(name.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_table_is_valid_and_populated() {
        let table = Table::embedded();
        assert!(table.blocking.contains("time.sleep"));
        assert!(table.blocking.contains("requests.get"));
        assert!(table.offloading.contains("to_thread"));
        assert!(table.offloading.contains("run_in_executor"));
    }

    #[test]
    fn every_blocking_entry_is_a_dotted_path() {
        // A bare name here would never match, because the index always
        // produces a module-qualified path.
        for entry in &Table::embedded().blocking {
            assert!(entry.contains('.'), "{entry} is not a dotted path");
        }
    }

    #[test]
    fn no_offloading_entry_is_dotted() {
        // These are matched on the called name alone.
        for entry in &Table::embedded().offloading {
            assert!(!entry.contains('.'), "{entry} should be a bare name");
        }
    }
}
