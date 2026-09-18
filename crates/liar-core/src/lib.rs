//! Static analysis for Python: the engine.

#![forbid(unsafe_code)]

pub mod ast;
pub mod check;
pub mod finding;
pub mod fixture;
pub mod ids;
pub mod messages;
pub mod source;
pub mod span;

pub use ast::{Ast, ConstantKind, Expr, ExprId, Param, ParseError, Stmt, StmtId, parse};
pub use check::{CheckId, Severity};
pub use finding::{Finding, Label, sort_findings};
pub use fixture::{Expectation, FixtureFailure, check_fixture, parse_expectations};
pub use ids::{Arena, FileId, Id, NodeId};
pub use messages::{MessageTable, Tone};
pub use source::{Position, SourceFile, SourceMap};
pub use span::Span;
