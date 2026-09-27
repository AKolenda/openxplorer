// SPDX-License-Identifier: AGPL-3.0-only
//! Character, escaping and path rules shared by the location parsers.
//!
//! `desktop/core.py` leans on a handful of Python standard-library functions
//! (`str.strip`, `urllib.parse.quote`/`unquote`, `posixpath.normpath`) and
//! the web UI on `decodeURIComponent`. Each helper here reproduces exactly
//! one of those behaviours so that the native app produces the same
//! canonical URIs as the Python app: both write them to the shared
//! `settings.json`.

use std::borrow::Cow;

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

use super::LocationError;

/// Characters Python's `quote()` never escapes: letters, digits and `_.-~`.
const PYTHON_ALWAYS_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

/// `quote(text, safe='/')`: the escaping of every canonical URI path.
pub(crate) const PYTHON_PATH_SAFE: &AsciiSet = &PYTHON_ALWAYS_SAFE.remove(b'/');

/// True if `text` contains a C0 control character or DEL, the characters
/// matched by `CONTROL = re.compile(r'[\x00-\x1f\x7f]')` in `core.py`.
pub(crate) fn has_control(text: &str) -> bool {
    text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// Python's `str.isspace()`. It is Unicode `White_Space` plus the four
/// information separators U+001C..U+001F, which Rust does not count.
pub(crate) fn is_python_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()` with no arguments.
pub(crate) fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_space)
}

/// `quote(text, safe='/')`: percent-encodes everything except letters,
/// digits, `_.-~` and `/`, using upper-case hex digits.
pub(crate) fn quote_path(text: &str) -> String {
    utf8_percent_encode(text, PYTHON_PATH_SAFE).to_string()
}

/// `quote(text, safe='')`: like [`quote_path`] but also escapes `/`, for a
/// single path component.
pub(crate) fn quote_component(text: &str) -> String {
    utf8_percent_encode(text, PYTHON_ALWAYS_SAFE).to_string()
}

/// `unquote(text, errors='strict')`: decodes `%XX` escapes and rejects a
/// result that is not UTF-8. Malformed escapes such as `%zz` stay literal.
pub(crate) fn unquote_strict(text: &str) -> Result<String, LocationError> {
    percent_decode_str(text)
        .decode_utf8()
        .map(Cow::into_owned)
        .map_err(|_| LocationError::new("Percent-encoded text in the address must be valid UTF-8."))
}

/// `unquote(text)` with Python's default `errors='replace'`: invalid UTF-8
/// becomes U+FFFD instead of failing.
pub(crate) fn unquote_lossy(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// JavaScript's `decodeURIComponent`: like [`unquote_strict`] but fails on
/// any `%` that does not start a two-digit hex escape. The web UI fell back
/// to showing the raw URI in that case, and so do the display helpers.
pub(crate) fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(offset) = bytes[index..].iter().position(|&b| b == b'%') {
        let escape = index + offset;
        let digits = bytes.get(escape + 1..escape + 3)?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        index = escape + 3;
    }
    percent_decode_str(text).decode_utf8().ok().map(Cow::into_owned)
}

/// `posixpath.normpath`: removes empty and `.` components and resolves
/// `..` without touching the filesystem.
///
/// Like POSIX (and Python), exactly two leading slashes are kept because
/// their meaning is implementation-defined; three or more become one. An
/// empty result is `.`.
#[expect(
    clippy::bool_to_int_with_if,
    reason = "the three leading-slash cases read best side by side"
)]
pub(crate) fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let leading_slashes = if path.starts_with("//") && !path.starts_with("///") {
        2
    } else if path.starts_with('/') {
        1
    } else {
        0
    };
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                // A relative path keeps `..` it cannot resolve; an absolute
                // path stops at the root.
                let unresolvable = matches!(components.last(), None | Some(&".."));
                if leading_slashes == 0 && unresolvable {
                    components.push(component);
                } else {
                    components.pop();
                }
            }
            _ => components.push(component),
        }
    }
    let joined = format!("{}{}", "/".repeat(leading_slashes), components.join("/"));
    if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

/// Removes at most one trailing `/`, as the web UI's `replace(/\/$/, '')`.
pub(crate) fn strip_one_trailing_slash(text: &str) -> &str {
    text.strip_suffix('/').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_match_the_python_pattern() {
        assert!(has_control("a\u{0}b"));
        assert!(has_control("line\n"));
        assert!(has_control("\u{1f}"));
        assert!(has_control("del\u{7f}"));
        assert!(!has_control("Résumé 2026.txt"));
        // C1 controls are not in `[\x00-\x1f\x7f]`.
        assert!(!has_control("\u{85}"));
    }

    #[test]
    fn strip_matches_python_whitespace() {
        assert_eq!(python_strip("  /tmp \t"), "/tmp");
        assert_eq!(python_strip("\u{1c}label\u{1f}"), "label");
        assert_eq!(python_strip("\u{3000}wide\u{a0}"), "wide");
        assert_eq!(python_strip("\u{0}x"), "\u{0}x");
    }

    #[test]
    fn quoting_matches_python() {
        assert_eq!(quote_path("/tmp/Été #1?.txt"), "/tmp/%C3%89t%C3%A9%20%231%3F.txt");
        assert_eq!(quote_path("/a(1)~_.-b"), "/a%281%29~_.-b");
        assert_eq!(quote_component("Team files/x"), "Team%20files%2Fx");
        assert_eq!(quote_path("100%.pdf"), "100%25.pdf");
    }

    #[test]
    fn unquote_is_strict_about_utf8_only() {
        assert_eq!(
            unquote_strict("Team%20files/100%25.pdf").as_deref(),
            Ok("Team files/100%.pdf")
        );
        assert_eq!(unquote_strict("%zz%4").as_deref(), Ok("%zz%4"));
        assert!(unquote_strict("%FF").is_err());
        assert_eq!(unquote_lossy("a%FFb"), "a\u{fffd}b");
    }

    #[test]
    fn decode_uri_component_rejects_malformed_escapes() {
        assert_eq!(decode_uri_component("Q3%20%231").as_deref(), Some("Q3 #1"));
        assert_eq!(decode_uri_component("%zz"), None);
        assert_eq!(decode_uri_component("50%"), None);
        assert_eq!(decode_uri_component("%C3"), None);
    }

    #[test]
    fn normpath_matches_posixpath() {
        let cases = [
            ("", "."),
            ("/", "/"),
            ("//", "//"),
            ("///", "/"),
            ("//tmp/x", "//tmp/x"),
            ("///tmp//x/", "/tmp/x"),
            ("/a/./b/../c", "/a/c"),
            ("/..", "/"),
            ("/../../x", "/x"),
            ("a/../..", ".."),
            ("../a", "../a"),
            ("a/b/..", "a"),
            ("a/..", "."),
        ];
        for (input, expected) in cases {
            assert_eq!(normpath(input), expected, "normpath({input:?})");
        }
    }
}
