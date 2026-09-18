//! The engine's own syntax tree.
//!
//! Deliberately a subset of Python. Node kinds are added as checkers need
//! them; everything not yet modelled becomes `Unsupported`, which carries its
//! span so it can still be skipped over precisely.
//!
//! No third-party parser type appears in this module or any module that
//! depends on it — see `convert.rs`, which is the only file that knows what
//! parsed the source.

use crate::define_id;
use crate::ids::Arena;
use crate::span::Span;

mod convert;
pub mod parse;

pub use parse::{ParseError, parse};

define_id!(StmtId);
define_id!(ExprId);

/// One `with` item: the context manager and the name it is bound to.
#[derive(Clone, PartialEq, Debug)]
pub struct WithItem {
    pub context: ExprId,
    pub target: Option<ExprId>,
    pub span: Span,
}

/// One `except` clause.
#[derive(Clone, PartialEq, Debug)]
pub struct ExceptHandler {
    pub exception_type: Option<ExprId>,
    /// The name in `except E as name`.
    pub name: Option<String>,
    pub body: Vec<StmtId>,
    pub span: Span,
}

/// One name introduced by an import statement.
#[derive(Clone, PartialEq, Debug)]
pub struct ImportAlias {
    /// The dotted module or name as written: `a.b` in `import a.b`.
    pub name: String,
    pub asname: Option<String>,
    pub span: Span,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Param {
    pub name: String,
    pub annotation: Option<ExprId>,
    pub span: Span,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Stmt {
    FunctionDef {
        name: String,
        is_async: bool,
        params: Vec<Param>,
        returns: Option<ExprId>,
        body: Vec<StmtId>,
        decorators: Vec<ExprId>,
        span: Span,
        /// Span of the name alone, for diagnostics about the name.
        name_span: Span,
    },
    ClassDef {
        name: String,
        body: Vec<StmtId>,
        decorators: Vec<ExprId>,
        span: Span,
        name_span: Span,
    },
    Assign {
        targets: Vec<ExprId>,
        value: ExprId,
        annotation: Option<ExprId>,
        span: Span,
    },
    Return {
        value: Option<ExprId>,
        span: Span,
    },
    /// An expression evaluated for effect, its value discarded. Central to C1:
    /// a coroutine discarded here never runs.
    Expr {
        value: ExprId,
        span: Span,
    },
    /// `elif` is desugared into a nested `If` in `orelse`, which keeps the
    /// branching structure exact rather than flattening it.
    If {
        test: ExprId,
        body: Vec<StmtId>,
        orelse: Vec<StmtId>,
        span: Span,
    },
    For {
        target: ExprId,
        iter: ExprId,
        body: Vec<StmtId>,
        orelse: Vec<StmtId>,
        is_async: bool,
        span: Span,
    },
    While {
        test: ExprId,
        body: Vec<StmtId>,
        orelse: Vec<StmtId>,
        span: Span,
    },
    With {
        items: Vec<WithItem>,
        body: Vec<StmtId>,
        is_async: bool,
        span: Span,
    },
    Try {
        body: Vec<StmtId>,
        handlers: Vec<ExceptHandler>,
        orelse: Vec<StmtId>,
        finalbody: Vec<StmtId>,
        span: Span,
    },
    /// `import a`, `import a.b as c`.
    Import {
        aliases: Vec<ImportAlias>,
        span: Span,
    },
    /// `from a.b import c`, `from . import d`.
    ///
    /// `level` is the number of leading dots, so a relative import can be
    /// resolved against the importing file's own package.
    ImportFrom {
        module: Option<String>,
        level: u32,
        aliases: Vec<ImportAlias>,
        span: Span,
    },
    Pass {
        span: Span,
    },
    /// A statement kind not yet modelled. Carries its span so it can be
    /// skipped precisely rather than silently.
    Unsupported {
        span: Span,
    },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::FunctionDef { span, .. }
            | Stmt::ClassDef { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Expr { span, .. }
            | Stmt::If { span, .. }
            | Stmt::For { span, .. }
            | Stmt::While { span, .. }
            | Stmt::With { span, .. }
            | Stmt::Try { span, .. }
            | Stmt::Import { span, .. }
            | Stmt::ImportFrom { span, .. }
            | Stmt::Pass { span }
            | Stmt::Unsupported { span } => *span,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    Name {
        name: String,
        span: Span,
    },
    Attribute {
        value: ExprId,
        attr: String,
        span: Span,
    },
    Call {
        func: ExprId,
        args: Vec<ExprId>,
        keywords: Vec<(Option<String>, ExprId)>,
        span: Span,
    },
    Await {
        value: ExprId,
        span: Span,
    },
    /// A literal. The kind matters for type inference; the value mostly does
    /// not.
    Constant {
        kind: ConstantKind,
        span: Span,
    },
    List {
        elements: Vec<ExprId>,
        span: Span,
    },
    Dict {
        span: Span,
    },
    Unsupported {
        span: Span,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConstantKind {
    Int,
    Float,
    Complex,
    Str,
    Bytes,
    Bool,
    None,
    Ellipsis,
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Name { span, .. }
            | Expr::Attribute { span, .. }
            | Expr::Call { span, .. }
            | Expr::Await { span, .. }
            | Expr::Constant { span, .. }
            | Expr::List { span, .. }
            | Expr::Dict { span }
            | Expr::Unsupported { span } => *span,
        }
    }
}

/// One file's syntax tree.
#[derive(Debug, Default)]
pub struct Ast {
    stmts: Arena<StmtId, Stmt>,
    exprs: Arena<ExprId, Expr>,
    body: Vec<StmtId>,
}

impl Ast {
    pub fn alloc_stmt(&mut self, stmt: Stmt) -> StmtId {
        self.stmts.alloc(stmt)
    }

    pub fn alloc_expr(&mut self, expr: Expr) -> ExprId {
        self.exprs.alloc(expr)
    }

    pub fn stmt(&self, id: StmtId) -> &Stmt {
        self.stmts.get(id)
    }

    pub fn expr(&self, id: ExprId) -> &Expr {
        self.exprs.get(id)
    }

    pub fn stmt_span(&self, id: StmtId) -> Span {
        self.stmts.get(id).span()
    }

    pub fn expr_span(&self, id: ExprId) -> Span {
        self.exprs.get(id).span()
    }

    /// The module's top-level statements.
    pub fn body(&self) -> &[StmtId] {
        &self.body
    }

    pub fn set_body(&mut self, body: Vec<StmtId>) {
        self.body = body;
    }

    /// Every expression in the file, in allocation order.
    pub fn exprs(&self) -> impl Iterator<Item = (ExprId, &Expr)> {
        self.exprs.iter()
    }

    /// Every statement in the file, in allocation order.
    pub fn stmts(&self) -> impl Iterator<Item = (StmtId, &Stmt)> {
        self.stmts.iter()
    }

    pub fn stmt_count(&self) -> usize {
        self.stmts.len()
    }

    pub fn expr_count(&self) -> usize {
        self.exprs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn an_empty_module_has_no_body() {
        let ast = Ast::default();
        assert!(ast.body().is_empty());
    }

    #[test]
    fn statements_round_trip_through_the_arena() {
        let mut ast = Ast::default();
        let id = ast.alloc_stmt(Stmt::Pass {
            span: Span::new(0, 4),
        });
        assert!(matches!(ast.stmt(id), Stmt::Pass { .. }));
        assert_eq!(ast.stmt_span(id), Span::new(0, 4));
    }

    #[test]
    fn expressions_round_trip_through_the_arena() {
        let mut ast = Ast::default();
        let id = ast.alloc_expr(Expr::Name {
            name: "x".into(),
            span: Span::new(0, 1),
        });
        assert!(matches!(ast.expr(id), Expr::Name { .. }));
        assert_eq!(ast.expr_span(id), Span::new(0, 1));
    }

    #[test]
    fn every_statement_variant_reports_a_span() {
        // If a variant is added without a span, this stops compiling, which is
        // the point: a node with no span cannot be pointed at in a diagnostic.
        let mut ast = Ast::default();
        let name = ast.alloc_expr(Expr::Name {
            name: "f".into(),
            span: Span::new(0, 1),
        });
        let variants = vec![
            Stmt::Pass {
                span: Span::new(0, 1),
            },
            Stmt::Return {
                value: None,
                span: Span::new(0, 1),
            },
            Stmt::Expr {
                value: name,
                span: Span::new(0, 1),
            },
            Stmt::Unsupported {
                span: Span::new(0, 1),
            },
        ];
        for v in variants {
            let id = ast.alloc_stmt(v);
            assert_eq!(ast.stmt_span(id), Span::new(0, 1));
        }
    }

    #[test]
    fn function_definitions_record_whether_they_are_async() {
        let mut ast = Ast::default();
        let sync = ast.alloc_stmt(Stmt::FunctionDef {
            name: "f".into(),
            is_async: false,
            params: vec![],
            returns: None,
            body: vec![],
            decorators: vec![],
            span: Span::new(0, 10),
            name_span: Span::new(4, 5),
        });
        let asynchronous = ast.alloc_stmt(Stmt::FunctionDef {
            name: "g".into(),
            is_async: true,
            params: vec![],
            returns: None,
            body: vec![],
            decorators: vec![],
            span: Span::new(0, 10),
            name_span: Span::new(10, 11),
        });

        assert!(matches!(
            ast.stmt(sync),
            Stmt::FunctionDef {
                is_async: false,
                ..
            }
        ));
        assert!(matches!(
            ast.stmt(asynchronous),
            Stmt::FunctionDef { is_async: true, .. }
        ));
    }

    #[test]
    fn a_function_carries_a_span_for_its_name_alone() {
        // Diagnostics about a name point at the name, not at the whole
        // function body.
        let mut ast = Ast::default();
        let id = ast.alloc_stmt(Stmt::FunctionDef {
            name: "is_ready".into(),
            is_async: false,
            params: vec![],
            returns: None,
            body: vec![],
            decorators: vec![],
            span: Span::new(0, 40),
            name_span: Span::new(4, 12),
        });
        match ast.stmt(id) {
            Stmt::FunctionDef { name_span, .. } => assert_eq!(*name_span, Span::new(4, 12)),
            _ => panic!("expected a function definition"),
        }
    }
}
