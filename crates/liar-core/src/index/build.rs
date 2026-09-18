//! Walking a syntax tree to populate a file's scopes and bindings.

use crate::ast::{Ast, Expr, Stmt, StmtId};
use crate::ids::FileId;
use crate::index::binding::{Binding, BindingKind};
use crate::index::scope::{ScopeId, ScopeKind, ScopeTree};
use std::collections::HashMap;

/// Everything known about one file.
#[derive(Debug)]
pub struct FileIndex {
    pub file: FileId,
    pub scopes: ScopeTree,
    /// The scope each statement sits in, so a checker walking the tree can ask
    /// where it is without repeating this traversal.
    pub stmt_scope: HashMap<StmtId, ScopeId>,
    /// For a function definition, the scope its body lives in.
    pub function_scope: HashMap<StmtId, ScopeId>,
    /// This file's dotted module path, once the project root is known.
    pub module_path: Option<String>,
}

impl FileIndex {
    pub fn scope_of(&self, stmt: StmtId) -> ScopeId {
        self.stmt_scope
            .get(&stmt)
            .copied()
            .unwrap_or_else(|| self.scopes.root())
    }
}

pub fn build_file(file: FileId, ast: &Ast) -> FileIndex {
    let module_span = ast
        .body()
        .first()
        .map(|&first| {
            let last = *ast.body().last().expect("body is non-empty here");
            crate::span::Span::new(ast.stmt_span(first).start, ast.stmt_span(last).end)
        })
        .unwrap_or_else(|| crate::span::Span::new(0, 0));

    let mut index = FileIndex {
        file,
        scopes: ScopeTree::new(module_span),
        stmt_scope: HashMap::new(),
        function_scope: HashMap::new(),
        module_path: None,
    };

    let root = index.scopes.root();
    walk(&mut index, ast, ast.body(), root);
    index
}

fn walk(index: &mut FileIndex, ast: &Ast, stmts: &[StmtId], scope: ScopeId) {
    for &id in stmts {
        index.stmt_scope.insert(id, scope);

        match ast.stmt(id) {
            Stmt::FunctionDef {
                name,
                is_async,
                params,
                body,
                name_span,
                span,
                ..
            } => {
                index.scopes.bind(
                    scope,
                    name.clone(),
                    Binding::new(
                        BindingKind::Function {
                            is_async: *is_async,
                        },
                        *name_span,
                    ),
                );

                let inner = index.scopes.push(ScopeKind::Function, scope, *span);
                index.function_scope.insert(id, inner);

                for param in params {
                    index.scopes.bind(
                        inner,
                        param.name.clone(),
                        Binding::new(BindingKind::Parameter, param.span),
                    );
                }

                walk(index, ast, body, inner);
            }

            Stmt::ClassDef {
                name,
                body,
                name_span,
                span,
                ..
            } => {
                index.scopes.bind(
                    scope,
                    name.clone(),
                    Binding::new(BindingKind::Class, *name_span),
                );
                let inner = index.scopes.push(ScopeKind::Class, scope, *span);
                walk(index, ast, body, inner);
            }

            Stmt::Assign { targets, .. } => {
                for &target in targets {
                    if let Expr::Name { name, span } = ast.expr(target) {
                        index.scopes.bind(
                            scope,
                            name.clone(),
                            Binding::new(BindingKind::Variable, *span),
                        );
                    }
                }
            }

            Stmt::Import { aliases, .. } => {
                for alias in aliases {
                    // `import a.b` binds `a`, not `a.b`; `import a.b as c`
                    // binds `c` and it refers to a.b.
                    let (bound, path) = match &alias.asname {
                        Some(asname) => (asname.clone(), alias.name.clone()),
                        None => {
                            let head = alias.name.split('.').next().unwrap_or(&alias.name);
                            (head.to_string(), head.to_string())
                        }
                    };
                    index.scopes.bind(
                        scope,
                        bound,
                        Binding::new(BindingKind::Module { path }, alias.span),
                    );
                }
            }

            Stmt::ImportFrom {
                module,
                level,
                aliases,
                ..
            } => {
                // A relative import keeps its leading dots until the project
                // layout is known. Its dotted path therefore never matches an
                // absolute name like `time.sleep`, which is correct: without
                // the package context there is nothing to match against.
                let base = format!(
                    "{}{}",
                    ".".repeat(*level as usize),
                    module.as_deref().unwrap_or("")
                );

                for alias in aliases {
                    // `from x import *` binds nothing this tool can name.
                    if alias.name == "*" {
                        continue;
                    }
                    let bound = alias.asname.clone().unwrap_or_else(|| alias.name.clone());
                    index.scopes.bind(
                        scope,
                        bound,
                        Binding::new(
                            BindingKind::FromImport {
                                module: base.clone(),
                                name: alias.name.clone(),
                            },
                            alias.span,
                        ),
                    );
                }
            }

            // Everything else either binds nothing or is not yet modelled, in
            // which case its contents are invisible and no name is invented.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;
    use crate::ids::Id;

    fn build(source: &str) -> (Ast, FileIndex) {
        let ast = parse(source).expect("fixture should parse");
        let index = build_file(FileId::from_index(0), &ast);
        (ast, index)
    }

    fn root_binding(source: &str, name: &str) -> Option<Binding> {
        let (_, index) = build(source);
        index
            .scopes
            .lookup_local(index.scopes.root(), name)
            .cloned()
    }

    #[test]
    fn a_function_binds_its_name() {
        let binding = root_binding("def greet():\n    pass\n", "greet").unwrap();
        assert_eq!(binding.kind, BindingKind::Function { is_async: false });
    }

    #[test]
    fn an_async_function_records_that_it_is_async() {
        let binding = root_binding("async def fetch():\n    pass\n", "fetch").unwrap();
        assert_eq!(binding.kind, BindingKind::Function { is_async: true });
        assert!(binding.kind.is_async_function());
    }

    #[test]
    fn a_class_binds_its_name() {
        let binding = root_binding("class C:\n    pass\n", "C").unwrap();
        assert_eq!(binding.kind, BindingKind::Class);
    }

    #[test]
    fn an_assignment_binds_a_variable() {
        let binding = root_binding("x = 1\n", "x").unwrap();
        assert_eq!(binding.kind, BindingKind::Variable);
    }

    #[test]
    fn a_decorated_function_still_binds() {
        let binding = root_binding("@cache\ndef f():\n    pass\n", "f").unwrap();
        assert!(binding.kind.is_function());
    }

    #[test]
    fn a_function_name_span_points_at_the_name() {
        let source = "def greet():\n    pass\n";
        let binding = root_binding(source, "greet").unwrap();
        let span = binding.name_span;
        assert_eq!(&source[span.start as usize..span.end as usize], "greet");
    }

    #[test]
    fn parameters_bind_in_the_functions_own_scope() {
        let (ast, index) = build("def f(a, b):\n    pass\n");
        let def = ast.body()[0];
        let inner = index.function_scope[&def];

        assert!(index.scopes.lookup_local(inner, "a").is_some());
        assert!(index.scopes.lookup_local(inner, "b").is_some());
        assert_eq!(
            index.scopes.lookup_local(inner, "a").unwrap().kind,
            BindingKind::Parameter
        );
        assert!(
            index
                .scopes
                .lookup_local(index.scopes.root(), "a")
                .is_none(),
            "parameters must not leak to the module scope"
        );
    }

    #[test]
    fn a_nested_function_lands_in_its_parents_scope() {
        let (ast, index) = build("def outer():\n    def inner():\n        pass\n");
        let outer = ast.body()[0];
        let outer_scope = index.function_scope[&outer];

        assert!(index.scopes.lookup_local(outer_scope, "inner").is_some());
        assert!(
            index
                .scopes
                .lookup_local(index.scopes.root(), "inner")
                .is_none()
        );
    }

    #[test]
    fn a_method_lands_in_its_classs_scope() {
        let (ast, index) = build("class C:\n    def m(self):\n        pass\n");
        let class = ast.body()[0];
        // The class body walked into a class scope, which is the parent of the
        // method's own scope.
        let (_, method_scope) = index
            .function_scope
            .iter()
            .next()
            .expect("the method introduced a scope");
        let class_scope = index.scopes.scope(*method_scope).parent.unwrap();

        assert_eq!(index.scopes.scope(class_scope).kind, ScopeKind::Class);
        assert!(index.scopes.lookup_local(class_scope, "m").is_some());
        assert!(
            index
                .scopes
                .lookup_local(index.scopes.root(), "C")
                .is_some()
        );
        assert!(index.stmt_scope.contains_key(&class));
    }

    #[test]
    fn redefining_a_name_keeps_one_binding_with_the_later_span() {
        let source = "def f():\n    pass\n\n\ndef f():\n    pass\n";
        let (_, index) = build(source);
        let root = index.scopes.root();

        assert_eq!(index.scopes.scope(root).bindings.len(), 1);
        let span = index.scopes.lookup_local(root, "f").unwrap().name_span;
        assert!(
            span.start > 10,
            "should be the second definition, got {span:?}"
        );
    }

    #[test]
    fn import_binds_the_top_package() {
        let binding = root_binding("import os.path\n", "os").unwrap();
        assert_eq!(binding.kind, BindingKind::Module { path: "os".into() });
        assert_eq!(binding.dotted_path().as_deref(), Some("os"));
    }

    #[test]
    fn a_plain_import_binds_the_module() {
        let binding = root_binding("import time\n", "time").unwrap();
        assert_eq!(binding.dotted_path().as_deref(), Some("time"));
    }

    #[test]
    fn an_aliased_import_binds_the_alias_to_the_full_module() {
        let binding = root_binding("import os.path as p\n", "p").unwrap();
        assert_eq!(
            binding.kind,
            BindingKind::Module {
                path: "os.path".into()
            }
        );
        assert_eq!(binding.dotted_path().as_deref(), Some("os.path"));
    }

    #[test]
    fn a_from_import_binds_the_name() {
        let binding = root_binding("from time import sleep\n", "sleep").unwrap();
        assert_eq!(binding.dotted_path().as_deref(), Some("time.sleep"));
    }

    #[test]
    fn an_aliased_from_import_binds_the_alias() {
        let binding = root_binding("from time import sleep as nap\n", "nap").unwrap();
        assert_eq!(binding.dotted_path().as_deref(), Some("time.sleep"));
        assert!(root_binding("from time import sleep as nap\n", "sleep").is_none());
    }

    #[test]
    fn a_relative_import_keeps_its_dots_and_so_matches_nothing_absolute() {
        let binding = root_binding("from .utils import helper\n", "helper").unwrap();
        assert_eq!(binding.dotted_path().as_deref(), Some(".utils.helper"));
    }

    #[test]
    fn a_star_import_binds_nothing() {
        // Guessing what names a star import brought in is exactly the kind of
        // invention that produces false positives.
        let (_, index) = build("from os import *\n");
        assert!(index.scopes.scope(index.scopes.root()).bindings.is_empty());
    }

    #[test]
    fn unmodelled_statements_bind_nothing_rather_than_guessing() {
        let (_, index) = build("while True:\n    x = 1\n");
        // The while loop is Unsupported, so its body is invisible. Inventing a
        // binding for x would claim knowledge the engine does not have.
        assert!(index.scopes.scope(index.scopes.root()).bindings.is_empty());
    }

    #[test]
    fn every_top_level_statement_records_its_scope() {
        let (ast, index) = build("x = 1\ny = 2\ndef f():\n    pass\n");
        for &stmt in ast.body() {
            assert_eq!(index.scope_of(stmt), index.scopes.root());
        }
    }
}
