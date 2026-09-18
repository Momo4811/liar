# liar — Plan 2: The index and the first checkers

**Goal:** `liar check` finds genuine un-awaited coroutines and blocking calls in
async code, in real Python packages.

**Architecture:** An index answers *what does this name refer to?* across a whole
project — scopes, bindings, imports resolved between files, and a call graph
built on top. Two checkers consume it. A corpus runner points the whole thing at
real packages from PyPI.

**Tech stack:** As Plan 1. Adds no dependencies to the engine; the corpus
fetcher is a small Python script, since `pip` already solves downloading and
unpacking and reimplementing that in Rust would be a week spent on plumbing.

**Spec:** `docs/design/2026-09-15-liar-design.md` §5 (C1, C2), §8.2 (the index).
**Previous:** `docs/plans/2026-09-15-liar-01-foundations.md`.

**On detail level:** Plan 1 wrote out every line of code because it was written
to be executed later. This plan is executed immediately, so it specifies design
decisions, module boundaries, interfaces and the complete test list for each
task, and lets the tests be the specification of behaviour — which is what
test-first means in practice. Every task still ends with fixtures passing, gates
green, and a commit.

---

## Global Constraints

Everything from Plan 1 still applies. Added:

- **No checker reads the syntax tree without going through the index.** A checker
  that pattern-matches on names rather than resolving them is a regex with extra
  steps, and it is how false positives get in.
- **An unresolved name produces no finding, ever.** Not a guess, not a
  heuristic fallback. This is §4 of the spec, and in this plan it is load-bearing
  for the first time.
- **Every suppression rule listed below gets a negative fixture.** A suppression
  with no fixture does not count as implemented.

---

## Design

### The index

Four layers, each a separate module under `crates/liar-core/src/index/`.

**Scopes** (`scope.rs`). A tree of scopes per file: module, class, function,
comprehension. Each holds a map from name to binding.

The one non-obvious rule: **class scopes are skipped when resolving from a
nested function.** In Python, a method body cannot see its class's attributes as
bare names. Getting this wrong invents bindings that do not exist, and every
checker downstream inherits the error.

```python
class C:
    x = 1
    def m(self):
        return x    # NameError — not C.x
```

**Bindings** (`binding.rs`). What a name was bound by, which is what the checkers
actually ask about:

| Kind | From |
|---|---|
| `Function { is_async }` | `def` / `async def` |
| `Class` | `class` |
| `Parameter` | a function's parameter list |
| `Variable` | assignment |
| `Module { path }` | `import x`, `import x.y as z` |
| `FromImport { module, name }` | `from x import y` |

`Function { is_async }` is what C1 is built on; `Module` and `FromImport` are
what let C2 recognise `time.sleep` whichever way it was imported.

**Imports** (`imports.rs`). Maps a module path to a file. A project root is
derived from the discovered files, and each file's path becomes a module path
(`pkg/mod.py` → `pkg.mod`, `pkg/__init__.py` → `pkg`). Relative imports resolve
against the importing file's own package.

Deliberately unresolved, each yielding `Unknown`: `import *`, imports inside
conditionals or functions, `importlib`, and anything whose module is not among
the files being analysed. A third-party import is not a failure — it is simply a
name this tool says nothing about.

**Resolution** (`mod.rs`). `Index::resolve(file, scope, name) -> Option<DefId>`,
walking the scope chain with the class-scope rule, then falling back to the
module scope and to imports. `DefId` indexes a global arena of definitions, so a
checker can ask about a definition in another file without knowing which.

A `liar debug resolve <file>:<line>:<col>` subcommand prints what a name resolves
to. Built in this plan rather than retrofitted, because without it every later
checker is debugged by guesswork.

### C1 — the `await` that isn't there

Two shapes, both unambiguous.

**Discarded**: a statement that is nothing but a call, where the callee resolves
to an `async def`.

```python
save_user(u)          # the value is built and dropped; the body never runs
```

Only the *top level* of an expression statement is examined. `gather(f(), g())`
is a call to `gather`, which is not async, so nothing is reported — and the
inner calls are never looked at. Every `asyncio` wrapper is therefore suppressed
by construction rather than by a list that would need maintaining.

**Assigned and never mentioned again**: `x = f()` where `f` is async and `x`
appears nowhere else in the enclosing function. Conservative on purpose — if the
name is used at all, for anything, the check says nothing, because it cannot
know whether that use awaits it.

Suppressed: callee unresolved; callee not async; call wrapped in `await`; call
appearing anywhere other than the top of an expression statement or the right of
a simple assignment; assigned name used again in any way.

### C2 — the blocking call inside async code

A call is **blocking** if its resolved dotted path is in `data/blocking.toml`,
or if it resolves to a project function that is not `async def` and itself
contains a blocking call. The second clause is a fixpoint over the call graph.

Resolving the dotted path is what the `Module` and `FromImport` bindings are
for — all three of these must reach `time.sleep`:

```python
import time;            time.sleep(1)
from time import sleep;      sleep(1)
import time as t;          t.sleep(1)
```

Reported only inside an `async def`. Suppressed when the call sits anywhere
inside an argument to a known offloading function — `asyncio.to_thread`,
`loop.run_in_executor`, `anyio.to_thread.run_sync` — which means tracking the
expression ancestry during the walk, not just the statement.

### The corpus

`corpus/fetch.py` downloads the top packages from PyPI with `pip download` and
unpacks them under `corpus/packages/`, which is gitignored. `liar check` then
runs over the lot. Committed: the package list with pinned versions, the
per-check counts as a baseline, and the triage notes.

Written in Python because `pip` already solves downloading, dependency-free
resolution and unpacking of both wheels and sdists. Reimplementing that in Rust
would be a week of zip and tar handling that teaches nothing and tests nothing.

---

## File Structure

```
crates/liar-core/src/
├── index/
│   ├── mod.rs        Index, DefId, Definition, resolve()
│   ├── scope.rs      ScopeId, Scope, ScopeKind, the scope tree
│   ├── binding.rs    Binding, BindingKind
│   ├── imports.rs    module paths, import resolution
│   └── build.rs      walks an Ast and populates the index
├── analysis.rs       the driver: files → index → checks → findings
└── checks/
    ├── mod.rs        the Check trait, the registry
    ├── c1_unawaited.rs
    └── c2_blocking.rs

data/blocking.toml    known-blocking functions, and the offloading wrappers
corpus/fetch.py       downloads packages
corpus/packages.txt   the pinned package list
corpus/baseline.json  per-check counts
tests/fixtures/C1/{positive,negative}/*.py
tests/fixtures/C2/{positive,negative}/*.py
crates/liar-core/tests/fixtures.rs   runs every fixture through the harness
```

---

## Tasks

Each ends with: fixtures passing, `cargo test`, `cargo clippy -- -D warnings`,
`cargo fmt --check`, commit.

### Task 1 — Scopes and bindings

**Files:** `index/scope.rs`, `index/binding.rs`

**Produces:** `ScopeId`, `ScopeKind::{Module, Class, Function, Comprehension}`,
`Scope { kind, parent, bindings, span }`, `Binding`, `BindingKind` as tabled
above, and `ScopeTree` with `push`/`pop`/`bind`/`lookup_local`.

**Tests:** a module scope exists for every file; a function introduces a scope;
a class introduces a scope; nested functions nest; parameters bind in the
function's own scope, not its parent; a rebinding replaces rather than
duplicates; `lookup_local` does not walk the parent chain.

### Task 2 — Building the index from a syntax tree

**Files:** `index/build.rs`, `index/mod.rs`

**Produces:** `Index::build(files: &[(FileId, &Ast)]) -> Index`, a global
`Arena<DefId, Definition>`, and `Definition { file, scope, name, kind, name_span }`.

**Tests:** functions, async functions, classes, parameters and assignments all
produce bindings of the right kind; an async function's binding records
`is_async`; nested definitions land in the right scope; a decorated function
still binds; the same name defined twice yields one binding with the later span.

### Task 3 — Resolution, imports, and `debug resolve`

**Files:** `index/imports.rs`, `index/mod.rs`, `liar-cli/src/main.rs`

**Produces:** `Index::resolve(FileId, ScopeId, &str) -> Option<DefId>`,
`Index::dotted_path(FileId, ScopeId, &Expr) -> Option<String>`, and a
`liar debug resolve` subcommand.

**Tests:** local beats enclosing beats module; **a nested function does not see
its class's scope**; a parameter shadows a module-level name; `import a.b` binds
`a`; `import a.b as c` binds `c`; `from a import b` binds `b`; `from . import x`
resolves relative to the file's package; an unknown module resolves to `None`;
`import *` resolves to `None` rather than guessing; `time.sleep` produces the
dotted path `time.sleep` under all three import forms; a dotted path through an
unresolved name is `None`.

### Task 4 — The analysis driver and the fixture runner

**Files:** `analysis.rs`, `checks/mod.rs`, `crates/liar-core/tests/fixtures.rs`

**Produces:** `trait Check { fn id(&self) -> CheckId; fn run(&self, ctx: &Ctx) -> Vec<Finding>; }`,
`analyse(sources, files) -> Vec<Finding>`, and a test that runs **every** file
under `tests/fixtures/` through `check_fixture`.

This is the task that makes every later fixture an actual assertion rather than
an inert file. It also wires `liar check` to produce findings for the first
time.

**Tests:** the driver produces no findings for an empty project; the fixture
runner passes on `harness/clean.py`; the fixture runner is proven to fail by
temporarily adding an expectation nothing satisfies.

### Task 5 — C1

**Files:** `checks/c1_unawaited.rs`, `tests/fixtures/C1/**`

**Positive fixtures:** discarded call to a local async function; to one imported
from another file; to an async method; assigned and never used again; inside a
sync function.

**Negative fixtures:** awaited; wrapped in `create_task`; in `gather`; in
`ensure_future`; passed as an argument; returned; callee is sync; callee
unresolved; callee imported from a module not being analysed; assigned then
awaited later; assigned then passed somewhere; a name shadowing an async
function with a sync local.

### Task 6 — The call graph and blocking propagation

**Files:** `index/mod.rs` (call graph), `checks/c2_blocking.rs` (propagation)

**Produces:** `Index::calls(DefId) -> &[DefId]`, and a fixpoint marking
functions blocking.

**Tests:** a direct call is recorded; a method call is recorded; an unresolved
call is not; the fixpoint terminates on mutual recursion; a function calling a
blocking function is blocking; an `async def` calling one is *not* marked
blocking itself, since awaiting it is fine; the fixpoint is order-independent.

### Task 7 — C2

**Files:** `data/blocking.toml`, `checks/c2_blocking.rs`, `tests/fixtures/C2/**`

**Positive fixtures:** `time.sleep` in an async function, under all three import
forms; `requests.get`; `subprocess.run`; a project helper that is transitively
blocking.

**Negative fixtures:** the same calls in a sync function; wrapped in
`asyncio.to_thread`; in `run_in_executor`; in `anyio.to_thread.run_sync`; an
async alternative (`asyncio.sleep`); an unresolved call; a local function named
`sleep` that shadows the import.

### Task 8 — The corpus

**Files:** `corpus/fetch.py`, `corpus/packages.txt`, `.gitignore`,
`crates/liar-cli/src/main.rs` (a `--format json` mode for aggregation)

**Deliverable:** a scan of real packages with **zero panics**, per-check counts
committed as a baseline, and the first genuine findings triaged by hand into
`corpus/triage.md`.

The panic-freedom assertion is the point. Real code will exercise assumptions the
fixtures let stand, and every crash it finds is a bug the fixtures could not
have.

---

## Done when

- [ ] `liar check` reports real C1 and C2 findings on the fixtures
- [ ] Every suppression rule above has a negative fixture that passes
- [ ] `liar debug resolve` answers correctly on a multi-file project
- [ ] The corpus scan completes over real packages with no panic
- [ ] `corpus/baseline.json` and `corpus/triage.md` are committed
- [ ] All gates green

## What Plan 3 picks up

The language server and the VS Code extension — the first point at which the
tool has a face.
