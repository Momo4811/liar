//! Control flow graphs.
//!
//! Every check before this one reasons about a statement, a name, or a call.
//! This reasons about *paths* — and specifically about the paths nobody writes
//! down.
//!
//! ```python
//! f = open(path)
//! data = parse(f.read())   # if this raises...
//! f.close()                # ...this never runs
//! ```
//!
//! Fine on the happy path, and leaks a handle every time `parse` raises.
//! Modelling that means admitting that almost every statement in Python has an
//! invisible edge leaving it, which is what this module does.

pub mod build;
pub mod dataflow;

pub use build::build;
pub use dataflow::{Analysis, Lattice, solve};

use crate::ast::StmtId;
use crate::define_id;
use crate::ids::Arena;
use crate::span::Span;

define_id!(BlockId);

/// A run of statements with one way in and one way out.
#[derive(Clone, Debug, Default)]
pub struct Block {
    pub stmts: Vec<StmtId>,
    pub span: Option<Span>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    /// Sequencing, a branch taken, a loop going round.
    Normal,
    /// The invisible one: something raised and control left from the middle of
    /// a statement rather than the end of it.
    Exception,
}

/// One function's control flow.
///
/// Two exits, not one. A function can finish by returning or by propagating an
/// exception, and the second is the entire point of this check — a resource
/// released just before `return` is still leaked on every path that never
/// reaches it.
#[derive(Debug)]
pub struct Cfg {
    blocks: Arena<BlockId, Block>,
    edges: Vec<(BlockId, BlockId, Edge)>,
    entry: BlockId,
    normal_exit: BlockId,
    exception_exit: BlockId,
}

impl Cfg {
    pub fn entry(&self) -> BlockId {
        self.entry
    }

    pub fn normal_exit(&self) -> BlockId {
        self.normal_exit
    }

    pub fn exception_exit(&self) -> BlockId {
        self.exception_exit
    }

    /// Both ways a function can finish.
    pub fn exits(&self) -> [BlockId; 2] {
        [self.normal_exit, self.exception_exit]
    }

    pub fn block(&self, id: BlockId) -> &Block {
        self.blocks.get(id)
    }

    pub fn blocks(&self) -> impl Iterator<Item = (BlockId, &Block)> {
        self.blocks.iter()
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn edges(&self) -> &[(BlockId, BlockId, Edge)] {
        &self.edges
    }

    pub fn successors(&self, block: BlockId) -> impl Iterator<Item = (BlockId, Edge)> + '_ {
        self.edges
            .iter()
            .filter(move |(from, _, _)| *from == block)
            .map(|(_, to, kind)| (*to, *kind))
    }

    pub fn predecessors(&self, block: BlockId) -> impl Iterator<Item = (BlockId, Edge)> + '_ {
        self.edges
            .iter()
            .filter(move |(_, to, _)| *to == block)
            .map(|(from, _, kind)| (*from, *kind))
    }

    /// Whether `block` can be reached from the entry.
    pub fn is_reachable(&self, block: BlockId) -> bool {
        self.reachable().contains(&block)
    }

    /// Every block reachable from the entry, in discovery order.
    pub fn reachable(&self) -> Vec<BlockId> {
        let mut seen = vec![self.entry];
        let mut queue = vec![self.entry];

        while let Some(current) = queue.pop() {
            for (next, _) in self.successors(current) {
                if !seen.contains(&next) {
                    seen.push(next);
                    queue.push(next);
                }
            }
        }

        seen
    }

    /// The graph as Graphviz, for `liar debug cfg` and for the golden files.
    ///
    /// A control flow graph is exactly the kind of thing that looks right and
    /// is subtly wrong, so the shapes are pinned as text and reviewed by eye.
    pub fn to_dot(&self, name: &str) -> String {
        let mut out = format!("digraph {name} {{\n");
        out.push_str("  node [shape=box];\n");

        for (id, block) in self.blocks() {
            let label = if id == self.entry {
                "entry".to_string()
            } else if id == self.normal_exit {
                "exit: return".to_string()
            } else if id == self.exception_exit {
                "exit: raised".to_string()
            } else if block.stmts.is_empty() {
                format!("b{}", id.index())
            } else {
                format!("b{} ({} stmts)", id.index(), block.stmts.len())
            };
            out.push_str(&format!("  b{} [label=\"{}\"];\n", id.index(), label));
        }

        for (from, to, kind) in &self.edges {
            let style = match kind {
                Edge::Normal => "",
                Edge::Exception => " [style=dashed, color=gray, label=\"raises\"]",
            };
            out.push_str(&format!(
                "  b{} -> b{}{};\n",
                from.index(),
                to.index(),
                style
            ));
        }

        out.push_str("}\n");
        out
    }
}

use crate::ids::Id;
