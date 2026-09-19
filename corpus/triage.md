# Corpus triage

Every finding the corpus scan produces is classified here by hand, and every
false positive is minimised into a permanent negative fixture *before* it is
fixed. The suite therefore grows with everything real code has taught it, and
no false positive can come back.

Reproduce with:

```bash
python corpus/fetch.py
cargo run --release -p liar-cli -- check corpus/packages
```

---

## Run 1 — 2026-09-19

**Corpus:** 19 async-heavy packages, 1,305 Python files (`corpus/packages.txt`).
**Findings:** 4.
**Verdicts:** 2 true positive, 2 false positive. **Precision 50%.**

### ✅ `redis/asyncio/cluster.py:994` — C2 — true positive

```python
async def transaction(self, func, *watches, **kwargs):
    async with self.pipeline(True, shard_hint) as pipe:
        while True:
            try:
                ...
            except WatchError:
                if watch_delay is not None and watch_delay > 0:
                    time.sleep(watch_delay)     # blocks the event loop
                continue
```

A synchronous sleep inside `async def transaction`, in the retry path of an
async Redis cluster client. Every other task in the process stops for the
duration of the delay. `await asyncio.sleep(watch_delay)` is the fix.

Unambiguous, and in a library with very wide usage. The strongest kind of
finding this tool can produce: not a style opinion, a real stall.

### ⚠️ `litestar/testing/client/subprocess_client.py:74` — C2 — true positive, not worth reporting

```python
@asynccontextmanager
async def subprocess_async_client(workdir, app):
    with run_app(workdir=workdir, app=app) as url:      # run_app sleeps
```

`run_app` retries in a loop with `time.sleep` while waiting for a subprocess to
boot, and it is entered from async code, so the loop does stall. But it is test
infrastructure and the blocking is the point — there is nothing else to do while
waiting for the app to come up.

**No longer reported**, because `run_app` is decorated and the fix below
suppresses decorated functions. Losing it is the right trade.

### ❌ `tornado/test/testing_test.py:324` and `:333` — C1 — false positive

```python
def test_native_coroutine(self):
    @gen_test
    async def test(self):
        self.finished = True

    test(self)          # reported, wrongly
```

`gen_test` converts a coroutine function into a synchronous one that runs the
coroutine on the IO loop. The call is correct; the decorator changed what
calling the function means.

**Cause:** the engine records that a definition is `async def` but was ignoring
its decorators, so it reasoned from a signature that was no longer the real one.
This is the case §4 of the spec exists for — when unsure, stay silent — and it
simply had not been implemented.

**Fixed by** recording `decorated` on a function binding and saying nothing
about decorated definitions, in both checks.

**Fixtures added first:**
- `tests/fixtures/C1/negative/decorated_callee.py`
- `tests/fixtures/C2/negative/blocking_behind_a_decorator.py`

### After the fix

Re-scanned the same corpus: **1 finding, 1 true positive, precision 100%.**

---

## Run 2 — 2026-09-19 — the C3 family arrives

**Findings:** 887 on the first pass. That is one finding per one and a half
files, which is not a linter, it is wallpaper.

Four rounds of triage brought it to **140**. Every false positive was minimised
into a permanent negative fixture before being fixed, so none can return.

| | first pass | now |
|---|---|---|
| C1 | 0 | 0 |
| C2 | 1 | 1 |
| C3a | 10 | 7 |
| C3b | 56 | 19 |
| C3e | **799** | 109 |
| C3f | 21 | 4 |
| **total** | **887** | **140** |

### ❌ A parameter's caret covered its annotation — a real bug

`readinto(self, b: bytearray)` underlined `b: bytearray` rather than `b`,
because `Param` carried a single span covering the whole parameter. Visible in
every parameter finding and invisible in the fixtures, which had no annotated
parameters.

Fixed by giving `Param` a `name_span`, as `FunctionDef` already had.

### ❌ TypeVars — 81 of the 799

`T`, `P`, `KT`. A short uppercase name is the convention, not a lapse. Short
uppercase names are now exempt outright.

### ❌ Single letters — roughly a hundred more

`r`, `p`, `c`, `m`, `b`. In a short scope they are idiomatic; in a long one
they are usually a parameter whose name is part of an interface the author did
not choose — `readinto(self, b)` is the standard library's own signature, and a
parameter name is API that callers can pass by keyword.

Single letters are no longer flagged at all. Only words that promise meaning and
deliver none.

### ❌ Verb phrases read as quantities

`_verify_content_length() -> None`, `populate_content_length() -> bool`,
`prepend_size = True`, `should_remove_content_length = True`.

Three rules, each narrow:

- A name that asks a question is a question, whatever it ends with.
- A function returning `None` is a procedure; its name says what it does.
- A boolean is never a broken promise about a number.

The last one costs a real finding: `count = True` now goes unmentioned. That is
a rarer mistake than the ones the rule prevents, and this tool would rather miss
than misfire.

### ❌ `None` initialisers

`should_terminate = None`, `count = None`. `None` is a legitimate initial value
for absolutely anything. A flag being set up is not a name lying about what it
holds.

### ⚖️ `data` × 187, `value` × 168, `result` × 84 — true, and useless

These are genuinely uninformative names. They are also so common that reporting
all of them is wallpaper, and a check that fires eight hundred times gets muted
exactly as fast as one that is wrong.

The C3e scope threshold moved from 20 lines to 60, which cut it to 109.

**C3e is the weakest check in the tool and this is an honest place to say so.**
It has no correctness content at all — it is a style opinion, and a defensible
one, but a team that disagrees should turn it off:

```toml
ignore = ["C3e"]
```

### ✅ Still standing

- **C2 × 1** — the blocking sleep in redis-py's async cluster client. Unchanged
  and still the best finding the tool has produced.
- **C3a × 4** — celery's `is_due()` returns `(is_due, next_run_time)`. A name
  that asks a yes-or-no question and hands back a pair. A real, if venerable,
  confusion.
- **C3b × 7** — `content_length` holding a `str`. True: it is the raw header
  value and the code parses it later. Low value, and arguably the protocol's
  fault rather than the author's.
- **C3b × 12** — `count`, `file_size`, `new_count`, `all_total_count` holding
  lists and strings. Plausible, individually unremarkable.

### ⚠️ Known false positives, deliberately not fixed

Two of the four C3f findings are wrong, and fixing them needs something the
engine does not currently keep.

**`response = str_if_bytes(response)`** in redis. The name means the same thing
throughout — it is being converted, not redefined. Detecting that needs the
value expression at each binding site, and `Types` records only the type. Not a
heuristic worth inventing; a structural change worth making deliberately.

**`client_max_window_bits: int | Literal[True] | None`** in websockets. The
annotation declares a union, so the author has already said it holds several
types. `int | X` is a binary operation the syntax tree does not model, so it
becomes `Unknown`, and the *other* binding sites are what trip the count.

Both are recorded here rather than papered over. They are the honest cost of
shipping C3f now rather than waiting.

---

## On recall

Recall is not reported here, and will not be. There is no way to know which
bugs were missed without a labelled ground truth that does not exist for this
corpus. Inventing a recall number would be the exact dishonesty this tool is
named after.

What can be said plainly is where the checks are deliberately blind, each with a
negative fixture proving it:

- a method on an object whose type is unknown (`service.save()`)
- anything imported from a module not being analysed
- any decorated definition
- a coroutine assigned to a name that is used again for anything at all
- destructuring assignment targets

## An earlier run worth recording

Before this corpus existed, the same build was pointed at the local Python
installation: **3,003 files, 205 async functions, zero findings, zero panics.**

That is not a failure. An un-awaited coroutine fails the first time the code
runs, so it does not survive into released packages, and almost none of that
3,003 files is async anyway. The lesson was that a checker about async code has
to be aimed at async code, which is why `packages.txt` is chosen for async
density rather than download count.

It remains the strongest available evidence on the precision side: 3,003 files
of real code, not one false positive.
