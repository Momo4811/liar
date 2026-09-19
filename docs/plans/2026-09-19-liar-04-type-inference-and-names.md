# liar — Plan 4: Type inference and the names that lie

**Goal:** `def is_ready() -> str` gets caught. So does `count = []`, and four
variables called `data` that mean four different things.

**Architecture:** A small type lattice, inference over it, and the C3 family of
checks on top. Inference is deliberately shallow — it answers "is this a
boolean", "is this a number", "is this a collection" and not much else, because
that is all these checks ask.

**Spec:** `docs/design/2026-09-15-liar-design.md` §5 (C3a–C3f), §7 (inference).
**Previous:** `docs/plans/2026-09-19-liar-03-editor-integration.md`.

---

## Global Constraints

Everything from Plans 1–3 still applies. Added:

- **`Unknown` is absorbing and no check may fire on it.** This is the silence
  rule from §4 made structural: it is not possible to forget it in one branch of
  one check, because the lattice carries it.
- **Name heuristics live in data, not code.** Which prefixes read as boolean,
  which words are uninformative, which nouns have no plural — all of it in
  `data/names.toml`, so disagreeing with a judgement is a one-line change plus a
  fixture, not a patch to a checker.
- **Every C3 sub-check gets negative fixtures for its heuristic's blind spots.**
  These checks are opinions about naming, which makes them the likeliest of all
  to annoy people. The ratio of negative to positive fixtures should get worse
  here, not better.

---

## Design

### The lattice

```rust
enum Ty {
    Unknown,                    // ⊤ — absorbing; silences every check
    Never,                      // ⊥ — no reachable value
    Int, Float, Complex, Str, Bytes, Bool, NoneType, Ellipsis,
    List, Dict, Set, Tuple,
    Instance(ClassKey),         // an instance of a class in this project
}

fn join(a, b) = if a == b { a }
                else if a == Never { b }
                else if b == Never { a }
                else { Unknown }
```

Two different concrete types join to `Unknown`, not to a union. A union type
would let a check reason about `int | str`, and there is nothing useful for
these checks to conclude from one. Collapsing is both simpler and more
conservative, and conservative is the direction that matters.

The five laws — commutativity, associativity, idempotence, `Never` as identity,
`Unknown` as absorbing — are property-tested. They are not stylistic: they are
the preconditions for the fixpoint converging at all, and for the answer not
depending on which order files happened to be visited.

### Two views of a binding

This is the decision the whole plan turns on. A name bound several times needs
to be seen two ways at once:

- **The join**, for C3a/C3b/C3c: if `count` is an `int` here and a `list` there,
  its type is `Unknown` and nothing is said about it.
- **The sites**, for C3f: *"four variables called `data`, none related"* is
  precisely a statement about the individual binding sites and their differing
  types. Joining them first would destroy the only information that check needs.

So inference records `Vec<(Span, Ty)>` per name per scope, and the join is
derived from it rather than stored instead of it.

### What is inferred

Annotations; literals; list/dict/set/tuple displays; constructor calls to
project classes; a small table of stdlib return types; a function's return type
as the join of its `return` expressions, with a recursion guard; and a variable's
type from what is assigned to it.

`Subscript` is added to the syntax tree for this, because without it every
`list[int]`, `Optional[str]` and `Dict[str, int]` annotation is `Unknown` — which
is most annotations in modern Python. Only the base is read: `list[int]` is a
`List`. The element type is not tracked and nothing here needs it.

### What is not

Generics and `TypeVar`. Protocols. Decorators that alter signatures — already
handled by staying silent about decorated definitions. Metaclasses,
`__getattr__`, dynamic attributes. `*args`/`**kwargs` forwarding. Unwrapping
`Optional[X]` to `X`, because `None` is a legitimate value of that annotation and
pretending otherwise would manufacture findings.

Everything out of scope yields `Unknown`, and therefore silence.

### The checks

| id | Fires when | Chief risk |
|----|-----------|-----------|
| C3a | `is_`/`has_`/`can_`/`should_` name, non-`bool` type | a predicate returning a truthy object on purpose |
| C3b | `count`/`num_`/`*_size` name, non-numeric type | `size` meaning dimensions, `count` meaning a counter object |
| C3c | plural name holding a scalar, or singular holding a collection | uncountable nouns — `data`, `status`, `address` |
| C3d | `get_*` that writes to `self`, a global, or a known mutator | caching getters, which are idiomatic |
| C3e | uninformative name across a scope longer than the threshold | short names in short scopes are *good* style |
| C3f | one name, three or more binding sites, two or more distinct types | a genuinely reused loop variable |

C3e's threshold is the important one. `i` in a three-line loop is good style, and
a tool that does not know this is a tool people uninstall. It fires only when the
scope exceeds a configurable number of lines, defaulting to 20.

---

## Tasks

Each ends with fixtures passing, gates green, and a commit.

**1 — `Ty` and the lattice.** The five laws as `proptest` properties; the
predicates `is_bool`, `is_numeric`, `is_collection`; a display name for messages.

**2 — `Subscript` in the syntax tree, and annotations to types.** `int`, `str`,
`bool`, `list`, `dict`, …; `-> None`; `list[int]` → `List`; an unknown name →
`Unknown`; a string annotation → `Unknown`.

**3 — Expression inference.** Literals, displays, calls to project functions and
classes, a stdlib return table, `await`. Everything else `Unknown`.

**4 — Binding types.** Per-site and joined, per scope. Function return types as
the join of `return` expressions, with a recursion guard, iterated to a fixpoint.

**5 — C3a and C3b.** The two that need only a type and a name.

**6 — C3e and C3f.** The two that need no type inference at all — and C3f is the
one that motivated the whole project.

**7 — C3c and C3d.** The fiddliest two. First on the cut list if they cannot be
made precise.

**8 — Corpus.** Re-scan, triage by hand, minimise every false positive into a
fixture before fixing it, update the baseline and the README.

---

## Done when

- [ ] `def is_ready() -> str` is caught; `def is_done() -> bool` is not
- [ ] `count = []` is caught; `count = 0` is not
- [ ] Four variables called `data` are one finding with three secondary labels
- [ ] `i` in a short loop is not mentioned; `data` across 80 lines is
- [ ] The lattice laws hold under `proptest`
- [ ] Negative fixtures still outnumber positive ones, by more than before
- [ ] The corpus is re-scanned and every finding triaged in writing
- [ ] All gates green

## What Plan 5 picks up

Control flow graphs with exception edges, and C4 — the resource that leaks when
something goes wrong. The technical centre of the project.
