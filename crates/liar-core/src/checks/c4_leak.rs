//! C4 — the cleanup that lies.
//!
//! ```python
//! def read_config(path):
//!     f = open(path)
//!     data = parse(f.read())   # if this raises...
//!     f.close()                # ...this never runs
//!     return data
//! ```
//!
//! Fine on the happy path. Leaks a file handle every time `parse` raises. Do
//! it in a request handler and the process eventually hits the operating
//! system's descriptor limit and dies a long way from the line responsible.
//!
//! This is the only check that needs a real control flow graph, because the
//! path it reports is one nobody wrote down.

use crate::ast::{Ast, Expr, ExprId, Stmt, StmtId};
use crate::cfg::{self, Analysis, BlockId, Cfg, Edge, Lattice};
use crate::check::CheckId;
use crate::checks::{Check, Ctx};
use crate::finding::Finding;
use crate::ids::FileId;
use crate::index::ScopeId;
use crate::span::Span;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

const EMBEDDED: &str = include_str!("../../../../data/resources.toml");

#[derive(Deserialize, Debug)]
struct Resources {
    /// Acquiring call → the method that releases what it returns.
    acquire: BTreeMap<String, String>,
}

impl Resources {
    fn embedded() -> &'static Resources {
        static TABLE: OnceLock<Resources> = OnceLock::new();
        TABLE.get_or_init(|| {
            toml::from_str(EMBEDDED).expect("the embedded resource table must be valid")
        })
    }
}

/// How alarming a resource's state is, least first.
///
/// This is a *may* analysis: joining takes the maximum, so a handle open on any
/// one path into an exit is open at that exit. That is the question C4 asks —
/// not "is it always leaked" but "is there a path on which it is".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum State {
    Unopened,
    Closed,
    /// Handed to somebody else, so no longer this function's problem.
    ///
    /// Ordered below `Open` on purpose: where a path that escaped meets a path
    /// that did not, the one that did not is what matters.
    Escaped,
    Open,
}

#[derive(Clone, PartialEq, Debug, Default)]
struct States(BTreeMap<String, State>);

impl Lattice for States {
    fn bottom() -> Self {
        States::default()
    }

    fn join(&self, other: &Self) -> Self {
        let mut joined = self.0.clone();
        for (name, state) in &other.0 {
            let existing = joined.get(name).copied().unwrap_or(State::Unopened);
            joined.insert(name.clone(), existing.max(*state));
        }
        States(joined)
    }
}

/// One resource worth tracking.
struct Tracked {
    name: String,
    /// Where it was acquired, for the caret.
    span: Span,
    /// The method that releases it.
    release: String,
    /// The statement that acquired it, so an exception *from that statement*
    /// can be treated as having bound nothing.
    acquisition: StmtId,
    /// The statements on which the handle leaves the function.
    ///
    /// Per statement rather than per function, because which *path* the escape
    /// happened on is the whole question: a socket returned on the path that
    /// succeeds is still lost on the path that retries.
    escapes_at: HashSet<StmtId>,
}

struct LeakAnalysis<'a> {
    ast: &'a Ast,
    cfg: &'a Cfg,
    tracked: &'a [Tracked],
}

impl Analysis for LeakAnalysis<'_> {
    type State = States;

    fn entry(&self) -> States {
        States::default()
    }

    fn transfer(&self, block: BlockId, edge: Edge, state: &States) -> States {
        let mut next = state.clone();

        for &stmt in &self.cfg.block(block).stmts {
            // An acquisition that raises has bound nothing. Anything else that
            // raises has already done whatever it was going to do to the
            // handles being tracked - a `close()` that raises still attempted
            // the close, and a `parse()` that raises never touched them.
            let acquisition_here = self.tracked.iter().find(|t| t.acquisition == stmt);
            if edge == Edge::Exception && acquisition_here.is_some() {
                continue;
            }

            if let Some(tracked) = acquisition_here {
                next.0.insert(tracked.name.clone(), State::Open);
                continue;
            }

            if let Some(released) = self.released_by(stmt) {
                next.0.insert(released, State::Closed);
            }

            for resource in self.tracked {
                if resource.escapes_at.contains(&stmt) {
                    next.0.insert(resource.name.clone(), State::Escaped);
                }
            }
        }

        next
    }
}

impl LeakAnalysis<'_> {
    /// The name released by this statement, if it is `handle.close()`.
    fn released_by(&self, stmt: StmtId) -> Option<String> {
        let Stmt::Expr { value, .. } = self.ast.stmt(stmt) else {
            return None;
        };
        let Expr::Call { func, .. } = self.ast.expr(*value) else {
            return None;
        };
        let Expr::Attribute {
            value: base, attr, ..
        } = self.ast.expr(*func)
        else {
            return None;
        };
        let Expr::Name { name, .. } = self.ast.expr(*base) else {
            return None;
        };

        self.tracked
            .iter()
            .find(|t| &t.name == name && &t.release == attr)
            .map(|t| t.name.clone())
    }
}

pub struct ResourceLeak;

impl Check for ResourceLeak {
    fn id(&self) -> CheckId {
        CheckId::C4
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let table = Resources::embedded();
        let mut findings = Vec::new();

        for (&file, ast) in ctx.asts {
            let Some(file_index) = ctx.index.file(file) else {
                continue;
            };

            let mut defs: Vec<StmtId> = file_index.stmt_scope.keys().copied().collect();
            defs.sort_unstable();

            for def in defs {
                let Stmt::FunctionDef { body, .. } = ast.stmt(def) else {
                    continue;
                };
                let Some(&scope) = file_index.function_scope.get(&def) else {
                    continue;
                };

                findings.extend(check_function(ctx, table, file, scope, ast, body));
            }
        }

        findings
    }
}

fn check_function(
    ctx: &Ctx<'_>,
    table: &Resources,
    file: FileId,
    scope: ScopeId,
    ast: &Ast,
    body: &[StmtId],
) -> Vec<Finding> {
    let tracked = find_resources(ctx, table, file, scope, ast, body);
    if tracked.is_empty() {
        return Vec::new();
    }

    let cfg = cfg::build(ast, body);
    let analysis = LeakAnalysis {
        ast,
        cfg: &cfg,
        tracked: &tracked,
    };
    let states = cfg::solve(&cfg, &analysis);

    let mut findings = Vec::new();

    for resource in &tracked {
        let mut leaked = false;
        let mut released = 0usize;
        let mut total = 0usize;

        // Acquiring into a name that already holds an open handle loses the
        // first one, whatever the function does with the second. This is what
        // a retry loop looks like: open, fail, `continue`, open again.
        if reacquired_while_open(&cfg, &states, &analysis, resource) {
            findings.push(
                Finding::new(CheckId::C4, file, resource.span)
                    .with_arg("name", &resource.name)
                    .with_arg("released", "0")
                    .with_arg("total", "1"),
            );
            continue;
        }

        for exit in cfg.exits() {
            if !cfg.is_reachable(exit) {
                continue;
            }
            let mut predecessors: Vec<_> = cfg.predecessors(exit).collect();
            predecessors.sort_by_key(|(from, _)| *from);

            for (from, edge) in predecessors {
                let Some(state) = states.get(&from) else {
                    continue;
                };
                let leaving = analysis.transfer(from, edge, state);
                match leaving
                    .0
                    .get(&resource.name)
                    .copied()
                    .unwrap_or(State::Unopened)
                {
                    State::Open => {
                        leaked = true;
                        total += 1;
                    }
                    State::Closed => {
                        released += 1;
                        total += 1;
                    }
                    // Handed to somebody else on this path; their problem now,
                    // and a path that did resolve the handle.
                    State::Escaped => {
                        released += 1;
                        total += 1;
                    }
                    // Never acquired on this path, so it is not a path out of
                    // anything and does not belong in the count.
                    State::Unopened => {}
                }
            }
        }

        if leaked {
            findings.push(
                Finding::new(CheckId::C4, file, resource.span)
                    .with_arg("name", &resource.name)
                    .with_arg("released", released.to_string())
                    .with_arg("total", total.to_string()),
            );
        }
    }

    findings
}

/// Whether the acquisition can be reached with the name already open.
///
/// The loop-retry shape: `_sock = socket.socket(...)`, something fails,
/// `continue`, and round again - losing the first socket. The escape rule
/// cannot see this, because the handle genuinely does escape on the path that
/// succeeds.
fn reacquired_while_open(
    cfg: &Cfg,
    states: &std::collections::HashMap<BlockId, States>,
    analysis: &LeakAnalysis<'_>,
    resource: &Tracked,
) -> bool {
    let Some((block, _)) = cfg
        .blocks()
        .find(|(_, b)| b.stmts.contains(&resource.acquisition))
    else {
        return false;
    };

    let Some(entry) = states.get(&block) else {
        return false;
    };

    // Apply whatever precedes the acquisition within its own block.
    let mut state = entry.clone();
    for &stmt in &cfg.block(block).stmts {
        if stmt == resource.acquisition {
            break;
        }
        if let Some(released) = analysis.released_by(stmt) {
            state.0.insert(released, State::Closed);
        }
    }

    state.0.get(&resource.name).copied() == Some(State::Open)
}

/// The resources this function acquires and keeps to itself.
fn find_resources(
    ctx: &Ctx<'_>,
    table: &Resources,
    file: FileId,
    scope: ScopeId,
    ast: &Ast,
    body: &[StmtId],
) -> Vec<Tracked> {
    let statements = all_statements(ast, body);
    let used = used_positions(ast, &statements);

    let mut tracked = Vec::new();

    for &stmt in &statements {
        let Stmt::Assign { targets, value, .. } = ast.stmt(stmt) else {
            continue;
        };
        if targets.len() != 1 {
            continue;
        }
        let Expr::Name { name, span } = ast.expr(targets[0]) else {
            continue;
        };
        let Expr::Call { func, .. } = ast.expr(*value) else {
            continue;
        };

        let path =
            ctx.index
                .dotted_path(file, scope, ast, *func)
                .or_else(|| match ast.expr(*func) {
                    // A builtin such as `open` resolves to nothing, and nothing is
                    // exactly what says it is not this project's own function.
                    Expr::Name { name, .. } if ctx.index.resolve(file, scope, name).is_none() => {
                        Some(name.clone())
                    }
                    _ => None,
                });

        let Some(release) = path.and_then(|path| table.acquire.get(&path)) else {
            continue;
        };

        // Once a handle leaves the function its lifetime is somebody else's
        // business. Recording *where* that happens rather than merely whether
        // is what lets the reacquire rule stay correct in a retry loop.
        let escapes_at = escape_statements(ast, &statements, &used, name, targets[0]);

        tracked.push(Tracked {
            name: name.clone(),
            span: *span,
            release: release.clone(),
            acquisition: stmt,
            escapes_at,
        });
    }

    tracked
}

/// Whether the handle leaves the function.
///
/// A *called* attribute is a use: `f.read()` reads through the handle and does
/// not hand it anywhere.
///
/// Everything else is an escape — the bare name in `parse(f)`, `return f`,
/// `self.handle = f`, and also an *uncalled* attribute. `return f.close` hands
/// the caller a bound method and with it the responsibility for closing, which
/// is a perfectly good design and not a leak.
fn escape_statements(
    ast: &Ast,
    statements: &[StmtId],
    used_positions: &HashSet<ExprId>,
    name: &str,
    acquisition_target: ExprId,
) -> HashSet<StmtId> {
    let mut escaping = HashSet::new();

    for &stmt in statements {
        for &expr in &ast.stmt_exprs(stmt) {
            for mention in expr_tree(ast, expr) {
                if mention == acquisition_target || used_positions.contains(&mention) {
                    continue;
                }
                if matches!(ast.expr(mention), Expr::Name { name: other, .. } if other == name) {
                    escaping.insert(stmt);
                }
            }
        }
    }

    escaping
}

/// Every expression sitting where a handle is merely *used*.
///
/// That means the object of an attribute access which is itself being called:
/// the `f` in `f.read()`. The `f` in `return f.close` is not here, because
/// handing over a bound method hands over the handle.
fn used_positions(ast: &Ast, statements: &[StmtId]) -> HashSet<ExprId> {
    let mut called = HashSet::new();
    for &stmt in statements {
        for &expr in &ast.stmt_exprs(stmt) {
            for node in expr_tree(ast, expr) {
                if let Expr::Call { func, .. } = ast.expr(node) {
                    called.insert(*func);
                }
            }
        }
    }

    let mut used = HashSet::new();
    for &stmt in statements {
        for &expr in &ast.stmt_exprs(stmt) {
            for node in expr_tree(ast, expr) {
                if let Expr::Attribute { value, .. } = ast.expr(node)
                    && called.contains(&node)
                {
                    used.insert(*value);
                }
            }
        }
    }
    used
}

fn expr_tree(ast: &Ast, root: ExprId) -> Vec<ExprId> {
    let mut all = vec![root];
    let mut queue = vec![root];
    while let Some(current) = queue.pop() {
        for child in ast.expr_children(current) {
            all.push(child);
            queue.push(child);
        }
    }
    all
}

/// Every statement in the body, nested ones included, but not the bodies of
/// nested definitions.
fn all_statements(ast: &Ast, body: &[StmtId]) -> Vec<StmtId> {
    let mut all = Vec::new();
    let mut queue: Vec<StmtId> = body.to_vec();

    while let Some(stmt) = queue.pop() {
        all.push(stmt);
        match ast.stmt(stmt) {
            Stmt::If { body, orelse, .. }
            | Stmt::While { body, orelse, .. }
            | Stmt::For { body, orelse, .. } => {
                queue.extend(body.iter().chain(orelse.iter()).copied());
            }
            Stmt::With { body, .. } => queue.extend(body.iter().copied()),
            Stmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
                ..
            } => {
                queue.extend(body.iter().copied());
                queue.extend(orelse.iter().copied());
                queue.extend(finalbody.iter().copied());
                for handler in handlers {
                    queue.extend(handler.body.iter().copied());
                }
            }
            // A nested definition does not run here.
            _ => {}
        }
    }

    all.sort_unstable();
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_table_is_valid() {
        let table = Resources::embedded();
        assert_eq!(table.acquire.get("open").map(String::as_str), Some("close"));
        assert!(table.acquire.contains_key("socket.socket"));
    }

    #[test]
    fn open_is_the_most_alarming_state() {
        // The ordering is what makes the join a "may be open" analysis.
        assert!(State::Open > State::Closed);
        assert!(State::Closed > State::Unopened);
    }

    #[test]
    fn joining_takes_the_more_alarming_side() {
        let open = States(BTreeMap::from([("f".to_string(), State::Open)]));
        let closed = States(BTreeMap::from([("f".to_string(), State::Closed)]));

        assert_eq!(open.join(&closed), open);
        assert_eq!(closed.join(&open), open);
        assert_eq!(States::bottom().join(&open), open);
    }

    #[test]
    fn a_name_absent_from_one_side_is_unopened_there() {
        let open = States(BTreeMap::from([("f".to_string(), State::Open)]));
        assert_eq!(States::bottom().join(&open), open);
    }
}
