//! Static analysis for Python: the engine.

#![forbid(unsafe_code)]

pub mod ast;
pub mod ids;
pub mod source;
pub mod span;

pub use ast::{Ast, ConstantKind, Expr, ExprId, Stmt, StmtId};
pub use ids::{Arena, FileId, Id, NodeId};
pub use source::{Position, SourceFile, SourceMap};
pub use span::Span;
