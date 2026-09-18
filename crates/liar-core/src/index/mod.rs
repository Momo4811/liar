//! What does this name refer to?
//!
//! Every checker asks the index rather than pattern-matching on the syntax
//! tree. A checker that matches names textually is a regex with extra steps,
//! and it is how false positives get in.

pub mod binding;
pub mod build;
pub mod scope;

pub use binding::{Binding, BindingKind};
pub use build::{FileIndex, build_file};
pub use scope::{Scope, ScopeId, ScopeKind, ScopeTree};
