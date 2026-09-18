//! One open project: its files, their text, and the findings in them.

use crate::convert::{to_diagnostic, to_lsp_range, to_url};
use liar_core::analysis::analyse;
use liar_core::ast::{Ast, parse};
use liar_core::ids::{FileId, Id};
use liar_core::messages::{MessageTable, Tone};
use liar_core::source::SourceMap;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Url};

/// The text of every Python file in the workspace.
///
/// Text is the only state. Everything else - trees, index, findings - is
/// derived on each analysis and thrown away.
///
/// That is deliberate. Parsing costs about 0.6ms a file, so a save re-derives
/// a hundred-file project in roughly sixty milliseconds, and there is no cache
/// to invalidate wrongly. The index in particular has to be rebuilt across the
/// whole workspace anyway: editing `helpers.py` can create or clear a finding
/// in `app.py`, which has not changed.
pub struct Workspace {
    texts: BTreeMap<PathBuf, String>,
    tone: Tone,
}

impl Workspace {
    pub fn new(tone: Tone) -> Self {
        Self {
            texts: BTreeMap::new(),
            tone,
        }
    }

    /// Loads every Python file under `root`.
    pub fn open(&mut self, root: &Path) {
        for path in discover(root) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                self.update(&path, text);
            }
        }
    }

    pub fn set_tone(&mut self, tone: Tone) {
        self.tone = tone;
    }

    /// Records the current text of a file.
    pub fn update(&mut self, path: &Path, text: String) {
        self.texts.insert(path.to_path_buf(), text);
    }

    pub fn forget(&mut self, path: &Path) {
        self.texts.remove(path);
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// The byte offset of an LSP position in `path`.
    pub fn offset_of(&self, path: &Path, position: tower_lsp::lsp_types::Position) -> Option<u32> {
        let text = self.texts.get(path)?;
        let mut sources = SourceMap::new();
        let id = sources.add(path.to_path_buf(), text.clone());
        crate::convert::from_lsp_position(sources.get(id), position)
    }

    /// Whether a byte offset in `path` sits inside an `async def`.
    ///
    /// Only the one file is parsed: whether the enclosing function is async is
    /// a purely local question, so there is no reason to rebuild the whole
    /// index to answer it.
    pub fn is_inside_async_function(&self, path: &Path, offset: u32) -> bool {
        let Some(text) = self.texts.get(path) else {
            return false;
        };
        let Ok(ast) = parse(text) else {
            return false;
        };

        let index = liar_core::index::build_file(FileId::from_index(0), &ast);

        // The smallest statement containing the offset is the one written
        // there; its scope is the scope that offset sits in.
        ast.stmts()
            .filter(|(id, _)| {
                index.stmt_scope.contains_key(id) && ast.stmt_span(*id).contains(offset)
            })
            .min_by_key(|(id, _)| ast.stmt_span(*id).len())
            .map(|(id, _)| index.scopes.in_async_function(index.scope_of(id)))
            .unwrap_or(false)
    }

    /// Diagnostics for every file, including the ones with none.
    ///
    /// An empty list is how LSP says "this file is clean now". Omitting a file
    /// would leave its last diagnostics on screen forever.
    pub fn diagnostics(&self) -> HashMap<Url, Vec<Diagnostic>> {
        let mut sources = SourceMap::new();
        let mut ids: BTreeMap<PathBuf, FileId> = BTreeMap::new();

        for (path, text) in &self.texts {
            ids.insert(path.clone(), sources.add(path.clone(), text.clone()));
        }

        let mut by_url: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
        let mut parsed: BTreeMap<FileId, Ast> = BTreeMap::new();

        for id in ids.values() {
            let file = sources.get(*id);
            let Some(url) = to_url(file) else {
                continue;
            };
            // Every file gets an entry, even a clean one: an empty list is how
            // LSP says "clean now", and omitting the file would leave its last
            // diagnostics on screen forever.
            by_url.entry(url.clone()).or_default();

            match parse(file.text()) {
                Ok(ast) => {
                    parsed.insert(*id, ast);
                }
                Err(error) => by_url.entry(url).or_default().push(Diagnostic {
                    range: to_lsp_range(file, error.span),
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("syntax".to_string())),
                    source: Some("liar".to_string()),
                    message: error.message.clone(),
                    ..Default::default()
                }),
            }
        }

        let messages = MessageTable::embedded();
        for finding in analyse(&sources, &parsed) {
            let file = sources.get(finding.primary.file);
            let Some(url) = to_url(file) else {
                continue;
            };
            let message = messages.render(&finding, self.tone);
            if let Some(diagnostic) = to_diagnostic(&finding, message, &sources) {
                by_url.entry(url).or_default().push(diagnostic);
            }
        }

        by_url
    }
}

fn discover(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if hidden {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("py" | "pyi")
            ) {
                found.push(path);
            }
        }
    }

    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().expect("temp dir");
        for (name, source) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, source).unwrap();
        }
        let mut workspace = Workspace::new(Tone::Dry);
        workspace.open(dir.path());
        (dir, workspace)
    }

    fn codes(workspace: &Workspace) -> Vec<String> {
        let mut found: Vec<String> = workspace
            .diagnostics()
            .values()
            .flatten()
            .filter_map(|d| match &d.code {
                Some(NumberOrString::String(code)) => Some(code.clone()),
                _ => None,
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn opening_a_directory_finds_its_python_files() {
        let (_dir, workspace) = workspace_with(&[
            ("a.py", "x = 1\n"),
            ("pkg/b.py", "y = 2\n"),
            ("notes.md", "not python"),
        ]);
        assert_eq!(workspace.len(), 2);
    }

    #[test]
    fn a_clean_project_reports_no_findings() {
        let (_dir, workspace) = workspace_with(&[("a.py", "def add(a, b):\n    return a + b\n")]);
        assert!(codes(&workspace).is_empty());
    }

    #[test]
    fn every_file_gets_an_entry_even_when_clean() {
        // An empty list is how LSP says "clean now". Omitting the file would
        // leave its last diagnostics on screen forever.
        let (_dir, workspace) = workspace_with(&[("a.py", "x = 1\n"), ("b.py", "y = 2\n")]);
        assert_eq!(workspace.diagnostics().len(), 2);
    }

    #[test]
    fn a_finding_is_reported() {
        let (_dir, workspace) = workspace_with(&[(
            "a.py",
            "async def save():\n    pass\n\n\nasync def handle():\n    save()\n",
        )]);
        assert_eq!(codes(&workspace), vec!["C1"]);
    }

    #[test]
    fn a_syntax_error_becomes_a_diagnostic_rather_than_a_failure() {
        let (_dir, workspace) = workspace_with(&[("a.py", "def (:\n")]);
        assert_eq!(codes(&workspace), vec!["syntax"]);
    }

    #[test]
    fn saving_re_analyses() {
        let (dir, mut workspace) = workspace_with(&[(
            "a.py",
            "async def save():\n    pass\n\n\nasync def handle():\n    save()\n",
        )]);
        assert_eq!(codes(&workspace), vec!["C1"]);

        workspace.update(
            &dir.path().join("a.py"),
            "async def save():\n    pass\n\n\nasync def handle():\n    await save()\n".into(),
        );
        assert!(
            codes(&workspace).is_empty(),
            "the fix should clear the finding"
        );
    }

    #[test]
    fn editing_one_file_updates_diagnostics_in_a_file_that_imports_it() {
        // The reason the index is rebuilt across the workspace rather than for
        // the saved file alone.
        let (dir, mut workspace) = workspace_with(&[
            ("helpers.py", "def save():\n    pass\n"),
            (
                "app.py",
                "from helpers import save\n\n\nasync def handle():\n    save()\n",
            ),
        ]);
        assert!(
            codes(&workspace).is_empty(),
            "save is sync, so nothing is wrong yet"
        );

        // Make it async. app.py has not changed, but it now has a bug.
        workspace.update(
            &dir.path().join("helpers.py"),
            "async def save():\n    pass\n".into(),
        );
        assert_eq!(codes(&workspace), vec!["C1"]);
    }

    #[test]
    fn forgetting_a_file_removes_its_diagnostics() {
        let (dir, mut workspace) = workspace_with(&[(
            "a.py",
            "async def save():\n    pass\n\n\nasync def handle():\n    save()\n",
        )]);
        assert_eq!(codes(&workspace).len(), 1);

        workspace.forget(&dir.path().join("a.py"));
        assert!(workspace.is_empty());
        assert!(workspace.diagnostics().is_empty());
    }

    #[test]
    fn the_tone_changes_the_message() {
        let (_dir, mut workspace) = workspace_with(&[(
            "a.py",
            "async def save():\n    pass\n\n\nasync def handle():\n    save()\n",
        )]);

        let dry = workspace
            .diagnostics()
            .values()
            .flatten()
            .next()
            .unwrap()
            .message
            .clone();
        workspace.set_tone(Tone::Brutal);
        let brutal = workspace
            .diagnostics()
            .values()
            .flatten()
            .next()
            .unwrap()
            .message
            .clone();

        assert_ne!(dry, brutal);
        assert!(brutal.contains("threw it away"), "got: {brutal}");
    }

    #[test]
    fn a_call_inside_an_async_function_is_recognised() {
        let (dir, workspace) = workspace_with(&[(
            "a.py",
            "async def save():\n    pass\n\n\nasync def handle():\n    save()\n",
        )]);
        let path = dir.path().join("a.py");
        let offset = "async def save():\n    pass\n\n\nasync def handle():\n    ".len() as u32;
        assert!(workspace.is_inside_async_function(&path, offset));
    }

    #[test]
    fn a_call_inside_a_plain_function_is_not() {
        // Inserting await here would be a syntax error, which is worse than
        // the bug it was meant to fix.
        let (dir, workspace) = workspace_with(&[(
            "a.py",
            "async def save():\n    pass\n\n\ndef handle():\n    save()\n",
        )]);
        let path = dir.path().join("a.py");
        let offset = "async def save():\n    pass\n\n\ndef handle():\n    ".len() as u32;
        assert!(!workspace.is_inside_async_function(&path, offset));
    }

    #[test]
    fn module_level_code_is_not_inside_an_async_function() {
        let (dir, workspace) = workspace_with(&[("a.py", "x = 1\n")]);
        assert!(!workspace.is_inside_async_function(&dir.path().join("a.py"), 0));
    }

    #[test]
    fn an_unknown_file_is_not_inside_anything() {
        let (dir, workspace) = workspace_with(&[("a.py", "x = 1\n")]);
        assert!(!workspace.is_inside_async_function(&dir.path().join("nope.py"), 0));
    }

    #[test]
    fn hidden_directories_are_skipped() {
        let (_dir, workspace) = workspace_with(&[("a.py", "x = 1\n"), (".venv/lib/b.py", "y\n")]);
        assert_eq!(workspace.len(), 1);
    }
}
