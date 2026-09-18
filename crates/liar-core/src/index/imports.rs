//! Working out which file a module name refers to.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Where a file sits in the package hierarchy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModuleLocation {
    /// The file's own dotted path: `pkg/sub/mod.py` is `pkg.sub.mod`.
    pub module: String,
    /// The package a relative import resolves against. For `pkg/sub/mod.py`
    /// that is `pkg.sub`; for a package's own `__init__.py` it is the package
    /// itself, since `from . import x` inside `pkg/__init__.py` means `pkg.x`.
    pub package: String,
}

/// Derives a module location for `path`, given every file being analysed.
///
/// A directory is part of the package hierarchy only if it contains an
/// `__init__.py` that is itself being analysed. Walking up past that would
/// invent package names from whatever the checkout happens to be called.
pub fn locate(path: &Path, all: &HashSet<PathBuf>) -> ModuleLocation {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let is_package_init = stem == "__init__";

    let mut packages = Vec::new();
    let mut dir = path.parent();
    while let Some(current) = dir {
        if !all.contains(&current.join("__init__.py")) {
            break;
        }
        match current.file_name().and_then(|n| n.to_str()) {
            Some(name) => packages.push(name.to_string()),
            None => break,
        }
        dir = current.parent();
    }
    packages.reverse();

    let package = packages.join(".");

    let module = if is_package_init {
        package.clone()
    } else if package.is_empty() {
        stem.to_string()
    } else {
        format!("{package}.{stem}")
    };

    ModuleLocation { module, package }
}

/// Resolves a module string as stored on a `FromImport` binding.
///
/// Absolute names come back unchanged. A relative name — one written with
/// leading dots — is resolved against `package`. Going up past the top of the
/// package tree yields `None` rather than a guess.
pub fn resolve_module_name(raw: &str, package: &str) -> Option<String> {
    let level = raw.chars().take_while(|c| *c == '.').count();
    let tail = &raw[level..];

    if level == 0 {
        return Some(raw.to_string());
    }

    let mut components: Vec<&str> = if package.is_empty() {
        Vec::new()
    } else {
        package.split('.').collect()
    };

    // One dot means "this package", so only the dots beyond the first walk up.
    let up = level - 1;
    if up > components.len() {
        return None;
    }
    components.truncate(components.len() - up);

    if !tail.is_empty() {
        components.push(tail);
    }

    if components.is_empty() {
        None
    } else {
        Some(components.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_standalone_file_is_its_own_module() {
        let all = files(&["script.py"]);
        let found = locate(Path::new("script.py"), &all);
        assert_eq!(found.module, "script");
        assert_eq!(found.package, "");
    }

    #[test]
    fn a_file_in_a_package_is_dotted() {
        let all = files(&["pkg/__init__.py", "pkg/mod.py"]);
        let found = locate(Path::new("pkg/mod.py"), &all);
        assert_eq!(found.module, "pkg.mod");
        assert_eq!(found.package, "pkg");
    }

    #[test]
    fn a_package_init_is_the_package_itself() {
        let all = files(&["pkg/__init__.py"]);
        let found = locate(Path::new("pkg/__init__.py"), &all);
        assert_eq!(found.module, "pkg");
        assert_eq!(found.package, "pkg");
    }

    #[test]
    fn nested_packages_nest() {
        let all = files(&["pkg/__init__.py", "pkg/sub/__init__.py", "pkg/sub/mod.py"]);
        let found = locate(Path::new("pkg/sub/mod.py"), &all);
        assert_eq!(found.module, "pkg.sub.mod");
        assert_eq!(found.package, "pkg.sub");
    }

    #[test]
    fn the_walk_stops_where_the_init_files_stop() {
        // No pkg/__init__.py, so pkg is not a package and the hierarchy starts
        // at sub. Walking further up would invent a package name from whatever
        // the checkout directory happens to be called.
        let all = files(&["pkg/sub/__init__.py", "pkg/sub/mod.py"]);
        let found = locate(Path::new("pkg/sub/mod.py"), &all);
        assert_eq!(found.module, "sub.mod");
        assert_eq!(found.package, "sub");
    }

    #[test]
    fn an_absolute_module_name_is_unchanged() {
        assert_eq!(resolve_module_name("time", "pkg"), Some("time".into()));
        assert_eq!(resolve_module_name("os.path", ""), Some("os.path".into()));
    }

    #[test]
    fn one_dot_means_the_current_package() {
        assert_eq!(
            resolve_module_name(".utils", "pkg.sub"),
            Some("pkg.sub.utils".into())
        );
    }

    #[test]
    fn a_bare_dot_is_the_package_itself() {
        assert_eq!(resolve_module_name(".", "pkg.sub"), Some("pkg.sub".into()));
    }

    #[test]
    fn two_dots_walk_up_one_level() {
        assert_eq!(
            resolve_module_name("..utils", "pkg.sub"),
            Some("pkg.utils".into())
        );
    }

    #[test]
    fn three_dots_walk_up_two_levels() {
        assert_eq!(
            resolve_module_name("...utils", "a.b.c"),
            Some("a.utils".into())
        );
    }

    #[test]
    fn walking_up_past_the_root_yields_nothing() {
        // Better to say nothing than to resolve to a module that does not
        // exist and then report findings about it.
        assert_eq!(resolve_module_name("...utils", "pkg"), None);
        assert_eq!(resolve_module_name("..x", ""), None);
    }

    #[test]
    fn a_relative_import_from_a_top_level_module_has_no_package_to_climb() {
        assert_eq!(resolve_module_name(".utils", ""), Some("utils".into()));
    }
}
