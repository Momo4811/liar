# liar — Plan 3: Editor integration

**Goal:** Findings appear in VS Code as you save, with the voice intact and a
one-click fix for the missing `await`.

**Architecture:** A language server (`liar-lsp`) wraps the existing engine and
speaks LSP over stdio. The VS Code extension is a launcher and nothing more —
it starts the server, forwards messages, and surfaces settings. Any logic that
appears in the extension belongs in `liar-core` instead.

**Tech stack:** `tower-lsp` for the server, TypeScript for the extension shell.
No change to the engine.

**Spec:** `docs/design/2026-09-15-liar-design.md` §6 (delivery), §9.5 (LSP
tests). **Previous:** `docs/plans/2026-09-18-liar-02-index-and-first-checkers.md`.

---

## Why this now

Two checks work and one of them found a real bug in a shipped library. But the
tool is still a thing you remember to run, and the class of bug it catches is
one that fails the first time the code executes — which is exactly why the
corpus scan of 3,003 files of *released* code found nothing. An un-awaited
coroutine does not survive to release; it is caught in testing, or by a
`RuntimeWarning`, or by the developer noticing nothing happened.

So the value is not in scanning finished code. It is in saying something the
moment the line is written. That is what this plan builds.

---

## Global Constraints

Everything from Plans 1 and 2 still applies. Added:

- **The extension holds no analysis logic.** Target: under 300 lines of
  TypeScript. If a rule, a threshold, or a message ever appears in it, it is in
  the wrong crate.
- **The server must never crash the editor.** A panic in analysis is caught and
  reported as a failed request, not propagated. An editor that dies because a
  linter hit an unmodelled construct is worse than no linter.
- **Analysis runs on save, not on keystroke.** Re-indexing a project on every
  character is a performance problem this plan deliberately does not take on.
  If it feels slow on save, that is the signal to revisit it — not before.
- **Binary distribution stays out of scope.** The extension finds `liar` on
  `PATH` or at a configured location and says something useful if it cannot.
  Bundling per-platform binaries serves reach, and reach is a non-goal.

---

## Design

### What the server holds

One workspace's `SourceMap`, `Ast` map, and the files discovered under the
workspace root. On `didSave`, the saved file is re-read and re-parsed, the index
is rebuilt across the workspace, and diagnostics are published for every file
that has any — not only the saved one, because a change in `helpers.py` can
create or clear a finding in `app.py`.

Rebuilding the whole index on each save is deliberate. It is simple, it is
correct, and at the speed already measured — 1,305 files in 1.2 seconds — it is
comfortably fast enough for the project sizes anyone will point this at. An
incremental index is the kind of thing that looks essential and turns out to be
a week of cache-invalidation bugs.

### Diagnostics

A `Finding` becomes an LSP `Diagnostic`:

| Finding | Diagnostic |
|---|---|
| `check.code()` | `code` |
| rendered message, in the configured tone | `message` |
| `Severity::Error` / `Warning` | `DiagnosticSeverity::ERROR` / `WARNING` |
| `primary.span` | `range`, converted through `SourceFile::position` |
| `secondary` labels | `relatedInformation` |

The span conversion is where this could go quietly wrong: LSP ranges are
line/character pairs, and its characters are **UTF-16 code units**, not
characters and not bytes. A file with an emoji before a finding on the same line
will underline the wrong text if this is done naively. That gets its own
function and its own tests.

### The quick fix

One code action: **insert `await`**, offered only on a C1 diagnostic and only
when the enclosing function is `async def`. Adding `await` inside a plain `def`
would produce a syntax error, which is a worse outcome than the original bug.

The engine already knows whether the enclosing function is async — that is what
`ScopeTree::in_async_function` answers — so the action is a text edit inserting
`await ` at the start of the diagnostic's range.

### Settings

`liar.tone`, `liar.enable`, `liar.path`, mirroring `liar.toml` so a project's
committed configuration and a developer's editor agree. The extension passes
them through; it does not interpret them.

---

## File Structure

```
crates/liar-lsp/
├── Cargo.toml
└── src/
    ├── main.rs        binary: stdio transport, panic guard
    ├── server.rs      the LanguageServer impl
    ├── workspace.rs   discovery, parsing, re-analysis
    └── convert.rs     Finding -> Diagnostic, spans -> UTF-16 ranges

editors/vscode/
├── package.json       manifest, settings, activation
├── tsconfig.json
└── src/extension.ts   launcher only
```

---

## Tasks

### Task 1 — UTF-16 range conversion

The piece most likely to be subtly wrong, so it is built first and alone.

**Produces:** `fn to_lsp_range(&SourceFile, Span) -> Range`, counting UTF-16
code units.

**Tests:** ASCII; a two-byte character before the span on the same line; an
astral-plane character before it (**two** UTF-16 units, one character, four
bytes — the case that separates a correct implementation from one that merely
looks right); a span at the start of a file; at the end; spanning lines; an
empty span; and a property test that a round trip through
`SourceFile::offset_at` returns the original offset.

### Task 2 — The workspace

**Produces:** `Workspace::open(root)`, `Workspace::save(path, text)`,
`Workspace::diagnostics() -> HashMap<Url, Vec<Diagnostic>>`.

**Tests:** opening a directory finds its Python files; saving re-analyses;
saving a file that does not parse reports the syntax error as a diagnostic
rather than failing the request; **editing a definition in one file updates the
diagnostics of a file that imports it**; a file whose findings are resolved has
its diagnostics cleared rather than left stale.

### Task 3 — The server

**Produces:** the `LanguageServer` impl: `initialize`, `initialized`,
`did_open`, `did_save`, `did_change` (text tracked, analysis deferred to save),
`did_close`, `shutdown`.

**Tests:** a protocol-level harness driving the server over stdio, with no
editor in the loop, so a failure is unambiguous about where it came from.
Initialize returns the expected capabilities; opening a file publishes
diagnostics; saving republishes; closing clears; a panic inside analysis
returns an error rather than killing the process.

### Task 4 — The `await` quick fix

**Produces:** `code_action`, offering one fix on C1 diagnostics.

**Tests:** offered on C1 inside an `async def`; **not** offered on C1 inside a
plain `def`; not offered on C2; the edit inserts `await ` at the right offset;
the edited text parses and the finding is gone.

### Task 5 — The extension

**Produces:** `editors/vscode/`, publishable, launching the server and
surfacing settings.

**Tests:** the standard VS Code integration harness — activates, finds the
binary or reports its absence usefully, surfaces diagnostics. Plus a manual
check with a screen recording for the README, which is the artifact most people
will actually judge this on.

---

## Done when

- [ ] Saving a Python file in VS Code shows liar's findings inline
- [ ] The tone setting changes the message without restarting the editor
- [ ] A finding on a line containing an emoji underlines the right text
- [ ] The `await` quick fix appears on C1 in async functions and nowhere else
- [ ] A change in one file updates diagnostics in a file that imports it
- [ ] The server survives a panic in analysis without taking the editor with it
- [ ] All gates green, and the extension builds

## What Plan 4 picks up

Type inference, and the C3 family — the checks with the most personality, and
the ones that need to know that `is_ready` returns a `str`.
