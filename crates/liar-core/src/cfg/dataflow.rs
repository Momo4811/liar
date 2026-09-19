//! A worklist fixpoint over a control flow graph.
//!
//! One solver, so a check supplies a lattice and two transfer functions and
//! gets convergence for free.

use crate::cfg::{BlockId, Cfg, Edge};
use std::collections::HashMap;

/// A join-semilattice.
///
/// The laws — commutativity, associativity, idempotence, and `bottom` as the
/// identity — are the preconditions for the fixpoint below converging to an
/// answer that does not depend on the order blocks happened to be visited in.
pub trait Lattice: Clone + PartialEq {
    fn bottom() -> Self;
    fn join(&self, other: &Self) -> Self;
}

pub trait Analysis {
    type State: Lattice;

    /// The state on entry to the function.
    fn entry(&self) -> Self::State;

    /// The state leaving `block` along an edge of kind `edge`.
    ///
    /// The edge kind is a parameter because leaving normally and leaving by
    /// exception are genuinely different. A block ending in `h = open(p)` that
    /// raises has bound nothing; the same block finishing normally has bound a
    /// handle.
    fn transfer(&self, block: BlockId, edge: Edge, state: &Self::State) -> Self::State;
}

/// How many rounds before giving up.
///
/// The fixpoint converges when the lattice has finite height and the transfer
/// functions are monotone. The cap turns a violation of either into a slow
/// answer rather than a hang.
const MAX_ROUNDS: usize = 10_000;

/// The state on entry to each reachable block.
pub fn solve<A: Analysis>(cfg: &Cfg, analysis: &A) -> HashMap<BlockId, A::State> {
    let mut states: HashMap<BlockId, A::State> = HashMap::new();
    states.insert(cfg.entry(), analysis.entry());

    // Blocks are visited in id order within a round, so the answer does not
    // depend on hash iteration order.
    let mut reachable = cfg.reachable();
    reachable.sort();

    for _ in 0..MAX_ROUNDS {
        let mut changed = false;

        for &block in &reachable {
            let incoming = if block == cfg.entry() {
                analysis.entry()
            } else {
                let mut joined = A::State::bottom();
                let mut predecessors: Vec<_> = cfg.predecessors(block).collect();
                predecessors.sort_by_key(|(from, _)| *from);

                for (from, edge) in predecessors {
                    let Some(state) = states.get(&from) else {
                        continue;
                    };
                    joined = joined.join(&analysis.transfer(from, edge, state));
                }
                joined
            };

            if states.get(&block) != Some(&incoming) {
                states.insert(block, incoming);
                changed = true;
            }
        }

        if !changed {
            return states;
        }
    }

    states
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Stmt, parse};
    use crate::cfg;

    /// Counts how many blocks deep we are, saturating - a finite-height
    /// lattice, so the fixpoint must converge.
    #[derive(Clone, PartialEq, Debug)]
    struct Depth(u32);

    impl Lattice for Depth {
        fn bottom() -> Self {
            Depth(0)
        }
        fn join(&self, other: &Self) -> Self {
            Depth(self.0.max(other.0).min(5))
        }
    }

    struct Counter;

    impl Analysis for Counter {
        type State = Depth;
        fn entry(&self) -> Depth {
            Depth(1)
        }
        fn transfer(&self, _block: BlockId, _edge: Edge, state: &Depth) -> Depth {
            Depth((state.0 + 1).min(5))
        }
    }

    fn graph(source: &str) -> (crate::ast::Ast, Cfg) {
        let ast = parse(source).expect("fixture should parse");
        let body = ast
            .body()
            .iter()
            .find_map(|&stmt| match ast.stmt(stmt) {
                Stmt::FunctionDef { body, .. } => Some(body.clone()),
                _ => None,
            })
            .unwrap_or_else(|| ast.body().to_vec());
        let cfg = cfg::build(&ast, &body);
        (ast, cfg)
    }

    #[test]
    fn the_entry_state_is_the_analysis_entry() {
        let (_, cfg) = graph("def f():\n    a = 1\n");
        let states = solve(&cfg, &Counter);
        assert_eq!(states[&cfg.entry()], Depth(1));
    }

    #[test]
    fn a_loop_reaches_a_fixpoint_rather_than_running_forever() {
        // The saturating join is what makes this terminate. Without a finite
        // height it would count upward until the round cap.
        let (_, cfg) = graph("def f(xs):\n    for x in xs:\n        use(x)\n    return 1\n");
        let states = solve(&cfg, &Counter);
        assert!(states.values().all(|Depth(n)| *n <= 5));
    }

    #[test]
    fn every_reachable_block_gets_a_state() {
        let (_, cfg) = graph(
            "def f(x):\n    try:\n        a = risky()\n    except E:\n        a = None\n    finally:\n        done()\n    return a\n",
        );
        let states = solve(&cfg, &Counter);
        for block in cfg.reachable() {
            assert!(states.contains_key(&block), "no state for {block:?}");
        }
    }

    #[test]
    fn the_answer_does_not_depend_on_visiting_order() {
        // Run it repeatedly; the fixpoint must be identical each time. An
        // answer that depends on iteration order is a bug that only shows up
        // on somebody else's machine.
        let (_, cfg) = graph(
            "def f(x):\n    if x:\n        a = one()\n    else:\n        a = two()\n    for i in a:\n        use(i)\n    return a\n",
        );
        let first = solve(&cfg, &Counter);
        for _ in 0..8 {
            assert_eq!(solve(&cfg, &Counter), first);
        }
    }

    #[test]
    fn an_unreachable_block_gets_no_state() {
        let (_, cfg) = graph("def f():\n    return 1\n    a = 2\n");
        let states = solve(&cfg, &Counter);
        let unreachable: Vec<_> = cfg
            .blocks()
            .filter(|(id, block)| !block.stmts.is_empty() && !cfg.is_reachable(*id))
            .map(|(id, _)| id)
            .collect();
        assert_eq!(unreachable.len(), 1);
        assert!(!states.contains_key(&unreachable[0]));
    }
}
