# liar — a static analyser for code that lies

**Status:** approved design, not yet implemented
**Date:** 2026-09-15
**Working name:** `liar`. Provisional — renaming costs two minutes while the repo is empty and is painful later, so settle it before week 1.

---

## 1. What this is

A static analyser for Python, written in Rust, shipped as a CLI and a VS Code
extension.

It finds one category of defect: **code that says one thing and does another.**
A function named `is_ready` that returns a string. A call that looks like it
saves a user and silently doesn't. A `close()` that only runs when nothing goes
wrong. A docstring describing a return value the function never produces.

The analysis is ordinary static analysis done properly. What distinguishes the
project is that the findings are *specific* and the diagnostics have a voice:

```
  is_ready() -> str
  ^^^^^^^^ returns str. one of us is confused.

  data, data_2, data_final, data_final_v2
  ^^^^ four variables called data in this file. none are related.
```

## 2. Why this project, specifically

It is a CV project. It is optimised for a stranger forming an accurate
impression of the author in ninety seconds, and for surviving thirty minutes of
an interviewer digging into it.

Those two goals pull in different directions, which is the whole design problem.
The ninety-second read wants personality, screenshots and a memorable idea. The
thirty-minute interrogation wants type inference, control flow and defensible
engineering judgement. This design serves the first with the delivery layer and
the second with the engine, and never lets the first contaminate the second.

**Non-goal: adoption.** Install counts are not a target. The repository itself is
the deliverable. This is a deliberate decision and it has consequences that
appear throughout: publishing to the marketplace stays in scope because
"shipped" is worth a day of work, but nothing is optimised for reach, and the
cross-platform binary distribution work that reach would require is cut.

## 3. Goals and non-goals

### Goals

1. A working analyser for the five checker families in §5, at high precision.
2. A VS Code extension that runs it live, with the personality intact.
3. A corpus run over real packages, with an honestly measured accuracy rate.
4. A README a stranger understands in ninety seconds.

### Non-goals

- Analysing any language other than Python.
- Competing with `ruff`, `mypy` or `pylint` on coverage. They win. This tool
  does a narrow thing they do not do.
- Autofix beyond a single quick-fix for the missing `await`.
- Incremental re-analysis on every keystroke. Re-analyse on save.
- Multi-platform binary bundling in the `.vsix`.
- Type-checking. Types are inferred only as far as the checkers need.

## 4. The governing rule

> **When unsure, stay silent.**

Every finding asks a stranger to change their code. A tool at 60% accuracy gets
muted; at 90% it gets read. So every ambiguity resolves toward silence: an
unresolvable name, an unknown decorator, a value that escapes the function —
all produce no finding, deliberately, at the cost of real misses.

This is enforced structurally rather than by discipline. The type lattice has an
`Unknown` element that is **absorbing**: anything joined with `Unknown` is
`Unknown`, and no checker may fire on an `Unknown`. It is therefore not possible
to forget the rule in one branch of one checker; the type system carries it.

This is also the most interesting thing in the project to be asked about, since
it is a judgement call rather than a technique, and §10 produces the number that
backs it up.

## 5. Checkers

Five families. Each specifies what it detects, what suppresses it, and why it is
worth having.

### C1 — the `await` that isn't there

```python
async def handle_request(req):
    save_user(req.user)        # nothing happens. at all.
    return {"ok": True}
```

Calling an async function without awaiting it constructs a coroutine and
discards it. The body never runs, nothing raises, and the caller returns
success.

**Detect:** a call whose callee resolves to an `async def`, where the result is
discarded (an expression statement) or bound to a local that is never awaited on
any path.

**Suppress when:** the callee cannot be resolved; the call is an argument to
`asyncio.create_task`, `ensure_future`, `gather`, `wait`, `run`,
`run_until_complete`, `run_coroutine_threadsafe`, or `anyio` equivalents; the
call is an argument to any other function (it may be awaited there); the result
is returned (a sync function returning a coroutine is a legitimate pattern).

**Quick fix:** insert `await`, offered only when the enclosing function is
`async def`.

**Why it leads:** there is no argument to be had. It is not a style preference
or a hypothetical. The code does not run. If upstream PRs happen (§10.4), these
are the ones that get merged.

### C2 — the blocking call inside async code

```python
async def fetch_all(urls):
    for u in urls:
        r = requests.get(u)    # freezes every other task in the process
```

**Detect:** inside an `async def`, a call that is blocking. A call is blocking if
it is in the seed table, or transitively calls something blocking and is not
itself `async def`. Propagated over the call graph to a fixpoint.

**Seed table:** `time.sleep`, `requests.*`, `urllib.request.urlopen`,
`subprocess.run/call/check_output/check_call`, `socket.socket.recv/send/connect`,
`os.system`, and blocking `sqlite3` / `psycopg2` entry points. Held in
`data/blocking.toml`, not in code, so extending it is a data change.

**Suppress when:** wrapped in `asyncio.to_thread`, `loop.run_in_executor`, or
`anyio.to_thread.run_sync`; the enclosing function is not async; the callee is
unresolved.

### C3 — names that lie

Six sub-checks, all built on the type inference in §7. This is the family that
carries the project's personality, and the one a reader will screenshot.

| id | Detects | Example |
|----|---------|---------|
| C3a | Boolean-shaped name, non-boolean type | `def is_ready() -> str` |
| C3b | Quantity-shaped name, non-numeric type | `count = []` |
| C3c | Plural name holding a scalar, or singular holding a collection | `user = [a, b]` |
| C3d | `get_*` that mutates state | `get_config()` writing `self.cache` |
| C3e | Meaningless name in a scope large enough to matter | `data` spanning 80 lines |
| C3f | One name, several unrelated meanings in a file | four different `data`s |

**C3a** matches `is_`, `has_`, `can_`, `should_`, `was_`, `does_`, `will_`
prefixes against an inferred type that is not `bool`. Applies to function return
types and to variables.

**C3b** matches `count`, `*_count`, `num_*`, `n_*`, `*_size`, `length`, `total`,
`*_index` against a type that is not `int` or `float`.

**C3c** uses a crude English pluralisation rule and only fires on unambiguous
cases — a plural name bound to a non-collection, or a singular name bound to
`list`/`set`/`tuple`. Known irregulars and non-count nouns (`data`, `status`,
`address`, `class`, `process`) are excluded via `data/plurals.toml`.

**C3d** flags a function named `get_*` that writes to `self.*`, to a global, or
calls a method in the known-mutating table. Requires the light side-effect
analysis in §8.4.

**C3e** flags names from a small stoplist (`data`, `temp`, `tmp`, `result`,
`obj`, `val`, `value`, `item`, `thing`, `stuff`, `info`, `foo`, `bar`, plus
single letters other than loop-idiomatic `i`/`j`/`k`) **only when the scope they
live in exceeds a threshold** (default 20 lines), or when they are parameters or
module-level names. Short names in short scopes are good style, and a tool that
does not know this is a tool people uninstall. The threshold is configurable.

**C3f** is the original motivating case. Precisely: an identifier has three or
more distinct **binding sites** in one file — an assignment, parameter,
`for` target, `with` target, comprehension target or import — and at least two
distinct non-`Unknown` inferred types among them. Rebinding the same value
(`x = x + 1`, or a parameter reassigned from itself) is one site, not several.
Reported once per group with secondary spans on every occurrence, never as N
separate findings.

### C4 — cleanup that lies

```python
def read_config(path):
    f = open(path)
    data = parse(f.read())   # if this raises...
    f.close()                # ...this never runs
    return data
```

**Detect:** a value from a resource-acquiring call bound to a local, where some
path from acquisition to a function exit — **including exceptional exits** —
does not pass through the matching release.

**Mechanism:** forward dataflow over the CFG of §8.3, which carries exception
edges. Lattice per tracked local: `Open | Closed | Unknown`, joined at merge
points, `Unknown` absorbing. Any local still `Open` at any exit is a finding.

**Resource table** (`data/resources.toml`): `open` → `close`, `socket.socket` →
`close`, `Lock.acquire` → `release`, `sqlite3.connect` → `close`,
`tempfile.NamedTemporaryFile` → `close`, and similar.

**Suppress when:** the value escapes — returned, stored on an attribute, global
or container, or passed to another function. Once it escapes, its lifetime is
someone else's business. This single rule accounts for most of the precision on
this check; without it the tool is unusable.

Already inside a `with` or a `try/finally` that releases on all paths: no
finding, which falls out of the dataflow rather than needing a special case.

**Why it matters most technically:** it is the check that requires a real CFG
with exception edges, and it is the reason existing linters miss this class of
bug. It is the centrepiece of an interview conversation.

### C5 — comments that lie

**Detect**, narrowly, only claims that can be checked reliably:

1. Docstring documents parameters that do not exist, or omits ones that do
   (Google, NumPy and Sphinx styles). Very high precision, cheap.
2. Docstring documents a return value, but every path returns `None`.
3. Docstring names a concrete return type ("returns a list of…") contradicted by
   the inferred return type. Restricted to a fixed pattern set —
   list/dict/set/tuple/bool/int/str/None — never general prose.

"Raises X" checking is excluded as too noisy.

First on the cut list (§12).

## 6. Delivery

Three surfaces over one engine.

```
   ┌────────────────────┐   ┌────────────────────┐
   │  VS Code extension │   │  liar (CLI)        │
   │  TypeScript, thin  │   │  Rust              │
   └─────────┬──────────┘   └─────────┬──────────┘
             │ LSP over stdio         │
   ┌─────────▼──────────┐             │
   │  liar-lsp          │             │
   └─────────┬──────────┘             │
             └──────────┬─────────────┘
                 ┌──────▼──────┐
                 │  liar-core  │
                 └─────────────┘
```

The extension is a launcher and nothing more — it starts the server, forwards
messages, and surfaces settings. Target: under 300 lines of TypeScript. Any
logic that appears in it belongs in `liar-core` instead.

The CLI is what runs the corpus (§10) and what CI runs. It renders diagnostics
in `rustc` style and exports SARIF.

**Binary distribution is out of scope.** The extension locates a `liar` binary
on `PATH` or at a configured location, and tells the user how to install it if
absent. Bundling per-platform binaries exists to serve installs at scale, and
scale is a non-goal.

## 7. Type inference

The deepest single component, and the one that makes C3 and C5 possible. Scoped
tightly enough to finish and no tighter.

### In scope

- Annotations on parameters, returns and variables — parsed and trusted.
- Literals: `int`, `float`, `str`, `bytes`, `bool`, `None`, list/dict/set/tuple
  displays, comprehensions.
- Constructor calls to project-defined classes → instance type.
- Stdlib return types from a table (`data/stdlib_returns.toml`).
- A function's return type as the join of its `return` expression types, with a
  recursion guard.
- Flow-sensitive local assignment tracking within a function.
- `isinstance` narrowing inside the guarded branch. Cheap, and worth a lot.
- Attribute types learned from `self.x = …` in `__init__`.
- Interprocedural propagation to a fixpoint over the call graph.

### Out of scope

Generics and `TypeVar`. Protocols and structural typing. Decorators that alter
signatures. Metaclasses. `__getattr__` and dynamically created attributes.
`*args`/`**kwargs` forwarding.

Anything out of scope yields `Unknown`, which by §4 means silence. A function
with an unrecognised decorator is `Unknown` end to end — the conservative
outcome, reached automatically.

### The lattice

```
                Unknown          (⊤ — absorbing, silences all checkers)
              /    |    \
           int    str   MyClass  …
              \    |    /
                 Never            (⊥ — no reachable value)
```

`join(Unknown, T) = Unknown` for every `T`. This is deliberately unlike a
type-checker's lattice, where `Unknown`/`Any` is usually permissive. Here it is
maximally restrictive, because the cost of a wrong answer is a false positive,
and false positives are the only thing that can kill this tool.

## 8. Engine architecture

Rust workspace:

| Crate | Responsibility |
|---|---|
| `liar-core` | Parse, index, infer, CFG, dataflow, checkers |
| `liar-cli` | Binary, diagnostic rendering, config, SARIF |
| `liar-lsp` | LSP server (`tower-lsp`) |
| `liar-corpus` | Corpus download, scan, triage tooling |
| `editors/vscode` | TypeScript extension |

Everything is **arena-allocated and referenced by index** — `Vec<T>` plus newtype
`u32` ids (`FileId`, `DefId`, `ScopeId`, `NodeId`, `BlockId`, `TyId`). No
`Rc<RefCell<…>>` graphs. This is how `rustc` and `ruff` are built, it sidesteps
the borrow checker on cyclic structures entirely, and it makes everything
trivially serialisable for debugging.

### 8.1 Parsing

**Decision to make on day one, by spike, not by argument.** Two candidates:
`rustpython-parser` and `ruff_python_parser`. Spend one hour on each, parsing a
real file and walking the tree, and choose on: fidelity of spans, whether the
AST is genuinely an AST rather than a CST, coverage of modern syntax (match
statements, f-string internals, `walrus`, PEP 695 generics), and API ergonomics.
Record the choice and the reasoning in `docs/decisions/001-parser.md`.

Tree-sitter is rejected. It is built for editors — resilient to broken input,
producing a tree that mirrors source text. For analysis, a real AST is the
better tool.

### 8.2 The index

Per file: a scope tree, with every binding recorded against the scope that owns
it. Across files: imports resolved to definitions, including `from x import y`,
aliasing, relative imports and re-exports.

Deliberately unresolved: `import *`, imports inside conditionals, names assembled
at runtime, `importlib`. Each yields `Unknown`.

Also builds the call graph used by C2 and by interprocedural inference.

A `liar debug resolve <file>:<line>:<col>` subcommand prints what a name resolves
to. Built in week 2, not retrofitted — without it, debugging the later checkers
is guesswork.

### 8.3 Control flow

Per function: basic blocks, with edges. Normal edges from sequencing, branches
and loops. **Exception edges** from any statement that can raise, to the nearest
enclosing `except`/`finally`, or to the exceptional function exit.

Being deliberately coarse: any call is assumed able to raise. This
over-approximates, which is the safe direction — it produces more paths on which
a resource might leak, and C4 only fires when a resource leaks on *some* path,
so over-approximation costs precision, not soundness. Reassess after the first
corpus run if C4's false-positive rate is bad.

`liar debug cfg <function>` emits Graphviz. Same reasoning as the resolve
subcommand.

### 8.4 Dataflow

One generic worklist solver, forward and backward, over a `Lattice` trait
(`bottom`, `join`, `eq`). Each checker supplies a lattice and transfer functions
and gets fixpoint iteration for free. Used by C4 and by the side-effect analysis
that C3d needs.

The side-effect analysis that C3d needs is small: for each function, does it
write to `self.*`, a global, or a known-mutating method, transitively. It is a
call-graph property and needs no CFG, so it reuses C2's fixpoint machinery from
week 3 and is nearly free. This is why C3d can land in week 6 alongside the rest
of C3, ahead of the CFG work in week 7.

### 8.5 Diagnostics and tone

Each finding carries a check id, severity, primary span, optional secondary
spans, a message, and optional help text.

**Messages live in one data file, three per check**, keyed by tone:

```toml
[C3f]
professional = "'{name}' is bound {n} times in this file with {k} different types"
dry          = "{n} variables called {name} in this file. none are related."
brutal       = "{n} things called {name}. pick a lane."
```

One file means the voice is reviewable in one sitting and testable by snapshot.
Scattering strings through the checkers would let the tone drift, and drift is
what makes a tool with personality feel amateur rather than designed.

```jsonc
"liar.tone": "professional" | "dry" | "brutal"   // default: dry
```

The humour comes from accuracy, not from jokes. *"four variables called data in
this file, none are related"* is funny because it is true and specific. A gag is
funny once and irritating on the two-hundredth run; a precise observation stays
funny because it keeps being right. No emoji. Nothing that reads as trying.

The `professional` tone exists so the tool is usable at work, which is both a
genuine kindness and, in itself, part of the joke.

### 8.6 Configuration

`liar.toml`, mirrored by VS Code settings: tone, enabled checks, per-check
options (C3e's scope threshold, C3f's occurrence threshold), ignore globs.
Inline suppression via `# liar: ignore[C3e]`.

## 9. Testing

**Fixtures.** `tests/fixtures/<check>/*.py`, each a small Python file with
expected findings declared inline:

```python
def is_ready() -> str:   # expect: C3a
    return "yes"
```

A harness parses the expectations and diffs against actual output. Adding a case
is adding a file.

**Snapshots.** `insta` over rendered diagnostics in all three tones, so neither
the formatting nor the voice can drift unnoticed.

**The corpus is the integration test.** Two assertions, enforced in CI:

1. **Zero panics on real code.** Non-negotiable. Real code finds every
   assumption the fixtures let stand.
2. **No unexplained jump in finding counts.** Per-check counts are committed to
   `corpus/baseline.json`. CI fails if any check's count moves by more than 20%
   without the baseline being updated in the same commit — which forces the
   change to be looked at and explained in the commit message rather than
   absorbed silently.

**Determinism.** Same input, same output, same order. Findings are sorted by
file, line, column, check id before rendering.

## 10. Evidence

Adoption is a non-goal, so the evidence has to live inside the repository and be
checkable by a stranger who installs nothing.

### 10.1 The corpus

The top ~100 packages from PyPI by download count, pinned by version, downloaded
by `liar-corpus`. Scanned on every CI run; results committed to `corpus/`.

Introduced in **week 3**, as soon as one checker works, not at the end. Every
subsequent checker meets real code the week it is written. Deferring this to
week 8 would mean discovering eight weeks of accumulated false-positive
patterns with no time to fix them.

### 10.2 Accuracy, measured honestly

Sample 100 findings uniformly at random with a fixed seed. Classify each by hand
as true positive, false positive, or unsure. Commit the verdicts and a one-line
reason each to `corpus/triage.md`.

Report **precision = TP / (TP + FP)**, with the sample published so anyone can
check the work.

**Recall is not reported, and the README says why.** There is no way to know
which bugs were missed without a labelled ground truth that does not exist.
Inventing a recall number would be the exact dishonesty this tool is named
after. Stating the limitation plainly is worth more than the number would be.

### 10.3 Performance

Wall-clock to scan a fixed large project, in the README, with the machine and
method stated. Not a competitive claim against `ruff` — it does more, and a
benchmark that flattered this tool by ignoring that would be its own small lie.

### 10.4 Upstream PRs — optional

The strongest single CV line available (*"12 patches merged into packages you
have used"*) and entirely outside the repository. Decide in week 7. If yes, file
early: review cycles run to weeks and roughly a third land.

## 11. Schedule

Eight weeks, ~12 hours a week.

| Wk | Work | Done when |
|----|------|-----------|
| 1 | Workspace, parser spike + decision, arenas and ids, diagnostic rendering, tone system, config, CLI skeleton | `liar check f.py` renders a finding in all three tones |
| 2 | Index: scopes, bindings, cross-file imports, call graph, `debug resolve` | Names resolve correctly across a real multi-file project |
| 3 | C1 and C2 end to end; `liar-corpus`; first real-world scan | It finds genuine un-awaited calls in real packages |
| 4 | `liar-lsp`, VS Code extension, `await` quick-fix, publish v0.1 | Findings appear live in the editor as you type |
| 5 | Type inference: literals, constructors, returns, `isinstance`, interprocedural fixpoint | `is_ready() -> str` is caught |
| 6 | C3a–C3f | All six pass fixtures; corpus false positives triaged |
| 7 | CFG with exception edges, dataflow solver, C4, `debug cfg` | The `f.close()` example is caught; escapes suppressed |
| 8 | Triage and accuracy number, C5 if it fits, README, GIF, write-up | A stranger gets it in ninety seconds |

Week 4 is deliberately the light one and deliberately early. Getting the
extension working as soon as one checker exists means a demo exists from week 4
onward, and the riskiest weeks (5 and 7) are not also the weeks a release is
due.

## 12. Risks, and what gets cut

**Cut in this order:**

1. **C5 (docstrings).** Pleasant, not load-bearing.
2. **C3c and C3d** (plurals, mutating getters). The two fiddliest naming checks
   for the least return.
3. **C4's escape analysis** — narrow to resources bound once to a plain local
   that is never reassigned, aliased, or passed anywhere. Keeps the CFG and the
   dataflow solver, which are the parts worth having, and drops the fiddliest
   third of the check.

**Do not cut C4 entirely.** It is the only check needing a real CFG with
exception edges, so it is the technical centre of the project and the best
interview material in it. A narrowed C4 is worth far more than none.

**Never cut:** the index, C1 working properly, the VS Code extension, the corpus
run, the README.

**Named risks:**

| Risk | Response |
|---|---|
| Type inference overruns week 5 | Tier it: literals and annotations first, interprocedural fixpoint last and droppable. C3a/C3b/C3e work on the tier-one subset alone. |
| C4 false positives on real code | The escape rule is the lever. If precision is still poor, restrict to resources acquired and released in the same function. |
| Parser choice proves wrong in week 5 | The AST is wrapped behind `liar-core`'s own node types from week 1, so swapping parsers is one crate, not the project. |
| Voice reads as trying too hard | The `professional` tone must be genuinely usable. If the dry tone cannot survive its own fixture snapshots being read aloud, it is too much. |
| Eight weeks becomes twelve | The cut list exists to be used. Week 8 ships whatever weeks 1–7 produced, finished and documented, rather than slipping. |

## 13. What "done" looks like

A README whose first screen contains:

- One sentence saying what it does.
- An animated GIF of it catching something real in VS Code.
- The findings table from the corpus run.
- The precision number, and the note on why recall is absent.
- Install instructions that work.

Then, below the fold, for the reader who keeps going: how the engine works, why
`Unknown` is absorbing, and what the tool deliberately does not do.

The first screen is for the ninety-second reader. Everything below it is for the
interviewer. Both should leave with an accurate impression, and they should be
the *same* impression.
