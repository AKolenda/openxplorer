// SPDX-License-Identifier: AGPL-3.0-only
//! Python's `str.splitlines()`, which `decode_clipboard` in
//! `desktop/file_clipboard.py` uses on every external file list.
//!
//! Rust's `str::lines` splits only on `\n` and `\r\n`, so a list written
//! with bare CR or a Unicode line separator would decode differently in
//! the two apps, and a valid selection could be refused.

/// Splits `text` the way Python's `str.splitlines()` does, including bare
/// CR and Unicode separators. A final line ending does not add an empty
/// line, and CRLF is one separator.
pub(super) fn python_split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !is_line_separator(character) {
            continue;
        }
        lines.push(&text[start..index]);
        start = index + character.len_utf8();
        let is_crlf = character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n');
        if is_crlf {
            characters.next();
            start += 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The characters Python's `str.splitlines()` splits on.
fn is_line_separator(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

#[cfg(test)]
mod tests {
    //! Every expected value is what Python 3's `str.splitlines()` returns
    //! for the same text.

    use super::*;

    #[test]
    fn every_python_line_ending_separates_lines() {
        let text = "a\r\nb\rc\u{85}d\u{2029}e\x0bf\x0cg\x1ch\x1di\x1ej\u{2028}k";
        assert_eq!(
            python_split_lines(text),
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]
        );
    }

    #[test]
    fn a_final_line_ending_adds_no_empty_line() {
        assert_eq!(python_split_lines("a\nb\n"), ["a", "b"]);
        assert_eq!(python_split_lines("a\r\n"), ["a"]);
        assert!(python_split_lines("").is_empty());
    }

    #[test]
    fn blank_lines_inside_the_text_are_kept() {
        assert_eq!(python_split_lines("a\n\nb"), ["a", "", "b"]);
        assert_eq!(python_split_lines("a\n\r\nb"), ["a", "", "b"]);
    }
}
