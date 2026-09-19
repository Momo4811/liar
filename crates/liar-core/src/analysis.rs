//! The driver: files in, findings out.

use crate::ast::Ast;
use crate::checks::{self, Ctx};
use crate::finding::{Finding, sort_findings};
use crate::ids::FileId;
use crate::index::{Index, IndexInput};
use crate::infer::Types;
use crate::source::SourceMap;
use std::collections::BTreeMap;

/// Runs every check over a whole project.
///
/// The index is built once across all files, so a name imported from another
/// file resolves to its real definition rather than stopping at the import.
/// Findings come back in the canonical order, so callers never have to sort.
pub fn analyse(sources: &SourceMap, asts: &BTreeMap<FileId, Ast>) -> Vec<Finding> {
    analyse_with(sources, asts, &Settings::default())
}

/// Settings a check needs, which live with the project rather than the engine.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub scope_threshold: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scope_threshold: 20,
        }
    }
}

pub fn analyse_with(
    sources: &SourceMap,
    asts: &BTreeMap<FileId, Ast>,
    settings: &Settings,
) -> Vec<Finding> {
    let inputs: Vec<IndexInput<'_>> = asts
        .iter()
        .map(|(&file, ast)| IndexInput {
            file,
            path: sources.get(file).path(),
            ast,
        })
        .collect();

    let index = Index::build(&inputs);
    let types = Types::infer(&index, asts);
    let ctx = Ctx {
        index: &index,
        sources,
        asts,
        types: &types,
        scope_threshold: settings.scope_threshold,
    };

    let mut findings: Vec<Finding> = checks::all()
        .iter()
        .flat_map(|check| check.run(&ctx))
        .collect();

    sort_findings(&mut findings, sources);
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;
    use crate::check::CheckId;
    use std::path::PathBuf;

    /// Parses `files`, analyses them together, and returns the findings.
    fn analyse_project(files: &[(&str, &str)]) -> (SourceMap, Vec<Finding>) {
        let mut sources = SourceMap::new();
        let mut asts = BTreeMap::new();

        for (path, source) in files {
            let id = sources.add(PathBuf::from(path), (*source).to_string());
            asts.insert(id, parse(source).expect("fixture should parse"));
        }

        let findings = analyse(&sources, &asts);
        (sources, findings)
    }

    fn codes(files: &[(&str, &str)]) -> Vec<&'static str> {
        analyse_project(files)
            .1
            .iter()
            .map(|f| f.check.code())
            .collect()
    }

    #[test]
    fn an_empty_project_produces_nothing() {
        assert!(codes(&[]).is_empty());
    }

    #[test]
    fn a_clean_file_produces_nothing() {
        assert!(codes(&[("a.py", "def add(a, b):\n    return a + b\n")]).is_empty());
    }

    #[test]
    fn a_discarded_async_call_is_reported() {
        let found = codes(&[(
            "a.py",
            "async def save():\n    pass\n\n\nasync def handle():\n    save()\n",
        )]);
        assert_eq!(found, vec![CheckId::C1.code()]);
    }

    #[test]
    fn the_finding_points_at_the_call() {
        let source = "async def save():\n    pass\n\n\nasync def handle():\n    save()\n";
        let (sources, findings) = analyse_project(&[("a.py", source)]);
        let finding = &findings[0];
        let span = finding.primary.span;

        assert_eq!(&source[span.start as usize..span.end as usize], "save()");
        assert_eq!(
            sources.get(finding.primary.file).position(span.start).line,
            6
        );
    }

    #[test]
    fn findings_come_back_sorted() {
        let (sources, findings) = analyse_project(&[
            (
                "b.py",
                "async def s():\n    pass\nasync def h():\n    s()\n",
            ),
            (
                "a.py",
                "async def s():\n    pass\nasync def h():\n    s()\n",
            ),
        ]);
        let paths: Vec<_> = findings
            .iter()
            .map(|f| sources.get(f.primary.file).path().display().to_string())
            .collect();
        assert_eq!(paths, vec!["a.py", "b.py"]);
    }

    #[test]
    fn analysis_is_deterministic() {
        let files: &[(&str, &str)] = &[
            (
                "a.py",
                "async def s():\n    pass\nasync def h():\n    s()\n    s()\n",
            ),
            ("b.py", "from a import s\nasync def g():\n    s()\n"),
        ];
        let first = analyse_project(files).1;
        let second = analyse_project(files).1;
        assert_eq!(first, second);
    }
}
