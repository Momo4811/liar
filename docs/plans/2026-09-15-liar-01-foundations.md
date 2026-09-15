# liar — Plan 1: Foundations

**Goal:** A CLI that parses Python files and renders findings in three tones,
with a fixture-test harness and CI gates running from the first commit.

**Architecture:** A Cargo workspace with `liar-core` (analysis library) and
`liar-cli` (binary). Everything is arena-allocated and referenced by typed
integer index — no `Rc<RefCell<…>>`. Third-party parser types are converted once
at the boundary into `liar-core`'s own AST and never appear anywhere else.

**Tech stack:** Rust (edition 2024), `ruff_python_parser` for parsing,
`annotate-snippets` for diagnostic rendering, `clap` for the CLI, `insta` for
snapshot tests, `proptest` for property tests, `toml`/`serde` for config.

**Spec:** `docs/design/2026-09-15-liar-design.md` — read it alongside this plan.
Section references below (§4, §8.1, §9…) point into it.

**Scope:** This plan builds no checkers. It builds the ground every checker
stands on, and proves that ground with tests. At the end, `liar check` runs, the
fixture harness works, and CI is green. Plan 2 adds the index and the first real
checkers.

---

## Global Constraints

Every task's requirements implicitly include all of these.

- **Rust edition 2024.** Workspace-wide, set once in the workspace manifest.
- **`#![forbid(unsafe_code)]`** at the top of every crate's `lib.rs` / `main.rs`.
- **`ruff_python_parser` and `ruff_python_ast` pinned at `=0.0.13`.** Exact-version
  pins, not caret. These are `0.0.x` crates published as an implementation detail
  of `ruff`; their API may break between patch releases (§8.1).
- **Parser types never leave `liar-core::ast::convert`.** No `ruff_*` type appears
  in any signature outside that module. This is what makes the pin above a
  contained risk rather than a structural one (§8.1.1).
- **Arena-allocated, index-referenced.** Typed `u32` newtype ids into `Vec`s. No
  `Rc`, no `RefCell`, no lifetimes threaded through data structures.
- **Deterministic output.** Findings sort by (file, line, column, check id)
  before rendering. Same input, same bytes out (§9.8).
- **Test first.** Write the failing test, watch it fail, then implement. Every
  task below is ordered that way and the order is not optional (§9).
- **CI gates from commit one:** `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`. A task is not done if any gate is red.
- **No AI references anywhere in this repository** — not in code, comments,
  commit messages, documentation, or directory names. Commits are authored as
  `Momo4811 <zcabmmo@ucl.ac.uk>`.

### A note on Tasks 5 and 10

Two tasks integrate a dependency whose exact API could not be verified when this
plan was written: `ruff_python_parser` in Task 5, and `annotate-snippets` 0.12
in Task 10. Neither publishes documentation in a form that could be checked from
here, and inventing plausible function signatures would produce code that looks
authoritative, does not compile, and costs an afternoon to unpick.

So both tasks specify the **contract** and the **complete tests**, and their
first step is establishing the real API from `cargo doc`. The tests define
correctness unambiguously; only the literal call syntax is discovered rather
than dictated. Where a task's code block contains a `todo!`, the comment above
it is the specification — keep that comment after replacing the `todo!`, since
it survives the dependency renaming things.

Every other task states its code outright.

---

## File Structure

```
liar/
├── Cargo.toml                       workspace manifest, shared dependency versions
├── rust-toolchain.toml              pinned toolchain
├── .gitignore
├── .github/workflows/ci.yml         fmt, clippy, test
├── data/
│   └── messages.toml                every check's message, in three tones
├── docs/
│   ├── design/                      the spec
│   ├── plans/                       this file
│   └── decisions/
│       └── 001-parser.md            why ruff_python_parser, and its real API
└── crates/
    ├── liar-core/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs               public surface; re-exports; forbid(unsafe_code)
    │       ├── ids.rs               define_id! macro, Id trait, Arena<I, T>
    │       ├── span.rs              Span (byte range)
    │       ├── source.rs            SourceFile, SourceMap, offset → line/column
    │       ├── check.rs             CheckId enum, codes, default severities
    │       ├── finding.rs           Finding, Label, Severity, deterministic Ord
    │       ├── messages.rs          Tone, MessageTable, loading, interpolation
    │       ├── fixture.rs           the `# expect:` fixture harness
    │       └── ast/
    │           ├── mod.rs           liar's own Stmt/Expr node types + Arena storage
    │           ├── parse.rs         public parse entry point, ParseError
    │           └── convert.rs       ruff AST → liar AST. THE ONLY parser-aware file
    └── liar-cli/
        ├── Cargo.toml
        └── src/
            ├── main.rs              clap CLI, exit codes
            ├── config.rs            liar.toml
            ├── discover.rs          file discovery, deterministic ordering
            └── render.rs            rustc-style rendering via annotate-snippets
```

**Why these boundaries:** `ids`, `span` and `source` are leaves with no
dependencies and heavy test coverage — they are where off-by-one and UTF-8 bugs
live. `ast/convert.rs` is deliberately isolated because it is the only file that
knows a third-party parser exists. `fixture.rs` lives in `liar-core` rather than
in a test directory because every later crate's tests use it.

---

## Task 1: Toolchain, workspace, and CI

**Files:**
- Create: `rust-toolchain.toml`, `Cargo.toml`, `.gitignore`,
  `.github/workflows/ci.yml`
- Create: `crates/liar-core/Cargo.toml`, `crates/liar-core/src/lib.rs`
- Create: `crates/liar-cli/Cargo.toml`, `crates/liar-cli/src/main.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling workspace with crates `liar-core` (lib) and `liar-cli`
  (bin, name `liar`). Every later task adds to these.

- [ ] **Step 1: Install the Rust toolchain**

There is no `cargo` on this machine. Install via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

On Windows, download and run `rustup-init.exe` from <https://rustup.rs> instead.
Then restart the shell and verify:

```bash
cargo --version && rustc --version
```

Expected: both print a version. If `cargo` is still not found, add
`$HOME/.cargo/bin` to `PATH`.

- [ ] **Step 2: Pin the toolchain**

Capture the version you just installed rather than guessing one:

```bash
rustc --version | awk '{print $2}'
```

Write that exact version into `rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.XX.Y"          # the version printed above, verbatim
components = ["rustfmt", "clippy"]
```

Pinning means CI and your machine agree, and a clippy lint added in a later
compiler release cannot turn CI red without you choosing it.

- [ ] **Step 3: Write the workspace manifest**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/liar-core", "crates/liar-cli"]

[workspace.package]
edition = "2024"
license = "MIT"
repository = "https://github.com/Momo4811/liar"

[workspace.dependencies]
ruff_python_parser = "=0.0.13"
ruff_python_ast    = "=0.0.13"
annotate-snippets  = "0.12"
clap               = { version = "4", features = ["derive"] }
serde              = { version = "1", features = ["derive"] }
toml               = "0.9"
thiserror          = "2"
insta              = "1"
proptest           = "1"

[workspace.lints.rust]
unsafe_code = "forbid"

[profile.release]
debug = 1
```

Pinning dependency versions once in `[workspace.dependencies]` keeps the two
crates from drifting apart.

- [ ] **Step 4: Write the crate manifests**

`crates/liar-core/Cargo.toml`:

```toml
[package]
name = "liar-core"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true

[dependencies]
ruff_python_parser.workspace = true
ruff_python_ast.workspace = true
serde.workspace = true
toml.workspace = true
thiserror.workspace = true

[dev-dependencies]
insta.workspace = true
proptest.workspace = true
```

`crates/liar-cli/Cargo.toml`:

```toml
[package]
name = "liar-cli"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "liar"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
liar-core = { path = "../liar-core" }
annotate-snippets.workspace = true
clap.workspace = true
serde.workspace = true
toml.workspace = true
thiserror.workspace = true

[dev-dependencies]
insta.workspace = true
```

- [ ] **Step 5: Write the crate roots**

`crates/liar-core/src/lib.rs`:

```rust
//! Static analysis for Python: the engine.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
```

`crates/liar-cli/src/main.rs`:

```rust
//! Static analysis for Python: the command line interface.

#![forbid(unsafe_code)]

fn main() {
    println!("liar");
}
```

- [ ] **Step 6: Write `.gitignore`**

```gitignore
/target
**/*.rs.bk
*.pdb

# insta writes these when a snapshot changes; review then accept, never commit
**/*.snap.new
```

- [ ] **Step 7: Verify the workspace builds and tests pass**

```bash
cargo test
```

Expected: compiles, `workspace_builds` passes. First compile downloads the
`ruff_*` crates; if that fails, the pinned versions are the first thing to check.

- [ ] **Step 8: Verify the gates pass locally before writing CI**

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

Expected: both silent. Fix anything they report now — CI is about to enforce
them.

- [ ] **Step 9: Write the CI workflow**

`.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
  pull_request:

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: Format
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy --all-targets -- -D warnings
      - name: Test
        run: cargo test --all
```

`dtolnay/rust-toolchain@stable` honours `rust-toolchain.toml`, so the pinned
version from Step 2 is what runs.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore .github crates
git commit -m "Set up the Cargo workspace and CI gates

Two crates: liar-core for the analysis engine, liar-cli for the binary.
Dependency versions are pinned once at the workspace level so the crates
cannot drift. unsafe_code is forbidden workspace-wide.

CI runs fmt, clippy with warnings denied, and tests from this commit
onward rather than being added once there is something to protect."
```

---

## Task 2: Typed ids and arenas

Everything in the engine is stored in a `Vec` and referred to by a typed integer
index. This sidesteps the borrow checker on the cyclic, self-referential
structures an AST and a control flow graph need, and it is how `rustc` and
`ruff` are both built.

**Files:**
- Create: `crates/liar-core/src/ids.rs`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `trait Id: Copy + Eq + Ord + Hash { fn from_index(u32) -> Self; fn index(self) -> u32; }`
  - `macro_rules! define_id` — declares a newtype implementing `Id`
  - `struct Arena<I: Id, T>` with `new()`, `alloc(T) -> I`, `get(I) -> &T`,
    `get_mut(I) -> &mut T`, `len() -> usize`, `is_empty() -> bool`,
    `iter() -> impl Iterator<Item = (I, &T)>`
  - `FileId` and `NodeId`, both declared via `define_id!`

- [ ] **Step 1: Write the failing tests**

`crates/liar-core/src/ids.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    define_id!(TestId);

    #[test]
    fn alloc_returns_sequential_ids() {
        let mut arena: Arena<TestId, &str> = Arena::new();
        let a = arena.alloc("a");
        let b = arena.alloc("b");
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
        assert_ne!(a, b);
    }

    #[test]
    fn get_returns_the_allocated_value() {
        let mut arena: Arena<TestId, u32> = Arena::new();
        let id = arena.alloc(42);
        assert_eq!(*arena.get(id), 42);
    }

    #[test]
    fn ids_remain_valid_after_further_allocation() {
        // The whole point of indices over references: growing the arena
        // cannot invalidate an id handed out earlier.
        let mut arena: Arena<TestId, u32> = Arena::new();
        let first = arena.alloc(1);
        for n in 2..1000 {
            arena.alloc(n);
        }
        assert_eq!(*arena.get(first), 1);
    }

    #[test]
    fn get_mut_mutates_in_place() {
        let mut arena: Arena<TestId, u32> = Arena::new();
        let id = arena.alloc(1);
        *arena.get_mut(id) = 7;
        assert_eq!(*arena.get(id), 7);
    }

    #[test]
    fn iter_yields_every_item_with_its_id_in_order() {
        let mut arena: Arena<TestId, char> = Arena::new();
        let a = arena.alloc('a');
        let b = arena.alloc('b');
        let collected: Vec<_> = arena.iter().collect();
        assert_eq!(collected, vec![(a, &'a'), (b, &'b')]);
    }

    #[test]
    fn empty_arena_reports_empty() {
        let arena: Arena<TestId, u32> = Arena::new();
        assert!(arena.is_empty());
        assert_eq!(arena.len(), 0);
    }

    #[test]
    #[should_panic(expected = "id out of range")]
    fn get_with_a_foreign_id_panics_loudly() {
        // A bug, not a recoverable condition. Panicking with a clear message
        // beats returning an Option every caller unwraps.
        let arena: Arena<TestId, u32> = Arena::new();
        let _ = arena.get(TestId::from_index(0));
    }

    #[test]
    fn ids_sort_by_index() {
        let mut ids = vec![TestId::from_index(2), TestId::from_index(0), TestId::from_index(1)];
        ids.sort();
        assert_eq!(ids, vec![
            TestId::from_index(0),
            TestId::from_index(1),
            TestId::from_index(2),
        ]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p liar-core ids
```

Expected: compile error — `define_id`, `Arena` and `Id` do not exist yet.

- [ ] **Step 3: Write the implementation**

Above the test module in `crates/liar-core/src/ids.rs`:

```rust
//! Typed indices into arenas.
//!
//! The engine stores nodes in `Vec`s and refers to them by index rather than by
//! reference. Indices are `Copy`, cannot dangle, do not borrow the arena, and
//! survive the arena growing — all of which matter for the cyclic structures an
//! AST and a control flow graph need.

use std::hash::Hash;
use std::marker::PhantomData;

/// A typed index into an [`Arena`].
pub trait Id: Copy + Eq + Ord + Hash {
    fn from_index(index: u32) -> Self;
    fn index(self) -> u32;
}

/// Declares a newtype index implementing [`Id`].
#[macro_export]
macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(u32);

        impl $crate::ids::Id for $name {
            fn from_index(index: u32) -> Self {
                Self(index)
            }

            fn index(self) -> u32 {
                self.0
            }
        }
    };
}

/// A growable, index-addressed store.
#[derive(Debug)]
pub struct Arena<I: Id, T> {
    items: Vec<T>,
    _marker: PhantomData<fn() -> I>,
}

impl<I: Id, T> Arena<I, T> {
    pub fn new() -> Self {
        Self { items: Vec::new(), _marker: PhantomData }
    }

    /// Appends `value` and returns its id.
    ///
    /// # Panics
    /// If the arena would exceed `u32::MAX` entries. A single Python file
    /// producing four billion nodes is a bug somewhere else.
    pub fn alloc(&mut self, value: T) -> I {
        let index = u32::try_from(self.items.len()).expect("arena exceeded u32::MAX entries");
        self.items.push(value);
        I::from_index(index)
    }

    /// # Panics
    /// If `id` did not come from this arena.
    pub fn get(&self, id: I) -> &T {
        self.items.get(id.index() as usize).expect("id out of range for this arena")
    }

    /// # Panics
    /// If `id` did not come from this arena.
    pub fn get_mut(&mut self, id: I) -> &mut T {
        self.items.get_mut(id.index() as usize).expect("id out of range for this arena")
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, item)| (I::from_index(i as u32), item))
    }
}

impl<I: Id, T> Default for Arena<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

define_id!(FileId);
define_id!(NodeId);
```

`PhantomData<fn() -> I>` rather than `PhantomData<I>` keeps `Arena` `Send` and
`Sync` regardless of `I`, which costs nothing and avoids a surprise later.

- [ ] **Step 4: Register the module**

In `crates/liar-core/src/lib.rs`, replace the placeholder test module with:

```rust
//! Static analysis for Python: the engine.

#![forbid(unsafe_code)]

pub mod ids;

pub use ids::{Arena, FileId, Id, NodeId};
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p liar-core ids
```

Expected: 8 passed.

- [ ] **Step 6: Run the gates**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Expected: all clean.

- [ ] **Step 7: Commit**

```bash
git add crates/liar-core/src/ids.rs crates/liar-core/src/lib.rs
git commit -m "Add typed arena indices

Nodes are stored in Vecs and referred to by typed u32 index rather than
by reference. Indices are Copy, cannot dangle, do not borrow the arena,
and stay valid as it grows, which is what the AST and later the control
flow graph need.

Out-of-range access panics rather than returning an Option: a foreign id
is a bug in the engine, not a condition callers should be made to handle
at every use site."
```

---

## Task 3: Spans and source files

Converting a byte offset to a line and column is where analysers quietly get
UTF-8, CRLF and BOMs wrong, and every diagnostic in the tool depends on it. It
gets tested disproportionately hard.

**Files:**
- Create: `crates/liar-core/src/span.rs`
- Create: `crates/liar-core/src/source.rs`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: `FileId` from Task 2.
- Produces:
  - `struct Span { start: u32, end: u32 }` with `new`, `len`, `is_empty`,
    `contains(u32)`
  - `struct Position { line: u32, column: u32 }` — both 1-based, column counted
    in Unicode characters, not bytes
  - `struct SourceFile { id: FileId, path: PathBuf, text: String }` with
    `position(offset: u32) -> Position`, `line_text(line: u32) -> &str`,
    `line_count() -> u32`
  - `struct SourceMap` with `add(PathBuf, String) -> FileId`, `get(FileId) -> &SourceFile`

- [ ] **Step 1: Write the failing span tests**

`crates/liar-core/src/span.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_is_the_byte_length() {
        assert_eq!(Span::new(3, 8).len(), 5);
    }

    #[test]
    fn an_empty_span_is_empty() {
        assert!(Span::new(4, 4).is_empty());
        assert!(!Span::new(4, 5).is_empty());
    }

    #[test]
    fn contains_is_half_open() {
        let span = Span::new(2, 5);
        assert!(!span.contains(1));
        assert!(span.contains(2));
        assert!(span.contains(4));
        assert!(!span.contains(5), "end is exclusive");
    }

    #[test]
    fn spans_sort_by_start_then_end() {
        let mut spans = vec![Span::new(5, 6), Span::new(1, 9), Span::new(1, 2)];
        spans.sort();
        assert_eq!(spans, vec![Span::new(1, 2), Span::new(1, 9), Span::new(5, 6)]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-core span
```

Expected: compile error, `Span` not found.

- [ ] **Step 3: Implement `Span`**

Above the tests in `crates/liar-core/src/span.rs`:

```rust
//! Byte ranges into a source file.

/// A half-open byte range `[start, end)` within one file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// # Panics
    /// If `end < start`.
    pub fn new(start: u32, end: u32) -> Self {
        assert!(end >= start, "span end {end} precedes start {start}");
        Self { start, end }
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn contains(self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }
}
```

- [ ] **Step 4: Run to verify the span tests pass**

```bash
cargo test -p liar-core span
```

Expected: 4 passed.

- [ ] **Step 5: Write the failing source tests**

`crates/liar-core/src/source.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(text: &str) -> SourceFile {
        let mut map = SourceMap::new();
        let id = map.add(PathBuf::from("t.py"), text.to_string());
        map.get(id).clone()
    }

    #[test]
    fn first_character_is_line_one_column_one() {
        let f = file("abc");
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
    }

    #[test]
    fn columns_advance_within_a_line() {
        let f = file("abc");
        assert_eq!(f.position(2), Position { line: 1, column: 3 });
    }

    #[test]
    fn newline_starts_the_next_line() {
        let f = file("ab\ncd");
        assert_eq!(f.position(3), Position { line: 2, column: 1 });
        assert_eq!(f.position(4), Position { line: 2, column: 2 });
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        // "é" is two bytes in UTF-8. The character after it is column 3,
        // not column 4 — pointing a caret at byte offsets would misalign
        // every diagnostic on a line containing non-ASCII.
        let f = file("aéb");
        assert_eq!(f.position(0), Position { line: 1, column: 1 }); // a
        assert_eq!(f.position(1), Position { line: 1, column: 2 }); // é
        assert_eq!(f.position(3), Position { line: 1, column: 3 }); // b
    }

    #[test]
    fn astral_plane_characters_count_as_one_column() {
        // "🦀" is four bytes.
        let f = file("🦀x");
        assert_eq!(f.position(4), Position { line: 1, column: 2 });
    }

    #[test]
    fn crlf_does_not_produce_a_phantom_column() {
        let f = file("ab\r\ncd");
        assert_eq!(f.position(4), Position { line: 2, column: 1 });
    }

    #[test]
    fn a_leading_bom_is_stripped() {
        // Python permits a UTF-8 BOM. If it is not stripped, every offset on
        // line 1 is three bytes off.
        let f = file("\u{feff}abc");
        assert_eq!(f.text(), "abc");
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
    }

    #[test]
    fn offset_at_end_of_file_is_valid() {
        let f = file("ab");
        assert_eq!(f.position(2), Position { line: 1, column: 3 });
    }

    #[test]
    fn empty_file_has_one_line() {
        let f = file("");
        assert_eq!(f.line_count(), 1);
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
    }

    #[test]
    fn trailing_newline_does_not_add_a_line() {
        let f = file("a\n");
        assert_eq!(f.line_count(), 1);
    }

    #[test]
    fn line_text_excludes_the_line_terminator() {
        let f = file("ab\ncd\r\nef");
        assert_eq!(f.line_text(1), "ab");
        assert_eq!(f.line_text(2), "cd");
        assert_eq!(f.line_text(3), "ef");
    }

    #[test]
    fn source_map_assigns_distinct_ids() {
        let mut map = SourceMap::new();
        let a = map.add(PathBuf::from("a.py"), "a".into());
        let b = map.add(PathBuf::from("b.py"), "b".into());
        assert_ne!(a, b);
        assert_eq!(map.get(a).text(), "a");
        assert_eq!(map.get(b).text(), "b");
    }

    #[test]
    #[should_panic(expected = "offset 99 past end of file")]
    fn offset_past_end_panics() {
        let f = file("ab");
        let _ = f.position(99);
    }
}
```

- [ ] **Step 6: Run to verify failure**

```bash
cargo test -p liar-core source
```

Expected: compile error, `SourceFile` not found.

- [ ] **Step 7: Implement source handling**

Above the tests in `crates/liar-core/src/source.rs`:

```rust
//! Source files, and the mapping from byte offsets to human positions.

use crate::ids::{Arena, FileId, Id};
use std::path::{Path, PathBuf};

/// A 1-based line and column. Columns count Unicode characters, not bytes,
/// because a column is a thing a human counts in a text editor.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    id: FileId,
    path: PathBuf,
    text: String,
    /// Byte offset at which each line begins. Always starts with 0, so its
    /// length is the line count and lookup is a binary search.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(id: FileId, path: PathBuf, text: String) -> Self {
        // A UTF-8 BOM is legal at the start of a Python file and is not part
        // of the program. Strip it here, once, so no offset downstream has to
        // know about it.
        let text = match text.strip_prefix('\u{feff}') {
            Some(stripped) => stripped.to_string(),
            None => text,
        };

        let mut line_starts = vec![0u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' && offset + 1 < text.len() {
                line_starts.push(offset as u32 + 1);
            }
        }

        Self { id, path, text, line_starts }
    }

    pub fn id(&self) -> FileId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// # Panics
    /// If `offset` is past the end of the file, or not on a character
    /// boundary — both are engine bugs.
    pub fn position(&self, offset: u32) -> Position {
        assert!(
            offset as usize <= self.text.len(),
            "offset {offset} past end of file {}",
            self.path.display()
        );

        // partition_point gives the number of line starts at or before the
        // offset, which is exactly the 1-based line number.
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let line_start = self.line_starts[line - 1] as usize;

        let column = self.text[line_start..offset as usize].chars().count() as u32 + 1;

        Position { line: line as u32, column }
    }

    /// Text of a 1-based line, without its terminator.
    ///
    /// # Panics
    /// If `line` is zero or past the end of the file.
    pub fn line_text(&self, line: u32) -> &str {
        assert!(line >= 1, "line numbers are 1-based");
        let index = (line - 1) as usize;
        let start = self.line_starts[index] as usize;
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next as usize);

        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[derive(Debug, Default)]
pub struct SourceMap {
    files: Arena<FileId, SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: PathBuf, text: String) -> FileId {
        // Allocate a placeholder to learn the id, then overwrite it with the
        // real file, since SourceFile needs to know its own id.
        let id = self.files.alloc(SourceFile {
            id: FileId::from_index(0),
            path: PathBuf::new(),
            text: String::new(),
            line_starts: vec![0],
        });
        *self.files.get_mut(id) = SourceFile::new(id, path, text);
        id
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        self.files.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        self.files.iter()
    }
}
```

Add `use crate::ids::Id;` at the top so `FileId::from_index` resolves.

- [ ] **Step 8: Register the modules**

In `crates/liar-core/src/lib.rs`:

```rust
pub mod ids;
pub mod source;
pub mod span;

pub use ids::{Arena, FileId, Id, NodeId};
pub use source::{Position, SourceFile, SourceMap};
pub use span::Span;
```

- [ ] **Step 9: Run the tests**

```bash
cargo test -p liar-core
```

Expected: all pass. If `trailing_newline_does_not_add_a_line` fails, the guard
`offset + 1 < text.len()` in `SourceFile::new` is the cause — a newline at the
very end of a file does not begin a new line.

- [ ] **Step 10: Add a property test for position robustness**

Append to the test module in `crates/liar-core/src/source.rs`:

```rust
    use proptest::prelude::*;

    proptest! {
        /// position() must never panic on any character boundary of any text,
        /// and must always return a line within the file.
        #[test]
        fn position_is_total_over_character_boundaries(text in ".{0,400}") {
            let f = file(&text);
            for (offset, _) in f.text().char_indices() {
                let p = f.position(offset as u32);
                prop_assert!(p.line >= 1);
                prop_assert!(p.line <= f.line_count());
                prop_assert!(p.column >= 1);
            }
            // The end-of-file offset is always valid too.
            let end = f.text().len() as u32;
            prop_assert!(f.position(end).line >= 1);
        }
    }
```

- [ ] **Step 11: Run the property test**

```bash
cargo test -p liar-core position_is_total
```

Expected: PASS after 256 generated cases. A failure prints the minimal input
that broke it — keep that string as a regular unit test before fixing.

- [ ] **Step 12: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-core/src/span.rs crates/liar-core/src/source.rs crates/liar-core/src/lib.rs
git commit -m "Add spans and byte-offset to line/column mapping

Columns count Unicode characters rather than bytes, so a caret under an
expression on a line containing non-ASCII lands in the right place. A
leading UTF-8 BOM is stripped once at load, since Python permits one and
leaving it in shifts every offset on the first line.

Line starts are precomputed so position lookup is a binary search rather
than a scan. Tested against multibyte characters, astral-plane
characters, CRLF, BOMs, empty files and trailing newlines, plus a
property test asserting the mapping is total over character boundaries."
```

---

## Task 4: The AST node types

`liar-core`'s own AST. Deliberately a subset — the node kinds the first checkers
need, plus an honest `Unsupported` variant carrying its span for everything
else. Widening it is a later task's job, not speculation now.

**Files:**
- Create: `crates/liar-core/src/ast/mod.rs`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: `Span` (Task 3), `NodeId`/`Arena` (Task 2).
- Produces:
  - `enum Stmt`, `enum Expr`, `struct Module`
  - `define_id!(StmtId)`, `define_id!(ExprId)`
  - `struct Ast` holding `Arena<StmtId, Stmt>`, `Arena<ExprId, Expr>`, the module
    body, and `span_of_stmt` / `span_of_expr`

- [ ] **Step 1: Write the failing tests**

`crates/liar-core/src/ast/mod.rs`:

```rust
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
        let id = ast.alloc_stmt(Stmt::Pass { span: Span::new(0, 4) });
        assert!(matches!(ast.stmt(id), Stmt::Pass { .. }));
        assert_eq!(ast.stmt_span(id), Span::new(0, 4));
    }

    #[test]
    fn expressions_round_trip_through_the_arena() {
        let mut ast = Ast::default();
        let id = ast.alloc_expr(Expr::Name { name: "x".into(), span: Span::new(0, 1) });
        assert!(matches!(ast.expr(id), Expr::Name { .. }));
        assert_eq!(ast.expr_span(id), Span::new(0, 1));
    }

    #[test]
    fn every_statement_variant_reports_a_span() {
        // If a variant is added without a span, this stops compiling, which is
        // the point: a node with no span cannot be pointed at in a diagnostic.
        let mut ast = Ast::default();
        let name = ast.alloc_expr(Expr::Name { name: "f".into(), span: Span::new(0, 1) });
        let variants = vec![
            Stmt::Pass { span: Span::new(0, 1) },
            Stmt::Return { value: None, span: Span::new(0, 1) },
            Stmt::Expr { value: name, span: Span::new(0, 1) },
            Stmt::Unsupported { span: Span::new(0, 1) },
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
        let async_ = ast.alloc_stmt(Stmt::FunctionDef {
            name: "g".into(),
            is_async: true,
            params: vec![],
            returns: None,
            body: vec![],
            decorators: vec![],
            span: Span::new(0, 10),
            name_span: Span::new(10, 11),
        });

        assert!(matches!(ast.stmt(sync), Stmt::FunctionDef { is_async: false, .. }));
        assert!(matches!(ast.stmt(async_), Stmt::FunctionDef { is_async: true, .. }));
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
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-core ast
```

Expected: compile error, `Ast` not found.

- [ ] **Step 3: Implement the node types**

Above the tests in `crates/liar-core/src/ast/mod.rs`:

```rust
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
use crate::ids::{Arena, Id};
use crate::span::Span;

define_id!(StmtId);
define_id!(ExprId);

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
    /// A literal. The kind matters for type inference; the value mostly does not.
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

    pub fn stmt_count(&self) -> usize {
        self.stmts.len()
    }

    pub fn expr_count(&self) -> usize {
        self.exprs.len()
    }
}
```

- [ ] **Step 4: Register the module**

In `crates/liar-core/src/lib.rs` add `pub mod ast;` and
`pub use ast::{Ast, ConstantKind, Expr, ExprId, Stmt, StmtId};`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p liar-core ast
```

Expected: 6 passed.

- [ ] **Step 6: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-core/src/ast/mod.rs crates/liar-core/src/lib.rs
git commit -m "Add the engine's own syntax tree

A deliberate subset of Python: the node kinds the first checkers need,
with an Unsupported variant carrying its span for everything else, so
unmodelled syntax is skipped precisely rather than silently.

Functions carry a span for the name alone as well as for the whole
definition, because a diagnostic about a name should underline the name.
Statements and expressions live in separate arenas and are referred to
by typed id."
```

---

## Task 5: Parsing, and the parser facade

The only task that touches `ruff_python_parser`. Its API could not be verified
when this plan was written, so Step 1 establishes it from the crate's own docs.
The tests in Step 2 are complete and define correctness regardless of how the
calls are spelled.

**Files:**
- Create: `crates/liar-core/src/ast/parse.rs`
- Create: `crates/liar-core/src/ast/convert.rs`
- Create: `docs/decisions/001-parser.md`
- Modify: `crates/liar-core/src/ast/mod.rs`

**Interfaces:**
- Consumes: `Ast`, `Stmt`, `Expr` (Task 4); `Span` (Task 3).
- Produces:
  - `fn parse(source: &str) -> Result<Ast, ParseError>`
  - `struct ParseError { message: String, span: Span }`

- [ ] **Step 1: Establish the real API**

```bash
cargo doc -p ruff_python_parser -p ruff_python_ast --no-deps --open
```

Read the entry points. You are looking for: the function that parses a string as
a module, the type it returns, how to reach the module body from it, how a node
reports its source range, and how syntax errors are surfaced. Note the exact
names — you will write them down in Step 7.

If `cargo doc` output is too thin, the crate source is the fallback:

```bash
cargo add ruff_python_parser@=0.0.13 --dry-run
ls ~/.cargo/registry/src/*/ruff_python_parser-0.0.13/src/
```

- [ ] **Step 2: Write the failing tests**

`crates/liar-core/src/ast/parse.rs`:

```rust
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
            Stmt::FunctionDef { name, is_async, body, .. } => {
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
        //          0123456789
        let src = "def greet():\n    pass\n";
        let ast = parse_ok(src);
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { name_span, .. } => {
                assert_eq!(&src[name_span.start as usize..name_span.end as usize], "greet");
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
    fn a_return_annotation_is_recorded() {
        let ast = parse_ok("def f() -> str:\n    pass\n");
        match ast.stmt(ast.body()[0]) {
            Stmt::FunctionDef { returns, .. } => assert!(returns.is_some()),
            other => panic!("expected a function definition, got {other:?}"),
        }
    }

    #[test]
    fn unmodelled_syntax_becomes_unsupported_rather_than_an_error() {
        let ast = parse_ok("while True:\n    pass\n");
        assert!(matches!(ast.stmt(ast.body()[0]), Stmt::Unsupported { .. }));
    }

    #[test]
    fn modern_syntax_parses_without_error() {
        // The reason this parser was chosen over rustpython-parser, which was
        // last published in 2024. If any of these fail, that decision is wrong
        // and docs/decisions/001-parser.md needs revisiting.
        let sources = [
            "match x:\n    case 1:\n        pass\n",       // structural pattern matching
            "if (n := f()) > 0:\n    pass\n",              // walrus
            "def f[T](x: T) -> T:\n    return x\n",        // PEP 695 generics
            "type Alias = list[int]\n",                    // PEP 695 type alias
            "f'{value!r:>{width}}'\n",                     // nested f-string format spec
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
}
```

- [ ] **Step 3: Run to verify failure**

```bash
cargo test -p liar-core parse
```

Expected: compile error, `parse` not found.

- [ ] **Step 4: Write the public entry point**

Above the tests in `crates/liar-core/src/ast/parse.rs`:

```rust
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
```

Register both modules at the top of `crates/liar-core/src/ast/mod.rs`:

```rust
mod convert;
pub mod parse;

pub use parse::{ParseError, parse};
```

`convert` is private — nothing outside `ast` may reach the parser-aware code,
which is the boundary from §8.1.1 expressed in the module system rather than in
a convention someone has to remember. Re-exporting `parse` means callers write
`liar_core::ast::parse(src)` rather than `liar_core::ast::parse::parse(src)`.

Then add to `crates/liar-core/src/lib.rs`:

```rust
pub use ast::{ParseError, parse};
```

- [ ] **Step 5: Write the conversion**

`crates/liar-core/src/ast/convert.rs` is the only file in the project permitted
to name a `ruff_*` type. Using the API you established in Step 1, implement:

```rust
pub(crate) fn parse_and_convert(source: &str) -> Result<Ast, ParseError>
```

**The contract, which the tests in Step 2 enforce:**

1. Parse `source` as a Python module. On a syntax error, return `ParseError`
   with the parser's message and the error's byte range as a `Span`.
2. Allocate every statement and expression into the `Ast` arenas bottom-up, so
   child ids exist before the parent that references them.
3. Set the module body via `Ast::set_body`.
4. Map each `ruff` node to the corresponding `Stmt` / `Expr` variant from Task 4:
   - function definitions → `Stmt::FunctionDef`, with `is_async` set from
     whether it is an async definition, `name_span` covering **the identifier
     alone** (not the `def` keyword and not the body), `returns` from the return
     annotation, and parameters mapped to `Param`
   - class definitions → `Stmt::ClassDef`
   - assignments, annotated or not → `Stmt::Assign`
   - `return` → `Stmt::Return`
   - a bare expression → `Stmt::Expr`
   - `pass` → `Stmt::Pass`
   - **anything else → `Stmt::Unsupported` carrying that node's span**
   - names, attributes, calls, `await`, literals, list and dict displays →
     their `Expr` counterparts; **anything else → `Expr::Unsupported`**
5. Every span is the node's own byte range in `source`, converted to `u32`.

**Two things to get right, because they are the ones that cause silent wrongness
later rather than a test failure now:**

- **`name_span` must cover only the identifier.** If `ruff` exposes a range for
  the whole definition but not the name, compute the name's range from the
  identifier node — do not approximate it with the definition's start. The whole
  C3 family points carets at names, and an approximation here misaligns every
  one of them.
- **Do not collapse `await f()` into the call.** `Expr::Await` wrapping
  `Expr::Call` is exactly the distinction C1 exists to detect. If `await` is
  flattened, C1 cannot be written at all.

- [ ] **Step 6: Run the tests**

```bash
cargo test -p liar-core parse
```

Expected: 15 passed. If `modern_syntax_parses_without_error` fails, stop — the
parser choice recorded in the spec is wrong and needs revisiting before any more
is built on it.

- [ ] **Step 7: Record the decision and the API**

`docs/decisions/001-parser.md`:

```markdown
# 001 — Python parser

**Decision:** `ruff_python_parser`, pinned at `=0.0.13`.

**Date:** 2026-09-15

## Context

The engine needs to parse Python into an AST. Two crates were candidates.

| Crate | Latest | Last published |
|---|---|---|
| `rustpython-parser` | 0.4.0 | 2024-08-06 |
| `ruff_python_parser` | 0.0.13 | 2026-09-10 |

## Decision

`ruff_python_parser`. A parser last published in 2024 cannot parse two years of
newer Python syntax, and a linter that fails on syntax its targets already use is
worthless. This crate is what `ruff` itself runs on, it is a hand-written
recursive descent parser producing a genuine AST rather than a concrete syntax
tree, and it is published continuously.

Tree-sitter was rejected earlier: it is built for editors, is resilient to
broken input, and produces a tree mirroring source text. An AST is the better
tool for analysis.

## Consequences

Astral publish these crates at `0.0.x` as an implementation detail of `ruff`,
not as a stable public API. Documentation is thin and the API may break between
patch releases.

Mitigated by pinning the exact version and by confining every `ruff_*` type to
`crates/liar-core/src/ast/convert.rs`. An upgrade that breaks the API breaks one
module, against a test suite that already defines correct behaviour.

## The API, as of 0.0.13

<!-- Record the entry points established in Task 5 Step 1: the parse function,
     its return type, how to reach the module body, how nodes report ranges,
     and how syntax errors surface. Future-you upgrading this dependency will
     want exactly this list. -->
```

Fill the final section in from what you found in Step 1.

- [ ] **Step 8: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-core/src/ast/ docs/decisions/001-parser.md
git commit -m "Parse Python into the engine's syntax tree

ruff_python_parser is confined to convert.rs; no third-party parser type
appears in any signature elsewhere. That boundary is what makes pinning
an unstable 0.0.x crate acceptable, since an API break lands in one
module against tests that already define correct behaviour.

Syntax the engine does not yet model becomes Unsupported rather than an
error, so an unmodelled construct is skipped precisely instead of
failing the file. await is kept as a distinct node wrapping its call,
which is the distinction the first checker exists to detect."
```

---

## Task 6: Check ids

**Files:**
- Create: `crates/liar-core/src/check.rs`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `enum CheckId` (all variants from spec §5), `enum Severity`, with
  `CheckId::code() -> &'static str`, `CheckId::from_code(&str) -> Option<Self>`,
  `CheckId::ALL: &[CheckId]`, `CheckId::default_severity() -> Severity`.

- [ ] **Step 1: Write the failing tests**

`crates/liar-core/src/check.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for &check in CheckId::ALL {
            let code = check.code();
            assert_eq!(CheckId::from_code(code), Some(check), "round trip failed for {code}");
        }
    }

    #[test]
    fn codes_are_unique() {
        let mut codes: Vec<_> = CheckId::ALL.iter().map(|c| c.code()).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before, "duplicate check code");
    }

    #[test]
    fn all_contains_every_variant() {
        // ALL is hand-written, so it can drift from the enum. Comparing against
        // the count here means adding a variant without adding it to ALL fails
        // a test rather than silently disabling the check.
        assert_eq!(CheckId::ALL.len(), 10);
    }

    #[test]
    fn unknown_codes_are_rejected() {
        assert_eq!(CheckId::from_code("C99"), None);
        assert_eq!(CheckId::from_code(""), None);
        assert_eq!(CheckId::from_code("c1"), None, "codes are case-sensitive");
    }

    #[test]
    fn the_spec_codes_are_present() {
        for code in ["C1", "C2", "C3a", "C3b", "C3c", "C3d", "C3e", "C3f", "C4", "C5"] {
            assert!(CheckId::from_code(code).is_some(), "missing check {code}");
        }
    }

    #[test]
    fn checks_sort_by_code() {
        let mut checks = vec![CheckId::C4, CheckId::C1, CheckId::C3a];
        checks.sort();
        assert_eq!(checks, vec![CheckId::C1, CheckId::C3a, CheckId::C4]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-core check
```

Expected: compile error, `CheckId` not found.

- [ ] **Step 3: Implement**

Above the tests in `crates/liar-core/src/check.rs`:

```rust
//! The catalogue of checks.
//!
//! Codes are public API the moment anyone writes `# liar: ignore[C1]` in their
//! source. They never change meaning; a retired check's code is retired with it.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Severity {
    /// Almost certainly wrong. The code does not do what it says.
    Error,
    /// Probably wrong, or wrong in a way that depends on context.
    Warning,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum CheckId {
    /// An async function called without `await`.
    C1,
    /// A blocking call inside async code.
    C2,
    /// A boolean-shaped name whose type is not boolean.
    C3a,
    /// A quantity-shaped name whose type is not numeric.
    C3b,
    /// A plural name holding a scalar, or a singular name holding a collection.
    C3c,
    /// A `get_*` function that mutates state.
    C3d,
    /// A meaningless name in a scope large enough to matter.
    C3e,
    /// One name bound to several unrelated meanings in a file.
    C3f,
    /// A resource not released on every path out.
    C4,
    /// A docstring contradicted by the code.
    C5,
}

impl CheckId {
    pub const ALL: &'static [CheckId] = &[
        CheckId::C1,
        CheckId::C2,
        CheckId::C3a,
        CheckId::C3b,
        CheckId::C3c,
        CheckId::C3d,
        CheckId::C3e,
        CheckId::C3f,
        CheckId::C4,
        CheckId::C5,
    ];

    pub fn code(self) -> &'static str {
        match self {
            CheckId::C1 => "C1",
            CheckId::C2 => "C2",
            CheckId::C3a => "C3a",
            CheckId::C3b => "C3b",
            CheckId::C3c => "C3c",
            CheckId::C3d => "C3d",
            CheckId::C3e => "C3e",
            CheckId::C3f => "C3f",
            CheckId::C4 => "C4",
            CheckId::C5 => "C5",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        CheckId::ALL.iter().copied().find(|c| c.code() == code)
    }

    pub fn default_severity(self) -> Severity {
        match self {
            // The code provably does not run, or provably leaks.
            CheckId::C1 | CheckId::C4 => Severity::Error,
            // Real, but contextual.
            _ => Severity::Warning,
        }
    }
}
```

- [ ] **Step 4: Register and run**

Add `pub mod check;` and `pub use check::{CheckId, Severity};` to `lib.rs`, then:

```bash
cargo test -p liar-core check
```

Expected: 6 passed.

- [ ] **Step 5: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-core/src/check.rs crates/liar-core/src/lib.rs
git commit -m "Add the check catalogue

Codes become public API the moment a user writes an ignore comment, so
they are tested for uniqueness and round-tripping. ALL is hand-written
and therefore tested against an expected count, so adding a variant
without registering it fails a test rather than silently disabling the
check.

C1 and C4 default to error severity because both are provable: the code
does not run, or the handle leaks. Everything else defaults to warning."
```

---

## Task 7: Findings

**Files:**
- Create: `crates/liar-core/src/finding.rs`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: `CheckId`, `Severity` (Task 6); `Span` (Task 3); `FileId` (Task 2).
- Produces: `struct Label { file: FileId, span: Span, note: Option<String> }`,
  `struct Finding { check, severity, primary: Label, secondary: Vec<Label>, args: Vec<(String, String)> }`,
  with `Finding::new(CheckId, FileId, Span)`, `.with_severity(…)`, `.with_note(…)`,
  `.with_secondary(…)`, `.with_arg(…)`, and
  `fn sort_findings(findings: &mut [Finding], sources: &SourceMap)`.

- [ ] **Step 1: Write the failing tests**

`crates/liar-core/src/finding.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;
    use std::path::PathBuf;

    fn map_with(files: &[(&str, &str)]) -> (SourceMap, Vec<FileId>) {
        let mut map = SourceMap::new();
        let ids = files
            .iter()
            .map(|(path, text)| map.add(PathBuf::from(path), (*text).to_string()))
            .collect();
        (map, ids)
    }

    #[test]
    fn a_finding_takes_its_severity_from_its_check() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C1, ids[0], Span::new(0, 1));
        assert_eq!(f.severity, Severity::Error);
    }

    #[test]
    fn severity_can_be_overridden() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C1, ids[0], Span::new(0, 1))
            .with_severity(Severity::Warning);
        assert_eq!(f.severity, Severity::Warning);
    }

    #[test]
    fn arguments_are_recorded_in_insertion_order() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C3f, ids[0], Span::new(0, 1))
            .with_arg("name", "data")
            .with_arg("n", "4");
        assert_eq!(f.args, vec![
            ("name".to_string(), "data".to_string()),
            ("n".to_string(), "4".to_string()),
        ]);
    }

    #[test]
    fn findings_sort_by_file_then_line_then_column_then_check() {
        let (map, ids) = map_with(&[("a.py", "one\ntwo\n"), ("b.py", "three\n")]);
        let (a, b) = (ids[0], ids[1]);

        let mut findings = vec![
            Finding::new(CheckId::C1, b, Span::new(0, 1)),   // b.py 1:1
            Finding::new(CheckId::C4, a, Span::new(4, 5)),   // a.py 2:1 C4
            Finding::new(CheckId::C1, a, Span::new(4, 5)),   // a.py 2:1 C1
            Finding::new(CheckId::C1, a, Span::new(1, 2)),   // a.py 1:2
            Finding::new(CheckId::C1, a, Span::new(0, 1)),   // a.py 1:1
        ];
        sort_findings(&mut findings, &map);

        let order: Vec<_> = findings
            .iter()
            .map(|f| {
                let file = map.get(f.primary.file).path().display().to_string();
                let pos = map.get(f.primary.file).position(f.primary.span.start);
                (file, pos.line, pos.column, f.check.code())
            })
            .collect();

        assert_eq!(order, vec![
            ("a.py".to_string(), 1, 1, "C1"),
            ("a.py".to_string(), 1, 2, "C1"),
            ("a.py".to_string(), 2, 1, "C1"),
            ("a.py".to_string(), 2, 1, "C4"),
            ("b.py".to_string(), 1, 1, "C1"),
        ]);
    }

    #[test]
    fn sorting_is_stable_across_input_orders() {
        // §9.8: the corpus baselines are meaningless if output order depends on
        // the order findings happened to be produced.
        let (map, ids) = map_with(&[("a.py", "one\ntwo\n")]);
        let a = ids[0];

        let make = || vec![
            Finding::new(CheckId::C4, a, Span::new(4, 5)),
            Finding::new(CheckId::C1, a, Span::new(0, 1)),
            Finding::new(CheckId::C1, a, Span::new(4, 5)),
        ];

        let mut forward = make();
        let mut reversed = make();
        reversed.reverse();

        sort_findings(&mut forward, &map);
        sort_findings(&mut reversed, &map);

        let key = |fs: &[Finding]| -> Vec<(u32, u32, &'static str)> {
            fs.iter()
                .map(|f| {
                    let p = map.get(f.primary.file).position(f.primary.span.start);
                    (p.line, p.column, f.check.code())
                })
                .collect()
        };
        assert_eq!(key(&forward), key(&reversed));
    }

    #[test]
    fn secondary_labels_are_attached() {
        let (_, ids) = map_with(&[("a.py", "one\ntwo\n")]);
        let f = Finding::new(CheckId::C3f, ids[0], Span::new(0, 3))
            .with_secondary(ids[0], Span::new(4, 7), Some("and here".into()));
        assert_eq!(f.secondary.len(), 1);
        assert_eq!(f.secondary[0].note.as_deref(), Some("and here"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-core finding
```

Expected: compile error, `Finding` not found.

- [ ] **Step 3: Implement**

Above the tests in `crates/liar-core/src/finding.rs`:

```rust
//! What a check produces.

use crate::check::{CheckId, Severity};
use crate::ids::FileId;
use crate::source::SourceMap;
use crate::span::Span;

/// A span worth pointing at, with an optional note.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Label {
    pub file: FileId,
    pub span: Span,
    pub note: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finding {
    pub check: CheckId,
    pub severity: Severity,
    /// The span the diagnostic is about. Underlined.
    pub primary: Label,
    /// Related spans, shown beneath. C3f uses these to show every occurrence
    /// of a name in one finding rather than emitting several.
    pub secondary: Vec<Label>,
    /// Values interpolated into the message. Order-preserving rather than a
    /// map, so rendering is deterministic.
    pub args: Vec<(String, String)>,
}

impl Finding {
    pub fn new(check: CheckId, file: FileId, span: Span) -> Self {
        Self {
            check,
            severity: check.default_severity(),
            primary: Label { file, span, note: None },
            secondary: Vec::new(),
            args: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.primary.note = Some(note.into());
        self
    }

    #[must_use]
    pub fn with_secondary(mut self, file: FileId, span: Span, note: Option<String>) -> Self {
        self.secondary.push(Label { file, span, note });
        self
    }

    #[must_use]
    pub fn with_arg(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.push((key.into(), value.into()));
        self
    }
}

/// Sorts findings into the order they are always reported in: by file path,
/// then line, then column, then check code.
///
/// Rendering order must not depend on the order checks happened to run, or the
/// corpus baselines in §9.6 compare noise.
pub fn sort_findings(findings: &mut [Finding], sources: &SourceMap) {
    findings.sort_by(|a, b| {
        let fa = sources.get(a.primary.file);
        let fb = sources.get(b.primary.file);
        let pa = fa.position(a.primary.span.start);
        let pb = fb.position(b.primary.span.start);

        fa.path()
            .cmp(fb.path())
            .then(pa.line.cmp(&pb.line))
            .then(pa.column.cmp(&pb.column))
            .then(a.check.cmp(&b.check))
            .then(a.primary.span.end.cmp(&b.primary.span.end))
    });
}
```

- [ ] **Step 4: Register and run**

Add `pub mod finding;` and `pub use finding::{Finding, Label, sort_findings};`
to `lib.rs`, then:

```bash
cargo test -p liar-core finding
```

Expected: 6 passed.

- [ ] **Step 5: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-core/src/finding.rs crates/liar-core/src/lib.rs
git commit -m "Add findings and a total ordering over them

Findings sort by file, line, column, then check code, with span end as a
final tiebreak so the order is total rather than merely consistent.
Tested by sorting the same set from two different input orders and
comparing, since corpus baselines compare noise if output order depends
on the order checks happened to run.

Secondary labels let one finding point at several spans, which is how
C3f reports four variables sharing a name as one finding rather than
four."
```

---

## Task 8: Tone and the message table

The voice lives in one data file, three variants per check. Scattering these
through the checkers is how a tool with personality drifts into inconsistency
(§8.5).

**Files:**
- Create: `data/messages.toml`
- Create: `crates/liar-core/src/messages.rs`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: `CheckId` (Task 6), `Finding` (Task 7).
- Produces: `enum Tone { Professional, Dry, Brutal }` with `FromStr`;
  `struct MessageTable` with `load(&str) -> Result<Self, MessageError>`,
  `embedded() -> &'static MessageTable`, `render(&Finding, Tone) -> String`.

- [ ] **Step 1: Write the message data**

`data/messages.toml`. Placeholders are `{name}`-style and must match the `args`
each check supplies.

```toml
# The tool's voice. Three tones per check, no exceptions.
#
# The humour comes from being accurate, not from jokes. "four variables called
# data in this file. none are related." is funny because it is true and
# specific. A gag is funny once and irritating on the two-hundredth run.
#
# The professional tone must be genuinely usable at work. That is both a
# kindness and, in itself, the joke.

[C1]
professional = "coroutine '{name}' is called but never awaited"
dry          = "'{name}' is never awaited. this line does nothing."
brutal       = "you wrote '{name}()' and then threw it away."

[C2]
professional = "blocking call '{name}' inside an async function"
dry          = "'{name}' blocks. every other task waits for it."
brutal       = "'{name}' here stops the entire event loop. all of it."

[C3a]
professional = "'{name}' is named as a boolean but its type is {ty}"
dry          = "'{name}' returns {ty}. one of us is confused."
brutal       = "'{name}' is not a question if the answer is a {ty}."

[C3b]
professional = "'{name}' is named as a quantity but its type is {ty}"
dry          = "'{name}' is a {ty}. a count is a number."
brutal       = "counting is done with numbers, not with a {ty}."

[C3c]
professional = "'{name}' is {name_number} but its value is {value_number}"
dry          = "'{name}' is {name_number}. its value is {value_number}."
brutal       = "pick one: is '{name}' {name_number} or {value_number}?"

[C3d]
professional = "'{name}' is named as an accessor but mutates {target}"
dry          = "'{name}' says get. it writes to {target}."
brutal       = "'{name}' is not getting anything. it is changing {target}."

[C3e]
professional = "'{name}' is uninformative across {lines} lines of scope"
dry          = "'{name}' tells the reader nothing, for {lines} lines."
brutal       = "{lines} lines of '{name}'. name it something."

[C3f]
professional = "'{name}' is bound {n} times in this file with {k} different types"
dry          = "{n} variables called '{name}' in this file. none are related."
brutal       = "{n} things called '{name}'. pick a lane."

[C4]
professional = "'{name}' is not released on every path out of this function"
dry          = "'{name}' is released on {released} of {total} paths."
brutal       = "'{name}' leaks the moment anything raises."

[C5]
professional = "the docstring of '{name}' does not match its implementation: {detail}"
dry          = "the docstring says {detail}. the code disagrees."
brutal       = "the docstring for '{name}' is fiction. {detail}"
```

- [ ] **Step 2: Write the failing tests**

`crates/liar-core/src/messages.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{FileId, Id};
    use crate::span::Span;

    fn finding(check: CheckId, args: &[(&str, &str)]) -> Finding {
        let mut f = Finding::new(check, FileId::from_index(0), Span::new(0, 1));
        for (k, v) in args {
            f = f.with_arg(*k, *v);
        }
        f
    }

    #[test]
    fn every_check_has_every_tone() {
        // The guard that stops a new check shipping with a missing voice.
        let table = MessageTable::embedded();
        for &check in CheckId::ALL {
            for tone in [Tone::Professional, Tone::Dry, Tone::Brutal] {
                assert!(
                    table.template(check, tone).is_some(),
                    "{} has no {tone:?} message",
                    check.code()
                );
            }
        }
    }

    #[test]
    fn no_template_is_empty() {
        let table = MessageTable::embedded();
        for &check in CheckId::ALL {
            for tone in [Tone::Professional, Tone::Dry, Tone::Brutal] {
                assert!(!table.template(check, tone).unwrap().trim().is_empty());
            }
        }
    }

    #[test]
    fn placeholders_are_interpolated() {
        let table = MessageTable::embedded();
        let f = finding(CheckId::C3f, &[("name", "data"), ("n", "4"), ("k", "3")]);
        let rendered = table.render(&f, Tone::Dry);
        assert_eq!(rendered, "4 variables called 'data' in this file. none are related.");
    }

    #[test]
    fn each_tone_renders_differently() {
        let table = MessageTable::embedded();
        let f = finding(CheckId::C3f, &[("name", "data"), ("n", "4"), ("k", "3")]);
        let p = table.render(&f, Tone::Professional);
        let d = table.render(&f, Tone::Dry);
        let b = table.render(&f, Tone::Brutal);
        assert_ne!(p, d);
        assert_ne!(d, b);
        assert_ne!(p, b);
    }

    #[test]
    fn a_missing_argument_is_an_error_not_a_silent_blank() {
        // A message reading "variables called  in this file" would ship
        // unnoticed. Failing loudly in tests is the point.
        let table = MessageTable::embedded();
        let f = finding(CheckId::C3f, &[("name", "data")]); // n and k missing
        let err = table.try_render(&f, Tone::Dry).expect_err("expected an error");
        assert!(err.to_string().contains('n'), "error should name the missing placeholder");
    }

    #[test]
    fn loading_rejects_a_check_with_a_missing_tone() {
        let toml = r#"
            [C1]
            professional = "a"
            dry = "b"
        "#;
        let err = MessageTable::load(toml).expect_err("expected an error");
        assert!(err.to_string().contains("brutal"));
    }

    #[test]
    fn loading_rejects_an_unknown_check_code() {
        let toml = r#"
            [C99]
            professional = "a"
            dry = "b"
            brutal = "c"
        "#;
        let err = MessageTable::load(toml).expect_err("expected an error");
        assert!(err.to_string().contains("C99"));
    }

    #[test]
    fn tone_parses_from_its_name() {
        assert_eq!("dry".parse::<Tone>().unwrap(), Tone::Dry);
        assert_eq!("professional".parse::<Tone>().unwrap(), Tone::Professional);
        assert_eq!("brutal".parse::<Tone>().unwrap(), Tone::Brutal);
        assert!("sarcastic".parse::<Tone>().is_err());
    }

    #[test]
    fn the_default_tone_is_dry() {
        assert_eq!(Tone::default(), Tone::Dry);
    }
}
```

- [ ] **Step 3: Run to verify failure**

```bash
cargo test -p liar-core messages
```

Expected: compile error, `MessageTable` not found.

- [ ] **Step 4: Implement**

Above the tests in `crates/liar-core/src/messages.rs`:

```rust
//! The tool's voice, in one place.

use crate::check::CheckId;
use crate::finding::Finding;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Tone {
    /// Usable at work without explaining yourself.
    Professional,
    /// Deadpan and specific. The default.
    #[default]
    Dry,
    /// Blunter. Still true.
    Brutal,
}

impl Tone {
    fn key(self) -> &'static str {
        match self {
            Tone::Professional => "professional",
            Tone::Dry => "dry",
            Tone::Brutal => "brutal",
        }
    }
}

impl FromStr for Tone {
    type Err = MessageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "professional" => Ok(Tone::Professional),
            "dry" => Ok(Tone::Dry),
            "brutal" => Ok(Tone::Brutal),
            other => Err(MessageError::UnknownTone(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("unknown tone '{0}', expected professional, dry or brutal")]
    UnknownTone(String),
    #[error("unknown check code '{0}'")]
    UnknownCheck(String),
    #[error("check {check} has no '{tone}' message")]
    MissingTone { check: String, tone: String },
    #[error("message for {check} needs a value for '{placeholder}' but none was supplied")]
    MissingArgument { check: String, placeholder: String },
    #[error("could not parse the message table: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug)]
pub struct MessageTable {
    templates: HashMap<(CheckId, Tone), String>,
}

const EMBEDDED: &str = include_str!("../../../data/messages.toml");

impl MessageTable {
    /// The table compiled into the binary, so the tool has a voice without
    /// needing its data directory alongside it.
    pub fn embedded() -> &'static MessageTable {
        static TABLE: OnceLock<MessageTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            MessageTable::load(EMBEDDED).expect("the embedded message table must be valid")
        })
    }

    pub fn load(source: &str) -> Result<Self, MessageError> {
        let raw: HashMap<String, HashMap<String, String>> = toml::from_str(source)?;
        let mut templates = HashMap::new();

        for (code, tones) in raw {
            let check =
                CheckId::from_code(&code).ok_or_else(|| MessageError::UnknownCheck(code.clone()))?;

            for tone in [Tone::Professional, Tone::Dry, Tone::Brutal] {
                let template = tones.get(tone.key()).ok_or_else(|| MessageError::MissingTone {
                    check: code.clone(),
                    tone: tone.key().to_string(),
                })?;
                templates.insert((check, tone), template.clone());
            }
        }

        Ok(Self { templates })
    }

    pub fn template(&self, check: CheckId, tone: Tone) -> Option<&str> {
        self.templates.get(&(check, tone)).map(String::as_str)
    }

    /// # Panics
    /// If a placeholder has no matching argument. Use [`try_render`] where that
    /// is recoverable; inside the engine it is a bug in the check.
    ///
    /// [`try_render`]: MessageTable::try_render
    pub fn render(&self, finding: &Finding, tone: Tone) -> String {
        self.try_render(finding, tone).expect("message rendering failed")
    }

    pub fn try_render(&self, finding: &Finding, tone: Tone) -> Result<String, MessageError> {
        let template = self
            .template(finding.check, tone)
            .ok_or_else(|| MessageError::MissingTone {
                check: finding.check.code().to_string(),
                tone: tone.key().to_string(),
            })?;

        let mut out = String::with_capacity(template.len());
        let mut rest = template;

        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let close = after.find('}').ok_or_else(|| MessageError::MissingArgument {
                check: finding.check.code().to_string(),
                placeholder: after.to_string(),
            })?;
            let placeholder = &after[..close];

            let value = finding
                .args
                .iter()
                .find(|(k, _)| k == placeholder)
                .map(|(_, v)| v.as_str())
                .ok_or_else(|| MessageError::MissingArgument {
                    check: finding.check.code().to_string(),
                    placeholder: placeholder.to_string(),
                })?;

            out.push_str(value);
            rest = &after[close + 1..];
        }
        out.push_str(rest);

        Ok(out)
    }
}
```

- [ ] **Step 5: Register and run**

Add `pub mod messages;` and `pub use messages::{MessageTable, Tone};` to
`lib.rs`, then:

```bash
cargo test -p liar-core messages
```

Expected: 9 passed. `every_check_has_every_tone` failing means `data/messages.toml`
is missing an entry — the intended behaviour.

- [ ] **Step 6: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add data/messages.toml crates/liar-core/src/messages.rs crates/liar-core/src/lib.rs
git commit -m "Put the tool's voice in one data file

Three tones for every check, loaded from data/messages.toml and embedded
in the binary. Keeping them together means the voice can be reviewed in
one sitting; scattering them through the checkers is how a tool with
personality drifts into inconsistency.

Loading rejects a check missing a tone or an unrecognised code, and
rendering treats a placeholder with no argument as an error rather than
substituting a blank, since a message reading 'variables called  in this
file' would otherwise ship unnoticed."
```

---

## Task 9: Config

**Files:**
- Create: `crates/liar-cli/src/config.rs`
- Modify: `crates/liar-cli/src/main.rs`

**Interfaces:**
- Consumes: `CheckId` (Task 6), `Tone` (Task 8).
- Produces: `struct Config { tone: Tone, enabled: Vec<CheckId>, ignore: Vec<String>, scope_threshold: u32 }`
  with `Config::default()`, `Config::from_toml(&str) -> Result<Self, ConfigError>`,
  `Config::load(Option<&Path>) -> Result<Self, ConfigError>`,
  `Config::is_enabled(CheckId) -> bool`.

- [ ] **Step 1: Write the failing tests**

`crates/liar-cli/src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_dry_tone_and_every_check_enabled() {
        let c = Config::default();
        assert_eq!(c.tone, Tone::Dry);
        for &check in CheckId::ALL {
            assert!(c.is_enabled(check), "{} should be enabled by default", check.code());
        }
    }

    #[test]
    fn an_empty_config_equals_the_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn tone_is_read_from_the_file() {
        let c = Config::from_toml(r#"tone = "brutal""#).unwrap();
        assert_eq!(c.tone, Tone::Brutal);
    }

    #[test]
    fn an_invalid_tone_is_rejected() {
        let err = Config::from_toml(r#"tone = "sarcastic""#).expect_err("expected an error");
        assert!(err.to_string().contains("sarcastic"));
    }

    #[test]
    fn select_restricts_the_enabled_checks() {
        let c = Config::from_toml(r#"select = ["C1", "C4"]"#).unwrap();
        assert!(c.is_enabled(CheckId::C1));
        assert!(c.is_enabled(CheckId::C4));
        assert!(!c.is_enabled(CheckId::C2));
    }

    #[test]
    fn ignore_removes_checks_from_the_default_set() {
        let c = Config::from_toml(r#"ignore = ["C5"]"#).unwrap();
        assert!(!c.is_enabled(CheckId::C5));
        assert!(c.is_enabled(CheckId::C1));
    }

    #[test]
    fn ignore_applies_after_select() {
        let c = Config::from_toml(
            r#"
            select = ["C1", "C4"]
            ignore = ["C4"]
            "#,
        )
        .unwrap();
        assert!(c.is_enabled(CheckId::C1));
        assert!(!c.is_enabled(CheckId::C4));
    }

    #[test]
    fn an_unknown_check_code_is_rejected() {
        let err = Config::from_toml(r#"select = ["C99"]"#).expect_err("expected an error");
        assert!(err.to_string().contains("C99"));
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        // A silently ignored typo is a setting the user believes is applied.
        let err = Config::from_toml(r#"toen = "dry""#).expect_err("expected an error");
        assert!(err.to_string().contains("toen"));
    }

    #[test]
    fn the_scope_threshold_is_configurable() {
        let c = Config::from_toml("scope-threshold = 40").unwrap();
        assert_eq!(c.scope_threshold, 40);
        assert_eq!(Config::default().scope_threshold, 20);
    }

    #[test]
    fn exclude_globs_are_read() {
        let c = Config::from_toml(r#"exclude = ["tests/**", "build/*"]"#).unwrap();
        assert_eq!(c.exclude, vec!["tests/**".to_string(), "build/*".to_string()]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-cli config
```

Expected: compile error, `Config` not found.

- [ ] **Step 3: Implement**

Above the tests in `crates/liar-cli/src/config.rs`:

```rust
//! liar.toml.

use liar_core::check::CheckId;
use liar_core::messages::Tone;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not parse the configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("unknown check code '{0}'")]
    UnknownCheck(String),
    #[error("unknown tone '{0}', expected professional, dry or brutal")]
    UnknownTone(String),
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The file as written. Separate from `Config` so `deny_unknown_fields` catches
/// typos and so string fields can be validated into real types.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawConfig {
    tone: Option<String>,
    select: Option<Vec<String>>,
    ignore: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    scope_threshold: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    pub tone: Tone,
    pub enabled: Vec<CheckId>,
    pub exclude: Vec<String>,
    /// C3e: how many lines a scope must span before an uninformative name in it
    /// is worth mentioning. Short names in short scopes are good style.
    pub scope_threshold: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tone: Tone::Dry,
            enabled: CheckId::ALL.to_vec(),
            exclude: Vec::new(),
            scope_threshold: 20,
        }
    }
}

impl Config {
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(source)?;
        let defaults = Config::default();

        let tone = match raw.tone {
            Some(t) => t.parse().map_err(|_| ConfigError::UnknownTone(t))?,
            None => defaults.tone,
        };

        let parse_codes = |codes: Vec<String>| -> Result<Vec<CheckId>, ConfigError> {
            codes
                .into_iter()
                .map(|c| CheckId::from_code(&c).ok_or(ConfigError::UnknownCheck(c)))
                .collect()
        };

        let selected = match raw.select {
            Some(codes) => parse_codes(codes)?,
            None => defaults.enabled,
        };
        let ignored = match raw.ignore {
            Some(codes) => parse_codes(codes)?,
            None => Vec::new(),
        };

        let enabled = selected.into_iter().filter(|c| !ignored.contains(c)).collect();

        Ok(Self {
            tone,
            enabled,
            exclude: raw.exclude.unwrap_or(defaults.exclude),
            scope_threshold: raw.scope_threshold.unwrap_or(defaults.scope_threshold),
        })
    }

    /// Loads from `path`, or returns defaults when `path` is `None`.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        match path {
            None => Ok(Config::default()),
            Some(p) => {
                let text = std::fs::read_to_string(p).map_err(|source| ConfigError::Io {
                    path: p.display().to_string(),
                    source,
                })?;
                Config::from_toml(&text)
            }
        }
    }

    pub fn is_enabled(&self, check: CheckId) -> bool {
        self.enabled.contains(&check)
    }
}
```

`liar_core::messages::Tone` must implement `FromStr` returning an error type;
Task 8 provides it. Add `mod config;` to `main.rs`.

- [ ] **Step 4: Run the tests**

```bash
cargo test -p liar-cli config
```

Expected: 11 passed.

- [ ] **Step 5: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-cli/src/config.rs crates/liar-cli/src/main.rs
git commit -m "Add liar.toml configuration

Parsed into a raw struct with deny_unknown_fields first, then validated
into real types, so a typo like 'toen' is rejected rather than silently
ignored. A silently ignored setting is one the user believes is applied.

ignore applies after select, so a project can select a small set and
still drop one from it. The C3e scope threshold is configurable because
short names in short scopes are good style, and a tool that does not
know that gets uninstalled."
```

---

## Task 10: Rendering

**Files:**
- Create: `crates/liar-cli/src/render.rs`
- Modify: `crates/liar-cli/src/main.rs`

**Interfaces:**
- Consumes: `Finding`, `SourceMap`, `MessageTable`, `Tone`, `Severity`.
- Produces: `fn render(findings: &[Finding], sources: &SourceMap, messages: &MessageTable, tone: Tone) -> String`.

- [ ] **Step 1: Write the failing snapshot tests**

`crates/liar-cli/src/render.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use liar_core::check::CheckId;
    use liar_core::finding::Finding;
    use liar_core::messages::MessageTable;
    use liar_core::source::SourceMap;
    use liar_core::span::Span;
    use std::path::PathBuf;

    fn setup(text: &str) -> (SourceMap, liar_core::ids::FileId) {
        let mut map = SourceMap::new();
        let id = map.add(PathBuf::from("example.py"), text.to_string());
        (map, id)
    }

    #[test]
    fn nothing_renders_to_nothing() {
        let (map, _) = setup("x = 1\n");
        let out = render(&[], &map, MessageTable::embedded(), Tone::Dry);
        assert!(out.is_empty());
    }

    #[test]
    fn a_single_finding_renders() {
        let (map, file) = setup("async def f():\n    save_user(u)\n");
        let finding = Finding::new(CheckId::C1, file, Span::new(19, 31)).with_arg("name", "save_user");
        let out = render(&[finding], &map, MessageTable::embedded(), Tone::Dry);
        insta::assert_snapshot!(out);
    }

    #[test]
    fn each_tone_renders_its_own_message() {
        let (map, file) = setup("async def f():\n    save_user(u)\n");
        let make = || Finding::new(CheckId::C1, file, Span::new(19, 31)).with_arg("name", "save_user");
        for tone in [Tone::Professional, Tone::Dry, Tone::Brutal] {
            let out = render(&[make()], &map, MessageTable::embedded(), tone);
            insta::assert_snapshot!(format!("tone_{tone:?}"), out);
        }
    }

    #[test]
    fn secondary_labels_render_beneath_the_primary() {
        let (map, file) = setup("data = 1\ndata = []\ndata = 'x'\n");
        let finding = Finding::new(CheckId::C3f, file, Span::new(0, 4))
            .with_arg("name", "data")
            .with_arg("n", "3")
            .with_arg("k", "3")
            .with_secondary(file, Span::new(9, 13), None)
            .with_secondary(file, Span::new(19, 23), None);
        let out = render(&[finding], &map, MessageTable::embedded(), Tone::Dry);
        insta::assert_snapshot!(out);
    }

    #[test]
    fn a_caret_on_a_line_with_multibyte_characters_aligns() {
        // If columns were counted in bytes, the caret would sit three columns
        // to the right of the name.
        let (map, file) = setup("# héllo ünïcode\nsave_user(u)\n");
        let finding = Finding::new(CheckId::C1, file, Span::new(19, 31)).with_arg("name", "save_user");
        let out = render(&[finding], &map, MessageTable::embedded(), Tone::Dry);
        insta::assert_snapshot!(out);
    }

    #[test]
    fn findings_render_in_sorted_order() {
        let (map, file) = setup("a()\nb()\n");
        let findings = vec![
            Finding::new(CheckId::C1, file, Span::new(4, 7)).with_arg("name", "b"),
            Finding::new(CheckId::C1, file, Span::new(0, 3)).with_arg("name", "a"),
        ];
        let out = render(&findings, &map, MessageTable::embedded(), Tone::Dry);
        let a_at = out.find("'a'").expect("expected a finding for a");
        let b_at = out.find("'b'").expect("expected a finding for b");
        assert!(a_at < b_at, "findings should render in source order");
    }

    #[test]
    fn rendering_is_deterministic() {
        let (map, file) = setup("a()\n");
        let make = || vec![Finding::new(CheckId::C1, file, Span::new(0, 3)).with_arg("name", "a")];
        let first = render(&make(), &map, MessageTable::embedded(), Tone::Dry);
        let second = render(&make(), &map, MessageTable::embedded(), Tone::Dry);
        assert_eq!(first, second);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-cli render
```

Expected: compile error, `render` not found.

- [ ] **Step 3: Implement**

Above the tests in `crates/liar-cli/src/render.rs`. Consult
`cargo doc -p annotate-snippets --open` for the exact builder names in 0.12; the
shape below is the structure to produce.

```rust
//! rustc-style diagnostic rendering.

use liar_core::check::Severity;
use liar_core::finding::Finding;
use liar_core::messages::{MessageTable, Tone};
use liar_core::source::SourceMap;

/// Renders findings as human-readable diagnostics.
///
/// Input is sorted before rendering, so callers need not have sorted it and the
/// output is a function of the finding set alone (§9.8).
pub fn render(
    findings: &[Finding],
    sources: &SourceMap,
    messages: &MessageTable,
    tone: Tone,
) -> String {
    if findings.is_empty() {
        return String::new();
    }

    let mut sorted = findings.to_vec();
    liar_core::finding::sort_findings(&mut sorted, sources);

    let mut out = String::new();
    for finding in &sorted {
        out.push_str(&render_one(finding, sources, messages, tone));
        out.push('\n');
    }
    out
}

fn render_one(
    finding: &Finding,
    sources: &SourceMap,
    messages: &MessageTable,
    tone: Tone,
) -> String {
    let file = sources.get(finding.primary.file);
    let message = messages.render(finding, tone);
    let level = match finding.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };

    // Build an annotate-snippets Message with:
    //   - level and the check code as the message id, e.g. error[C1]
    //   - the rendered message as the title
    //   - a snippet over file.text(), origin file.path()
    //   - a primary annotation over finding.primary.span
    //   - one annotation per secondary label, carrying its note where present
    // then render it to a String.
    //
    // annotate-snippets takes byte offsets into the source and does its own
    // character counting, so pass spans through unchanged — do not pre-convert
    // to the columns from SourceFile::position. Those exist for SARIF and the
    // LSP, which want line/column pairs.
    todo!("build and render the annotate-snippets Message as described above")
}
```

Replace the `todo!` with the real builder calls. Keep the comment above it — it
is the specification of what the code must produce, and it survives an
`annotate-snippets` upgrade changing the spelling.

- [ ] **Step 4: Run and review the snapshots**

```bash
cargo test -p liar-cli render
```

First run: `insta` writes `.snap.new` files and fails. Review them:

```bash
cargo insta review
```

Read each rendering properly before accepting. This is the tool's face — check
that carets sit under the right text, that the multibyte case aligns, and that
each tone reads the way it should. Accepting a wrong snapshot bakes the bug in.

- [ ] **Step 5: Re-run to confirm green**

```bash
cargo test -p liar-cli render
```

Expected: 7 passed.

- [ ] **Step 6: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-cli/src/render.rs crates/liar-cli/src/snapshots crates/liar-cli/src/main.rs
git commit -m "Render findings as rustc-style diagnostics

Snapshot-tested in all three tones so neither the formatting nor the
voice can drift unnoticed. One snapshot covers a caret on a line
containing multibyte characters, which is where byte-counted columns
would put the underline in the wrong place.

render sorts its input rather than trusting callers, so output is a
function of the finding set alone."
```

---

## Task 11: The CLI

**Files:**
- Create: `crates/liar-cli/src/discover.rs`
- Modify: `crates/liar-cli/src/main.rs`
- Create: `crates/liar-cli/tests/cli.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: the `liar` binary —
  `liar check <PATHS>... [--tone <TONE>] [--config <FILE>]`.
  Exit codes: `0` no findings, `1` findings, `2` an error.

- [ ] **Step 1: Write the failing discovery tests**

`crates/liar-cli/src/discover.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for path in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "pass\n").unwrap();
        }
        dir
    }

    fn names(paths: &[std::path::PathBuf], root: &std::path::Path) -> Vec<String> {
        paths
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn a_single_file_is_returned_as_is() {
        let dir = tree(&["a.py"]);
        let found = discover(&[dir.path().join("a.py")], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn a_directory_is_walked_recursively() {
        let dir = tree(&["a.py", "pkg/b.py", "pkg/sub/c.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py", "pkg/b.py", "pkg/sub/c.py"]);
    }

    #[test]
    fn non_python_files_are_skipped() {
        let dir = tree(&["a.py", "readme.md", "b.pyc"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn pyi_stubs_are_included() {
        let dir = tree(&["a.py", "a.pyi"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py", "a.pyi"]);
    }

    #[test]
    fn exclude_globs_filter_results() {
        let dir = tree(&["a.py", "tests/b.py", "tests/sub/c.py"]);
        let found = discover(&[dir.path().to_path_buf()], &["tests/**".into()]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn results_are_sorted_regardless_of_filesystem_order() {
        // §9.8: discovery order must not leak into output order.
        let dir = tree(&["z.py", "a.py", "m.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py", "m.py", "z.py"]);
    }

    #[test]
    fn duplicates_are_removed() {
        let dir = tree(&["a.py"]);
        let path = dir.path().join("a.py");
        let found = discover(&[path.clone(), path], &[]).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_missing_path_is_an_error() {
        let dir = tree(&[]);
        let err = discover(&[dir.path().join("nope.py")], &[]).expect_err("expected an error");
        assert!(err.to_string().contains("nope.py"));
    }

    #[test]
    fn hidden_directories_are_skipped() {
        let dir = tree(&["a.py", ".venv/lib/b.py", ".git/c.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }
}
```

Add to `crates/liar-cli/Cargo.toml`:

```toml
[dependencies]
walkdir = "2"
globset = "0.4"

[dev-dependencies]
tempfile = "3"
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-cli discover
```

Expected: compile error, `discover` not found.

- [ ] **Step 3: Implement discovery**

Above the tests in `crates/liar-cli/src/discover.rs`:

```rust
//! Finding the files to analyse.

use globset::{Glob, GlobSetBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("no such file or directory: {0}")]
    NotFound(String),
    #[error("invalid exclude pattern '{pattern}': {source}")]
    BadPattern {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Collects the Python files under `roots`, minus anything matching `exclude`.
///
/// Results are deduplicated and sorted, so the caller sees the same list
/// whatever order the filesystem returned entries in.
pub fn discover(roots: &[PathBuf], exclude: &[String]) -> Result<Vec<PathBuf>, DiscoverError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in exclude {
        let glob = Glob::new(pattern).map_err(|source| DiscoverError::BadPattern {
            pattern: pattern.clone(),
            source,
        })?;
        builder.add(glob);
    }
    let excluded = builder.build().map_err(|source| DiscoverError::BadPattern {
        pattern: exclude.join(", "),
        source,
    })?;

    let mut found = Vec::new();

    for root in roots {
        if !root.exists() {
            return Err(DiscoverError::NotFound(root.display().to_string()));
        }

        if root.is_file() {
            found.push(root.clone());
            continue;
        }

        for entry in WalkDir::new(root).into_iter().filter_entry(|e| !is_hidden(e.path())) {
            let entry = entry.map_err(|e| DiscoverError::Io {
                path: root.display().to_string(),
                source: e.into(),
            })?;

            if !entry.file_type().is_file() || !is_python(entry.path()) {
                continue;
            }

            // Match excludes against the path relative to the root, so a
            // pattern like "tests/**" means what the user expects.
            let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
            if excluded.is_match(relative) {
                continue;
            }

            found.push(entry.path().to_path_buf());
        }
    }

    found.sort();
    found.dedup();
    Ok(found)
}

fn is_python(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "pyi")
    )
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') && n != "." && n != "..")
}
```

- [ ] **Step 4: Run the discovery tests**

```bash
cargo test -p liar-cli discover
```

Expected: 9 passed.

- [ ] **Step 5: Write the failing CLI tests**

`crates/liar-cli/tests/cli.rs`:

```rust
use assert_cmd::Command;
use std::fs;

fn liar() -> Command {
    Command::cargo_bin("liar").expect("binary should build")
}

#[test]
fn a_clean_file_exits_zero_and_prints_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("clean.py");
    fs::write(&file, "def f():\n    pass\n").unwrap();

    liar().arg("check").arg(&file).assert().success().stdout("");
}

#[test]
fn a_syntax_error_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("broken.py");
    fs::write(&file, "def (:\n").unwrap();

    liar().arg("check").arg(&file).assert().code(2);
}

#[test]
fn a_missing_path_exits_two_with_a_message() {
    liar()
        .arg("check")
        .arg("definitely-not-here.py")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("definitely-not-here.py"));
}

#[test]
fn an_invalid_tone_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    fs::write(&file, "pass\n").unwrap();

    liar()
        .arg("check")
        .arg(&file)
        .args(["--tone", "sarcastic"])
        .assert()
        .failure();
}

#[test]
fn an_invalid_config_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    fs::write(&file, "pass\n").unwrap();
    let config = dir.path().join("liar.toml");
    fs::write(&config, "toen = \"dry\"\n").unwrap();

    liar()
        .arg("check")
        .arg(&file)
        .args(["--config", config.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("toen"));
}

#[test]
fn the_version_flag_works() {
    liar().arg("--version").assert().success();
}
```

- [ ] **Step 6: Run to verify failure**

```bash
cargo test -p liar-cli --test cli
```

Expected: failures — the binary has no `check` subcommand yet.

- [ ] **Step 7: Implement the CLI**

`crates/liar-cli/src/main.rs`:

```rust
//! Static analysis for Python: the command line interface.

#![forbid(unsafe_code)]

mod config;
mod discover;
mod render;

use clap::{Parser, Subcommand};
use config::Config;
use liar_core::ast::parse;
use liar_core::finding::Finding;
use liar_core::messages::{MessageTable, Tone};
use liar_core::source::SourceMap;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "liar", version, about = "Finds code that says one thing and does another")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyse files or directories
    Check {
        /// Files or directories to analyse
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// professional, dry, or brutal
        #[arg(long)]
        tone: Option<String>,

        /// Path to liar.toml
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(found_something) => {
            if found_something {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(message) => {
            eprintln!("liar: {message}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether any findings were reported.
fn run(cli: Cli) -> Result<bool, String> {
    let Commands::Check { paths, tone, config } = cli.command;

    let mut settings = Config::load(config.as_deref()).map_err(|e| e.to_string())?;
    if let Some(tone) = tone {
        settings.tone = tone.parse::<Tone>().map_err(|e| e.to_string())?;
    }

    let files = discover::discover(&paths, &settings.exclude).map_err(|e| e.to_string())?;

    let mut sources = SourceMap::new();
    let findings: Vec<Finding> = Vec::new();

    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let file_id = sources.add(path.clone(), text);

        // Parsing is the whole pipeline for now. Plan 2 adds the index and the
        // checks that turn an Ast into findings.
        parse(sources.get(file_id).text()).map_err(|e| {
            let position = sources.get(file_id).position(e.span.start);
            format!(
                "{}:{}:{}: {}",
                path.display(),
                position.line,
                position.column,
                e.message
            )
        })?;
    }

    if !findings.is_empty() {
        print!("{}", render::render(&findings, &sources, MessageTable::embedded(), settings.tone));
    }

    Ok(!findings.is_empty())
}
```

- [ ] **Step 8: Run the CLI tests**

```bash
cargo test -p liar-cli
```

Expected: all pass. `a_clean_file_exits_zero_and_prints_nothing` proves the
pipeline runs end to end; `a_syntax_error_exits_two` proves errors surface with
a position.

- [ ] **Step 9: Try it by hand**

```bash
cargo run -p liar-cli -- check crates --tone brutal
echo "exit code: $?"
```

Expected: no output, exit 0 — there are no Python files under `crates`. Then:

```bash
printf 'def (:\n' > /tmp/broken.py
cargo run -p liar-cli -- check /tmp/broken.py
echo "exit code: $?"
```

Expected: a `path:line:col: message` error on stderr, exit 2.

- [ ] **Step 10: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-cli
git commit -m "Add the check subcommand and file discovery

Discovery walks directories, skips hidden ones and non-Python files,
applies exclude globs relative to each root, and returns a sorted
deduplicated list, so filesystem ordering cannot leak into output
ordering.

Exit codes follow the usual convention for a linter: 0 clean, 1
findings, 2 the tool itself failed. Syntax errors report a path, line
and column rather than a bare message."
```

---

## Task 12: The fixture harness

Every checker from Plan 2 onward is tested through this. It carries the rule
from §9.2 that an *unexpected* finding fails as loudly as a missing one —
without which negative fixtures quietly stop testing anything.

**Files:**
- Create: `crates/liar-core/src/fixture.rs`
- Create: `tests/fixtures/harness/*.py`
- Modify: `crates/liar-core/src/lib.rs`

**Interfaces:**
- Consumes: `CheckId`, `Finding`, `SourceFile`, `SourceMap`.
- Produces:
  - `struct Expectation { check: CheckId, line: u32 }`
  - `fn parse_expectations(&SourceFile) -> Result<Vec<Expectation>, FixtureError>`
  - `fn check_fixture(&SourceFile, &[Finding], &SourceMap) -> Result<(), FixtureFailure>`
  - `struct FixtureFailure { missing: Vec<Expectation>, unexpected: Vec<(CheckId, u32)> }`

- [ ] **Step 1: Write the failing tests**

`crates/liar-core/src/fixture.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FileId;
    use crate::span::Span;
    use std::path::PathBuf;

    fn file(text: &str) -> (SourceMap, FileId) {
        let mut map = SourceMap::new();
        let id = map.add(PathBuf::from("fixture.py"), text.to_string());
        (map, id)
    }

    fn finding_on_line(check: CheckId, file: FileId, sources: &SourceMap, line: u32) -> Finding {
        let src = sources.get(file);
        let mut offset = 0u32;
        for current in 1..line {
            offset += src.line_text(current).len() as u32 + 1;
        }
        Finding::new(check, file, Span::new(offset, offset + 1))
    }

    #[test]
    fn an_expectation_comment_is_parsed() {
        let (map, id) = file("def f():  # expect: C1\n    pass\n");
        let found = parse_expectations(map.get(id)).unwrap();
        assert_eq!(found, vec![Expectation { check: CheckId::C1, line: 1 }]);
    }

    #[test]
    fn several_expectations_on_one_line_are_parsed() {
        let (map, id) = file("x = f()  # expect: C1, C3b\n");
        let found = parse_expectations(map.get(id)).unwrap();
        assert_eq!(found, vec![
            Expectation { check: CheckId::C1, line: 1 },
            Expectation { check: CheckId::C3b, line: 1 },
        ]);
    }

    #[test]
    fn a_file_with_no_comments_expects_nothing() {
        let (map, id) = file("def f():\n    pass\n");
        assert!(parse_expectations(map.get(id)).unwrap().is_empty());
    }

    #[test]
    fn an_unknown_code_in_a_comment_is_an_error() {
        // A typo would otherwise turn an assertion into no assertion at all.
        let (map, id) = file("x = 1  # expect: C99\n");
        let err = parse_expectations(map.get(id)).expect_err("expected an error");
        assert!(err.to_string().contains("C99"));
    }

    #[test]
    fn a_matching_finding_passes() {
        let (map, id) = file("x = f()  # expect: C1\n");
        let findings = vec![finding_on_line(CheckId::C1, id, &map, 1)];
        assert!(check_fixture(map.get(id), &findings, &map).is_ok());
    }

    #[test]
    fn a_missing_finding_fails_and_names_it() {
        let (map, id) = file("x = f()  # expect: C1\n");
        let failure = check_fixture(map.get(id), &[], &map).expect_err("expected a failure");
        assert_eq!(failure.missing, vec![Expectation { check: CheckId::C1, line: 1 }]);
        assert!(failure.unexpected.is_empty());
    }

    #[test]
    fn an_unexpected_finding_fails() {
        // The rule that makes negative fixtures mean anything.
        let (map, id) = file("x = f()\n");
        let findings = vec![finding_on_line(CheckId::C1, id, &map, 1)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.unexpected, vec![(CheckId::C1, 1)]);
        assert!(failure.missing.is_empty());
    }

    #[test]
    fn a_finding_on_the_wrong_line_fails_both_ways() {
        let (map, id) = file("x = f()  # expect: C1\ny = g()\n");
        let findings = vec![finding_on_line(CheckId::C1, id, &map, 2)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.missing.len(), 1);
        assert_eq!(failure.unexpected.len(), 1);
    }

    #[test]
    fn the_wrong_check_on_the_right_line_fails() {
        let (map, id) = file("x = f()  # expect: C1\n");
        let findings = vec![finding_on_line(CheckId::C4, id, &map, 1)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.missing, vec![Expectation { check: CheckId::C1, line: 1 }]);
        assert_eq!(failure.unexpected, vec![(CheckId::C4, 1)]);
    }

    #[test]
    fn duplicate_findings_on_one_line_are_counted() {
        // Two findings where one was expected is a bug worth failing on.
        let (map, id) = file("x = f()  # expect: C1\n");
        let findings = vec![
            finding_on_line(CheckId::C1, id, &map, 1),
            finding_on_line(CheckId::C1, id, &map, 1),
        ];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.unexpected, vec![(CheckId::C1, 1)]);
    }

    #[test]
    fn the_failure_message_shows_both_sides() {
        let (map, id) = file("x = f()  # expect: C1\ny = g()\n");
        let findings = vec![finding_on_line(CheckId::C4, id, &map, 2)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        let text = failure.to_string();
        assert!(text.contains("C1"), "should name the missing check");
        assert!(text.contains("C4"), "should name the unexpected check");
        assert!(text.contains("fixture.py"), "should name the file");
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p liar-core fixture
```

Expected: compile error, `parse_expectations` not found.

- [ ] **Step 3: Implement**

Above the tests in `crates/liar-core/src/fixture.rs`:

```rust
//! The `# expect:` fixture harness.
//!
//! A fixture is a Python file that declares, inline, which findings it should
//! produce. A line with no comment declares that it produces none — which is
//! how the negative fixtures in §9.1 assert silence.

use crate::check::CheckId;
use crate::finding::Finding;
use crate::source::{SourceFile, SourceMap};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Expectation {
    pub check: CheckId,
    pub line: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("{file}:{line}: unknown check code '{code}' in an expect comment")]
    UnknownCheck { file: String, line: u32, code: String },
    #[error("{file}:{line}: empty expect comment")]
    Empty { file: String, line: u32 },
}

/// What a fixture expected versus what it got.
#[derive(Debug, PartialEq, Eq)]
pub struct FixtureFailure {
    pub file: String,
    pub missing: Vec<Expectation>,
    pub unexpected: Vec<(CheckId, u32)>,
}

impl fmt::Display for FixtureFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "fixture mismatch in {}", self.file)?;
        for e in &self.missing {
            writeln!(f, "  expected {} on line {}, but it was not reported", e.check.code(), e.line)?;
        }
        for (check, line) in &self.unexpected {
            writeln!(f, "  reported {} on line {}, but it was not expected", check.code(), line)?;
        }
        Ok(())
    }
}

impl std::error::Error for FixtureFailure {}

const MARKER: &str = "# expect:";

pub fn parse_expectations(file: &SourceFile) -> Result<Vec<Expectation>, FixtureError> {
    let name = file.path().display().to_string();
    let mut expectations = Vec::new();

    for line_number in 1..=file.line_count() {
        let text = file.line_text(line_number);
        let Some(marker_at) = text.find(MARKER) else {
            continue;
        };

        let codes = text[marker_at + MARKER.len()..].trim();
        if codes.is_empty() {
            return Err(FixtureError::Empty { file: name, line: line_number });
        }

        for code in codes.split(',').map(str::trim).filter(|c| !c.is_empty()) {
            let check = CheckId::from_code(code).ok_or_else(|| FixtureError::UnknownCheck {
                file: name.clone(),
                line: line_number,
                code: code.to_string(),
            })?;
            expectations.push(Expectation { check, line: line_number });
        }
    }

    Ok(expectations)
}

/// Compares what a fixture expected against what the analyser produced.
///
/// Both directions fail. A finding that was not expected is as much a failure
/// as an expectation that was not met — otherwise a file with no comments would
/// assert nothing, and every negative fixture in the suite would be inert.
pub fn check_fixture(
    file: &SourceFile,
    findings: &[Finding],
    sources: &SourceMap,
) -> Result<(), FixtureFailure> {
    let expectations = parse_expectations(file).expect("fixture expectations must parse");

    let mut expected: Vec<(CheckId, u32)> =
        expectations.iter().map(|e| (e.check, e.line)).collect();
    let mut actual: Vec<(CheckId, u32)> = findings
        .iter()
        .filter(|f| f.primary.file == file.id())
        .map(|f| {
            let line = sources.get(f.primary.file).position(f.primary.span.start).line;
            (f.check, line)
        })
        .collect();

    expected.sort_unstable();
    actual.sort_unstable();

    // Multiset difference in both directions, so a duplicate counts.
    let mut missing = Vec::new();
    let mut remaining = actual.clone();
    for item in &expected {
        match remaining.iter().position(|a| a == item) {
            Some(index) => {
                remaining.remove(index);
            }
            None => missing.push(Expectation { check: item.0, line: item.1 }),
        }
    }

    if missing.is_empty() && remaining.is_empty() {
        return Ok(());
    }

    Err(FixtureFailure {
        file: file.path().display().to_string(),
        missing,
        unexpected: remaining,
    })
}
```

- [ ] **Step 4: Register the module and run the tests**

Add to `crates/liar-core/src/lib.rs`:

```rust
pub mod fixture;

pub use fixture::{Expectation, FixtureFailure, check_fixture, parse_expectations};
```

```bash
cargo test -p liar-core fixture
```

Expected: 11 passed.

- [ ] **Step 5: Add fixtures that exercise the harness itself**

`tests/fixtures/harness/clean.py`:

```python
# A file that should produce nothing. The shape every negative fixture takes.
def add(a, b):
    return a + b
```

`tests/fixtures/harness/expectations.py`:

```python
# Exercises the comment syntax. No checker exists yet to satisfy these, so
# they are parsed rather than run — see tests/fixture_syntax.rs.
x = f()          # expect: C1
y = g()          # expect: C1, C3b
z = h()
```

`crates/liar-core/tests/fixture_syntax.rs`:

```rust
//! The expectation comments in tests/fixtures parse, so a typo in a fixture
//! fails here rather than silently disabling an assertion in Plan 2.

use liar_core::check::CheckId;
use liar_core::fixture::parse_expectations;
use liar_core::source::SourceMap;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn every_fixture_has_parseable_expectations() {
    let mut checked = 0;
    for entry in walkdir::WalkDir::new(fixture_dir()) {
        let entry = entry.expect("walking the fixture directory");
        if entry.path().extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }

        let text = std::fs::read_to_string(entry.path()).expect("reading a fixture");
        let mut map = SourceMap::new();
        let id = map.add(entry.path().to_path_buf(), text);

        parse_expectations(map.get(id))
            .unwrap_or_else(|e| panic!("bad expectation comment: {e}"));
        checked += 1;
    }
    assert!(checked >= 2, "expected to find fixtures, found {checked}");
}

#[test]
fn the_sample_fixture_declares_what_it_should() {
    let path = fixture_dir().join("harness/expectations.py");
    let text = std::fs::read_to_string(&path).expect("reading the fixture");
    let mut map = SourceMap::new();
    let id = map.add(path, text);

    let found = parse_expectations(map.get(id)).unwrap();
    let codes: Vec<_> = found.iter().map(|e| e.check).collect();
    assert_eq!(codes, vec![CheckId::C1, CheckId::C1, CheckId::C3b]);
}
```

Add `walkdir` to `liar-core`'s `[dev-dependencies]`.

- [ ] **Step 6: Run the whole suite**

```bash
cargo test
```

Expected: everything passes.

- [ ] **Step 7: Prove CI actually fails**

The week-1 acceptance criterion from spec §11 is that a deliberately failing
fixture turns CI red. Verify it rather than assuming it.

Temporarily add to `tests/fixtures/harness/expectations.py`:

```python
w = i()          # expect: C99
```

Then:

```bash
cargo test --test fixture_syntax
```

Expected: FAIL, naming `C99`. **Now remove that line** and re-run to confirm
green. A gate never seen to fail is a gate nobody knows works.

- [ ] **Step 8: Run the gates and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add crates/liar-core/src/fixture.rs crates/liar-core/src/lib.rs crates/liar-core/tests tests/fixtures
git commit -m "Add the expect-comment fixture harness

A fixture is a Python file declaring inline which findings it should
produce. A line with no comment declares that it produces none, which is
how negative fixtures assert silence.

Mismatches fail in both directions: a finding that was not expected
fails as loudly as an expectation that was not met. Without that, a file
with no comments would assert nothing and every negative fixture would
be inert. Comparison is a multiset difference, so two findings where one
was expected also fails.

Unknown check codes in a comment are an error, since a typo would
otherwise turn an assertion into no assertion at all."
```

---

## Done when

- [ ] `cargo test` passes with every test in Tasks 1–12
- [ ] `cargo clippy --all-targets -- -D warnings` is silent
- [ ] `cargo fmt --check` is silent
- [ ] CI is green on GitHub, and has been *seen* to fail (Task 12 Step 7)
- [ ] `cargo run -p liar-cli -- check <a python project>` exits 0 and prints nothing
- [ ] `cargo run -p liar-cli -- check <a file with a syntax error>` exits 2 with a
      `path:line:col: message`
- [ ] `docs/decisions/001-parser.md` records the real `ruff_python_parser` API
- [ ] No `ruff_*` type appears outside `crates/liar-core/src/ast/convert.rs` —
      verify with `grep -rn "ruff_" crates/ --include=*.rs | grep -v convert.rs`,
      which should print nothing
- [ ] No AI reference anywhere — verify with
      `git log --all --format='%B%an%ae' | grep -iE 'claude|anthropic|copilot'`,
      which should print nothing

## What Plan 2 picks up

The index: scopes, bindings, imports resolved across files, and the call graph —
then C1 and C2 on top of it, and the corpus runner pointed at real packages for
the first time.
