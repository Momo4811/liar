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
