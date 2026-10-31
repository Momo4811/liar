//! Finding the files to analyse.

use globset::{Glob, GlobSet, GlobSetBuilder};
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
    let filter = Filter::new(exclude)?;
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
            .filter_entry(|entry| entry.depth() == 0 || !is_skipped(entry.path()));

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
            if filter.excluded.is_match(relative) {
                continue;
            }

            found.push(entry.path().to_path_buf());
        }
    }

    found.sort();
    found.dedup();
    Ok(found)
}

/// The rules `discover` applies, for asking about one file at a time.
///
/// An editor opens files one by one — including ones well outside the project,
/// such as an installed library reached through "go to definition". Each has
/// to be judged by the same rules a walk of the project would have used.
pub struct Filter {
    excluded: GlobSet,
}

impl Filter {
    pub fn new(exclude: &[String]) -> Result<Self, DiscoverError> {
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
        Ok(Self { excluded })
    }

    /// Whether walking `root` would have found `path`.
    pub fn includes(&self, root: &Path, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(root) else {
            return false;
        };
        if !is_python(path) || self.excluded.is_match(relative) {
            return false;
        }

        // Every directory between the root and the file must be one the walk
        // would have entered.
        let mut dir = root.to_path_buf();
        let parents = relative.parent().into_iter().flat_map(Path::components);
        for component in parents {
            dir.push(component);
            if is_skipped(&dir) {
                return false;
            }
        }
        true
    }
}

fn is_python(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "pyi")
    )
}

/// Directories that hold someone else's code.
///
/// A virtual environment is recognised by its `pyvenv.cfg` rather than by its
/// name: `venv` and `env` are conventions, not rules, and a project is free to
/// have a package called `env`. Analysing an environment means re-indexing
/// thousands of installed files on every save and reporting findings in code
/// the user cannot change.
fn is_skipped(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    is_hidden(path)
        || matches!(name, Some("__pycache__" | "node_modules" | "site-packages"))
        || path.join("pyvenv.cfg").is_file()
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
    fn a_virtual_environment_is_skipped_whatever_it_is_called() {
        let dir = tree(&["a.py", "venv/lib/b.py", "my-env/lib/c.py"]);
        fs::write(dir.path().join("venv/pyvenv.cfg"), "").unwrap();
        fs::write(dir.path().join("my-env/pyvenv.cfg"), "").unwrap();
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn a_package_called_env_is_still_analysed() {
        // Only a pyvenv.cfg marks an environment. The name alone does not.
        let dir = tree(&["env/settings.py"]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["env/settings.py"]);
    }

    #[test]
    fn installed_and_generated_directories_are_skipped() {
        let dir = tree(&[
            "a.py",
            "__pycache__/a.py",
            "node_modules/pkg/b.py",
            "lib/site-packages/c.py",
        ]);
        let found = discover(&[dir.path().to_path_buf()], &[]).unwrap();
        assert_eq!(names(&found, dir.path()), vec!["a.py"]);
    }

    #[test]
    fn the_filter_agrees_with_the_walk() {
        let dir = tree(&[
            "a.py",
            "pkg/b.py",
            "tests/c.py",
            "venv/lib/d.py",
            ".git/e.py",
            "notes.md",
        ]);
        fs::write(dir.path().join("venv/pyvenv.cfg"), "").unwrap();
        let exclude = vec!["tests/**".to_string()];

        let walked = discover(&[dir.path().to_path_buf()], &exclude).unwrap();
        let filter = Filter::new(&exclude).unwrap();
        for file in [
            "a.py",
            "pkg/b.py",
            "tests/c.py",
            "venv/lib/d.py",
            ".git/e.py",
            "notes.md",
        ] {
            let path = dir.path().join(file);
            assert_eq!(
                filter.includes(dir.path(), &path),
                walked.contains(&path),
                "{file}"
            );
        }
    }

    #[test]
    fn the_filter_rejects_files_outside_the_root() {
        let inside = tree(&["a.py"]);
        let outside = tree(&["b.py"]);
        let filter = Filter::new(&[]).unwrap();
        assert!(!filter.includes(inside.path(), &outside.path().join("b.py")));
    }

    #[test]
    fn an_invalid_exclude_pattern_is_an_error() {
        let dir = tree(&["a.py"]);
        let err =
            discover(&[dir.path().to_path_buf()], &["[".into()]).expect_err("expected an error");
        assert!(err.to_string().contains("invalid exclude pattern"));
    }
}
