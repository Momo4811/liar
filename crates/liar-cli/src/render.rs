//! rustc-style diagnostic rendering.

use annotate_snippets::{AnnotationKind, Level, Renderer, Snippet};
use liar_core::check::Severity;
use liar_core::finding::Finding;
use liar_core::messages::{MessageTable, Tone};
use liar_core::source::SourceMap;

/// Renders findings as human-readable diagnostics.
///
/// Input is sorted before rendering, so callers need not have sorted it and
/// the output is a function of the finding set alone.
pub fn render(
    findings: &[Finding],
    sources: &SourceMap,
    messages: &MessageTable,
    tone: Tone,
) -> String {
    if findings.is_empty() {
        return String::new();
    }

    let mut sorted = findings.to_vec();
    liar_core::finding::sort_findings(&mut sorted, sources);

    let mut out = String::new();
    for finding in &sorted {
        out.push_str(&render_one(finding, sources, messages, tone));
        out.push('\n');
    }
    out
}

fn render_one(
    finding: &Finding,
    sources: &SourceMap,
    messages: &MessageTable,
    tone: Tone,
) -> String {
    let file = sources.get(finding.primary.file);
    let message = messages.render(finding, tone);
    let path = file.path().display().to_string();

    let level = match finding.severity {
        Severity::Error => Level::ERROR,
        Severity::Warning => Level::WARNING,
    };

    let mut annotations = Vec::new();

    let primary = AnnotationKind::Primary
        .span(finding.primary.span.start as usize..finding.primary.span.end as usize);
    annotations.push(match &finding.primary.note {
        Some(note) => primary.label(note.as_str()),
        None => primary,
    });

    // Secondary labels are what let one finding point at several places: C3f
    // reports four variables sharing a name as one diagnostic, not four.
    for label in &finding.secondary {
        let annotation =
            AnnotationKind::Context.span(label.span.start as usize..label.span.end as usize);
        annotations.push(match &label.note {
            Some(note) => annotation.label(note.as_str()),
            None => annotation,
        });
    }

    // Spans are byte offsets; annotate-snippets does its own character
    // counting for column alignment. SourceFile::position exists for SARIF and
    // the language server, which want line/column pairs instead.
    let snippet = Snippet::source(file.text())
        .path(path.as_str())
        .annotations(annotations);

    let group = level
        .primary_title(message.as_str())
        .id(finding.check.code())
        .element(snippet);

    Renderer::plain().render(&[group])
}

#[cfg(test)]
mod tests {
    use super::*;
    use liar_core::check::CheckId;
    use liar_core::ids::FileId;
    use liar_core::span::Span;
    use std::path::PathBuf;

    fn setup(text: &str) -> (SourceMap, FileId) {
        let mut map = SourceMap::new();
        let id = map.add(PathBuf::from("example.py"), text.to_string());
        (map, id)
    }

    #[test]
    fn nothing_renders_to_nothing() {
        let (map, _) = setup("x = 1\n");
        let out = render(&[], &map, MessageTable::embedded(), Tone::Dry);
        assert!(out.is_empty());
    }

    #[test]
    fn a_single_finding_renders() {
        let (map, file) = setup("async def f():\n    save_user(u)\n");
        let finding =
            Finding::new(CheckId::C1, file, Span::new(19, 31)).with_arg("name", "save_user");
        let out = render(&[finding], &map, MessageTable::embedded(), Tone::Dry);
        insta::assert_snapshot!(out);
    }

    #[test]
    fn each_tone_renders_its_own_message() {
        let (map, file) = setup("async def f():\n    save_user(u)\n");
        let make =
            || Finding::new(CheckId::C1, file, Span::new(19, 31)).with_arg("name", "save_user");
        for tone in [Tone::Professional, Tone::Dry, Tone::Brutal] {
            let out = render(&[make()], &map, MessageTable::embedded(), tone);
            insta::assert_snapshot!(format!("tone_{tone:?}"), out);
        }
    }

    #[test]
    fn secondary_labels_render_beneath_the_primary() {
        let (map, file) = setup("data = 1\ndata = []\ndata = 'x'\n");
        let finding = Finding::new(CheckId::C3f, file, Span::new(0, 4))
            .with_arg("name", "data")
            .with_arg("n", "3")
            .with_arg("k", "3")
            .with_secondary(file, Span::new(9, 13), None)
            .with_secondary(file, Span::new(19, 23), None);
        let out = render(&[finding], &map, MessageTable::embedded(), Tone::Dry);
        insta::assert_snapshot!(out);
    }

    #[test]
    fn a_caret_after_multibyte_characters_on_the_same_line_aligns() {
        // The multibyte characters must precede the span on the *same* line;
        // putting them on an earlier line only tests line offsets, which are
        // byte-agnostic anyway.
        //
        // "réçultat" is 8 characters but 10 bytes, so `save_user` begins at
        // byte 13 and character column 12. A renderer counting columns in
        // bytes would report 14 and draw the caret two columns too far right.
        let source = "r\u{e9}\u{e7}ultat = save_user(u)\n";
        let (map, file) = setup(source);

        let start = source.find("save_user").expect("the call is in the source") as u32;
        assert_eq!(start, 13, "byte offset");

        let finding = Finding::new(CheckId::C1, file, Span::new(start, start + 9))
            .with_arg("name", "save_user");
        let out = render(&[finding], &map, MessageTable::embedded(), Tone::Dry);

        assert!(
            out.contains(":1:12"),
            "expected character column 12, got: {out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn findings_render_in_sorted_order() {
        let (map, file) = setup("a()\nb()\n");
        let findings = vec![
            Finding::new(CheckId::C1, file, Span::new(4, 7)).with_arg("name", "b"),
            Finding::new(CheckId::C1, file, Span::new(0, 3)).with_arg("name", "a"),
        ];
        let out = render(&findings, &map, MessageTable::embedded(), Tone::Dry);
        let a_at = out.find("'a'").expect("expected a finding for a");
        let b_at = out.find("'b'").expect("expected a finding for b");
        assert!(a_at < b_at, "findings should render in source order");
    }

    #[test]
    fn rendering_is_deterministic() {
        let (map, file) = setup("a()\n");
        let make = || vec![Finding::new(CheckId::C1, file, Span::new(0, 3)).with_arg("name", "a")];
        let first = render(&make(), &map, MessageTable::embedded(), Tone::Dry);
        let second = render(&make(), &map, MessageTable::embedded(), Tone::Dry);
        assert_eq!(first, second);
    }

    #[test]
    fn severity_selects_the_level() {
        // C1 is an error, C3e a warning. The rendered output should say so.
        let (map, file) = setup("x = f()\n");
        let error = Finding::new(CheckId::C1, file, Span::new(4, 7)).with_arg("name", "f");
        let warning = Finding::new(CheckId::C3e, file, Span::new(0, 1))
            .with_arg("name", "x")
            .with_arg("lines", "80");

        let rendered_error = render(&[error], &map, MessageTable::embedded(), Tone::Dry);
        let rendered_warning = render(&[warning], &map, MessageTable::embedded(), Tone::Dry);

        assert!(rendered_error.contains("error"), "got: {rendered_error}");
        assert!(
            rendered_warning.contains("warning"),
            "got: {rendered_warning}"
        );
    }
}
