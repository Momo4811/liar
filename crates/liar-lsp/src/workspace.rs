//! One open project: its files, their text, and the findings in them.

use crate::convert::{to_diagnostic, to_lsp_range, to_url};
use liar_core::analysis::analyse_with;
use liar_core::ast::{Ast, parse};
use liar_core::config::{Config, ConfigError};
use liar_core::discover::{DiscoverError, Filter, discover};
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
    root: Option<PathBuf>,
    /// The project's `liar.toml`. Its tone is not used: the tone is the
    /// editor's setting, a personal preference rather than a project one.
    config: Config,
    filter: Filter,
    /// Makes analysis panic, so the server's guard against it can be tested.
    /// Compiled out of the real binary.
    #[cfg(test)]
    pub(crate) panic_on_analysis: bool,
}

/// Why a project's configuration could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("liar.toml: {0}")]
    Config(#[from] ConfigError),
    #[error("liar.toml: {0}")]
    Discover(#[from] DiscoverError),
}

impl Workspace {
    pub fn new(tone: Tone) -> Self {
        Self {
            texts: BTreeMap::new(),
            tone,
            root: None,
            config: Config::default(),
            filter: Filter::new(&[]).expect("no patterns cannot be a bad pattern"),
            #[cfg(test)]
            panic_on_analysis: false,
        }
    }

    /// Loads `root`'s configuration and every Python file it selects.
    ///
    /// The files are the ones `liar check` would analyse from the same root,
    /// by the same code, so the editor and the command line cannot disagree.
    /// A broken `liar.toml` is reported and the defaults are used: the user
    /// still gets findings, and is told why they are not the ones configured.
    pub fn open(&mut self, root: &Path) -> Result<(), OpenError> {
        self.texts.clear();
        self.root = Some(root.to_path_buf());

        let configured = Config::for_project(root)
            .map_err(OpenError::from)
            .and_then(|config| Ok((Filter::new(&config.exclude)?, config)));
        let (result, (filter, config)) = match configured {
            Ok(applied) => (Ok(()), applied),
            Err(error) => (Err(error), (Filter::new(&[])?, Config::default())),
        };
        self.filter = filter;

        let files = discover(&[root.to_path_buf()], &config.exclude).unwrap_or_default();
        self.config = config;
        for path in files {
            if let Ok(text) = std::fs::read_to_string(&path) {
                self.update(&path, text);
            }
        }
        result
    }

    /// Re-reads the configuration and the files from disk.
    pub fn reload(&mut self) -> Result<(), OpenError> {
        match self.root.clone() {
            Some(root) => self.open(&root),
            None => Ok(()),
        }
    }

    /// Whether `path` belongs to the project, by the rules discovery uses.
    ///
    /// A file the editor opens is not necessarily part of the project: "go to
    /// definition" lands in installed libraries, and a file under an excluded
    /// directory is excluded whether or not it is open.
    pub fn accepts(&self, path: &Path) -> bool {
        match &self.root {
            Some(root) => self.filter.includes(root, path),
            None => true,
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
        #[cfg(test)]
        if self.panic_on_analysis {
            panic!("analysis panicked, as the test asked");
        }

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
        let findings = analyse_with(&sources, &parsed, &self.config.settings());
        for finding in findings
            .into_iter()
            .filter(|finding| self.config.is_enabled(finding.check))
        {
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
        workspace.open(dir.path()).expect("a valid project");
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
    fn liar_toml_turns_a_check_off() {
        let (_dir, workspace) = workspace_with(&[
            (
                "liar.toml",
                "ignore = [\"C1\"]
",
            ),
            (
                "a.py",
                "async def save():
    pass


async def handle():
    save()
",
            ),
        ]);
        assert!(codes(&workspace).is_empty());
    }

    #[test]
    fn liar_toml_excludes_files() {
        let (dir, workspace) = workspace_with(&[
            (
                "liar.toml",
                "exclude = [\"generated/**\"]
",
            ),
            (
                "a.py", "x = 1
",
            ),
            (
                "generated/b.py",
                "y = 2
",
            ),
        ]);
        assert_eq!(workspace.len(), 1);
        assert!(!workspace.accepts(&dir.path().join("generated/b.py")));
        assert!(workspace.accepts(&dir.path().join("a.py")));
    }

    #[test]
    fn a_broken_liar_toml_is_reported_and_the_defaults_used() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("liar.toml"),
            "toen = 1
",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("a.py"),
            "x = 1
",
        )
        .unwrap();

        let mut workspace = Workspace::new(Tone::Dry);
        let error = workspace.open(dir.path()).expect_err("a broken config");
        assert!(error.to_string().contains("toen"), "got: {error}");
        assert_eq!(workspace.len(), 1, "the project is still analysed");
    }

    #[test]
    fn editing_liar_toml_applies_on_reload() {
        let (dir, mut workspace) = workspace_with(&[(
            "a.py",
            "async def save():
    pass


async def handle():
    save()
",
        )]);
        assert_eq!(codes(&workspace), vec!["C1"]);

        std::fs::write(
            dir.path().join("liar.toml"),
            "ignore = [\"C1\"]
",
        )
        .unwrap();
        workspace.reload().unwrap();
        assert!(codes(&workspace).is_empty());
    }

    #[test]
    fn a_virtual_environment_is_not_part_of_the_project() {
        let (dir, workspace) = workspace_with(&[
            (
                "a.py", "x = 1
",
            ),
            ("venv/pyvenv.cfg", ""),
            (
                "venv/lib/site.py",
                "y = 2
",
            ),
        ]);
        assert_eq!(workspace.len(), 1);
        assert!(!workspace.accepts(&dir.path().join("venv/lib/site.py")));
    }

    #[test]
    fn a_file_outside_the_workspace_is_not_accepted() {
        let (_dir, workspace) = workspace_with(&[(
            "a.py", "x = 1
",
        )]);
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(!workspace.accepts(&elsewhere.path().join("lib.py")));
    }

    #[test]
    fn hidden_directories_are_skipped() {
        let (_dir, workspace) = workspace_with(&[("a.py", "x = 1\n"), (".venv/lib/b.py", "y\n")]);
        assert_eq!(workspace.len(), 1);
    }
}
