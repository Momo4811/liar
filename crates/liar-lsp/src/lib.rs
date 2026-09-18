//! Static analysis for Python: the language server.
//!
//! The editor extension is a launcher for the binary in `main.rs` and holds no
//! analysis logic of its own.

#![forbid(unsafe_code)]

pub mod convert;
pub mod server;
pub mod workspace;

pub use server::Backend;
pub use workspace::Workspace;
