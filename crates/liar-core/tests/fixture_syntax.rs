//! The expectation comments in `tests/fixtures` parse.
//!
//! `fixtures.rs` would also fail on a malformed comment, but by way of a
//! mismatch report that blames the checker. This fails first, and says which
//! fixture and which line.

use liar_core::fixture::parse_expectations;
use liar_core::source::SourceMap;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn every_fixture_has_parseable_expectations() {
    let mut checked = 0;
    for entry in walkdir::WalkDir::new(fixture_dir()) {
        let entry = entry.expect("walking the fixture directory");
        if entry.path().extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }

        let text = std::fs::read_to_string(entry.path()).expect("reading a fixture");
        let mut map = SourceMap::new();
        let id = map.add(entry.path().to_path_buf(), text);

        parse_expectations(map.get(id)).unwrap_or_else(|e| panic!("bad expectation comment: {e}"));
        checked += 1;
    }
    assert!(checked >= 2, "expected to find fixtures, found {checked}");
}

#[test]
fn the_clean_fixture_expects_nothing() {
    // The shape of every negative fixture: no comments, therefore no findings
    // permitted. If this ever parses to a non-empty list, the marker syntax has
    // started matching something it should not.
    let path = fixture_dir().join("harness/clean.py");
    let text = std::fs::read_to_string(&path).expect("reading the fixture");
    let mut map = SourceMap::new();
    let id = map.add(path, text);

    assert!(parse_expectations(map.get(id)).unwrap().is_empty());
}

#[test]
fn negative_fixtures_outnumber_positive_ones() {
    // For a tool whose rule is "when unsure, stay silent", the negative
    // fixtures are the important half. Tests asserting a bug is found measure
    // recall; false positives are the only thing that can kill the tool. If
    // this ever inverts, the suite has stopped testing what matters.
    let (mut positive, mut negative) = (0, 0);

    for entry in walkdir::WalkDir::new(fixture_dir()) {
        let entry = entry.expect("walking the fixture directory");
        if entry.path().extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }
        let path = entry.path().display().to_string().replace('\\', "/");
        if path.contains("/positive/") {
            positive += 1;
        } else if path.contains("/negative/") {
            negative += 1;
        }
    }

    assert!(
        positive > 0 && negative > 0,
        "expected both kinds of fixture"
    );
    assert!(
        negative > positive,
        "{negative} negative fixtures against {positive} positive"
    );
}
