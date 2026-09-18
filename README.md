# liar

**A static analyser that finds Python code which says one thing and does another.**

```
error[C1]: 'save_user' is never awaited. this line does nothing.
 --> handlers.py:12:5
  |
12|     save_user(request.user)
  |     ^^^^^^^^^^^^^^^^^^^^^^^
```

Calling an async function without `await` builds a coroutine and throws it away.
The body never runs, nothing raises, and the endpoint cheerfully returns
success. It isn't a style opinion — the code does not run.

---

## It finds real bugs

Pointed at 19 async-heavy packages from PyPI — 1,305 files — it found this:

```
warning[C2]: 'time.sleep' blocks. every other task waits for it.
   --> redis/asyncio/cluster.py:994:25
    |
994 |                         time.sleep(watch_delay)
    |                         ^^^^^^^^^^^^^^^^^^^^^^^
```

A synchronous sleep inside `async def transaction`, in the retry path of an
async Redis cluster client. Async code shares one thread between many tasks, so
for the length of that delay *every other task in the process stops*. The fix is
`await asyncio.sleep(watch_delay)`.

One finding, one true positive. The full triage, including a false positive
found and fixed, is in [`corpus/triage.md`](corpus/triage.md).

## What it checks

| | |
|---|---|
| **C1** | An async function called without `await` |
| **C2** | A blocking call inside async code, directly or through a sync helper |

Planned: names that contradict their types (`def is_ready() -> str`), resources
not released on every path out, and docstrings the code disagrees with.

## The rule the whole thing hangs on

> **When unsure, stay silent.**

Every finding asks a stranger to go and change their code. A tool at 60%
accuracy gets muted; at 90% it gets read. So every ambiguity resolves toward
saying nothing — an unresolvable name, an unrecognised decorator, a value whose
type can't be known. It misses real bugs this way, deliberately.

The test suite is shaped by that: **27 negative fixtures against 15 positive
ones**. Tests asserting a bug is found measure recall. False positives are the
only thing that can kill a linter, so most of the suite asserts that it
correctly says *nothing*.

Places it is deliberately blind, each with a fixture proving it:

- a method on an object whose type is unknown — `service.save()`
- anything imported from a module not being analysed
- any decorated definition — `@gen_test` can turn a coroutine function into a
  synchronous one, and then the un-awaited call is correct
- a coroutine assigned to a name that is used again for anything at all

**Recall is not reported, and won't be.** There's no way to know which bugs were
missed without a labelled ground truth that doesn't exist. Inventing a recall
number would be the exact dishonesty this tool is named after.

## It has a voice

The humour comes from being accurate, not from jokes — a gag is funny once and
irritating on the two-hundredth run.

```
professional   coroutine 'save_user' is called but never awaited
dry            'save_user' is never awaited. this line does nothing.
brutal         you wrote 'save_user()' and then threw it away.
```

```bash
liar check src/ --tone brutal
```

Dry is the default. `professional` exists so the tool is usable at work, which
is both a genuine kindness and, in itself, the joke.

## Running it

```bash
cargo run --release -p liar-cli -- check path/to/project
```

Exit codes follow the usual linter convention: `0` clean, `1` findings, `2` the
tool itself failed — so a build script can tell "your code has problems" from
"liar could not run".

When a finding surprises you, ask the engine what it thought:

```bash
$ liar debug resolve app.py:6:5
name:   save
scope:  Function { is_async: true }

resolves to: Function { is_async: true, decorated: false }
defined at:  helpers.py:1:11
```

## How it works

```
  .py files  →  parse  →  index  →  checks  →  report
```

**Parse.** `ruff_python_parser`, confined to a single private module. No
third-party parser type appears anywhere else, which is what makes depending on
an unstable `0.0.x` crate a contained risk rather than a structural one.

**Index.** Scopes, bindings, and imports resolved *between* files, so a name
imported from elsewhere resolves to its real definition. Every check goes
through it rather than matching names textually — a check that matches text is a
regex with extra steps, and it's how false positives get in.

One rule there matters more than the rest: a class body's names are invisible to
functions nested inside it. `class C: x = 1` does not make `x` a bare name in a
method — Python raises `NameError`. Treating it as visible would invent bindings
that don't exist, and every check downstream would inherit the mistake.

**Checks.** Each declares only what it tracks and what counts as a bug. C2's
table of blocking functions is data, keyed on real module paths, so `time.sleep`
matches whether it was written `import time`, `import time as t`, or
`from time import sleep`.

Everything is arena-allocated and referenced by typed integer index — the way
`rustc` and `ruff` are both built. `#![forbid(unsafe_code)]`.

## Tests

```bash
cargo test        # 220 tests
```

Fixtures declare their expectations inline, and **an unexpected finding fails as
loudly as a missing one** — without that, a file with no comments would assert
nothing and every negative fixture would be inert:

```python
def is_ready() -> str:   # expect: C3a
    return "yes"

def is_done() -> bool:   # no comment == must produce nothing
    return True
```

Every false positive found in the corpus is minimised into a permanent negative
fixture *before* it is fixed, so the suite grows with everything real code has
taught it and no false positive can return.

## Status

Working: the engine, the index, C1 and C2, the corpus, the CLI.
Next: a language server and an editor extension, then the remaining checks.

Design notes and the reasoning behind each decision are in
[`docs/`](docs/) — including why the parser was chosen and what was given up
for it.

## Licence

MIT.
