//! The `# expect:` fixture harness.
//!
//! A fixture is a Python file that declares, inline, which findings it should
//! produce. A line with no comment declares that it produces none — which is
//! how negative fixtures assert silence.

use crate::check::CheckId;
use crate::finding::Finding;
use crate::source::{SourceFile, SourceMap};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Expectation {
    pub check: CheckId,
    pub line: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("{file}:{line}: unknown check code '{code}' in an expect comment")]
    UnknownCheck {
        file: String,
        line: u32,
        code: String,
    },
    #[error("{file}:{line}: empty expect comment")]
    Empty { file: String, line: u32 },
}

/// What a fixture expected versus what it got.
#[derive(Debug, PartialEq, Eq)]
pub struct FixtureFailure {
    pub file: String,
    pub missing: Vec<Expectation>,
    pub unexpected: Vec<(CheckId, u32)>,
}

impl fmt::Display for FixtureFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "fixture mismatch in {}", self.file)?;
        for expectation in &self.missing {
            writeln!(
                f,
                "  expected {} on line {}, but it was not reported",
                expectation.check.code(),
                expectation.line
            )?;
        }
        for (check, line) in &self.unexpected {
            writeln!(
                f,
                "  reported {} on line {}, but it was not expected",
                check.code(),
                line
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for FixtureFailure {}

const MARKER: &str = "# expect:";

pub fn parse_expectations(file: &SourceFile) -> Result<Vec<Expectation>, FixtureError> {
    let name = file.path().display().to_string();
    let mut expectations = Vec::new();

    for line_number in 1..=file.line_count() {
        let text = file.line_text(line_number);
        let Some(marker_at) = text.find(MARKER) else {
            continue;
        };

        let codes = text[marker_at + MARKER.len()..].trim();
        if codes.is_empty() {
            return Err(FixtureError::Empty {
                file: name,
                line: line_number,
            });
        }

        for code in codes.split(',').map(str::trim).filter(|c| !c.is_empty()) {
            let check = CheckId::from_code(code).ok_or_else(|| FixtureError::UnknownCheck {
                file: name.clone(),
                line: line_number,
                code: code.to_string(),
            })?;
            expectations.push(Expectation {
                check,
                line: line_number,
            });
        }
    }

    Ok(expectations)
}

/// Compares what a fixture expected against what the analyser produced.
///
/// Both directions fail. A finding that was not expected is as much a failure
/// as an expectation that was not met — otherwise a file with no comments
/// would assert nothing, and every negative fixture in the suite would be
/// inert.
pub fn check_fixture(
    file: &SourceFile,
    findings: &[Finding],
    sources: &SourceMap,
) -> Result<(), FixtureFailure> {
    let expectations = parse_expectations(file).expect("fixture expectations must parse");

    let mut expected: Vec<(CheckId, u32)> =
        expectations.iter().map(|e| (e.check, e.line)).collect();
    let mut actual: Vec<(CheckId, u32)> = findings
        .iter()
        .filter(|f| f.primary.file == file.id())
        .map(|f| {
            let line = sources
                .get(f.primary.file)
                .position(f.primary.span.start)
                .line;
            (f.check, line)
        })
        .collect();

    expected.sort_unstable();
    actual.sort_unstable();

    // Multiset difference in both directions, so two findings where one was
    // expected also fails.
    let mut missing = Vec::new();
    let mut remaining = actual;
    for item in &expected {
        match remaining.iter().position(|a| a == item) {
            Some(index) => {
                remaining.remove(index);
            }
            None => missing.push(Expectation {
                check: item.0,
                line: item.1,
            }),
        }
    }

    if missing.is_empty() && remaining.is_empty() {
        return Ok(());
    }

    Err(FixtureFailure {
        file: file.path().display().to_string(),
        missing,
        unexpected: remaining,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FileId;
    use crate::span::Span;
    use std::path::PathBuf;

    fn file(text: &str) -> (SourceMap, FileId) {
        let mut map = SourceMap::new();
        let id = map.add(PathBuf::from("fixture.py"), text.to_string());
        (map, id)
    }

    fn finding_on_line(check: CheckId, file: FileId, sources: &SourceMap, line: u32) -> Finding {
        let source = sources.get(file);
        let mut offset = 0u32;
        for current in 1..line {
            offset += source.line_text(current).len() as u32 + 1;
        }
        Finding::new(check, file, Span::new(offset, offset + 1))
    }

    #[test]
    fn an_expectation_comment_is_parsed() {
        let (map, id) = file("def f():  # expect: C1\n    pass\n");
        let found = parse_expectations(map.get(id)).unwrap();
        assert_eq!(
            found,
            vec![Expectation {
                check: CheckId::C1,
                line: 1
            }]
        );
    }

    #[test]
    fn several_expectations_on_one_line_are_parsed() {
        let (map, id) = file("x = f()  # expect: C1, C3b\n");
        let found = parse_expectations(map.get(id)).unwrap();
        assert_eq!(
            found,
            vec![
                Expectation {
                    check: CheckId::C1,
                    line: 1
                },
                Expectation {
                    check: CheckId::C3b,
                    line: 1
                },
            ]
        );
    }

    #[test]
    fn a_file_with_no_comments_expects_nothing() {
        let (map, id) = file("def f():\n    pass\n");
        assert!(parse_expectations(map.get(id)).unwrap().is_empty());
    }

    #[test]
    fn an_unknown_code_in_a_comment_is_an_error() {
        // A typo would otherwise turn an assertion into no assertion at all.
        let (map, id) = file("x = 1  # expect: C99\n");
        let err = parse_expectations(map.get(id)).expect_err("expected an error");
        assert!(err.to_string().contains("C99"));
    }

    #[test]
    fn an_empty_expect_comment_is_an_error() {
        let (map, id) = file("x = 1  # expect:\n");
        let err = parse_expectations(map.get(id)).expect_err("expected an error");
        assert!(err.to_string().contains("empty expect comment"));
    }

    #[test]
    fn a_matching_finding_passes() {
        let (map, id) = file("x = f()  # expect: C1\n");
        let findings = vec![finding_on_line(CheckId::C1, id, &map, 1)];
        assert!(check_fixture(map.get(id), &findings, &map).is_ok());
    }

    #[test]
    fn a_missing_finding_fails_and_names_it() {
        let (map, id) = file("x = f()  # expect: C1\n");
        let failure = check_fixture(map.get(id), &[], &map).expect_err("expected a failure");
        assert_eq!(
            failure.missing,
            vec![Expectation {
                check: CheckId::C1,
                line: 1
            }]
        );
        assert!(failure.unexpected.is_empty());
    }

    #[test]
    fn an_unexpected_finding_fails() {
        // The rule that makes negative fixtures mean anything.
        let (map, id) = file("x = f()\n");
        let findings = vec![finding_on_line(CheckId::C1, id, &map, 1)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.unexpected, vec![(CheckId::C1, 1)]);
        assert!(failure.missing.is_empty());
    }

    #[test]
    fn a_finding_on_the_wrong_line_fails_both_ways() {
        let (map, id) = file("x = f()  # expect: C1\ny = g()\n");
        let findings = vec![finding_on_line(CheckId::C1, id, &map, 2)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.missing.len(), 1);
        assert_eq!(failure.unexpected.len(), 1);
    }

    #[test]
    fn the_wrong_check_on_the_right_line_fails() {
        let (map, id) = file("x = f()  # expect: C1\n");
        let findings = vec![finding_on_line(CheckId::C4, id, &map, 1)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(
            failure.missing,
            vec![Expectation {
                check: CheckId::C1,
                line: 1
            }]
        );
        assert_eq!(failure.unexpected, vec![(CheckId::C4, 1)]);
    }

    #[test]
    fn duplicate_findings_on_one_line_are_counted() {
        // Two findings where one was expected is a bug worth failing on.
        let (map, id) = file("x = f()  # expect: C1\n");
        let findings = vec![
            finding_on_line(CheckId::C1, id, &map, 1),
            finding_on_line(CheckId::C1, id, &map, 1),
        ];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        assert_eq!(failure.unexpected, vec![(CheckId::C1, 1)]);
    }

    #[test]
    fn the_failure_message_shows_both_sides() {
        let (map, id) = file("x = f()  # expect: C1\ny = g()\n");
        let findings = vec![finding_on_line(CheckId::C4, id, &map, 2)];
        let failure = check_fixture(map.get(id), &findings, &map).expect_err("expected a failure");
        let text = failure.to_string();
        assert!(text.contains("C1"), "should name the missing check");
        assert!(text.contains("C4"), "should name the unexpected check");
        assert!(text.contains("fixture.py"), "should name the file");
    }

    #[test]
    fn findings_from_other_files_are_ignored() {
        let mut map = SourceMap::new();
        let a = map.add(PathBuf::from("a.py"), "x = f()\n".to_string());
        let b = map.add(PathBuf::from("b.py"), "y = g()\n".to_string());
        let findings = vec![finding_on_line(CheckId::C1, b, &map, 1)];
        // a.py expects nothing and the only finding belongs to b.py.
        assert!(check_fixture(map.get(a), &findings, &map).is_ok());
    }
}
