//! Static analysis for Python: the engine.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod ast;
pub mod check;
pub mod checks;
pub mod finding;
pub mod fixture;
pub mod ids;
pub mod index;
pub mod infer;
pub mod messages;
pub mod source;
pub mod span;

pub use analysis::{Settings, analyse, analyse_with};
pub use ast::{
    Ast, ConstantKind, Expr, ExprId, ImportAlias, Param, ParseError, Stmt, StmtId, parse,
};
pub use check::{CheckId, Severity};
pub use checks::{Check, Ctx};
pub use finding::{Finding, Label, sort_findings};
pub use fixture::{Expectation, FixtureFailure, check_fixture, parse_expectations};
pub use ids::{Arena, FileId, Id, NodeId};
pub use index::{
    Binding, BindingKind, FileIndex, Index, IndexInput, Resolved, ScopeId, ScopeKind, ScopeTree,
};
pub use infer::{Ty, Types};
pub use messages::{MessageTable, Tone};
pub use source::{Position, SourceFile, SourceMap};
pub use span::Span;
