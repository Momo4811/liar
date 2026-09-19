//! What type does each name hold, and what does each function return?

use crate::ast::{Ast, ConstantKind, Expr, ExprId, Stmt};
use crate::ids::FileId;
use crate::index::{BindingKind, Index, ScopeId};
use crate::infer::Ty;
use crate::infer::annotation::from_annotation;
use crate::span::Span;
use std::collections::{BTreeMap, HashMap};

/// A function or class definition: its file, and the span of its name.
pub type DefKey = (FileId, Span);

/// How many times inference sweeps the project before giving up.
///
/// The lattice has height three - Never, then a concrete type, then Unknown -
/// and every entry only ever moves up it, so this converges in a handful of
/// passes. The cap exists so that a bug cannot turn into a hang.
const MAX_PASSES: usize = 8;

/// Types for every binding and every function in a project.
#[derive(Debug, Default)]
pub struct Types {
    /// Every binding site of a name in a scope, with the type bound there.
    ///
    /// Sites rather than a single joined type, because the two are needed for
    /// different things: C3a asks what `count` *is* and wants the join, while
    /// C3f asks how many different things `data` has meant and would be left
    /// with nothing if the sites had been collapsed first.
    bindings: HashMap<(FileId, ScopeId, String), Vec<(Span, Ty)>>,
    returns: HashMap<DefKey, Ty>,
}

impl Types {
    /// Every site at which `name` is bound in `scope`, in source order.
    pub fn sites(&self, file: FileId, scope: ScopeId, name: &str) -> &[(Span, Ty)] {
        self.bindings
            .get(&(file, scope, name.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// The type of `name` in `scope`: the join over all its binding sites.
    ///
    /// A name bound to an `int` here and a `list` there is `Unknown`, and
    /// nothing is said about it.
    pub fn binding_ty(&self, file: FileId, scope: ScopeId, name: &str) -> Ty {
        self.sites(file, scope, name)
            .iter()
            .fold(Ty::Never, |acc, (_, ty)| acc.join(ty))
    }

    /// What a function returns.
    pub fn return_ty(&self, key: DefKey) -> Ty {
        self.returns.get(&key).cloned().unwrap_or(Ty::Unknown)
    }

    pub fn infer(index: &Index, asts: &BTreeMap<FileId, Ast>) -> Types {
        let mut types = Types::default();

        // Annotated returns are facts and never change. Everything else starts
        // at the bottom of the lattice and is raised by the sweeps below.
        for (&file, ast) in asts {
            let Some(file_index) = index.file(file) else {
                continue;
            };
            for (&stmt, &scope) in &file_index.stmt_scope {
                let Stmt::FunctionDef {
                    returns, name_span, ..
                } = ast.stmt(stmt)
                else {
                    continue;
                };
                let ty = match returns {
                    Some(annotation) => from_annotation(index, file, scope, ast, *annotation),
                    None => Ty::Never,
                };
                types.returns.insert((file, *name_span), ty);
            }
        }

        for _ in 0..MAX_PASSES {
            let before = (types.bindings.clone(), types.returns.clone());

            types.sweep_bindings(index, asts);
            types.sweep_returns(index, asts);

            if before == (types.bindings.clone(), types.returns.clone()) {
                break;
            }
        }

        types
    }

    fn sweep_bindings(&mut self, index: &Index, asts: &BTreeMap<FileId, Ast>) {
        let mut found: HashMap<(FileId, ScopeId, String), Vec<(Span, Ty)>> = HashMap::new();

        for (&file, ast) in asts {
            let Some(file_index) = index.file(file) else {
                continue;
            };

            let mut stmts: Vec<_> = file_index.stmt_scope.iter().collect();
            // Source order, so binding sites are reported in the order a
            // reader would meet them.
            stmts.sort_by_key(|(stmt, _)| ast.stmt_span(**stmt).start);

            for (&stmt, &scope) in stmts {
                match ast.stmt(stmt) {
                    Stmt::Assign {
                        targets,
                        value,
                        annotation,
                        ..
                    } => {
                        // An annotation is a statement of intent and wins over
                        // whatever was inferred from the value.
                        let ty = match annotation {
                            Some(annotation) => {
                                from_annotation(index, file, scope, ast, *annotation)
                            }
                            None => self.expr_ty(index, file, scope, ast, *value, 0),
                        };

                        for &target in targets {
                            if let Expr::Name { name, span } = ast.expr(target) {
                                found
                                    .entry((file, scope, name.clone()))
                                    .or_default()
                                    .push((*span, ty.clone()));
                            }
                        }
                    }

                    Stmt::FunctionDef { params, .. } => {
                        let Some(&body) = file_index.function_scope.get(&stmt) else {
                            continue;
                        };
                        for param in params {
                            let ty = match param.annotation {
                                Some(annotation) => {
                                    from_annotation(index, file, body, ast, annotation)
                                }
                                None => Ty::Unknown,
                            };
                            found
                                .entry((file, body, param.name.clone()))
                                .or_default()
                                .push((param.name_span, ty));
                        }
                    }

                    _ => {}
                }
            }
        }

        self.bindings = found;
    }

    fn sweep_returns(&mut self, index: &Index, asts: &BTreeMap<FileId, Ast>) {
        let mut found: HashMap<DefKey, Ty> = HashMap::new();

        for (&file, ast) in asts {
            let Some(file_index) = index.file(file) else {
                continue;
            };

            for &stmt in file_index.stmt_scope.keys() {
                let Stmt::FunctionDef {
                    returns, name_span, ..
                } = ast.stmt(stmt)
                else {
                    continue;
                };

                // An annotation is authoritative; do not second-guess it.
                if returns.is_some() {
                    found.insert((file, *name_span), self.return_ty((file, *name_span)));
                    continue;
                }

                let Some(&body) = file_index.function_scope.get(&stmt) else {
                    continue;
                };

                let mut ty = Ty::Never;
                let mut saw_return = false;

                for returning in file_index.stmts_in(body) {
                    let Stmt::Return { value, .. } = ast.stmt(returning) else {
                        continue;
                    };
                    saw_return = true;
                    ty = match value {
                        // A bare `return` yields None.
                        None => ty.join(&Ty::NoneType),
                        Some(value) => ty.join(&self.expr_ty(index, file, body, ast, *value, 0)),
                    };
                }

                // A function that never returns anything returns None - which
                // is what makes "the docstring promises a value" checkable.
                if !saw_return {
                    ty = Ty::NoneType;
                }

                found.insert((file, *name_span), ty);
            }
        }

        self.returns = found;
    }

    /// The type of an expression.
    ///
    /// `depth` guards against self-referential bindings such as `x = x`, which
    /// are legal to write and would otherwise recurse forever.
    fn expr_ty(
        &self,
        index: &Index,
        file: FileId,
        scope: ScopeId,
        ast: &Ast,
        expr: ExprId,
        depth: usize,
    ) -> Ty {
        if depth > 4 {
            return Ty::Unknown;
        }

        match ast.expr(expr) {
            Expr::Constant { kind, .. } => match kind {
                ConstantKind::Int => Ty::Int,
                ConstantKind::Float => Ty::Float,
                ConstantKind::Complex => Ty::Complex,
                ConstantKind::Str => Ty::Str,
                ConstantKind::Bytes => Ty::Bytes,
                ConstantKind::Bool => Ty::Bool,
                ConstantKind::None => Ty::NoneType,
                ConstantKind::Ellipsis => Ty::Ellipsis,
            },

            Expr::List { .. } => Ty::List,
            Expr::Tuple { .. } => Ty::Tuple,
            Expr::Dict { .. } => Ty::Dict,

            Expr::Name { name, .. } => {
                let Some(resolved) = index.resolve(file, scope, name) else {
                    return Ty::Unknown;
                };
                match resolved.binding.kind {
                    BindingKind::Variable | BindingKind::Parameter => {
                        self.binding_ty(resolved.file, resolved.scope, name)
                    }
                    // A bare class or function name is the callable itself,
                    // not an instance or a result.
                    _ => Ty::Unknown,
                }
            }

            // `await f()` yields whatever f returns.
            Expr::Await { value, .. } => self.expr_ty(index, file, scope, ast, *value, depth + 1),

            Expr::Call { func, .. } => self.call_ty(index, file, scope, ast, *func),

            // Indexing a collection, and attribute access, are not modelled.
            Expr::Subscript { .. } | Expr::Attribute { .. } | Expr::Unsupported { .. } => {
                Ty::Unknown
            }
        }
    }

    fn call_ty(&self, index: &Index, file: FileId, scope: ScopeId, ast: &Ast, func: ExprId) -> Ty {
        let Expr::Name { name, .. } = ast.expr(func) else {
            return Ty::Unknown;
        };

        match index.resolve(file, scope, name) {
            Some(resolved) => match resolved.binding.kind {
                BindingKind::Class => Ty::Instance((resolved.file, resolved.binding.name_span)),
                // A decorator can change what a call returns, and the engine
                // does not model decorators.
                BindingKind::Function {
                    decorated: true, ..
                } => Ty::Unknown,
                BindingKind::Function { .. } => {
                    self.return_ty((resolved.file, resolved.binding.name_span))
                }
                _ => Ty::Unknown,
            },
            // Unresolved means not defined in this project, so it may be a
            // builtin.
            None => builtin_call(name),
        }
    }
}

/// The result type of a builtin that is worth knowing about.
///
/// Only calls whose result type is unambiguous. `sum` is absent because it
/// returns an int or a float depending on its argument, and guessing would be
/// the kind of invention this tool is named after.
fn builtin_call(name: &str) -> Ty {
    match name {
        "int" | "len" | "ord" | "hash" | "id" => Ty::Int,
        "float" => Ty::Float,
        "complex" => Ty::Complex,
        "str" | "repr" | "chr" | "format" | "input" => Ty::Str,
        "bytes" | "bytearray" => Ty::Bytes,
        "bool" | "isinstance" | "issubclass" | "callable" | "any" | "all" | "hasattr" => Ty::Bool,
        "list" | "sorted" => Ty::List,
        "dict" => Ty::Dict,
        "set" | "frozenset" => Ty::Set,
        "tuple" => Ty::Tuple,
        "print" => Ty::NoneType,
        _ => Ty::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;
    use crate::ids::Id;
    use crate::index::IndexInput;
    use std::path::PathBuf;

    struct Project {
        ast: Ast,
        path: PathBuf,
    }

    fn project(source: &str) -> (Project, Index, Types) {
        let project = Project {
            ast: parse(source).expect("fixture should parse"),
            path: PathBuf::from("m.py"),
        };
        let file = FileId::from_index(0);
        let index = Index::build(&[IndexInput {
            file,
            path: &project.path,
            ast: &project.ast,
        }]);
        let mut asts = BTreeMap::new();
        asts.insert(file, parse(source).expect("fixture should parse"));
        let types = Types::infer(&index, &asts);
        (project, index, types)
    }

    fn file() -> FileId {
        FileId::from_index(0)
    }

    /// The type of a module-level name.
    fn global(source: &str, name: &str) -> Ty {
        let (_, index, types) = project(source);
        let root = index.file(file()).unwrap().scopes.root();
        types.binding_ty(file(), root, name)
    }

    /// The return type of the named function.
    fn returns(source: &str, name: &str) -> Ty {
        let (_, index, types) = project(source);
        let root = index.file(file()).unwrap().scopes.root();
        let resolved = index.resolve(file(), root, name).expect("the function");
        types.return_ty((resolved.file, resolved.binding.name_span))
    }

    #[test]
    fn literals_infer() {
        for (source, expected) in [
            ("x = 1", Ty::Int),
            ("x = 1.5", Ty::Float),
            ("x = 'a'", Ty::Str),
            ("x = b'a'", Ty::Bytes),
            ("x = True", Ty::Bool),
            ("x = None", Ty::NoneType),
            ("x = []", Ty::List),
            ("x = {}", Ty::Dict),
        ] {
            assert_eq!(global(source, "x"), expected, "for {source}");
        }
    }

    #[test]
    fn an_annotation_beats_the_value() {
        // The annotation is a statement of intent, and disagreeing with it is
        // not this checker's business.
        assert_eq!(global("x: float = 1", "x"), Ty::Float);
    }

    #[test]
    fn a_name_takes_the_type_of_what_it_was_assigned() {
        assert_eq!(global("a = 1\nb = a", "b"), Ty::Int);
    }

    #[test]
    fn two_different_assignments_collapse_to_unknown() {
        assert_eq!(global("x = 1\nx = 'a'", "x"), Ty::Unknown);
    }

    #[test]
    fn two_matching_assignments_keep_the_type() {
        assert_eq!(global("x = 1\nx = 2", "x"), Ty::Int);
    }

    #[test]
    fn both_binding_sites_are_recorded() {
        let (_, index, types) = project("x = 1\nx = 'a'");
        let root = index.file(file()).unwrap().scopes.root();
        let sites = types.sites(file(), root, "x");

        assert_eq!(sites.len(), 2, "C3f needs the sites, not just the join");
        assert_eq!(sites[0].1, Ty::Int);
        assert_eq!(sites[1].1, Ty::Str);
    }

    #[test]
    fn a_self_referential_binding_terminates() {
        // Legal to write, and would recurse forever without the depth guard.
        // It infers Never rather than Unknown - there is genuinely no reachable
        // value - and what matters either way is that no check can act on it.
        let ty = global("x = x", "x");
        assert!(!ty.is_known(), "got {ty:?}");
    }

    #[test]
    fn builtin_calls_infer() {
        assert_eq!(global("n = len(xs)", "n"), Ty::Int);
        assert_eq!(global("s = str(x)", "s"), Ty::Str);
        assert_eq!(global("f = isinstance(x, int)", "f"), Ty::Bool);
        assert_eq!(global("xs = sorted(ys)", "xs"), Ty::List);
    }

    #[test]
    fn an_ambiguous_builtin_is_not_guessed() {
        // sum returns an int or a float depending on its argument.
        assert!(global("t = sum(xs)", "t").is_unknown());
    }

    #[test]
    fn a_constructor_call_yields_an_instance() {
        let ty = global("class Report:\n    pass\n\n\nr = Report()\n", "r");
        assert!(matches!(ty, Ty::Instance(_)), "got {ty:?}");
    }

    #[test]
    fn an_annotated_return_is_taken_as_written() {
        assert_eq!(returns("def f() -> str:\n    return 1\n", "f"), Ty::Str);
    }

    #[test]
    fn an_unannotated_return_is_inferred() {
        assert_eq!(returns("def f():\n    return 'a'\n", "f"), Ty::Str);
    }

    #[test]
    fn two_different_returns_collapse_to_unknown() {
        let source = "def f(flag):\n    if flag:\n        return 1\n    return 'a'\n";
        assert_eq!(returns(source, "f"), Ty::Unknown);
    }

    #[test]
    fn a_function_with_no_return_returns_none() {
        assert_eq!(returns("def f():\n    pass\n", "f"), Ty::NoneType);
    }

    #[test]
    fn a_bare_return_is_none() {
        assert_eq!(returns("def f():\n    return\n", "f"), Ty::NoneType);
    }

    #[test]
    fn a_nested_functions_return_does_not_leak_outward() {
        let source = "def outer():\n    def inner():\n        return 'a'\n";
        assert_eq!(returns(source, "outer"), Ty::NoneType);
    }

    #[test]
    fn a_call_takes_the_callees_return_type() {
        assert_eq!(
            global("def f() -> int:\n    return 1\n\n\nx = f()\n", "x"),
            Ty::Int
        );
    }

    #[test]
    fn awaiting_yields_what_the_coroutine_returns() {
        let source = "async def f() -> str:\n    return 'a'\n\n\nasync def g():\n    x = await f()\n    return x\n";
        assert_eq!(returns(source, "g"), Ty::Str);
    }

    #[test]
    fn a_decorated_call_is_unknown() {
        // A decorator can change what calling a function returns.
        let source =
            "def wrap(f):\n    return f\n\n\n@wrap\ndef g() -> int:\n    return 1\n\n\nx = g()\n";
        assert!(global(source, "x").is_unknown());
    }

    #[test]
    fn a_parameter_takes_its_annotation() {
        let (_, index, types) = project("def f(n: int):\n    pass\n");
        let file_index = index.file(file()).unwrap();
        let (_, &body) = file_index.function_scope.iter().next().unwrap();
        assert_eq!(types.binding_ty(file(), body, "n"), Ty::Int);
    }

    #[test]
    fn an_unannotated_parameter_is_unknown() {
        let (_, index, types) = project("def f(n):\n    pass\n");
        let file_index = index.file(file()).unwrap();
        let (_, &body) = file_index.function_scope.iter().next().unwrap();
        assert!(types.binding_ty(file(), body, "n").is_unknown());
    }

    #[test]
    fn mutual_recursion_terminates() {
        let source = "def a():\n    return b()\n\n\ndef b():\n    return a()\n";
        let _ = returns(source, "a");
    }

    #[test]
    fn inference_is_deterministic() {
        let source = "x = 1\ny = 'a'\n\n\ndef f():\n    return x\n";
        let first = global(source, "x");
        let second = global(source, "x");
        assert_eq!(first, second);
        assert_eq!(returns(source, "f"), Ty::Int);
    }
}
