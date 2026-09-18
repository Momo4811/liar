//! Source files, and the mapping from byte offsets to human positions.

use crate::ids::{Arena, FileId, Id};
use std::path::{Path, PathBuf};

/// A 1-based line and column. Columns count Unicode characters, not bytes,
/// because a column is a thing a human counts in a text editor.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    id: FileId,
    path: PathBuf,
    text: String,
    /// Byte offset at which each line begins. Always starts with 0, so its
    /// length is the line count and lookup is a binary search.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(id: FileId, path: PathBuf, text: String) -> Self {
        // A UTF-8 BOM is legal at the start of a Python file and is not part
        // of the program. Strip it here, once, so no offset downstream has to
        // know about it.
        let text = match text.strip_prefix('\u{feff}') {
            Some(stripped) => stripped.to_string(),
            None => text,
        };

        let mut line_starts = vec![0u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' && offset + 1 < text.len() {
                line_starts.push(offset as u32 + 1);
            }
        }

        Self {
            id,
            path,
            text,
            line_starts,
        }
    }

    pub fn id(&self) -> FileId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// # Panics
    /// If `offset` is past the end of the file, or not on a character
    /// boundary — both are engine bugs.
    pub fn position(&self, offset: u32) -> Position {
        assert!(
            offset as usize <= self.text.len(),
            "offset {offset} past end of file {}",
            self.path.display()
        );

        // partition_point gives the number of line starts at or before the
        // offset, which is exactly the 1-based line number.
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let line_start = self.line_starts[line - 1] as usize;

        let column = self.text[line_start..offset as usize].chars().count() as u32 + 1;

        Position {
            line: line as u32,
            column,
        }
    }

    /// Text of a 1-based line, without its terminator.
    ///
    /// # Panics
    /// If `line` is zero or past the end of the file.
    pub fn line_text(&self, line: u32) -> &str {
        assert!(line >= 1, "line numbers are 1-based");
        let index = (line - 1) as usize;
        let start = self.line_starts[index] as usize;
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next as usize);

        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[derive(Debug, Default)]
pub struct SourceMap {
    files: Arena<FileId, SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: PathBuf, text: String) -> FileId {
        // Allocate a placeholder to learn the id, then overwrite it with the
        // real file, since SourceFile needs to know its own id.
        let id = self.files.alloc(SourceFile {
            id: FileId::from_index(0),
            path: PathBuf::new(),
            text: String::new(),
            line_starts: vec![0],
        });
        *self.files.get_mut(id) = SourceFile::new(id, path, text);
        id
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        self.files.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        self.files.iter()
    }
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

    #[test]
    fn first_character_is_line_one_column_one() {
        let f = file("abc");
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
    }

    #[test]
    fn columns_advance_within_a_line() {
        let f = file("abc");
        assert_eq!(f.position(2), Position { line: 1, column: 3 });
    }

    #[test]
    fn newline_starts_the_next_line() {
        let f = file("ab\ncd");
        assert_eq!(f.position(3), Position { line: 2, column: 1 });
        assert_eq!(f.position(4), Position { line: 2, column: 2 });
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        // U+00E9 is two bytes in UTF-8. The character after it is column 3,
        // not column 4 — pointing a caret at byte offsets would misalign every
        // diagnostic on a line containing non-ASCII.
        let f = file("a\u{e9}b");
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
        assert_eq!(f.position(1), Position { line: 1, column: 2 });
        assert_eq!(f.position(3), Position { line: 1, column: 3 });
    }

    #[test]
    fn astral_plane_characters_count_as_one_column() {
        // A four-byte character.
        let f = file("\u{1f980}x");
        assert_eq!(f.position(4), Position { line: 1, column: 2 });
    }

    #[test]
    fn crlf_does_not_produce_a_phantom_column() {
        let f = file("ab\r\ncd");
        assert_eq!(f.position(4), Position { line: 2, column: 1 });
    }

    #[test]
    fn a_leading_bom_is_stripped() {
        // Python permits a UTF-8 BOM. If it is not stripped, every offset on
        // line 1 is three bytes off.
        let f = file("\u{feff}abc");
        assert_eq!(f.text(), "abc");
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
    }

    #[test]
    fn offset_at_end_of_file_is_valid() {
        let f = file("ab");
        assert_eq!(f.position(2), Position { line: 1, column: 3 });
    }

    #[test]
    fn empty_file_has_one_line() {
        let f = file("");
        assert_eq!(f.line_count(), 1);
        assert_eq!(f.position(0), Position { line: 1, column: 1 });
    }

    #[test]
    fn trailing_newline_does_not_add_a_line() {
        let f = file("a\n");
        assert_eq!(f.line_count(), 1);
    }

    #[test]
    fn line_text_excludes_the_line_terminator() {
        let f = file("ab\ncd\r\nef");
        assert_eq!(f.line_text(1), "ab");
        assert_eq!(f.line_text(2), "cd");
        assert_eq!(f.line_text(3), "ef");
    }

    #[test]
    fn source_map_assigns_distinct_ids() {
        let mut map = SourceMap::new();
        let a = map.add(PathBuf::from("a.py"), "a".into());
        let b = map.add(PathBuf::from("b.py"), "b".into());
        assert_ne!(a, b);
        assert_eq!(map.get(a).text(), "a");
        assert_eq!(map.get(b).text(), "b");
    }

    #[test]
    #[should_panic(expected = "offset 99 past end of file")]
    fn offset_past_end_panics() {
        let f = file("ab");
        let _ = f.position(99);
    }

    proptest! {
        /// position() must never panic on any character boundary of any text,
        /// and must always return a line within the file.
        #[test]
        fn position_is_total_over_character_boundaries(text in ".{0,400}") {
            let f = file(&text);
            for (offset, _) in f.text().char_indices() {
                let p = f.position(offset as u32);
                prop_assert!(p.line >= 1);
                prop_assert!(p.line <= f.line_count());
                prop_assert!(p.column >= 1);
            }
            let end = f.text().len() as u32;
            prop_assert!(f.position(end).line >= 1);
        }
    }
}
