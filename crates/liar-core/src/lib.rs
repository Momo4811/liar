//! Static analysis for Python: the engine.

#![forbid(unsafe_code)]

pub mod ids;

pub use ids::{Arena, FileId, Id, NodeId};
