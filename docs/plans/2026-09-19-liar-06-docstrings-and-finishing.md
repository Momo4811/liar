# liar — Plan 6: Docstrings that lie, and finishing

**Goal:** the last check, and the repository in a state a stranger can judge.

**Spec:** `docs/design/2026-09-15-liar-design.md` §5 (C5), §13 (done).
**Previous:** `docs/plans/2026-09-19-liar-05-control-flow-and-leaks.md`.

---

## Design

### Getting at the text

A docstring is the first statement of a function, being a string literal. The
syntax tree records that a literal is a `Str` but not what it said, and widening
it to carry every string's contents would cost memory on every file to serve one
check.

So C5 slices the source using the literal's span and strips the quoting. That is
the only place in the project where a check reads raw text, and it is worth
saying why it is acceptable here: the text *is* the subject. Every other check
reads text only to guess at structure, which is the thing this tool is against.

### What is checked

Three claims, chosen because each can be checked without understanding prose.

**C5 — a documented parameter that does not exist.**

```python
def fetch(url, timeout):
    """
    Args:
        url: where to fetch from
        retries: how many times to try      <- there is no `retries`
    """
```

Almost always a rename that missed the docstring. Google, NumPy and Sphinx
styles are all recognised.

The converse — a parameter that exists and is undocumented — is deliberately
**not** checked. Partial docstrings are normal and useful, and reporting them
would be a style opinion dressed up as a correctness one.

**C5 — a documented return value from a function that returns nothing.**

```python
def save(record):
    """Returns the saved record's id."""
    self.records.append(record)       # returns None on every path
```

Uses the inferred return type from Plan 4. Fires only when the type is exactly
`NoneType` — never on `Unknown`.

**C5 — an `:rtype:` that contradicts the inferred type.**

Only the Sphinx form, because it names a type unambiguously. Prose like
"returns a list of users" is not parsed: guessing at English is exactly the
kind of invention this tool is named after.

### Precision rules

As ever: decorated functions are skipped; `self` and `cls` are never counted as
parameters; `*args` and `**kwargs` absorb any documented name, so a function
taking them is not judged on its parameter list at all.

---

## Tasks

**1 — Reading a docstring.** Extraction from a span, quote and prefix stripping,
and parsing the three section styles. Tested hard, because this is the one place
text is read directly.

**2 — C5.** The three claims, with fixtures.

**3 — The corpus.** Scan, triage in writing, fixtures before fixes.

**4 — Finishing.** The README's front page, the honest status, and whatever the
last scan turns up.

---

## Done when

- [ ] A documented parameter that does not exist is caught
- [ ] A partial docstring is not
- [ ] A promised return value from a function returning `None` is caught
- [ ] All three docstring styles are recognised
- [ ] The corpus is scanned and triaged
- [ ] All gates green, and the README is something a stranger can judge
