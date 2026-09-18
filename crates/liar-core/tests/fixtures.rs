//! Runs every fixture under `tests/fixtures` through the analyser.
//!
//! A fixture declares inline which findings it should produce. A line with no
//! comment declares that it produces none, and an unexpected finding fails as
//! loudly as a missing one — which is what makes the negative fixtures assert
//! anything at all.
//!
//! A `.py` file sitting directly in a directory is analysed on its own. A
//! subdirectory is analysed as one project, so a fixture can span several files
//! and exercise resolution across imports.

use liar_core::analysis::analyse;
use liar_core::ast::parse;
use liar_core::fixture::check_fixture;
use liar_core::source::SourceMap;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Every group of files that should be analysed together.
fn projects(root: &Path) -> Vec<(String, Vec<PathBuf>)> {
    let mut groups: Vec<(String, Vec<PathBuf>)> = Vec::new();

    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let entry = entry.expect("walking the fixture tree");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }

        // Files directly under positive/ or negative/ stand alone; anything a
        // level deeper is grouped with its siblings.
        let parent = path.parent().expect("a file has a parent");
        let grouped = matches!(
            parent.file_name().and_then(|n| n.to_str()),
            Some(name) if name != "positive" && name != "negative"
        );

        let key = if grouped {
            parent.display().to_string()
        } else {
            path.display().to_string()
        };

        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, files)) => files.push(path.to_path_buf()),
            None => groups.push((key, vec![path.to_path_buf()])),
        }
    }

    groups
}

#[test]
fn every_fixture_matches_its_expectations() {
    let root = fixture_root();
    let groups = projects(&root);
    assert!(
        !groups.is_empty(),
        "no fixtures found under {}",
        root.display()
    );

    let mut failures = Vec::new();
    let mut checked = 0;

    for (name, paths) in &groups {
        let mut sources = SourceMap::new();
        let mut asts = BTreeMap::new();

        for path in paths {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));

            // A fixture that does not parse is a broken fixture, and saying so
            // plainly beats a mismatch report that blames the checker.
            let ast = parse(&text)
                .unwrap_or_else(|e| panic!("fixture {} does not parse: {}", path.display(), e));

            // Fixtures are keyed by their name within the fixture tree, so the
            // module path a cross-file fixture imports by is stable wherever
            // the repository is checked out.
            let relative = path.strip_prefix(&root).unwrap_or(path);
            let id = sources.add(relative.to_path_buf(), text);
            asts.insert(id, ast);
        }

        let findings = analyse(&sources, &asts);

        for (id, file) in sources.iter() {
            let _ = id;
            if let Err(failure) = check_fixture(file, &findings, &sources) {
                failures.push(format!("{name}\n{failure}"));
            }
            checked += 1;
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {checked} fixture files did not match:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}
