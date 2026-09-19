//! C5 — comments that lie.
//!
//! ```python
//! def save(record):
//!     """Returns the saved record's id."""
//!     self.records.append(record)          # returns None on every path
//! ```
//!
//! Everyone has been burned by a docstring that describes an earlier version of
//! the function. Three claims are checked, each chosen because it can be
//! verified without understanding prose.

use crate::ast::{Ast, ConstantKind, Expr, Param, Stmt, StmtId};
use crate::check::CheckId;
use crate::checks::docstring;
use crate::checks::{Check, Ctx};
use crate::finding::Finding;
use crate::infer::Ty;

pub struct DocstringMismatch;

impl Check for DocstringMismatch {
    fn id(&self) -> CheckId {
        CheckId::C5
    }

    fn run(&self, ctx: &Ctx<'_>) -> Vec<Finding> {
        let mut findings = Vec::new();

        for (&file, ast) in ctx.asts {
            let Some(file_index) = ctx.index.file(file) else {
                continue;
            };
            let source = ctx.sources.get(file).text();

            let mut defs: Vec<StmtId> = file_index.stmt_scope.keys().copied().collect();
            defs.sort_unstable();

            for def in defs {
                let Stmt::FunctionDef {
                    name,
                    params,
                    body,
                    decorators,
                    name_span,
                    ..
                } = ast.stmt(def)
                else {
                    continue;
                };
                // A decorator can change what the function is, and so what its
                // docstring is describing.
                if !decorators.is_empty() {
                    continue;
                }

                let Some(text) = docstring_of(ast, source, body) else {
                    continue;
                };

                let ty = ctx.types.return_ty((file, *name_span));
                let returns_explicitly = has_a_return(ast, file_index, def);

                for detail in claims(&text, params, &ty, returns_explicitly) {
                    findings.push(
                        Finding::new(CheckId::C5, file, *name_span)
                            .with_arg("name", name)
                            .with_arg("detail", detail),
                    );
                }
            }
        }

        findings
    }
}

/// Everything the docstring says that the code disagrees with.
fn claims(text: &str, params: &[Param], ty: &Ty, returns_explicitly: bool) -> Vec<String> {
    let mut found = Vec::new();

    // A function taking *args or **kwargs can legitimately document names that
    // are nowhere in its signature, so its parameter list is not judged at all.
    //
    // Asked of the parameter rather than of its name: quart writes
    // `**options`, and guessing from the name got it wrong.
    let catch_all = params.iter().any(|p| p.is_catch_all);

    if !catch_all {
        let declared: Vec<&str> = params
            .iter()
            .map(|p| p.name.as_str())
            // `self` and `cls` are never documented and never missing.
            .filter(|name| !matches!(*name, "self" | "cls"))
            .collect();

        for documented in docstring::documented_params(text) {
            if matches!(documented.as_str(), "self" | "cls") {
                continue;
            }
            if !declared.contains(&documented.as_str()) {
                found.push(format!(
                    "it documents a parameter '{documented}' that does not exist"
                ));
            }
        }
    }

    // Promising a value from a function that returns None on every path.
    //
    // Only when the function actually has a `return`, all of them bare. A
    // function with none at all has too many benign explanations: a generator
    // whose `yield` the engine does not model, an abstract method that raises,
    // a stub. That costs the case where a docstring promises a value and the
    // body simply never returns, which is the honest price of only reporting
    // what can be checked.
    if returns_explicitly && *ty == Ty::NoneType && docstring::documents_a_return(text) {
        found.push("it promises a return value, and the function returns None".to_string());
    }

    // An :rtype: that names something the function does not return.
    if let Some(named) = docstring::documented_return_type(text)
        && ty.is_known()
        && !matches!(ty, Ty::Instance(_))
        && named != ty.name()
        // A union or a generic in the rtype names more than one thing.
        && !named.contains('[')
        && !named.contains('|')
        && !named.contains(" or ")
    {
        found.push(format!(
            "its :rtype: says {named}, and the function returns {}",
            ty.name()
        ));
    }

    found
}

/// Whether the function has a `return` statement of its own.
fn has_a_return(ast: &Ast, file_index: &crate::index::FileIndex, def: StmtId) -> bool {
    let Some(&body) = file_index.function_scope.get(&def) else {
        return false;
    };
    file_index
        .stmts_in(body)
        .into_iter()
        .any(|stmt| matches!(ast.stmt(stmt), Stmt::Return { .. }))
}

/// The text of the function's docstring, if it has one.
fn docstring_of(ast: &Ast, source: &str, body: &[StmtId]) -> Option<String> {
    let first = *body.first()?;
    let Stmt::Expr { value, .. } = ast.stmt(first) else {
        return None;
    };
    let Expr::Constant {
        kind: ConstantKind::Str,
        span,
    } = ast.expr(*value)
    else {
        return None;
    };

    let literal = source.get(span.start as usize..span.end as usize)?;
    docstring::unquote(literal).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn param(name: &str) -> Param {
        Param {
            name: name.to_string(),
            annotation: None,
            span: Span::new(0, 1),
            name_span: Span::new(0, 1),
            is_catch_all: false,
        }
    }

    fn catch_all(name: &str) -> Param {
        Param {
            is_catch_all: true,
            ..param(name)
        }
    }

    #[test]
    fn a_documented_parameter_that_does_not_exist_is_reported() {
        let doc = "Args:\n    url: where from\n    retries: how many\n";
        let found = claims(doc, &[param("url")], &Ty::Str, true);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("retries"), "got {found:?}");
    }

    #[test]
    fn a_partial_docstring_is_fine() {
        // Documenting some parameters and not others is normal and useful.
        // Reporting it would be a style opinion dressed as a correctness one.
        let doc = "Args:\n    url: where from\n";
        assert!(claims(doc, &[param("url"), param("timeout")], &Ty::Str, true).is_empty());
    }

    #[test]
    fn self_is_never_counted() {
        let doc = "Args:\n    url: where from\n";
        assert!(claims(doc, &[param("self"), param("url")], &Ty::Str, true).is_empty());
    }

    #[test]
    fn a_catch_all_excuses_the_whole_parameter_list() {
        // Named `options`, not `kwargs` - which is what quart writes, and what
        // guessing from the name got wrong.
        let doc = "Args:\n    anything: at all\n    whatever: really\n";
        assert!(claims(doc, &[catch_all("options")], &Ty::Str, true).is_empty());
    }

    #[test]
    fn promising_a_return_from_a_none_function_is_reported() {
        let doc = "Saves it.\n\nReturns:\n    the new id\n";
        let found = claims(doc, &[], &Ty::NoneType, true);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("returns None"), "got {found:?}");
    }

    #[test]
    fn a_function_with_no_return_at_all_is_not_judged() {
        // A generator, an abstract method, or a stub. Too many benign
        // explanations, and 135 of them in one corpus scan.
        let doc = "Saves it.\n\nReturns:\n    the new id\n";
        assert!(claims(doc, &[], &Ty::NoneType, false).is_empty());
    }

    #[test]
    fn promising_a_return_from_a_real_function_is_fine() {
        let doc = "Saves it.\n\nReturns:\n    the new id\n";
        assert!(claims(doc, &[], &Ty::Int, true).is_empty());
    }

    #[test]
    fn an_unknown_return_type_is_never_judged() {
        let doc = "Saves it.\n\nReturns:\n    the new id\n";
        assert!(claims(doc, &[], &Ty::Unknown, true).is_empty());
    }

    #[test]
    fn a_contradicted_rtype_is_reported() {
        let found = claims(":rtype: str\n", &[], &Ty::Int, true);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("str"), "got {found:?}");
    }

    #[test]
    fn a_matching_rtype_is_fine() {
        assert!(claims(":rtype: str\n", &[], &Ty::Str, true).is_empty());
    }

    #[test]
    fn a_generic_or_union_rtype_is_not_judged() {
        // Naming more than one thing is not a contradiction.
        for rtype in [
            ":rtype: list[str]\n",
            ":rtype: str | None\n",
            ":rtype: str or None\n",
        ] {
            assert!(claims(rtype, &[], &Ty::Str, true).is_empty(), "{rtype:?}");
        }
    }

    #[test]
    fn an_rtype_naming_a_class_is_not_judged() {
        use crate::ids::{FileId, Id};
        let instance = Ty::Instance((FileId::from_index(0), Span::new(0, 1)));
        assert!(claims(":rtype: Report\n", &[], &instance, true).is_empty());
    }

    #[test]
    fn a_docstring_saying_nothing_checkable_produces_nothing() {
        assert!(claims("Fetches the thing.", &[param("url")], &Ty::Str, true).is_empty());
    }
}
