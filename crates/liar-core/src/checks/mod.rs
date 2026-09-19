//! The checks.
//!
//! Every check goes through the index rather than pattern-matching on names.
//! A check that matches text is a regex with extra steps, and it is how false
//! positives get in.

pub mod c1_unawaited;
pub mod c2_blocking;
pub mod c3_names;
pub mod c4_leak;
pub mod c5_docstring;
pub mod docstring;

use crate::ast::Ast;
use crate::check::CheckId;
use crate::finding::Finding;
use crate::ids::FileId;
use crate::index::Index;
use crate::infer::Types;
use crate::source::SourceMap;
use std::collections::BTreeMap;

/// Everything a check is allowed to see.
pub struct Ctx<'a> {
    pub index: &'a Index,
    pub sources: &'a SourceMap,
    /// Ordered, so a check that iterates files does so deterministically even
    /// before findings are sorted.
    pub asts: &'a BTreeMap<FileId, Ast>,
    pub types: &'a Types,
    /// C3e: how many lines a scope must span before an uninformative name in
    /// it is worth mentioning. Short names in short scopes are good style.
    pub scope_threshold: u32,
}

pub trait Check {
    fn id(&self) -> CheckId;
    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding>;
}

pub fn all() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(c1_unawaited::UnawaitedCall),
        Box::new(c2_blocking::BlockingCall),
        Box::new(c3_names::BooleanName),
        Box::new(c3_names::QuantityName),
        Box::new(c3_names::UninformativeName),
        Box::new(c3_names::OverloadedName),
        Box::new(c4_leak::ResourceLeak),
        Box::new(c5_docstring::DocstringMismatch),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_check_reports_a_distinct_id() {
        let mut ids: Vec<CheckId> = all().iter().map(|check| check.id()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "two checks claim the same id");
    }
}
