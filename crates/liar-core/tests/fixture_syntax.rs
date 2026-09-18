//! The expectation comments in tests/fixtures parse, so a typo in a fixture
//! fails here rather than silently disabling an assertion once the checkers
//! exist.

use liar_core::check::CheckId;
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
fn the_sample_fixture_declares_what_it_should() {
    let path = fixture_dir().join("harness/expectations.py");
    let text = std::fs::read_to_string(&path).expect("reading the fixture");
    let mut map = SourceMap::new();
    let id = map.add(path, text);

    let found = parse_expectations(map.get(id)).unwrap();
    let codes: Vec<_> = found.iter().map(|e| e.check).collect();
    assert_eq!(codes, vec![CheckId::C1, CheckId::C1, CheckId::C3b]);
}

#[test]
fn the_clean_fixture_expects_nothing() {
    // The shape of every negative fixture: no comments, therefore no findings
    // permitted. If this ever parses to a non-empty list, the marker syntax
    // has started matching something it should not.
    let path = fixture_dir().join("harness/clean.py");
    let text = std::fs::read_to_string(&path).expect("reading the fixture");
    let mut map = SourceMap::new();
    let id = map.add(path, text);

    assert!(parse_expectations(map.get(id)).unwrap().is_empty());
}
