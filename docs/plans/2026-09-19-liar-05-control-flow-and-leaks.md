# liar — Plan 5: Control flow, and the cleanup that lies

**Goal:**

```python
def read_config(path):
    f = open(path)
    data = parse(f.read())   # if this raises...
    f.close()                # ...this never runs
    return data
```

gets caught.

**Architecture:** A control flow graph per function, carrying **exception
edges**; a generic dataflow solver over it; and C4 on top.

**Spec:** `docs/design/2026-09-15-liar-design.md` §5 (C4), §8.3 (control flow),
§8.4 (dataflow). **Previous:**
`docs/plans/2026-09-19-liar-04-type-inference-and-names.md`.

---

## Why this is the interesting one

Every check so far reasons about a statement, or a name, or a call. This one
reasons about *paths* — and specifically about the paths nobody writes down.

The function above is fine on the happy path. It leaks a file handle every time
`parse` raises. Do it in a request handler and you eventually hit the operating
system's file limit and the process dies, a long way from the line responsible.

Existing linters miss this because modelling it requires admitting that almost
every statement in Python has an invisible edge leaving it. That admission is
the whole plan.

---

## Global Constraints

Everything from Plans 1–4 still applies. Added:

- **Every statement that can raise gets an exception edge.** The
  over-approximation is deliberate and the direction is safe: more paths means
  more places a resource *might* leak, and C4 only reports a leak on *some*
  path, so over-approximating costs precision rather than soundness.
- **A resource that escapes is not tracked.** Returned, stored on an attribute,
  put in a container, or handed to another function — once it leaves, its
  lifetime is somebody else's business. This single rule is most of the
  precision on this check; without it the tool is unusable.
- **Golden CFG files for every statement shape.** A control flow graph is
  exactly the kind of thing that looks right and is subtly wrong, so the shapes
  are pinned as text and reviewed by eye once.

---

## Design

### The graph

```rust
struct Block  { stmts: Vec<StmtId>, span: Span }
enum  Edge    { Normal, Exception }
struct Cfg    { blocks, edges, entry, normal_exit, exception_exit }
```

Two exits, not one. A function can finish by returning or by propagating an
exception, and C4 has to check both — the second is the whole point.

Construction, by statement kind:

| | |
|---|---|
| a run of simple statements | one block |
| `if` | branch to body and to `orelse`, rejoining after |
| `while` / `for` | a header block, an edge into the body and one past it, and a back edge |
| `try` | exception edges from the body into every handler; handlers, `else` and `finally` rejoin |
| `return` | an edge to the normal exit; anything after it is unreachable |
| anything that can raise | an extra edge to the nearest enclosing handler, or to the exception exit |

**"Can raise" means "contains a call."** Crude, and deliberately so. Almost
anything in Python can raise; assuming calls do is the cheapest rule that
catches the case this check exists for, and erring toward more paths errs
toward silence.

### The solver

One generic worklist fixpoint over a `Lattice` trait — `bottom`, `join`, `eq` —
so a check supplies a lattice and transfer functions and gets convergence for
free. C4 is its first user; the side-effect analysis C3d would need is its
second, if that check is ever built.

### C4

Track each local bound from a resource-acquiring call. Per variable, the state
is `Open`, `Closed` or `Unknown`, joined at merge points with `Unknown`
absorbing as ever.

At **every** exit, normal and exceptional, a variable still `Open` is a finding,
reported with how many paths released it and how many there were.

Not tracked at all: anything bound by a `with` statement, which is the language
solving the problem correctly; and any resource that escapes.

---

## Tasks

**1 — The graph.** Blocks, edges, two exits, construction for every statement
shape the syntax tree models. Golden Graphviz files, pinned and reviewed.

**2 — `liar debug cfg`.** Prints a function's graph. The same reasoning as
`debug resolve`: without it, a wrong graph is debugged by staring.

**3 — The solver.** Generic worklist fixpoint. Property-tested for termination
and for independence from worklist order.

**4 — C4.** The resource table as data, the escape rule, the fixture set.

**5 — Corpus.** Scan, triage in writing, minimise every false positive into a
fixture before fixing it.

---

## Done when

- [ ] The `read_config` example above is caught
- [ ] The same code inside a `with` is not
- [ ] The same code inside `try`/`finally` is not
- [ ] A handle that is returned, stored or passed on is not
- [ ] Golden CFG files exist for `if`, `while`, `for`, `try`, `return` and a
      bare sequence
- [ ] The solver's fixpoint is independent of worklist order
- [ ] All gates green, corpus triaged

## What Plan 6 picks up

C5 — docstrings the code disagrees with — then the README, the screen
recording, and whatever the last corpus run turns up.
