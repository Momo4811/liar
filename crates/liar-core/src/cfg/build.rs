//! Building a control flow graph from a statement list.

use crate::ast::{Ast, Expr, ExprId, Stmt, StmtId};
use crate::cfg::{Block, BlockId, Cfg, Edge};
use crate::ids::Arena;

/// Builds the control flow of one function body, or of a module.
pub fn build(ast: &Ast, body: &[StmtId]) -> Cfg {
    let mut builder = Builder::new();

    builder.current = builder.entry;
    builder.statements(ast, body);

    // Whatever the body ended in, falling off the end is a normal finish.
    builder.edge(builder.current, builder.normal_exit, Edge::Normal);

    builder.finish()
}

struct Builder {
    blocks: Arena<BlockId, Block>,
    edges: Vec<(BlockId, BlockId, Edge)>,
    entry: BlockId,
    normal_exit: BlockId,
    exception_exit: BlockId,
    current: BlockId,
    /// `finally` blocks a `return` must pass through, innermost last.
    ///
    /// Python runs every enclosing finally before leaving, which is the whole
    /// reason try/finally is correct cleanup. A return that jumped straight to
    /// the exit would skip the close and C4 would report a leak that is not
    /// there.
    finallys: Vec<BlockId>,
    /// Where an exception raised right now would go, innermost last.
    ///
    /// Several targets at one level because a `try` can have several `except`
    /// clauses and which one catches depends on the exception's type — so the
    /// graph admits all of them.
    handlers: Vec<Vec<BlockId>>,
}

impl Builder {
    fn new() -> Self {
        let mut blocks = Arena::new();
        let entry = blocks.alloc(Block::default());
        let normal_exit = blocks.alloc(Block::default());
        let exception_exit = blocks.alloc(Block::default());

        Self {
            blocks,
            edges: Vec::new(),
            entry,
            normal_exit,
            exception_exit,
            current: entry,
            finallys: Vec::new(),
            handlers: Vec::new(),
        }
    }

    fn finish(self) -> Cfg {
        Cfg {
            blocks: self.blocks,
            edges: self.edges,
            entry: self.entry,
            normal_exit: self.normal_exit,
            exception_exit: self.exception_exit,
        }
    }

    fn new_block(&mut self) -> BlockId {
        self.blocks.alloc(Block::default())
    }

    fn edge(&mut self, from: BlockId, to: BlockId, kind: Edge) {
        if !self.edges.contains(&(from, to, kind)) {
            self.edges.push((from, to, kind));
        }
    }

    fn push(&mut self, ast: &Ast, stmt: StmtId) {
        let span = ast.stmt_span(stmt);
        let block = self.blocks.get_mut(self.current);
        block.stmts.push(stmt);
        block.span = Some(match block.span {
            None => span,
            Some(existing) => crate::span::Span::new(existing.start, span.end),
        });
    }

    /// Where an exception raised in the current block would go.
    fn exception_targets(&self) -> Vec<BlockId> {
        match self.handlers.last() {
            Some(targets) => targets.clone(),
            None => vec![self.exception_exit],
        }
    }

    /// Adds the invisible edge, if this statement can raise.
    ///
    /// Does not end the block. Only for statements that branch anyway - an
    /// `if` header already has two successors, and splitting it would separate
    /// the test from the branch it controls.
    fn maybe_raises_here(&mut self, ast: &Ast, stmt: StmtId) {
        if !can_raise(ast, stmt) {
            return;
        }
        for target in self.exception_targets() {
            self.edge(self.current, target, Edge::Exception);
        }
    }

    /// Adds the invisible edge and **ends the block**.
    ///
    /// A raising statement has to terminate its block, or the graph cannot
    /// distinguish an exception thrown after it from one thrown before. With
    /// open(), parse() and close() in a single block there is no path on which
    /// the handle is open and the close has not run - which is the entire bug
    /// C4 exists to find.
    fn maybe_raises(&mut self, ast: &Ast, stmt: StmtId) {
        if !can_raise(ast, stmt) {
            return;
        }
        for target in self.exception_targets() {
            self.edge(self.current, target, Edge::Exception);
        }
        let next = self.new_block();
        self.edge(self.current, next, Edge::Normal);
        self.current = next;
    }

    fn statements(&mut self, ast: &Ast, stmts: &[StmtId]) {
        for &stmt in stmts {
            self.statement(ast, stmt);
        }
    }

    fn statement(&mut self, ast: &Ast, stmt: StmtId) {
        match ast.stmt(stmt) {
            Stmt::Return { .. } => {
                self.push(ast, stmt);
                self.maybe_raises_here(ast, stmt);
                // Through the innermost finally, if there is one. Falling out
                // of that block continues to the code after the try rather
                // than to the exit, which is imprecise about *where* control
                // resumes but exact about *what runs first* - and what runs
                // first is the cleanup this check is about.
                let target = self.finallys.last().copied().unwrap_or(self.normal_exit);
                self.edge(self.current, target, Edge::Normal);
                // Anything after a return is unreachable, and lands in a block
                // nothing points at.
                self.current = self.new_block();
            }

            Stmt::If { body, orelse, .. } => {
                // The statement itself represents evaluating the test.
                self.push(ast, stmt);
                self.maybe_raises_here(ast, stmt);
                let header = self.current;

                let then_block = self.new_block();
                self.edge(header, then_block, Edge::Normal);
                self.current = then_block;
                self.statements(ast, body);
                let then_end = self.current;

                let else_block = self.new_block();
                self.edge(header, else_block, Edge::Normal);
                self.current = else_block;
                self.statements(ast, orelse);
                let else_end = self.current;

                let join = self.new_block();
                self.edge(then_end, join, Edge::Normal);
                self.edge(else_end, join, Edge::Normal);
                self.current = join;
            }

            Stmt::While { body, orelse, .. } | Stmt::For { body, orelse, .. } => {
                let header = self.new_block();
                self.edge(self.current, header, Edge::Normal);
                self.current = header;
                self.push(ast, stmt);
                self.maybe_raises_here(ast, stmt);

                let body_block = self.new_block();
                self.edge(header, body_block, Edge::Normal);
                self.current = body_block;
                self.statements(ast, body);
                // The back edge: this is what makes it a loop.
                self.edge(self.current, header, Edge::Normal);

                // The loop finishing normally runs `orelse`, then carries on.
                let else_block = self.new_block();
                self.edge(header, else_block, Edge::Normal);
                self.current = else_block;
                self.statements(ast, orelse);

                let after = self.new_block();
                self.edge(self.current, after, Edge::Normal);
                self.current = after;
            }

            Stmt::With { body, .. } => {
                // `with` is the language solving the cleanup problem properly.
                // It is still ordinary control flow: enter, run the body, and
                // the exit runs whatever happens.
                self.push(ast, stmt);
                self.maybe_raises(ast, stmt);
                let inner = self.new_block();
                self.edge(self.current, inner, Edge::Normal);
                self.current = inner;
                self.statements(ast, body);
            }

            Stmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
                ..
            } => {
                let finally_entry = self.new_block();

                let handler_entries: Vec<BlockId> =
                    handlers.iter().map(|_| self.new_block()).collect();

                // With no `except` clause, an exception still runs `finally`
                // before continuing outward.
                let targets = if handler_entries.is_empty() {
                    vec![finally_entry]
                } else {
                    handler_entries.clone()
                };

                let body_block = self.new_block();
                self.edge(self.current, body_block, Edge::Normal);
                self.current = body_block;

                // A finally with nothing in it is not worth routing through.
                let routes_returns = !finalbody.is_empty();
                if routes_returns {
                    self.finallys.push(finally_entry);
                }

                self.handlers.push(targets);
                self.statements(ast, body);
                self.handlers.pop();

                // The body finishing normally runs `else`, then `finally`.
                let else_block = self.new_block();
                self.edge(self.current, else_block, Edge::Normal);
                self.current = else_block;
                self.statements(ast, orelse);
                self.edge(self.current, finally_entry, Edge::Normal);

                for (entry, handler) in handler_entries.iter().zip(handlers) {
                    self.current = *entry;
                    self.statements(ast, &handler.body);
                    self.edge(self.current, finally_entry, Edge::Normal);
                }

                // Popped before building the finally itself: a return inside
                // a finally does not re-enter it.
                if routes_returns {
                    self.finallys.pop();
                }

                self.current = finally_entry;
                self.statements(ast, finalbody);

                // After `finally` the exception continues outward, which is
                // what makes try/finally correct cleanup and try/except
                // swallowing.
                if handler_entries.is_empty() {
                    for target in self.exception_targets() {
                        self.edge(self.current, target, Edge::Exception);
                    }
                }

                let after = self.new_block();
                self.edge(self.current, after, Edge::Normal);
                self.current = after;
            }

            // A nested definition does not run here; it is a binding. Its own
            // body gets its own graph.
            Stmt::FunctionDef { .. } | Stmt::ClassDef { .. } => {
                self.push(ast, stmt);
            }

            _ => {
                self.push(ast, stmt);
                self.maybe_raises(ast, stmt);
            }
        }
    }
}

/// Whether a statement can raise.
///
/// "Contains a call" — crude, and deliberately so. Almost anything in Python
/// can raise, and assuming calls do is the cheapest rule that catches the case
/// this exists for. Erring toward more paths errs toward more places a resource
/// might leak, which costs precision rather than soundness.
fn can_raise(ast: &Ast, stmt: StmtId) -> bool {
    ast.stmt_exprs(stmt)
        .iter()
        .any(|&expr| contains_call(ast, expr))
}

fn contains_call(ast: &Ast, expr: ExprId) -> bool {
    if matches!(ast.expr(expr), Expr::Call { .. }) {
        return true;
    }
    ast.expr_children(expr)
        .iter()
        .any(|&child| contains_call(ast, child))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;

    /// Builds the graph of the first function in `source`, or of the module
    /// when there is no function.
    fn graph(source: &str) -> (Ast, Cfg) {
        let ast = parse(source).expect("fixture should parse");
        let body = ast
            .body()
            .iter()
            .find_map(|&stmt| match ast.stmt(stmt) {
                Stmt::FunctionDef { body, .. } => Some(body.clone()),
                _ => None,
            })
            .unwrap_or_else(|| ast.body().to_vec());
        let cfg = build(&ast, &body);
        (ast, cfg)
    }

    fn exception_edges(cfg: &Cfg) -> usize {
        cfg.edges()
            .iter()
            .filter(|(_, _, kind)| *kind == Edge::Exception)
            .count()
    }

    #[test]
    fn a_bare_sequence_is_one_path() {
        let (_, cfg) = graph("def f():\n    a = 1\n    b = 2\n");
        assert_eq!(exception_edges(&cfg), 0, "nothing here can raise");
        assert!(cfg.is_reachable(cfg.normal_exit()));
    }

    #[test]
    fn a_call_gets_an_exception_edge() {
        let (_, cfg) = graph("def f():\n    a = compute()\n");
        assert_eq!(exception_edges(&cfg), 1);
        assert!(
            cfg.is_reachable(cfg.exception_exit()),
            "the exception exit must be reachable once something can raise"
        );
    }

    #[test]
    fn the_canonical_leak_has_a_path_that_skips_the_close() {
        // The example the whole check exists for.
        let (_, cfg) = graph(
            "def read_config(path):\n    f = open(path)\n    data = parse(f.read())\n    f.close()\n    return data\n",
        );
        assert!(cfg.is_reachable(cfg.exception_exit()));
        assert!(cfg.is_reachable(cfg.normal_exit()));
        assert!(
            exception_edges(&cfg) >= 3,
            "open, parse and close can all raise"
        );
    }

    #[test]
    fn both_exits_exist_and_are_distinct() {
        let (_, cfg) = graph("def f():\n    return compute()\n");
        assert_ne!(cfg.normal_exit(), cfg.exception_exit());
        assert!(cfg.is_reachable(cfg.normal_exit()));
        assert!(cfg.is_reachable(cfg.exception_exit()));
    }

    #[test]
    fn an_if_branches_and_rejoins() {
        let (_, cfg) = graph(
            "def f(flag):\n    if flag:\n        a = 1\n    else:\n        a = 2\n    return a\n",
        );

        // The header has two successors, and they meet again.
        let header = cfg
            .blocks()
            .find(|(id, _)| {
                cfg.successors(*id)
                    .filter(|(_, k)| *k == Edge::Normal)
                    .count()
                    == 2
            })
            .map(|(id, _)| id)
            .expect("a branching block");
        let branches: Vec<_> = cfg.successors(header).map(|(to, _)| to).collect();
        assert_eq!(branches.len(), 2);

        let first: Vec<_> = cfg.successors(branches[0]).map(|(to, _)| to).collect();
        let second: Vec<_> = cfg.successors(branches[1]).map(|(to, _)| to).collect();
        assert_eq!(first, second, "the branches rejoin at the same block");
    }

    #[test]
    fn a_loop_has_a_back_edge() {
        let (_, cfg) = graph("def f(items):\n    for item in items:\n        use(item)\n");

        // A back edge is one whose target can reach its source.
        let has_back_edge = cfg.edges().iter().any(|(from, to, _)| {
            let mut seen = vec![*to];
            let mut queue = vec![*to];
            while let Some(current) = queue.pop() {
                for (next, _) in cfg.successors(current) {
                    if next == *from {
                        return true;
                    }
                    if !seen.contains(&next) {
                        seen.push(next);
                        queue.push(next);
                    }
                }
            }
            false
        });
        assert!(has_back_edge, "a loop must be able to go round");
    }

    #[test]
    fn a_try_routes_exceptions_into_its_handler() {
        let (_, cfg) = graph(
            "def f():\n    try:\n        risky()\n    except ValueError:\n        recover()\n",
        );

        // The raising block's exception edge must not reach the exception exit
        // directly: the handler catches it.
        let to_handler = cfg
            .edges()
            .iter()
            .any(|(_, to, kind)| *kind == Edge::Exception && *to != cfg.exception_exit());
        assert!(to_handler, "the handler should catch it");
    }

    #[test]
    fn a_try_finally_lets_the_exception_continue_outward() {
        // finally runs, then the exception carries on. That is what makes
        // try/finally correct cleanup and try/except swallowing.
        let (_, cfg) =
            graph("def f():\n    try:\n        risky()\n    finally:\n        cleanup()\n");
        assert!(
            cfg.is_reachable(cfg.exception_exit()),
            "an unhandled exception must still leave the function"
        );
    }

    #[test]
    fn a_return_inside_try_finally_runs_the_finally_first() {
        // Python runs every enclosing finally before leaving. A return that
        // jumped straight to the exit would skip the cleanup, and C4 would
        // report a leak that is not there - which is exactly what happened
        // before this.
        let (ast, cfg) = graph(
            "def f(p):
    h = open(p)
    try:
        return read(h)
    finally:
        h.close()
",
        );

        // Find the block holding the return, and the block holding the close.
        let find = |needle: &str| -> BlockId {
            cfg.blocks()
                .find(|(_, block)| {
                    block.stmts.iter().any(|&stmt| {
                        let span = ast.stmt_span(stmt);
                        let text = "def f(p):
    h = open(p)
    try:
        return read(h)
    finally:
        h.close()
";
                        text[span.start as usize..span.end as usize].contains(needle)
                    })
                })
                .map(|(id, _)| id)
                .unwrap_or_else(|| panic!("no block containing {needle}"))
        };

        let returning = find("return read");
        let closing = find("h.close()");

        // The return must be able to reach the close.
        let mut seen = vec![returning];
        let mut queue = vec![returning];
        let mut reaches_close = false;
        while let Some(current) = queue.pop() {
            for (next, _) in cfg.successors(current) {
                if next == closing {
                    reaches_close = true;
                }
                if !seen.contains(&next) {
                    seen.push(next);
                    queue.push(next);
                }
            }
        }
        assert!(reaches_close, "the return must pass through the finally");
    }

    #[test]
    fn code_after_a_return_is_unreachable() {
        let (ast, cfg) = graph("def f():\n    return 1\n    a = 2\n");

        let unreachable: Vec<_> = cfg
            .blocks()
            .filter(|(id, block)| !block.stmts.is_empty() && !cfg.is_reachable(*id))
            .collect();
        assert_eq!(unreachable.len(), 1, "the statement after return");
        assert!(matches!(
            ast.stmt(unreachable[0].1.stmts[0]),
            Stmt::Assign { .. }
        ));
    }

    #[test]
    fn a_nested_function_body_is_not_inlined() {
        // A def is a binding, not a jump. Its body gets its own graph.
        let (_, cfg) = graph("def outer():\n    def inner():\n        risky()\n    return 1\n");
        assert_eq!(exception_edges(&cfg), 0, "nothing in outer can raise");
    }

    #[test]
    fn every_block_is_reachable_or_follows_a_return() {
        // A sanity property: unreachable blocks only ever appear after a jump.
        let sources = [
            "def f():\n    a = 1\n",
            "def f(x):\n    if x:\n        a = 1\n    return a\n",
            "def f(xs):\n    for x in xs:\n        use(x)\n    return 1\n",
            "def f():\n    try:\n        a()\n    except E:\n        b()\n    finally:\n        c()\n",
        ];
        for source in sources {
            let (_, cfg) = graph(source);
            for (id, block) in cfg.blocks() {
                if !block.stmts.is_empty() {
                    assert!(cfg.is_reachable(id), "unreachable code in {source:?}");
                }
            }
        }
    }

    /// Golden graphs. A control flow graph is exactly the kind of thing that
    /// looks right and is subtly wrong, so the shapes are pinned as text and
    /// reviewed by eye once.
    #[test]
    fn golden_shapes() {
        for (name, source) in [
            (
                "sequence",
                "def f():
    a = 1
    b = 2
",
            ),
            (
                "branch",
                "def f(x):
    if x:
        a = 1
    else:
        a = 2
",
            ),
            (
                "loop",
                "def f(xs):
    for x in xs:
        use(x)
",
            ),
            (
                "try_except",
                "def f():
    try:
        risky()
    except E:
        recover()
",
            ),
            (
                "try_finally",
                "def f():
    try:
        risky()
    finally:
        cleanup()
",
            ),
            (
                "early_return",
                "def f(x):
    if x:
        return 1
    return 2
",
            ),
            (
                "the_leak",
                "def f(p):
    h = open(p)
    d = parse(h.read())
    h.close()
    return d
",
            ),
        ] {
            let (_, cfg) = graph(source);
            insta::assert_snapshot!(format!("cfg_{name}"), cfg.to_dot(name));
        }
    }

    #[test]
    fn construction_is_deterministic() {
        let source = "def f(x):\n    try:\n        a = open(x)\n    except OSError:\n        return None\n    finally:\n        done()\n    return a\n";
        let (_, first) = graph(source);
        let (_, second) = graph(source);
        assert_eq!(first.to_dot("f"), second.to_dot("f"));
    }
}
