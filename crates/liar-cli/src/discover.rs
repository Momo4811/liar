//! Finding the files to analyse.

use globset::{Glob, GlobSetBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("no such file or directory: {0}")]
    NotFound(String),
    #[error("invalid exclude pattern '{pattern}': {source}")]
    BadPattern {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Collects the Python files under `roots`, minus anything matching `exclude`.
///
/// Results are deduplicated and sorted, so the caller sees the same list
/// whatever order the filesystem returned entries in.
pub fn discover(roots: &[PathBuf], exclude: &[String]) -> Result<Vec<PathBuf>, DiscoverError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in exclude {
        let glob = Glob::new(pattern).map_err(|source| DiscoverError::BadPattern {
            pattern: pattern.clone(),
            source,
        })?;
        builder.add(glob);
    }
    let excluded = builder
        .build()
        .map_err(|source| DiscoverError::BadPattern {
            pattern: exclude.join(", "),
            source,
        })?;

    let mut found = Vec::new();

    for root in roots {
        if !root.exists() {
            return Err(DiscoverError::NotFound(root.display().to_string()));
        }

        if root.is_file() {
            found.push(root.clone());
            continue;
        }

        // depth 0 is the root itself, which is not skipped even if its own
        // name begins with a dot — the user named it explicitly.
        let walk = WalkDir::new(root)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry.path()));

        for entry in walk {
            let entry = entry.map_err(|source| DiscoverError::Io {
                path: root.display().to_string(),
                source: source.into(),
            })?;

            if !entry.file_type().is_file() || !is_python(entry.path()) {
                continue;
            }

            // Match excludes against the path relative to the root, so a
            // pattern like "tests/**" means what the user expects.
            let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
            if excluded.is_match(relative) {
                continue;
            }

            found.push(entry.path().to_path_buf());
        }
    }

    found.sort();
    found.dedup();
    Ok(found)
}

fn is_python(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "pyi")
    )
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') && name != "." && name != "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for path in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "pass\n").unwrap();
        }
        dir
    }

    fn names(paths: &[PathBuf], root: &Path) -> Vec<String> {
        paths
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn a_single_file_is_returned_as_is() {
        let dir = tree(&["a.py"]);
        let found = discover(&[dir.path().join("a.py")], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn a_directory_is_walked_recursively() {
        let dir = tree(&["a.py", "pkg/b.py", "pkg/sub/c.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(
            names(&found, dir.path()),
            vec!["a.py", "pkg/b.py", "pkg/sub/c.py"]
        );
    }

    #[test]
    fn non_python_files_are_skipped() {
        let dir = tree(&["a.py", "readme.md", "b.pyc"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn pyi_stubs_are_included() {
        let dir = tree(&["a.py", "a.pyi"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py", "a.pyi"]);
    }

    #[test]
    fn exclude_globs_filter_results() {
        let dir = tree(&["a.py", "tests/b.py", "tests/sub/c.py"]);
        let found = discover(&[dir.path().to_path_buf()], &["tests/**".into()]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn results_are_sorted_regardless_of_filesystem_order() {
        // Discovery order must not leak into output order.
        let dir = tree(&["z.py", "a.py", "m.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py", "m.py", "z.py"]);
    }

    #[test]
    fn duplicates_are_removed() {
        let dir = tree(&["a.py"]);
        let path = dir.path().join("a.py");
        let found = discover(&[path.clone(), path], &[]).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_missing_path_is_an_error() {
        let dir = tree(&[]);
        let err = discover(&[dir.path().join("nope.py")], &[]).expect_err("expected an error");
        assert!(err.to_string().contains("nope.py"));
    }

    #[test]
    fn hidden_directories_are_skipped() {
        let dir = tree(&["a.py", ".venv/lib/b.py", ".git/c.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn an_invalid_exclude_pattern_is_an_error() {
        let dir = tree(&["a.py"]);
        let err =
            discover(&[dir.path().to_path_buf()], &["[".into()]).expect_err("expected an error");
        assert!(err.to_string().contains("invalid exclude pattern"));
    }
}
