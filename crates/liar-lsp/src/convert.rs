//! Turning the engine's spans into the positions an editor understands.
//!
//! This is the one place in the project where a subtle mistake is invisible
//! rather than loud. The engine counts bytes. `SourceFile::position` counts
//! characters, because that is what a human counts in an editor. **LSP counts
//! UTF-16 code units**, which is neither.
//!
//! For ASCII all three agree, so a naive implementation passes every casual
//! test. It goes wrong on an astral-plane character — an emoji is one
//! character, four bytes, and *two* UTF-16 code units — and the only symptom is
//! that the underline sits in the wrong place on some lines.

use liar_core::check::Severity;
use liar_core::finding::Finding;
use liar_core::source::{SourceFile, SourceMap};
use liar_core::span::Span;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Url,
};

/// Converts a byte offset into an LSP position.
///
/// LSP lines and characters are both zero-based, and a character is a UTF-16
/// code unit.
pub fn to_lsp_position(file: &SourceFile, offset: u32) -> Position {
    let position = file.position(offset);

    // Column 1 of the same line is where the line begins.
    let line_start = file.offset_at(position.line, 1).unwrap_or(offset);
    let prefix = &file.text()[line_start as usize..offset as usize];

    Position {
        line: position.line - 1,
        character: prefix.encode_utf16().count() as u32,
    }
}

/// Converts an LSP position back into a byte offset.
///
/// The inverse of [`to_lsp_position`], and just as easy to get wrong: the
/// incoming character is a count of UTF-16 code units, so it has to be walked
/// rather than used as an index.
pub fn from_lsp_position(file: &SourceFile, position: Position) -> Option<u32> {
    let line = position.line.checked_add(1)?;
    if line > file.line_count() {
        return None;
    }

    let start = file.offset_at(line, 1)?;
    let text = file.line_text(line);

    let mut units = 0u32;
    for (offset, character) in text.char_indices() {
        if units == position.character {
            return Some(start + offset as u32);
        }
        units += character.len_utf16() as u32;
    }

    // One past the last character is where a cursor sits at end of line.
    (units == position.character).then_some(start + text.len() as u32)
}

pub fn to_lsp_range(file: &SourceFile, span: Span) -> Range {
    Range {
        start: to_lsp_position(file, span.start),
        end: to_lsp_position(file, span.end),
    }
}

/// The URI an editor knows a file by.
pub fn to_url(file: &SourceFile) -> Option<Url> {
    Url::from_file_path(std::fs::canonicalize(file.path()).ok()?).ok()
}

pub fn to_diagnostic(
    finding: &Finding,
    message: String,
    sources: &SourceMap,
) -> Option<Diagnostic> {
    let file = sources.get(finding.primary.file);

    // Secondary labels become related information, which is how one diagnostic
    // points at several places - four variables sharing a name are one finding,
    // not four.
    let related: Vec<DiagnosticRelatedInformation> = finding
        .secondary
        .iter()
        .filter_map(|label| {
            let labelled = sources.get(label.file);
            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri: to_url(labelled)?,
                    range: to_lsp_range(labelled, label.span),
                },
                message: label.note.clone().unwrap_or_else(|| "here".to_string()),
            })
        })
        .collect();

    Some(Diagnostic {
        range: to_lsp_range(file, finding.primary.span),
        severity: Some(match finding.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(finding.check.code().to_string())),
        source: Some("liar".to_string()),
        message,
        related_information: if related.is_empty() {
            None
        } else {
            Some(related)
        },
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn file(text: &str) -> SourceFile {
        let mut map = SourceMap::new();
        let id = map.add(PathBuf::from("t.py"), text.to_string());
        map.get(id).clone()
    }

    fn at(text: &str, offset: u32) -> (u32, u32) {
        let position = to_lsp_position(&file(text), offset);
        (position.line, position.character)
    }

    #[test]
    fn the_first_character_is_zero_zero() {
        assert_eq!(at("abc", 0), (0, 0));
    }

    #[test]
    fn ascii_characters_advance_by_one() {
        assert_eq!(at("abc", 2), (0, 2));
    }

    #[test]
    fn lines_are_zero_based() {
        assert_eq!(at("ab\ncd", 3), (1, 0));
        assert_eq!(at("ab\ncd", 4), (1, 1));
    }

    #[test]
    fn a_two_byte_character_counts_as_one_unit() {
        // U+00E9 is two bytes and one UTF-16 unit, so the character after it is
        // at LSP character 1, not 2.
        assert_eq!(at("\u{e9}x", 2), (0, 1));
    }

    #[test]
    fn an_astral_plane_character_counts_as_two_units() {
        // The case that separates a correct implementation from one that only
        // looks right. U+1F980 is one character, four bytes, and two UTF-16
        // code units. Counting characters would say 1; counting bytes would say
        // 4; LSP wants 2.
        assert_eq!(at("\u{1f980}x", 4), (0, 2));
    }

    #[test]
    fn several_astral_characters_accumulate() {
        assert_eq!(at("\u{1f980}\u{1f980}x", 8), (0, 4));
    }

    #[test]
    fn a_mixture_counts_correctly() {
        // "a" 1 unit, U+00E9 1 unit, U+1F980 2 units = 4 before "x".
        let text = "a\u{e9}\u{1f980}x";
        let offset = text.find('x').unwrap() as u32;
        assert_eq!(at(text, offset), (0, 4));
    }

    #[test]
    fn counting_restarts_on_each_line() {
        // The emoji on line 1 must not shift line 2.
        let text = "\u{1f980}\u{1f980}\nxy";
        let offset = text.find('y').unwrap() as u32;
        assert_eq!(at(text, offset), (1, 1));
    }

    #[test]
    fn the_end_of_a_file_is_a_valid_position() {
        assert_eq!(at("ab", 2), (0, 2));
    }

    #[test]
    fn an_empty_file_has_one_position() {
        assert_eq!(at("", 0), (0, 0));
    }

    #[test]
    fn crlf_does_not_add_a_unit_to_the_next_line() {
        assert_eq!(at("ab\r\ncd", 4), (1, 0));
    }

    #[test]
    fn a_range_spans_from_start_to_end() {
        let source = file("save_user(u)");
        let range = to_lsp_range(&source, Span::new(0, 9));
        assert_eq!(
            range.start,
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            range.end,
            Position {
                line: 0,
                character: 9
            }
        );
    }

    #[test]
    fn a_range_may_span_lines() {
        let source = file("ab\ncd");
        let range = to_lsp_range(&source, Span::new(1, 4));
        assert_eq!(
            range.start,
            Position {
                line: 0,
                character: 1
            }
        );
        assert_eq!(
            range.end,
            Position {
                line: 1,
                character: 1
            }
        );
    }

    #[test]
    fn an_empty_span_is_an_empty_range() {
        let source = file("abc");
        let range = to_lsp_range(&source, Span::new(1, 1));
        assert_eq!(range.start, range.end);
    }

    #[test]
    fn a_position_converts_back_to_its_offset() {
        let source = file("a\u{e9}\u{1f980}xy\nzz");
        for (offset, _) in source.text().char_indices() {
            let position = to_lsp_position(&source, offset as u32);
            assert_eq!(
                from_lsp_position(&source, position),
                Some(offset as u32),
                "round trip failed at offset {offset}"
            );
        }
    }

    #[test]
    fn a_position_past_the_end_of_the_file_converts_to_nothing() {
        let source = file("ab\ncd");
        assert_eq!(
            from_lsp_position(
                &source,
                Position {
                    line: 9,
                    character: 0
                }
            ),
            None
        );
        assert_eq!(
            from_lsp_position(
                &source,
                Position {
                    line: 0,
                    character: 99
                }
            ),
            None
        );
    }

    #[test]
    fn the_end_of_a_line_converts() {
        let source = file("ab\ncd");
        assert_eq!(
            from_lsp_position(
                &source,
                Position {
                    line: 0,
                    character: 2
                }
            ),
            Some(2)
        );
    }

    proptest! {
        /// Every LSP position the converter produces converts back to the byte
        /// offset it came from. A quick fix that edits the wrong offset is
        /// worse than no quick fix.
        #[test]
        fn conversion_round_trips(text in ".{0,300}") {
            let source = file(&text);
            for (offset, _) in source.text().char_indices() {
                let position = to_lsp_position(&source, offset as u32);
                prop_assert_eq!(from_lsp_position(&source, position), Some(offset as u32));
            }
        }

        /// Whatever the text, an LSP position is always reachable and its line
        /// is within the file. A panic here would take the editor with it.
        #[test]
        fn positions_are_total_over_character_boundaries(text in ".{0,300}") {
            let source = file(&text);
            for (offset, _) in source.text().char_indices() {
                let position = to_lsp_position(&source, offset as u32);
                prop_assert!(position.line < source.line_count());
            }
        }

        /// The character count never exceeds the UTF-16 length of its line,
        /// which is what an editor will index into.
        #[test]
        fn characters_stay_within_their_line(text in ".{0,300}") {
            let source = file(&text);
            for (offset, _) in source.text().char_indices() {
                let position = to_lsp_position(&source, offset as u32);
                let line = source.line_text(position.line + 1);
                prop_assert!(position.character as usize <= line.encode_utf16().count());
            }
        }
    }
}
