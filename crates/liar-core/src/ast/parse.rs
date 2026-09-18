//! Turning Python source into the engine's syntax tree.

use crate::ast::Ast;
use crate::ast::convert;
use crate::span::Span;

/// A syntax error. Carries a span so it can be rendered like any finding.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

/// Parses Python source into the engine's syntax tree.
///
/// Syntax not yet modelled becomes `Stmt::Unsupported` or `Expr::Unsupported`
/// rather than an error; only genuinely invalid Python fails.
pub fn parse(source: &str) -> Result<Ast, ParseError> {
    convert::parse_and_convert(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ConstantKind, Expr, Stmt};

    fn parse_ok(src: &str) -> Ast {
        parse(src).unwrap_or_else(|e| panic!("expected {src:?} to parse, got: {}", e.message))
    }

    #[test]
    fn an_empty_module_parses_to_an_empty_body() {
        let ast = parse_ok("");
        assert!(ast.body().is_empty());
    }

    #[test]
    fn a_pass_statement_parses() {
        let ast = parse_ok("pass");
        assert_eq!(ast.body().len(), 1);
        assert!(matches!(ast.stmt(ast.body()[0]), Stmt::Pass { .. }));
    }

    #[test]
    fn a_function_definition_records_its_name() {
        let ast = parse_ok("def greet():\n    pass\n");
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef {
                name,
                is_async,
                body,
                ..
            } => {
                assert_eq!(name, "greet");
                assert!(!is_async);
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn an_async_function_is_marked_async() {
        let ast = parse_ok("async def fetch():\n    pass\n");
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { is_async, .. } => assert!(is_async),
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn a_function_name_span_covers_the_name_alone() {
        let src = "def greet():\n    pass\n";
        let ast = parse_ok(src);
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { name_span, .. } => {
                assert_eq!(
                    &src[name_span.start as usize..name_span.end as usize],
                    "greet"
                );
            }
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_call_becomes_an_expression_statement() {
        // The shape C1 depends on: a call whose value is discarded.
        let ast = parse_ok("save()");
        match ast.stmt(ast.body()[0]) {
            Stmt::Expr { value, .. } => {
                assert!(matches!(ast.expr(*value), Expr::Call { .. }));
            }
            other => panic!("expected an expression statement, got {other:?}"),
        }
    }

    #[test]
    fn an_awaited_call_is_wrapped_in_await() {
        let ast = parse_ok("async def f():\n    await save()\n");
        let Stmt::FunctionDef { body, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected a function definition");
        };
        match ast.stmt(body[0]) {
            Stmt::Expr { value, .. } => {
                assert!(matches!(ast.expr(*value), Expr::Await { .. }));
            }
            other => panic!("expected an expression statement, got {other:?}"),
        }
    }

    #[test]
    fn call_arguments_and_keywords_are_recorded() {
        let ast = parse_ok("f(a, b=1)");
        let Stmt::Expr { value, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected an expression statement");
        };
        match ast.expr(*value) {
            Expr::Call { args, keywords, .. } => {
                assert_eq!(args.len(), 1);
                assert_eq!(keywords.len(), 1);
                assert_eq!(keywords[0].0.as_deref(), Some("b"));
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn literal_kinds_are_distinguished() {
        let cases = [
            ("1", ConstantKind::Int),
            ("1.5", ConstantKind::Float),
            ("'s'", ConstantKind::Str),
            ("b's'", ConstantKind::Bytes),
            ("True", ConstantKind::Bool),
            ("None", ConstantKind::None),
        ];
        for (src, expected) in cases {
            let ast = parse_ok(src);
            let Stmt::Expr { value, .. } = ast.stmt(ast.body()[0]) else {
                panic!("expected an expression statement for {src}");
            };
            match ast.expr(*value) {
                Expr::Constant { kind, .. } => assert_eq!(*kind, expected, "for source {src}"),
                other => panic!("expected a constant for {src}, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_fstring_is_a_string() {
        let ast = parse_ok("f'{x}'");
        let Stmt::Expr { value, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected an expression statement");
        };
        match ast.expr(*value) {
            Expr::Constant { kind, .. } => assert_eq!(*kind, ConstantKind::Str),
            other => panic!("expected a constant, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_list_literal_parses() {
        // The shape C3b depends on: `count = []`.
        let ast = parse_ok("count = []");
        match ast.stmt(ast.body()[0]) {
            Stmt::Assign { value, .. } => {
                assert!(matches!(ast.expr(*value), Expr::List { .. }));
            }
            other => panic!("expected an assignment, got {other:?}"),
        }
    }

    #[test]
    fn an_annotated_assignment_records_its_annotation() {
        let ast = parse_ok("count: int = 0");
        match ast.stmt(ast.body()[0]) {
            Stmt::Assign {
                annotation,
                targets,
                ..
            } => {
                assert!(annotation.is_some());
                assert_eq!(targets.len(), 1);
            }
            other => panic!("expected an assignment, got {other:?}"),
        }
    }

    #[test]
    fn a_return_annotation_is_recorded() {
        let ast = parse_ok("def f() -> str:\n    pass\n");
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { returns, .. } => assert!(returns.is_some()),
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn parameters_and_their_annotations_are_recorded() {
        let ast = parse_ok("def f(a, b: int = 1, *rest, c: str, **kw):\n    pass\n");
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { params, .. } => {
                let names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
                assert_eq!(names, vec!["a", "b", "rest", "c", "kw"]);
                assert!(params[0].annotation.is_none());
                assert!(params[1].annotation.is_some());
                assert!(params[3].annotation.is_some());
            }
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn decorators_are_recorded() {
        let ast = parse_ok("@cache\ndef f():\n    pass\n");
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { decorators, .. } => assert_eq!(decorators.len(), 1),
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn unmodelled_syntax_becomes_unsupported_rather_than_an_error() {
        // `del` is not modelled. It parses, carries a span, and is skipped
        // precisely rather than failing the file.
        let ast = parse_ok("del x\n");
        assert!(matches!(ast.stmt(ast.body()[0]), Stmt::Unsupported { .. }));
    }

    #[test]
    fn compound_statements_expose_their_bodies() {
        // Control flow has to be transparent, or a checker cannot see a call
        // inside an `if` - which is where most calls live.
        for (source, label) in [
            ("if x:\n    f()\n", "if"),
            ("for i in xs:\n    f()\n", "for"),
            ("while x:\n    f()\n", "while"),
            ("with open(p) as h:\n    f()\n", "with"),
            ("try:\n    f()\nexcept E:\n    pass\n", "try"),
        ] {
            let ast = parse_ok(source);
            assert!(
                !matches!(ast.stmt(ast.body()[0]), Stmt::Unsupported { .. }),
                "{label} should be modelled"
            );
        }
    }

    #[test]
    fn elif_desugars_into_a_nested_if() {
        let ast = parse_ok("if a:\n    pass\nelif b:\n    pass\nelse:\n    pass\n");
        let Stmt::If { orelse, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected an if");
        };
        assert_eq!(orelse.len(), 1, "the elif should be one nested statement");

        let Stmt::If { orelse: inner, .. } = ast.stmt(orelse[0]) else {
            panic!("the elif should itself be an if");
        };
        assert_eq!(inner.len(), 1, "the else body should hang off the elif");
    }

    #[test]
    fn a_plain_else_is_not_wrapped_in_an_if() {
        let ast = parse_ok("if a:\n    pass\nelse:\n    f()\n");
        let Stmt::If { orelse, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected an if");
        };
        assert_eq!(orelse.len(), 1);
        assert!(matches!(ast.stmt(orelse[0]), Stmt::Expr { .. }));
    }

    #[test]
    fn modern_syntax_parses_without_error() {
        // The reason this parser was chosen over rustpython-parser, which was
        // last published in 2024. If any of these fail, that decision is wrong
        // and docs/decisions/001-parser.md needs revisiting.
        let sources = [
            "match x:\n    case 1:\n        pass\n",
            "if (n := f()) > 0:\n    pass\n",
            "def f[T](x: T) -> T:\n    return x\n",
            "type Alias = list[int]\n",
            "f'{value!r:>{width}}'\n",
            "async def f():\n    async with a as b:\n        pass\n",
        ];
        for src in sources {
            assert!(parse(src).is_ok(), "expected {src:?} to parse");
        }
    }

    #[test]
    fn a_syntax_error_reports_a_message_and_a_span() {
        let err = parse("def (:").expect_err("expected a syntax error");
        assert!(!err.message.is_empty());
        assert!(err.span.end >= err.span.start);
    }

    #[test]
    fn parsing_is_deterministic() {
        let src = "def f():\n    g()\n";
        let a = parse_ok(src);
        let b = parse_ok(src);
        assert_eq!(a.stmt_count(), b.stmt_count());
        assert_eq!(a.expr_count(), b.expr_count());
    }

    #[test]
    fn spans_point_at_the_right_text() {
        let src = "save_user(u)";
        let ast = parse_ok(src);
        let Stmt::Expr { value, .. } = ast.stmt(ast.body()[0]) else {
            panic!("expected an expression statement");
        };
        let Expr::Call { func, .. } = ast.expr(*value) else {
            panic!("expected a call");
        };
        let span = ast.expr_span(*func);
        assert_eq!(&src[span.start as usize..span.end as usize], "save_user");
    }
}
