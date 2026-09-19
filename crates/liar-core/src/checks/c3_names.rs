//! C3 — names that lie.
//!
//! ```python
//! def is_ready() -> str:      # returns str. one of us is confused.
//! count = []                  # a count is a number. this is a list.
//! data, data_2, data_final    # four variables called data. none are related.
//! ```
//!
//! These are opinions about naming, which makes them the likeliest checks in
//! the tool to annoy somebody. Two rules keep them bearable:
//!
//! 1. **Never fire on an instance of a project class.** A class can implement
//!    `__bool__`, `__int__`, `__len__` or anything else, so a name like
//!    `is_valid` holding one may be telling the truth. Only the builtin types,
//!    whose behaviour is known, are judged.
//! 2. **Short names in short scopes are good style.** `i` in a three-line loop
//!    is correct, and a tool that does not know this is a tool people
//!    uninstall.

use crate::ast::Stmt;
use crate::check::CheckId;
use crate::checks::{Check, Ctx};
use crate::finding::Finding;
use crate::ids::FileId;
use crate::index::ScopeId;
use crate::infer::Ty;
use crate::span::Span;
use serde::Deserialize;
use std::sync::OnceLock;

const EMBEDDED: &str = include_str!("../../../../data/names.toml");

#[derive(Deserialize, Debug)]
struct Names {
    boolean_prefixes: Vec<String>,
    quantity_exact: Vec<String>,
    quantity_prefixes: Vec<String>,
    quantity_suffixes: Vec<String>,
    uninformative: Vec<String>,
    allowed_short: Vec<String>,
}

impl Names {
    fn embedded() -> &'static Names {
        static NAMES: OnceLock<Names> = OnceLock::new();
        NAMES.get_or_init(|| {
            toml::from_str(EMBEDDED).expect("the embedded name table must be valid")
        })
    }

    fn reads_as_boolean(&self, name: &str) -> bool {
        let name = name.trim_start_matches('_');
        self.boolean_prefixes
            .iter()
            .any(|prefix| name.starts_with(prefix.as_str()))
    }

    fn reads_as_quantity(&self, name: &str) -> bool {
        let name = name.trim_start_matches('_');
        self.quantity_exact.iter().any(|exact| name == exact)
            || self
                .quantity_prefixes
                .iter()
                .any(|p| name.starts_with(p.as_str()))
            || self
                .quantity_suffixes
                .iter()
                .any(|s| name.ends_with(s.as_str()))
    }

    fn is_uninformative(&self, name: &str) -> bool {
        if self.allowed_short.iter().any(|allowed| name == allowed) {
            return false;
        }
        self.uninformative.iter().any(|dull| name == dull)
            || (name.chars().count() == 1 && name != "_")
    }
}

/// Whether a check is allowed to judge this type.
///
/// Only builtins. An instance of a project class can implement `__bool__`,
/// `__len__` or `__int__`, so a name like `is_valid` holding one may well be
/// telling the truth, and this tool has no way to know.
fn judgeable(ty: &Ty) -> bool {
    ty.is_known() && !matches!(ty, Ty::Instance(_))
}

// ---------------------------------------------------------------------------

/// C3a — a name that asks a question whose answer is not a boolean.
pub struct BooleanName;

impl Check for BooleanName {
    fn id(&self) -> CheckId {
        CheckId::C3a
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let names = Names::embedded();
        let mut findings = Vec::new();

        for_each_name(ctx, &mut |file, _scope, name, span, ty| {
            if names.reads_as_boolean(name) && judgeable(&ty) && !ty.is_bool() {
                findings.push(
                    Finding::new(CheckId::C3a, file, span)
                        .with_arg("name", name)
                        .with_arg("ty", ty.name()),
                );
            }
        });

        findings
    }
}

/// C3b — a name that promises a number and holds something else.
pub struct QuantityName;

impl Check for QuantityName {
    fn id(&self) -> CheckId {
        CheckId::C3b
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let names = Names::embedded();
        let mut findings = Vec::new();

        for_each_name(ctx, &mut |file, _scope, name, span, ty| {
            if names.reads_as_quantity(name) && judgeable(&ty) && !ty.is_numeric() {
                findings.push(
                    Finding::new(CheckId::C3b, file, span)
                        .with_arg("name", name)
                        .with_arg("ty", ty.name()),
                );
            }
        });

        findings
    }
}

/// C3e — a name that tells the reader nothing, over a scope long enough for
/// that to matter.
pub struct UninformativeName;

impl Check for UninformativeName {
    fn id(&self) -> CheckId {
        CheckId::C3e
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let names = Names::embedded();
        let mut findings = Vec::new();

        for &file in ctx.asts.keys() {
            let Some(file_index) = ctx.index.file(file) else {
                continue;
            };

            for (scope, _) in file_index.scopes.iter() {
                let lines = scope_lines(ctx, file, scope);
                if lines < ctx.scope_threshold {
                    continue;
                }

                let mut named: Vec<(&String, &crate::index::Binding)> =
                    file_index.scopes.scope(scope).bindings.iter().collect();
                named.sort_by_key(|(_, binding)| binding.name_span.start);

                for (name, binding) in named {
                    if !names.is_uninformative(name) {
                        continue;
                    }
                    // Only names a person chose for a value: a module or an
                    // imported symbol was named by somebody else.
                    if !matches!(
                        binding.kind,
                        crate::index::BindingKind::Variable | crate::index::BindingKind::Parameter
                    ) {
                        continue;
                    }
                    findings.push(
                        Finding::new(CheckId::C3e, file, binding.name_span)
                            .with_arg("name", name)
                            .with_arg("lines", lines.to_string()),
                    );
                }
            }
        }

        findings
    }
}

/// C3f — one name, several unrelated meanings in a file.
///
/// The check that motivated the whole project.
pub struct OverloadedName;

impl Check for OverloadedName {
    fn id(&self) -> CheckId {
        CheckId::C3f
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let mut findings = Vec::new();

        for &file in ctx.asts.keys() {
            let Some(file_index) = ctx.index.file(file) else {
                continue;
            };

            for (scope, _) in file_index.scopes.iter() {
                let mut names: Vec<&String> =
                    file_index.scopes.scope(scope).bindings.keys().collect();
                names.sort();

                for name in names {
                    let sites = ctx.types.sites(file, scope, name);
                    if sites.len() < 3 {
                        continue;
                    }

                    let mut distinct: Vec<&Ty> = Vec::new();
                    for (_, ty) in sites {
                        if judgeable(ty) && !distinct.contains(&ty) {
                            distinct.push(ty);
                        }
                    }
                    if distinct.len() < 2 {
                        continue;
                    }

                    // One finding pointing at every occurrence, not one per
                    // occurrence: the complaint is about the group.
                    let mut finding = Finding::new(CheckId::C3f, file, sites[0].0)
                        .with_arg("name", name)
                        .with_arg("n", sites.len().to_string())
                        .with_arg("k", distinct.len().to_string());

                    for (span, ty) in &sites[1..] {
                        finding = finding.with_secondary(file, *span, Some(ty.name().to_string()));
                    }

                    findings.push(finding);
                }
            }
        }

        findings
    }
}

// ---------------------------------------------------------------------------

/// How many lines a scope spans.
fn scope_lines(ctx: &Ctx<'_>, file: FileId, scope: ScopeId) -> u32 {
    let Some(file_index) = ctx.index.file(file) else {
        return 0;
    };
    let span = file_index.scopes.scope(scope).span;
    let source = ctx.sources.get(file);

    let start = source
        .position(span.start.min(source.text().len() as u32))
        .line;
    let end = source
        .position(span.end.min(source.text().len() as u32))
        .line;
    end.saturating_sub(start) + 1
}

/// Calls `visit` for every named thing with a type: each function's return, and
/// each variable binding.
fn for_each_name(ctx: &Ctx<'_>, visit: &mut dyn FnMut(FileId, ScopeId, &str, Span, Ty)) {
    for (&file, ast) in ctx.asts {
        let Some(file_index) = ctx.index.file(file) else {
            continue;
        };

        // Function returns. A decorated function is skipped: a decorator can
        // change what calling it produces.
        let mut defs: Vec<_> = file_index.stmt_scope.keys().copied().collect();
        defs.sort_unstable();

        for stmt in defs {
            let Stmt::FunctionDef {
                name,
                name_span,
                decorators,
                ..
            } = ast.stmt(stmt)
            else {
                continue;
            };
            if !decorators.is_empty() {
                continue;
            }
            let scope = file_index.scope_of(stmt);
            let ty = ctx.types.return_ty((file, *name_span));
            visit(file, scope, name, *name_span, ty);
        }

        // Variables, at every binding site.
        for (scope, _) in file_index.scopes.iter() {
            let mut names: Vec<&String> = file_index.scopes.scope(scope).bindings.keys().collect();
            names.sort();

            for name in names {
                for (span, ty) in ctx.types.sites(file, scope, name) {
                    visit(file, scope, name, *span, ty.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_table_is_valid() {
        let names = Names::embedded();
        assert!(!names.boolean_prefixes.is_empty());
        assert!(!names.uninformative.is_empty());
    }

    #[test]
    fn boolean_prefixes_are_recognised() {
        let names = Names::embedded();
        for name in [
            "is_ready",
            "has_items",
            "can_edit",
            "should_retry",
            "_is_ready",
        ] {
            assert!(names.reads_as_boolean(name), "{name}");
        }
        for name in ["island", "history", "ready", "iscsi_target"] {
            assert!(!names.reads_as_boolean(name), "{name}");
        }
    }

    #[test]
    fn quantity_names_are_recognised() {
        let names = Names::embedded();
        for name in ["count", "num_items", "n_rows", "item_count", "buffer_size"] {
            assert!(names.reads_as_quantity(name), "{name}");
        }
        // Left out deliberately: an index can be an object, a level can be a
        // logging level, a rank can be a label.
        for name in ["index", "position", "level", "rank", "counter", "account"] {
            assert!(!names.reads_as_quantity(name), "{name}");
        }
    }

    #[test]
    fn idiomatic_short_names_are_not_uninformative() {
        let names = Names::embedded();
        for name in ["i", "j", "k", "n", "_", "x"] {
            assert!(!names.is_uninformative(name), "{name}");
        }
        for name in ["data", "temp", "q", "w"] {
            assert!(names.is_uninformative(name), "{name}");
        }
    }

    #[test]
    fn an_instance_is_never_judged() {
        // A class can implement __bool__, __len__ or __int__, so a name like
        // is_valid holding one may be telling the truth.
        use crate::ids::{FileId, Id};
        let instance = Ty::Instance((FileId::from_index(0), Span::new(0, 1)));
        assert!(!judgeable(&instance));
        assert!(!judgeable(&Ty::Unknown));
        assert!(!judgeable(&Ty::Never));
        assert!(judgeable(&Ty::Str));
    }
}
