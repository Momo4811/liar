//! What type is this?
//!
//! Shallow on purpose: the checks ask whether something is a boolean, a number
//! or a collection, and the inference answers exactly that.

pub mod annotation;
pub mod ty;

pub use annotation::from_annotation;
pub use ty::{ClassKey, Ty};
