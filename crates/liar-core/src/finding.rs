//! What a check produces.

use crate::check::{CheckId, Severity};
use crate::ids::FileId;
use crate::source::SourceMap;
use crate::span::Span;

/// A span worth pointing at, with an optional note.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Label {
    pub file: FileId,
    pub span: Span,
    pub note: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finding {
    pub check: CheckId,
    pub severity: Severity,
    /// The span the diagnostic is about. Underlined.
    pub primary: Label,
    /// Related spans, shown beneath. C3f uses these to show every occurrence
    /// of a name in one finding rather than emitting several.
    pub secondary: Vec<Label>,
    /// Values interpolated into the message. Order-preserving rather than a
    /// map, so rendering is deterministic.
    pub args: Vec<(String, String)>,
}

impl Finding {
    pub fn new(check: CheckId, file: FileId, span: Span) -> Self {
        Self {
            check,
            severity: check.default_severity(),
            primary: Label {
                file,
                span,
                note: None,
            },
            secondary: Vec::new(),
            args: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.primary.note = Some(note.into());
        self
    }

    #[must_use]
    pub fn with_secondary(mut self, file: FileId, span: Span, note: Option<String>) -> Self {
        self.secondary.push(Label { file, span, note });
        self
    }

    #[must_use]
    pub fn with_arg(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.push((key.into(), value.into()));
        self
    }
}

/// Sorts findings into the order they are always reported in: by file path,
/// then line, then column, then check code.
///
/// Rendering order must not depend on the order checks happened to run, or the
/// corpus baselines compare noise rather than behaviour.
pub fn sort_findings(findings: &mut [Finding], sources: &SourceMap) {
    findings.sort_by(|a, b| {
        let file_a = sources.get(a.primary.file);
        let file_b = sources.get(b.primary.file);
        let pos_a = file_a.position(a.primary.span.start);
        let pos_b = file_b.position(b.primary.span.start);

        file_a
            .path()
            .cmp(file_b.path())
            .then(pos_a.line.cmp(&pos_b.line))
            .then(pos_a.column.cmp(&pos_b.column))
            .then(a.check.cmp(&b.check))
            .then(a.primary.span.end.cmp(&b.primary.span.end))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;
    use std::path::PathBuf;

    fn map_with(files: &[(&str, &str)]) -> (SourceMap, Vec<FileId>) {
        let mut map = SourceMap::new();
        let ids = files
            .iter()
            .map(|(path, text)| map.add(PathBuf::from(path), (*text).to_string()))
            .collect();
        (map, ids)
    }

    #[test]
    fn a_finding_takes_its_severity_from_its_check() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C1, ids[0], Span::new(0, 1));
        assert_eq!(f.severity, Severity::Error);
    }

    #[test]
    fn severity_can_be_overridden() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C1, ids[0], Span::new(0, 1)).with_severity(Severity::Warning);
        assert_eq!(f.severity, Severity::Warning);
    }

    #[test]
    fn arguments_are_recorded_in_insertion_order() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C3f, ids[0], Span::new(0, 1))
            .with_arg("name", "data")
            .with_arg("n", "4");
        assert_eq!(
            f.args,
            vec![
                ("name".to_string(), "data".to_string()),
                ("n".to_string(), "4".to_string()),
            ]
        );
    }

    #[test]
    fn findings_sort_by_file_then_line_then_column_then_check() {
        let (map, ids) = map_with(&[("a.py", "one\ntwo\n"), ("b.py", "three\n")]);
        let (a, b) = (ids[0], ids[1]);

        let mut findings = vec![
            Finding::new(CheckId::C1, b, Span::new(0, 1)), // b.py 1:1
            Finding::new(CheckId::C4, a, Span::new(4, 5)), // a.py 2:1 C4
            Finding::new(CheckId::C1, a, Span::new(4, 5)), // a.py 2:1 C1
            Finding::new(CheckId::C1, a, Span::new(1, 2)), // a.py 1:2
            Finding::new(CheckId::C1, a, Span::new(0, 1)), // a.py 1:1
        ];
        sort_findings(&mut findings, &map);

        let order: Vec<_> = findings
            .iter()
            .map(|f| {
                let file = map.get(f.primary.file).path().display().to_string();
                let pos = map.get(f.primary.file).position(f.primary.span.start);
                (file, pos.line, pos.column, f.check.code())
            })
            .collect();

        assert_eq!(
            order,
            vec![
                ("a.py".to_string(), 1, 1, "C1"),
                ("a.py".to_string(), 1, 2, "C1"),
                ("a.py".to_string(), 2, 1, "C1"),
                ("a.py".to_string(), 2, 1, "C4"),
                ("b.py".to_string(), 1, 1, "C1"),
            ]
        );
    }

    #[test]
    fn sorting_is_stable_across_input_orders() {
        // Corpus baselines compare noise if output order depends on the order
        // checks happened to run.
        let (map, ids) = map_with(&[("a.py", "one\ntwo\n")]);
        let a = ids[0];

        let make = || {
            vec![
                Finding::new(CheckId::C4, a, Span::new(4, 5)),
                Finding::new(CheckId::C1, a, Span::new(0, 1)),
                Finding::new(CheckId::C1, a, Span::new(4, 5)),
            ]
        };

        let mut forward = make();
        let mut reversed = make();
        reversed.reverse();

        sort_findings(&mut forward, &map);
        sort_findings(&mut reversed, &map);

        let key = |fs: &[Finding]| -> Vec<(u32, u32, &'static str)> {
            fs.iter()
                .map(|f| {
                    let p = map.get(f.primary.file).position(f.primary.span.start);
                    (p.line, p.column, f.check.code())
                })
                .collect()
        };
        assert_eq!(key(&forward), key(&reversed));
    }

    #[test]
    fn secondary_labels_are_attached() {
        let (_, ids) = map_with(&[("a.py", "one\ntwo\n")]);
        let f = Finding::new(CheckId::C3f, ids[0], Span::new(0, 3)).with_secondary(
            ids[0],
            Span::new(4, 7),
            Some("and here".into()),
        );
        assert_eq!(f.secondary.len(), 1);
        assert_eq!(f.secondary[0].note.as_deref(), Some("and here"));
    }

    #[test]
    fn a_note_attaches_to_the_primary_label() {
        let (_, ids) = map_with(&[("a.py", "x")]);
        let f = Finding::new(CheckId::C1, ids[0], Span::new(0, 1)).with_note("try awaiting it");
        assert_eq!(f.primary.note.as_deref(), Some("try awaiting it"));
    }
}
