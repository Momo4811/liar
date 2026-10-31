# Changelog

## 0.1.0

First release.

- Findings from all eight checks, on save, across the whole project.
- **Add await** quick fix for C1, inside `async def` only.
- `liar.tone`, `liar.enable` and `liar.path` settings.
- `liar.toml` at the workspace root is applied, and re-applied when it changes.
- Virtual environments, `node_modules` and `__pycache__` are skipped.
