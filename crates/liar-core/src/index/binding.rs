//! What a name was bound by.
//!
//! The kind is what the checkers actually ask about. C1 needs to know whether a
//! name is an `async def`; C2 needs to know whether a name reaches a module, so
//! that `time.sleep` is recognisable however it was imported.

use crate::span::Span;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BindingKind {
    /// `def` or `async def`.
    Function {
        is_async: bool,
    },
    Class,
    /// A parameter of the enclosing function.
    Parameter,
    /// Bound by assignment, a `for` target, a `with` target, and so on.
    Variable,
    /// `import x` binds `x` to module `x`; `import x.y as z` binds `z` to
    /// module `x.y`. `path` is always the module the *name* refers to, which
    /// is what makes attribute access resolvable into a dotted path.
    Module {
        path: String,
    },
    /// `from x import y` — and `from x import y as z`, where the binding is on
    /// `z` but still refers to `x.y`.
    FromImport {
        module: String,
        name: String,
    },
}

impl BindingKind {
    pub fn is_async_function(&self) -> bool {
        matches!(self, BindingKind::Function { is_async: true })
    }

    pub fn is_function(&self) -> bool {
        matches!(self, BindingKind::Function { .. })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Binding {
    pub kind: BindingKind,
    /// Where the name itself was written, for diagnostics that point at a
    /// definition rather than at its whole body.
    pub name_span: Span,
}

impl Binding {
    pub fn new(kind: BindingKind, name_span: Span) -> Self {
        Self { kind, name_span }
    }

    /// The dotted module path this binding reaches, if any.
    ///
    /// `import time` gives `time`; `from time import sleep` gives `time.sleep`.
    /// Both are what C2 compares against its table of blocking functions.
    pub fn dotted_path(&self) -> Option<String> {
        match &self.kind {
            BindingKind::Module { path } => Some(path.clone()),
            BindingKind::FromImport { module, name } => Some(format!("{module}.{name}")),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 1)
    }

    #[test]
    fn an_async_function_reports_itself_as_one() {
        let binding = Binding::new(BindingKind::Function { is_async: true }, span());
        assert!(binding.kind.is_async_function());
        assert!(binding.kind.is_function());
    }

    #[test]
    fn a_sync_function_is_a_function_but_not_async() {
        let binding = Binding::new(BindingKind::Function { is_async: false }, span());
        assert!(!binding.kind.is_async_function());
        assert!(binding.kind.is_function());
    }

    #[test]
    fn a_variable_is_not_a_function() {
        let binding = Binding::new(BindingKind::Variable, span());
        assert!(!binding.kind.is_function());
        assert!(!binding.kind.is_async_function());
    }

    #[test]
    fn a_module_binding_gives_its_path() {
        let binding = Binding::new(
            BindingKind::Module {
                path: "time".into(),
            },
            span(),
        );
        assert_eq!(binding.dotted_path().as_deref(), Some("time"));
    }

    #[test]
    fn an_aliased_module_binding_gives_the_real_module() {
        // `import os.path as p` binds `p`, but the module is still os.path.
        let binding = Binding::new(
            BindingKind::Module {
                path: "os.path".into(),
            },
            span(),
        );
        assert_eq!(binding.dotted_path().as_deref(), Some("os.path"));
    }

    #[test]
    fn a_from_import_joins_module_and_name() {
        let binding = Binding::new(
            BindingKind::FromImport {
                module: "time".into(),
                name: "sleep".into(),
            },
            span(),
        );
        assert_eq!(binding.dotted_path().as_deref(), Some("time.sleep"));
    }

    #[test]
    fn bindings_that_are_not_imports_have_no_path() {
        for kind in [
            BindingKind::Function { is_async: false },
            BindingKind::Class,
            BindingKind::Parameter,
            BindingKind::Variable,
        ] {
            assert_eq!(Binding::new(kind, span()).dotted_path(), None);
        }
    }
}
