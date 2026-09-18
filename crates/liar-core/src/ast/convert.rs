//! The boundary between the third-party parser and the engine.
//!
//! This is the only file in the project permitted to name a `ruff_*` type.
//! Every other module, crate, checker and test sees only `crate::ast`.
//!
//! That is not ceremony. `ruff_python_parser` is published at `0.0.x` as an
//! implementation detail of `ruff`, so its API can break between patch
//! releases. Confining it here means such a break lands in one module, against
//! a test suite that already defines correct behaviour.

use crate::ast::parse::ParseError;
use crate::ast::{Ast, ConstantKind, Expr, ExprId, Param, Stmt, StmtId};
use crate::span::Span;
use ruff_python_ast as py;
use ruff_text_size::{Ranged, TextRange};

pub(crate) fn parse_and_convert(source: &str) -> Result<Ast, ParseError> {
    let parsed = ruff_python_parser::parse_module(source).map_err(|error| ParseError {
        message: error.error.to_string(),
        span: span(error.location),
    })?;

    let mut ast = Ast::default();
    let mut body = Vec::new();
    for stmt in parsed.suite() {
        body.push(convert_stmt(&mut ast, stmt));
    }
    ast.set_body(body);

    Ok(ast)
}

fn span(range: TextRange) -> Span {
    Span::new(range.start().to_u32(), range.end().to_u32())
}

fn convert_body(ast: &mut Ast, stmts: &[py::Stmt]) -> Vec<StmtId> {
    stmts.iter().map(|s| convert_stmt(ast, s)).collect()
}

fn convert_decorators(ast: &mut Ast, decorators: &[py::Decorator]) -> Vec<ExprId> {
    decorators
        .iter()
        .map(|d| convert_expr(ast, &d.expression))
        .collect()
}

fn convert_params(ast: &mut Ast, parameters: &py::Parameters) -> Vec<Param> {
    let mut params = Vec::new();

    let mut push = |ast: &mut Ast, p: &py::Parameter| {
        let annotation = p.annotation.as_ref().map(|a| convert_expr(ast, a));
        params.push(Param {
            name: p.name.id.to_string(),
            annotation,
            span: span(p.range),
        });
    };

    // Declaration order, so a diagnostic about "the third parameter" means
    // what a reader counting them would mean.
    for p in &parameters.posonlyargs {
        push(ast, &p.parameter);
    }
    for p in &parameters.args {
        push(ast, &p.parameter);
    }
    if let Some(vararg) = &parameters.vararg {
        push(ast, vararg);
    }
    for p in &parameters.kwonlyargs {
        push(ast, &p.parameter);
    }
    if let Some(kwarg) = &parameters.kwarg {
        push(ast, kwarg);
    }

    params
}

fn convert_stmt(ast: &mut Ast, stmt: &py::Stmt) -> StmtId {
    let converted = match stmt {
        py::Stmt::FunctionDef(node) => {
            let params = convert_params(ast, &node.parameters);
            let returns = node.returns.as_ref().map(|r| convert_expr(ast, r));
            let decorators = convert_decorators(ast, &node.decorator_list);
            let body = convert_body(ast, &node.body);
            Stmt::FunctionDef {
                name: node.name.id.to_string(),
                is_async: node.is_async,
                params,
                returns,
                body,
                decorators,
                span: span(node.range),
                name_span: span(node.name.range),
            }
        }

        py::Stmt::ClassDef(node) => {
            let decorators = convert_decorators(ast, &node.decorator_list);
            let body = convert_body(ast, &node.body);
            Stmt::ClassDef {
                name: node.name.id.to_string(),
                body,
                decorators,
                span: span(node.range),
                name_span: span(node.name.range),
            }
        }

        py::Stmt::Assign(node) => {
            let targets = node.targets.iter().map(|t| convert_expr(ast, t)).collect();
            let value = convert_expr(ast, &node.value);
            Stmt::Assign {
                targets,
                value,
                annotation: None,
                span: span(node.range),
            }
        }

        // An annotated assignment with no value (`x: int`) declares a type but
        // binds nothing, so there is no value to reason about. It becomes
        // Unsupported rather than an Assign with a fabricated value.
        py::Stmt::AnnAssign(node) => match &node.value {
            Some(value) => {
                let target = convert_expr(ast, &node.target);
                let annotation = convert_expr(ast, &node.annotation);
                let value = convert_expr(ast, value);
                Stmt::Assign {
                    targets: vec![target],
                    value,
                    annotation: Some(annotation),
                    span: span(node.range),
                }
            }
            None => Stmt::Unsupported {
                span: span(node.range),
            },
        },

        py::Stmt::Return(node) => {
            let value = node.value.as_ref().map(|v| convert_expr(ast, v));
            Stmt::Return {
                value,
                span: span(node.range),
            }
        }

        py::Stmt::Expr(node) => {
            let value = convert_expr(ast, &node.value);
            Stmt::Expr {
                value,
                span: span(node.range),
            }
        }

        py::Stmt::Pass(node) => Stmt::Pass {
            span: span(node.range),
        },

        other => Stmt::Unsupported {
            span: span(other.range()),
        },
    };

    ast.alloc_stmt(converted)
}

fn convert_expr(ast: &mut Ast, expr: &py::Expr) -> ExprId {
    let converted = match expr {
        py::Expr::Name(node) => Expr::Name {
            name: node.id.to_string(),
            span: span(node.range),
        },

        py::Expr::Attribute(node) => {
            let value = convert_expr(ast, &node.value);
            Expr::Attribute {
                value,
                attr: node.attr.id.to_string(),
                span: span(node.range),
            }
        }

        py::Expr::Call(node) => {
            let func = convert_expr(ast, &node.func);
            let args = node
                .arguments
                .args
                .iter()
                .map(|a| convert_expr(ast, a))
                .collect();
            let keywords = node
                .arguments
                .keywords
                .iter()
                .map(|k| {
                    let name = k.arg.as_ref().map(|a| a.id.to_string());
                    (name, convert_expr(ast, &k.value))
                })
                .collect();
            // ExprCall stores only a start offset; its full range comes from
            // the Ranged impl, which extends to the end of the arguments.
            Expr::Call {
                func,
                args,
                keywords,
                span: span(node.range()),
            }
        }

        // Kept distinct from the call it wraps. C1 exists precisely to tell
        // `await f()` from `f()`, so flattening this would make the check
        // impossible to write.
        py::Expr::Await(node) => {
            let value = convert_expr(ast, &node.value);
            Expr::Await {
                value,
                span: span(node.range),
            }
        }

        py::Expr::NumberLiteral(node) => {
            let kind = match node.value {
                py::Number::Int(_) => ConstantKind::Int,
                py::Number::Float(_) => ConstantKind::Float,
                py::Number::Complex { .. } => ConstantKind::Complex,
            };
            Expr::Constant {
                kind,
                span: span(node.range),
            }
        }

        py::Expr::StringLiteral(node) => Expr::Constant {
            kind: ConstantKind::Str,
            span: span(node.range),
        },

        // An f-string evaluates to a str, so for the purpose of asking what
        // type a name holds it is one.
        py::Expr::FString(node) => Expr::Constant {
            kind: ConstantKind::Str,
            span: span(node.range),
        },

        py::Expr::BytesLiteral(node) => Expr::Constant {
            kind: ConstantKind::Bytes,
            span: span(node.range),
        },

        py::Expr::BooleanLiteral(node) => Expr::Constant {
            kind: ConstantKind::Bool,
            span: span(node.range),
        },

        py::Expr::NoneLiteral(node) => Expr::Constant {
            kind: ConstantKind::None,
            span: span(node.range),
        },

        py::Expr::EllipsisLiteral(node) => Expr::Constant {
            kind: ConstantKind::Ellipsis,
            span: span(node.range),
        },

        py::Expr::List(node) => {
            let elements = node.elts.iter().map(|e| convert_expr(ast, e)).collect();
            Expr::List {
                elements,
                span: span(node.range),
            }
        }

        py::Expr::Dict(node) => Expr::Dict {
            span: span(node.range),
        },

        other => Expr::Unsupported {
            span: span(other.range()),
        },
    };

    ast.alloc_expr(converted)
}
