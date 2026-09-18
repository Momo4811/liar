# 001 — Python parser

**Decision:** `ruff_python_parser`, pinned at `=0.0.13`.

**Date:** 2026-09-18

## Context

The engine needs to parse Python into a syntax tree. Two crates were
candidates, and one fact decided it.

| Crate | Latest | Last published |
|---|---|---|
| `rustpython-parser` | 0.4.0 | 2024-08-06 |
| `ruff_python_parser` | 0.0.13 | 2026-09-10 |

## Decision

`ruff_python_parser`.

A parser last published in 2024 cannot parse two years of newer Python syntax,
and a linter that fails on syntax its targets already use is worthless. This
crate is what `ruff` itself runs on, it is a hand-written recursive descent
parser producing a genuine AST rather than a concrete syntax tree, and it is
published continuously.

This is now verified rather than assumed. `modern_syntax_parses_without_error`
in `ast/parse.rs` parses structural pattern matching, the walrus operator, PEP
695 generic functions and type aliases, nested f-string format specifiers, and
`async with`. If that test ever fails, this decision needs revisiting.

Tree-sitter was rejected earlier: it is built for editors, is resilient to
broken input, and produces a tree mirroring source text. A real AST is the
better tool for analysis.

## Consequences

Astral publish these crates at `0.0.x` as an implementation detail of `ruff`,
not as a stable public API. Documentation is thin — the crate's API had to be
read from its source — and it may break between patch releases.

Mitigated by pinning exact versions and by confining every `ruff_*` type to
`crates/liar-core/src/ast/convert.rs`, which is a private module. An upgrade
that breaks the API breaks one file, against a test suite that already defines
correct behaviour.

## The API, as of 0.0.13

Three crates, all pinned together: `ruff_python_parser`, `ruff_python_ast`,
`ruff_text_size`.

**Entry point**

```rust
ruff_python_parser::parse_module(source: &str) -> Result<Parsed<ModModule>, ParseError>
```

`Parsed::suite() -> &Suite` gives the module's top-level statements. There is
also a general `parse(source, ParseOptions)`, and `parse_unchecked*` variants
that return a tree even for invalid input — unused here, since a file that does
not parse is a file this tool says nothing about.

**Errors**

```rust
pub struct ParseError {
    pub error: ParseErrorType,   // Display
    pub location: TextRange,
}
```

**Ranges**

Most nodes expose a public `range: TextRange` field. `TextRange::start()` and
`end()` give `TextSize`, which converts with `.to_u32()`.

**`ExprCall` is the exception** and the one trap in the whole API: it stores
`range_start: TextSize`, *not* a `range` field. Its full range comes from the
`Ranged` trait impl, which extends to the end of the arguments. Reaching for
`node.range` on a call does not compile; `node.range()` with
`use ruff_text_size::Ranged` is what works. Any node whose range is taken
generically must go through `Ranged` for the same reason.

**Node shapes actually used**

```rust
StmtFunctionDef { range, is_async, decorator_list, name: Identifier,
                  type_params, parameters: Box<Parameters>,
                  returns: Option<Box<Expr>>, body }
StmtClassDef    { range, decorator_list, name: Identifier, type_params,
                  arguments, body }
StmtAssign      { range, targets: Vec<Expr>, value: Box<Expr> }
StmtAnnAssign   { range, target, annotation, value: Option<Box<Expr>>, simple }
StmtReturn      { range, value: Option<Box<Expr>> }
StmtExpr        { range, value: Box<Expr> }
StmtPass        { range }

ExprName      { range, id: Name, ctx }
ExprAttribute { range, value: Box<Expr>, attr: Identifier, ctx }
ExprCall      { range_start, func: Box<Expr>, arguments: Arguments }
ExprAwait     { range, value: Box<Expr> }
ExprList      { range, elts: Vec<Expr>, ctx }
ExprDict      { range, items: Vec<DictItem> }

Identifier    { id: Name, range, node_index }
Arguments     { range, args: Box<[Expr]>, keywords: ThinVec<Keyword> }
Keyword       { range, arg: Option<Identifier>, value: Expr }
Parameters    { range, posonlyargs, args, vararg: Option<Box<Parameter>>,
                kwonlyargs, kwarg: Option<Box<Parameter>> }
Parameter     { range, name: Identifier, annotation: Option<Box<Expr>> }
ParameterWithDefault { range, parameter: Parameter, default: Option<Box<Expr>> }
```

`Identifier` carrying its own `range` is what makes `name_span` exact — a
diagnostic about a function's name underlines the name, not the whole
definition. Deriving it by arithmetic from the definition's start would have
been wrong for decorated and async definitions alike.

Literals are separate node kinds rather than one `Constant` with a value:
`ExprNumberLiteral` (holding `Number::{Int, Float, Complex}`),
`ExprStringLiteral`, `ExprBytesLiteral`, `ExprBooleanLiteral`,
`ExprNoneLiteral`, `ExprEllipsisLiteral`, plus `ExprFString` and `ExprTString`.
