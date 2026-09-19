//! Reading a docstring.
//!
//! The only place in the project where a check reads raw text, and worth
//! saying why that is acceptable: here the text *is* the subject. Every other
//! check reads text only to guess at structure, which is the thing this tool is
//! against.

/// Strips the quoting from a string literal as it appears in source.
///
/// Handles every prefix Python allows and both quote lengths. Returns `None`
/// for anything that is not a recognisable string literal, because a docstring
/// this cannot read is a docstring it says nothing about.
pub fn unquote(literal: &str) -> Option<&str> {
    let rest = literal.trim_start_matches(['r', 'R', 'u', 'U', 'f', 'F', 'b', 'B']);

    for fence in ["\"\"\"", "'''"] {
        if let Some(inner) = rest.strip_prefix(fence).and_then(|s| s.strip_suffix(fence)) {
            return Some(inner);
        }
    }
    for quote in ["\"", "'"] {
        if let Some(inner) = rest.strip_prefix(quote).and_then(|s| s.strip_suffix(quote)) {
            return Some(inner);
        }
    }

    None
}

/// Every parameter name the docstring claims to document.
///
/// Recognises the three styles in common use. A name is only collected where
/// the style makes it unambiguous, so prose mentioning a parameter in passing
/// is not mistaken for documenting it.
pub fn documented_params(docstring: &str) -> Vec<String> {
    let lines: Vec<&str> = docstring.lines().collect();
    let mut found = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let raw = lines[index];
        let line = raw.trim();

        // Sphinx: `:param name:` anywhere, no section needed.
        if let Some(rest) = line.strip_prefix(":param ")
            && let Some((name, _)) = rest.split_once(':')
        {
            // `:param int count:` names the type first.
            push(&mut found, name.split_whitespace().last().unwrap_or(name));
        }

        // NumPy: a header underlined with dashes.
        if is_underlined(&lines, index) {
            if matches!(line, "Parameters" | "Other Parameters") {
                index = collect_numpy(&lines, index + 2, &mut found);
                continue;
            }
            index += 2;
            continue;
        }

        // Google: a header ending in a colon, its entries indented under it.
        if matches!(
            line,
            "Args:" | "Arguments:" | "Parameters:" | "Keyword Args:"
        ) {
            index = collect_google(&lines, index + 1, indent_of(raw), &mut found);
            continue;
        }

        index += 1;
    }

    found
}

/// Whether `lines[index]` is a NumPy section header, underlined with dashes.
fn is_underlined(lines: &[&str], index: usize) -> bool {
    let Some(next) = lines.get(index + 1) else {
        return false;
    };
    let underline = next.trim();
    !lines[index].trim().is_empty() && underline.len() >= 3 && underline.chars().all(|c| c == '-')
}

/// Collects a NumPy parameter section, returning where it ended.
///
/// Entries sit at the section's own indentation; their descriptions are
/// indented further. The section ends at a blank line or the next header.
fn collect_numpy(lines: &[&str], mut index: usize, found: &mut Vec<String>) -> usize {
    let base = lines.get(index).map_or(0, |line| indent_of(line));

    while index < lines.len() {
        let raw = lines[index];
        let line = raw.trim();

        if line.is_empty() || is_underlined(lines, index) {
            return index;
        }
        // Indented further than the entries: a description, not a name.
        if indent_of(raw) > base {
            index += 1;
            continue;
        }

        push(found, line.split(':').next().unwrap_or(line));
        index += 1;
    }

    index
}

/// Collects a Google parameter section, returning where it ended.
///
/// Entries are indented *past the header*, which is what ends the section: a
/// docstring inside a function is itself indented, so `Returns:` sits at the
/// same depth as `Args:` and not at zero.
fn collect_google(
    lines: &[&str],
    mut index: usize,
    header_indent: usize,
    found: &mut Vec<String>,
) -> usize {
    // Entries all sit at one depth; anything deeper is prose. Without this, a
    // continuation line such as `Default: localhost` reads as a parameter.
    let mut entry_indent: Option<usize> = None;

    while index < lines.len() {
        let raw = lines[index];
        let line = raw.trim();

        if line.is_empty() {
            index += 1;
            continue;
        }
        let indent = indent_of(raw);
        if indent <= header_indent {
            return index;
        }

        let entry = *entry_indent.get_or_insert(indent);
        if indent > entry {
            index += 1;
            continue;
        }

        if let Some((name, _)) = line.split_once(':') {
            // `url (str): where from`
            push(found, name.split('(').next().unwrap_or(name));
        }
        index += 1;
    }

    index
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Whether the docstring claims the function gives something back.
pub fn documents_a_return(docstring: &str) -> bool {
    docstring.lines().any(|raw| {
        let line = raw.trim();
        matches!(line, "Returns:" | "Yields:" | "Returns" | "Yields")
            || line.starts_with(":return")
            || line.starts_with(":rtype:")
    })
}

/// The type named by a Sphinx `:rtype:`, if there is one.
///
/// Only this form. Prose such as "returns a list of users" is not parsed:
/// guessing at English is exactly the invention this tool is named after.
pub fn documented_return_type(docstring: &str) -> Option<String> {
    docstring.lines().find_map(|raw| {
        raw.trim()
            .strip_prefix(":rtype:")
            .map(|rest| rest.trim().to_string())
            .filter(|name| !name.is_empty())
    })
}

fn push(found: &mut Vec<String>, name: &str) {
    let name = name.trim().trim_start_matches('*');
    if name.is_empty() || !is_identifier(name) {
        return;
    }
    let name = name.to_string();
    if !found.contains(&name) {
        found.push(name);
    }
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triple_quotes_are_stripped() {
        assert_eq!(unquote("\"\"\"hello\"\"\""), Some("hello"));
        assert_eq!(unquote("'''hello'''"), Some("hello"));
    }

    #[test]
    fn single_quotes_are_stripped() {
        assert_eq!(unquote("\"hello\""), Some("hello"));
        assert_eq!(unquote("'hello'"), Some("hello"));
    }

    #[test]
    fn prefixes_are_stripped() {
        assert_eq!(unquote("r\"\"\"raw\"\"\""), Some("raw"));
        assert_eq!(unquote("Rb'bytes'"), Some("bytes"));
    }

    #[test]
    fn something_that_is_not_a_literal_reads_as_nothing() {
        assert_eq!(unquote("hello"), None);
        assert_eq!(unquote("\"unterminated"), None);
    }

    #[test]
    fn google_style_parameters_are_found() {
        let doc = "Fetch a thing.\n\nArgs:\n    url: where from\n    timeout: how long\n";
        assert_eq!(documented_params(doc), vec!["url", "timeout"]);
    }

    #[test]
    fn google_style_types_in_brackets_are_ignored() {
        let doc = "Args:\n    url (str): where from\n    count (int): how many\n";
        assert_eq!(documented_params(doc), vec!["url", "count"]);
    }

    #[test]
    fn a_google_section_ends_at_the_next_one() {
        let doc = "Args:\n    url: where from\n\nReturns:\n    The body, as text.\n";
        assert_eq!(documented_params(doc), vec!["url"]);
    }

    #[test]
    fn a_google_section_ends_at_the_next_one_when_everything_is_indented() {
        // How a docstring inside a function actually looks. The test above uses
        // unindented text and so never caught this: the corpus produced 1,403
        // findings, almost all of them "Returns" and "Raises" read as names.
        let doc = concat!(
            "Fetch a thing.\n",
            "\n",
            "    Args:\n",
            "        url: where from\n",
            "\n",
            "    Returns:\n",
            "        the body\n",
            "\n",
            "    Raises:\n",
            "        ClientError: when it fails\n",
            "    ",
        );
        assert_eq!(documented_params(doc), vec!["url"]);
    }

    #[test]
    fn a_continuation_line_is_prose_not_an_entry() {
        // From sanic and aiohttp: a description continued on the next line,
        // itself containing a colon.
        let doc = concat!(
            "    Args:\n",
            "        host: the host to bind\n",
            "            Default: localhost\n",
            "        port: the port\n",
        );
        assert_eq!(documented_params(doc), vec!["host", "port"]);
    }

    #[test]
    fn numpy_style_parameters_are_found() {
        let doc = "Fetch.\n\nParameters\n----------\nurl : str\n    where from\ntimeout : int\n";
        let found = documented_params(doc);
        assert!(found.contains(&"url".to_string()), "got {found:?}");
        assert!(found.contains(&"timeout".to_string()), "got {found:?}");
    }

    #[test]
    fn sphinx_style_parameters_are_found() {
        let doc = "Fetch.\n\n:param url: where from\n:param timeout: how long\n";
        assert_eq!(documented_params(doc), vec!["url", "timeout"]);
    }

    #[test]
    fn a_sphinx_type_before_the_name_is_handled() {
        let doc = ":param int count: how many\n";
        assert_eq!(documented_params(doc), vec!["count"]);
    }

    #[test]
    fn prose_mentioning_a_name_documents_nothing() {
        // The reason names are only collected where the style is unambiguous.
        let doc = "Fetches url and returns the body. Pass timeout to bound it.\n";
        assert!(documented_params(doc).is_empty());
    }

    #[test]
    fn a_docstring_with_no_sections_documents_nothing() {
        assert!(documented_params("Just a sentence.").is_empty());
    }

    #[test]
    fn returns_sections_are_recognised_in_every_style() {
        for doc in [
            "Returns:\n    the body\n",
            "Returns\n-------\nstr\n",
            ":return: the body\n",
            ":rtype: str\n",
            "Yields:\n    each row\n",
        ] {
            assert!(documents_a_return(doc), "{doc:?}");
        }
    }

    #[test]
    fn a_docstring_that_promises_nothing_is_not_a_return() {
        for doc in [
            "Saves the record.",
            "Args:\n    record: the thing\n",
            "This returns quickly.",
        ] {
            assert!(!documents_a_return(doc), "{doc:?}");
        }
    }

    #[test]
    fn an_rtype_names_a_type() {
        assert_eq!(
            documented_return_type(":rtype: str\n").as_deref(),
            Some("str")
        );
        assert_eq!(
            documented_return_type("Stuff.\n\n:rtype: list\n").as_deref(),
            Some("list")
        );
    }

    #[test]
    fn prose_does_not_name_a_type() {
        // "returns a list of users" is not parsed. Guessing at English is the
        // invention this tool is named after.
        assert_eq!(
            documented_return_type("Returns:\n    a list of users\n"),
            None
        );
    }

    #[test]
    fn stars_are_stripped_from_documented_names() {
        let doc = ":param *args: extras\n:param **kwargs: more\n";
        assert_eq!(documented_params(doc), vec!["args", "kwargs"]);
    }
}
