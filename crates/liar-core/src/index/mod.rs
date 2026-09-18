//! What does this name refer to?
//!
//! Every checker asks the index rather than pattern-matching on the syntax
//! tree. A checker that matches names textually is a regex with extra steps,
//! and it is how false positives get in.

pub mod binding;
pub mod build;
pub mod imports;
pub mod scope;

pub use binding::{Binding, BindingKind};
pub use build::{FileIndex, build_file};
pub use imports::ModuleLocation;
pub use scope::{Scope, ScopeId, ScopeKind, ScopeTree};

use crate::ast::{Ast, Expr, ExprId};
use crate::ids::FileId;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A file to be indexed.
pub struct IndexInput<'a> {
    pub file: FileId,
    pub path: &'a Path,
    pub ast: &'a Ast,
}

/// What a name turned out to refer to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resolved {
    /// The file the definition lives in, which is not necessarily the file the
    /// name was written in.
    pub file: FileId,
    pub scope: ScopeId,
    pub binding: Binding,
}

/// How many imports deep resolution will follow before giving up.
///
/// Import cycles are legal Python and do occur. Giving up quietly is the right
/// answer: a name this tool cannot pin down is a name it says nothing about.
const MAX_IMPORT_DEPTH: usize = 8;

#[derive(Debug, Default)]
pub struct Index {
    files: HashMap<FileId, FileIndex>,
    locations: HashMap<FileId, ModuleLocation>,
    by_module: HashMap<String, FileId>,
}

impl Index {
    pub fn build(inputs: &[IndexInput<'_>]) -> Self {
        let all_paths: HashSet<PathBuf> = inputs
            .iter()
            .map(|input| input.path.to_path_buf())
            .collect();

        let mut index = Index::default();

        for input in inputs {
            let file_index = build_file(input.file, input.ast);
            let location = imports::locate(input.path, &all_paths);

            index.by_module.insert(location.module.clone(), input.file);
            index.locations.insert(input.file, location);
            index.files.insert(input.file, file_index);
        }

        // Record each file's own module path on its index, so a checker holding
        // only a FileIndex can still say where it is.
        for (file, location) in &index.locations {
            if let Some(file_index) = index.files.get_mut(file) {
                file_index.module_path = Some(location.module.clone());
            }
        }

        index
    }

    pub fn file(&self, file: FileId) -> Option<&FileIndex> {
        self.files.get(&file)
    }

    pub fn location(&self, file: FileId) -> Option<&ModuleLocation> {
        self.locations.get(&file)
    }

    pub fn files(&self) -> impl Iterator<Item = (FileId, &FileIndex)> {
        self.files.iter().map(|(&id, index)| (id, index))
    }

    /// Resolves `name` as seen from `scope` in `file`.
    ///
    /// Follows a from-import into another analysed file, so an async function
    /// imported from elsewhere resolves to its real definition. Returns `None`
    /// for anything it cannot pin down — a third-party import, a star import, a
    /// name bound by syntax the engine does not model. That is not a failure;
    /// it is the tool declining to guess.
    pub fn resolve(&self, file: FileId, scope: ScopeId, name: &str) -> Option<Resolved> {
        self.resolve_inner(file, scope, name, 0)
    }

    fn resolve_inner(
        &self,
        file: FileId,
        scope: ScopeId,
        name: &str,
        depth: usize,
    ) -> Option<Resolved> {
        if depth > MAX_IMPORT_DEPTH {
            return None;
        }

        let file_index = self.files.get(&file)?;

        for (position, &current) in file_index.scopes.ancestry(scope).iter().enumerate() {
            // A class body's names are not visible to functions nested inside
            // it. `class C: x = 1` does not make `x` a bare name in a method.
            // Treating them as visible would invent bindings that do not
            // exist, and every checker downstream would inherit the error.
            let is_enclosing_class =
                position > 0 && file_index.scopes.scope(current).kind == ScopeKind::Class;
            if is_enclosing_class {
                continue;
            }

            let Some(binding) = file_index.scopes.lookup_local(current, name) else {
                continue;
            };

            if let BindingKind::FromImport {
                module,
                name: imported,
            } = &binding.kind
                && let Some(followed) = self.follow_import(file, module, imported, depth)
            {
                return Some(followed);
            }

            return Some(Resolved {
                file,
                scope: current,
                binding: binding.clone(),
            });
        }

        None
    }

    /// Follows `from <module> import <name>` into the file it names, if that
    /// file is one of the ones being analysed.
    fn follow_import(
        &self,
        from: FileId,
        module: &str,
        name: &str,
        depth: usize,
    ) -> Option<Resolved> {
        let package = self
            .locations
            .get(&from)
            .map(|l| l.package.as_str())
            .unwrap_or("");
        let absolute = imports::resolve_module_name(module, package)?;
        let target = *self.by_module.get(&absolute)?;
        let target_index = self.files.get(&target)?;
        self.resolve_inner(target, target_index.scopes.root(), name, depth + 1)
    }

    /// The dotted path an expression names, if it names one.
    ///
    /// `time.sleep` is `time.sleep` whether `time` was imported plainly, under
    /// an alias, or the function pulled in directly with a from-import. This is
    /// what lets C2 keep its table of blocking functions in terms of real
    /// module paths rather than whatever spelling a file happened to use.
    pub fn dotted_path(
        &self,
        file: FileId,
        scope: ScopeId,
        ast: &Ast,
        expr: ExprId,
    ) -> Option<String> {
        match ast.expr(expr) {
            Expr::Name { name, .. } => self.resolve(file, scope, name)?.binding.dotted_path(),
            Expr::Attribute { value, attr, .. } => {
                let base = self.dotted_path(file, scope, ast, *value)?;
                Some(format!("{base}.{attr}"))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;
    use crate::ids::Id;

    struct Project {
        asts: Vec<Ast>,
        paths: Vec<PathBuf>,
    }

    impl Project {
        fn new(files: &[(&str, &str)]) -> Self {
            Self {
                asts: files
                    .iter()
                    .map(|(_, src)| parse(src).expect("fixture should parse"))
                    .collect(),
                paths: files.iter().map(|(path, _)| PathBuf::from(path)).collect(),
            }
        }

        fn index(&self) -> Index {
            let inputs: Vec<IndexInput<'_>> = self
                .asts
                .iter()
                .zip(&self.paths)
                .enumerate()
                .map(|(i, (ast, path))| IndexInput {
                    file: FileId::from_index(i as u32),
                    path,
                    ast,
                })
                .collect();
            Index::build(&inputs)
        }
    }

    fn single(source: &str) -> (Project, Index) {
        let project = Project::new(&[("m.py", source)]);
        let index = project.index();
        (project, index)
    }

    fn file(n: u32) -> FileId {
        FileId::from_index(n)
    }

    #[test]
    fn a_module_level_name_resolves() {
        let (_, index) = single("def f():\n    pass\n");
        let root = index.file(file(0)).unwrap().scopes.root();
        let found = index.resolve(file(0), root, "f").unwrap();
        assert!(found.binding.kind.is_function());
    }

    #[test]
    fn an_unknown_name_resolves_to_nothing() {
        let (_, index) = single("x = 1\n");
        let root = index.file(file(0)).unwrap().scopes.root();
        assert!(index.resolve(file(0), root, "nope").is_none());
    }

    #[test]
    fn a_local_beats_an_enclosing_name() {
        let (project, index) = single("f = 1\ndef outer(f):\n    pass\n");
        let file_index = index.file(file(0)).unwrap();
        let def = project.asts[0].body()[1];
        let inner = file_index.function_scope[&def];

        let found = index.resolve(file(0), inner, "f").unwrap();
        assert_eq!(found.binding.kind, BindingKind::Parameter);
    }

    #[test]
    fn a_function_sees_module_level_names() {
        let (project, index) = single("async def helper():\n    pass\ndef f():\n    pass\n");
        let file_index = index.file(file(0)).unwrap();
        let def = project.asts[0].body()[1];
        let inner = file_index.function_scope[&def];

        let found = index.resolve(file(0), inner, "helper").unwrap();
        assert!(found.binding.kind.is_async_function());
    }

    #[test]
    fn a_method_does_not_see_its_classs_scope() {
        // The rule that matters most. `class C: x = 1` does not make `x` a bare
        // name inside a method; Python raises NameError. Resolving it would
        // invent a binding and every checker would inherit the mistake.
        let source = "class C:\n    helper = 1\n    def m(self):\n        pass\n";
        let (_, index) = single(source);
        let file_index = index.file(file(0)).unwrap();
        let (_, &method_scope) = file_index.function_scope.iter().next().unwrap();

        assert!(
            index.resolve(file(0), method_scope, "helper").is_none(),
            "a class attribute must not resolve as a bare name in a method"
        );
        assert!(
            index.resolve(file(0), method_scope, "self").is_some(),
            "but the method's own parameters must"
        );
    }

    #[test]
    fn a_class_body_does_see_its_own_names() {
        // The exclusion applies to scopes *enclosing* the one being resolved
        // from, not to the class body itself.
        let source = "class C:\n    helper = 1\n";
        let (_, index) = single(source);
        let file_index = index.file(file(0)).unwrap();
        let class_scope = file_index
            .scopes
            .iter()
            .find(|(_, scope)| scope.kind == ScopeKind::Class)
            .map(|(id, _)| id)
            .unwrap();

        assert!(index.resolve(file(0), class_scope, "helper").is_some());
    }

    #[test]
    fn a_from_import_resolves_into_the_other_file() {
        let project = Project::new(&[
            ("app.py", "from helpers import save\nsave()\n"),
            ("helpers.py", "async def save():\n    pass\n"),
        ]);
        let index = project.index();
        let root = index.file(file(0)).unwrap().scopes.root();

        let found = index.resolve(file(0), root, "save").unwrap();
        assert!(
            found.binding.kind.is_async_function(),
            "should follow into helpers.py"
        );
        assert_eq!(found.file, file(1));
    }

    #[test]
    fn a_from_import_of_an_unanalysed_module_stops_at_the_import() {
        let (_, index) = single("from requests import get\n");
        let root = index.file(file(0)).unwrap().scopes.root();

        let found = index.resolve(file(0), root, "get").unwrap();
        assert_eq!(found.binding.dotted_path().as_deref(), Some("requests.get"));
        assert!(
            !found.binding.kind.is_function(),
            "nothing is known about its body"
        );
    }

    #[test]
    fn a_relative_import_resolves_within_a_package() {
        let project = Project::new(&[
            ("pkg/__init__.py", ""),
            ("pkg/app.py", "from .helpers import save\n"),
            ("pkg/helpers.py", "async def save():\n    pass\n"),
        ]);
        let index = project.index();
        let root = index.file(file(1)).unwrap().scopes.root();

        let found = index.resolve(file(1), root, "save").unwrap();
        assert!(found.binding.kind.is_async_function());
        assert_eq!(found.file, file(2));
    }

    #[test]
    fn a_star_import_resolves_to_nothing() {
        let project = Project::new(&[
            ("app.py", "from helpers import *\n"),
            ("helpers.py", "async def save():\n    pass\n"),
        ]);
        let index = project.index();
        let root = index.file(file(0)).unwrap().scopes.root();
        assert!(index.resolve(file(0), root, "save").is_none());
    }

    #[test]
    fn an_import_cycle_terminates() {
        let project = Project::new(&[
            ("a.py", "from b import thing\n"),
            ("b.py", "from a import thing\n"),
        ]);
        let index = project.index();
        let root = index.file(file(0)).unwrap().scopes.root();
        // The only requirement is that this returns rather than recursing
        // forever.
        let _ = index.resolve(file(0), root, "thing");
    }

    #[test]
    fn dotted_paths_survive_every_import_spelling() {
        for (source, expression_index) in [
            ("import time\ntime.sleep(1)\n", 1usize),
            ("import time as t\nt.sleep(1)\n", 1),
            ("from time import sleep\nsleep(1)\n", 1),
        ] {
            let project = Project::new(&[("m.py", source)]);
            let index = project.index();
            let ast = &project.asts[0];
            let root = index.file(file(0)).unwrap().scopes.root();

            let stmt = ast.body()[expression_index];
            let crate::ast::Stmt::Expr { value, .. } = ast.stmt(stmt) else {
                panic!("expected an expression statement in {source:?}");
            };
            let crate::ast::Expr::Call { func, .. } = ast.expr(*value) else {
                panic!("expected a call in {source:?}");
            };

            assert_eq!(
                index.dotted_path(file(0), root, ast, *func).as_deref(),
                Some("time.sleep"),
                "for {source:?}"
            );
        }
    }

    #[test]
    fn a_dotted_path_through_an_unresolved_name_is_nothing() {
        let project = Project::new(&[("m.py", "mystery.sleep(1)\n")]);
        let index = project.index();
        let ast = &project.asts[0];
        let root = index.file(file(0)).unwrap().scopes.root();

        let crate::ast::Stmt::Expr { value, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected an expression statement");
        };
        let crate::ast::Expr::Call { func, .. } = ast.expr(*value) else {
            panic!("expected a call");
        };
        assert!(index.dotted_path(file(0), root, ast, *func).is_none());
    }

    #[test]
    fn a_local_shadowing_an_import_wins() {
        // `def sleep()` after `from time import sleep` means the later binding
        // is what the name refers to, and it is not time.sleep.
        let project = Project::new(&[("m.py", "from time import sleep\ndef sleep():\n    pass\n")]);
        let index = project.index();
        let root = index.file(file(0)).unwrap().scopes.root();

        let found = index.resolve(file(0), root, "sleep").unwrap();
        assert!(found.binding.kind.is_function());
        assert_eq!(found.binding.dotted_path(), None);
    }

    #[test]
    fn module_paths_are_recorded_on_each_file() {
        let project = Project::new(&[("pkg/__init__.py", ""), ("pkg/mod.py", "")]);
        let index = project.index();
        assert_eq!(index.location(file(0)).unwrap().module, "pkg");
        assert_eq!(index.location(file(1)).unwrap().module, "pkg.mod");
        assert_eq!(
            index.file(file(1)).unwrap().module_path.as_deref(),
            Some("pkg.mod")
        );
    }
}
