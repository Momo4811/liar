//! Reading a type out of an annotation.

use crate::ast::{Ast, ConstantKind, Expr, ExprId};
use crate::ids::FileId;
use crate::index::{BindingKind, Index, ScopeId};
use crate::infer::Ty;

/// The type an annotation names.
///
/// A name is looked up in the index first. Only a name that resolves to
/// *nothing* is treated as a builtin — which is exactly right, because a
/// builtin is by definition not something this project defines. A file that
/// shadows `int` with a class of its own therefore does not get misread.
pub fn from_annotation(index: &Index, file: FileId, scope: ScopeId, ast: &Ast, expr: ExprId) -> Ty {
    match ast.expr(expr) {
        Expr::Name { name, .. } => match index.resolve(file, scope, name) {
            // `x: MyClass` is an instance of that class.
            Some(resolved) if resolved.binding.kind == BindingKind::Class => {
                Ty::Instance((resolved.file, resolved.binding.name_span))
            }
            // Resolved to something that is not a class — a function, a
            // variable, an import. Nothing can be concluded.
            Some(_) => Ty::Unknown,
            None => builtin(name),
        },

        // `-> None` is a constant, not a name.
        Expr::Constant {
            kind: ConstantKind::None,
            ..
        } => Ty::NoneType,

        // `list[int]` is a list. The element type is not tracked, and
        // `Optional[str]` deliberately stays Unknown: None is a legitimate
        // value of that annotation, so unwrapping it to `str` would
        // manufacture findings.
        Expr::Subscript { value, .. } => from_annotation(index, file, scope, ast, *value),

        _ => Ty::Unknown,
    }
}

fn builtin(name: &str) -> Ty {
    match name {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "complex" => Ty::Complex,
        "str" => Ty::Str,
        "bytes" | "bytearray" => Ty::Bytes,
        "bool" => Ty::Bool,
        "None" | "NoneType" => Ty::NoneType,
        // The typing spellings are included because plenty of code still uses
        // them, and they mean exactly the builtin.
        "list" | "List" => Ty::List,
        "dict" | "Dict" | "Mapping" | "DefaultDict" => Ty::Dict,
        "set" | "Set" | "frozenset" | "FrozenSet" => Ty::Set,
        "tuple" | "Tuple" => Ty::Tuple,
        _ => Ty::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Stmt, parse};
    use crate::ids::Id;
    use crate::index::IndexInput;
    use std::path::PathBuf;

    /// Parses `source`, finds the first annotated function, and returns the
    /// type of its return annotation.
    fn return_ty(source: &str) -> Ty {
        let ast = parse(source).expect("fixture should parse");
        let path = PathBuf::from("m.py");
        let file = FileId::from_index(0);
        let index = Index::build(&[IndexInput {
            file,
            path: &path,
            ast: &ast,
        }]);
        let scope = index.file(file).unwrap().scopes.root();

        for &stmt in ast.body() {
            if let Stmt::FunctionDef {
                returns: Some(returns),
                ..
            } = ast.stmt(stmt)
            {
                return from_annotation(&index, file, scope, &ast, *returns);
            }
        }
        panic!("no annotated function in {source:?}");
    }

    #[test]
    fn builtin_names_map_to_their_types() {
        for (annotation, expected) in [
            ("int", Ty::Int),
            ("float", Ty::Float),
            ("str", Ty::Str),
            ("bytes", Ty::Bytes),
            ("bool", Ty::Bool),
            ("list", Ty::List),
            ("dict", Ty::Dict),
            ("set", Ty::Set),
            ("tuple", Ty::Tuple),
        ] {
            let source = format!("def f() -> {annotation}:\n    pass\n");
            assert_eq!(return_ty(&source), expected, "for {annotation}");
        }
    }

    #[test]
    fn a_none_return_annotation_is_the_none_type() {
        assert_eq!(return_ty("def f() -> None:\n    pass\n"), Ty::NoneType);
    }

    #[test]
    fn a_subscript_reads_its_base() {
        assert_eq!(return_ty("def f() -> list[int]:\n    pass\n"), Ty::List);
        assert_eq!(
            return_ty("def f() -> dict[str, int]:\n    pass\n"),
            Ty::Dict
        );
    }

    #[test]
    fn the_typing_spellings_work_too() {
        assert_eq!(return_ty("def f() -> List[int]:\n    pass\n"), Ty::List);
        assert_eq!(
            return_ty("def f() -> Dict[str, int]:\n    pass\n"),
            Ty::Dict
        );
    }

    #[test]
    fn optional_stays_unknown() {
        // None is a legitimate value of Optional[str], so unwrapping it to str
        // would manufacture findings about names holding None.
        assert_eq!(
            return_ty("def f() -> Optional[str]:\n    pass\n"),
            Ty::Unknown
        );
    }

    #[test]
    fn an_unrecognised_name_is_unknown() {
        assert_eq!(return_ty("def f() -> Whatever:\n    pass\n"), Ty::Unknown);
    }

    #[test]
    fn a_string_annotation_is_unknown() {
        // Forward references are a string literal; parsing their contents is
        // not something this does.
        assert_eq!(return_ty("def f() -> 'MyClass':\n    pass\n"), Ty::Unknown);
    }

    #[test]
    fn a_project_class_becomes_an_instance_of_itself() {
        let ty = return_ty("class Report:\n    pass\n\n\ndef f() -> Report:\n    pass\n");
        assert!(matches!(ty, Ty::Instance(_)), "got {ty:?}");
    }

    #[test]
    fn a_class_shadowing_a_builtin_is_not_the_builtin() {
        // The reason names are resolved before the builtin table is consulted.
        // A project that defines `class int` has not written a Python integer.
        let ty = return_ty("class int:\n    pass\n\n\ndef f() -> int:\n    pass\n");
        assert!(
            matches!(ty, Ty::Instance(_)),
            "got {ty:?}, expected an instance"
        );
        assert_ne!(ty, Ty::Int);
    }

    #[test]
    fn an_annotation_naming_a_function_is_unknown() {
        let ty = return_ty("def helper():\n    pass\n\n\ndef f() -> helper:\n    pass\n");
        assert_eq!(ty, Ty::Unknown);
    }
}
