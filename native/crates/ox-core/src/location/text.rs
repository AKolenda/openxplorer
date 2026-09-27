// SPDX-License-Identifier: AGPL-3.0-only
//! Character, escaping and path rules shared by the location parsers.
//!
//! `desktop/core.py` leans on a handful of Python standard-library functions
//! (`str.strip`, `str.isspace`, `urllib.parse.quote`/`unquote`,
//! `posixpath.normpath`) and the web UI on `decodeURIComponent`. Each helper
//! here reproduces exactly one of those behaviours so that the native app
//! produces the same canonical URIs as the Python app: both write them to the
//! shared `settings.json`.

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
pub(crate) fn has_control_character(text: &str) -> bool {
    text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// Python's `str.isspace()`. It is Unicode `White_Space` plus the four
/// information separators U+001C..U+001F, which Rust does not count.
pub(crate) fn is_python_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// True if `text` contains a character Python's `str.isspace()` accepts,
/// like `any(c.isspace() for c in text)` in `core.py`.
pub(crate) fn contains_python_space(text: &str) -> bool {
    text.chars().any(is_python_space)
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

/// [`unquote_strict`] for the path of an address, which must not decode to
/// a control character.
///
/// Safety rule (`core.py`: `if CONTROL.search(decoded)` after every
/// `unquote`): a `%00` or `%0A` in an address never reaches GIO or a file
/// name.
pub(crate) fn unquote_without_controls(text: &str) -> Result<String, LocationError> {
    let decoded = unquote_strict(text)?;
    if has_control_character(&decoded) {
        return Err(LocationError::new("Encoded control characters are not allowed."));
    }
    Ok(decoded)
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
    if !has_only_complete_escapes(text) {
        return None;
    }
    percent_decode_str(text).decode_utf8().ok().map(Cow::into_owned)
}

/// True when every `%` in `text` starts a `%XX` escape.
fn has_only_complete_escapes(text: &str) -> bool {
    text.split('%').skip(1).all(starts_with_two_hex_digits)
}

fn starts_with_two_hex_digits(text: &str) -> bool {
    let Some(digits) = text.as_bytes().get(..2) else {
        return false;
    };
    digits.iter().all(u8::is_ascii_hexdigit)
}

/// `posixpath.normpath`: removes empty and `.` components and resolves
/// `..` without touching the filesystem. An empty result is `.`.
pub(crate) fn normpath(path: &str) -> String {
    let root = posix_root(path);
    let components = resolve_dot_segments(path, !root.is_empty());
    let normal = format!("{root}{}", components.join("/"));
    if normal.is_empty() {
        ".".into()
    } else {
        normal
    }
}

/// The leading slashes `normpath` keeps. Like POSIX (and Python), exactly
/// two are kept because their meaning is implementation-defined; three or
/// more become one.
fn posix_root(path: &str) -> &'static str {
    if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    }
}

/// The components of `path` without empty and `.` ones, each `..` removing
/// the component before it. An absolute path stops at the root; a relative
/// path keeps the `..` it cannot resolve.
fn resolve_dot_segments(path: &str, is_absolute: bool) -> Vec<&str> {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." if !is_absolute && matches!(components.last(), None | Some(&"..")) => {
                components.push(component);
            }
            ".." => {
                components.pop();
            }
            _ => components.push(component),
        }
    }
    components
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
        assert!(has_control_character("a\u{0}b"));
        assert!(has_control_character("line\n"));
        assert!(has_control_character("\u{1f}"));
        assert!(has_control_character("del\u{7f}"));
        assert!(!has_control_character("Résumé 2026.txt"));
        // C1 controls are not in `[\x00-\x1f\x7f]`.
        assert!(!has_control_character("\u{85}"));
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
    fn encoded_control_characters_are_refused_in_address_paths() {
        assert_eq!(unquote_without_controls("a%20b").as_deref(), Ok("a b"));
        for escaped in ["a%00b", "line%0A", "%1F", "del%7F"] {
            let error = unquote_without_controls(escaped).expect_err(escaped);
            assert_eq!(error.message(), "Encoded control characters are not allowed.");
        }
        // C1 controls are not in `[\x00-\x1f\x7f]`.
        assert_eq!(unquote_without_controls("%C2%85").as_deref(), Ok("\u{85}"));
    }

    #[test]
    fn decode_uri_component_rejects_malformed_escapes() {
        assert_eq!(decode_uri_component("Q3%20%231").as_deref(), Some("Q3 #1"));
        assert_eq!(decode_uri_component("%zz"), None);
        assert_eq!(decode_uri_component("50%"), None);
        assert_eq!(decode_uri_component("%C3"), None);
    }

    /// One `posixpath.normpath` example.
    struct NormpathCase {
        path: &'static str,
        normal: &'static str,
    }

    #[test]
    fn normpath_matches_posixpath() {
        let cases = [
            NormpathCase {
                path: "",
                normal: ".",
            },
            NormpathCase {
                path: "/",
                normal: "/",
            },
            NormpathCase {
                path: "//",
                normal: "//",
            },
            NormpathCase {
                path: "///",
                normal: "/",
            },
            NormpathCase {
                path: "//tmp/x",
                normal: "//tmp/x",
            },
            NormpathCase {
                path: "///tmp//x/",
                normal: "/tmp/x",
            },
            NormpathCase {
                path: "/a/./b/../c",
                normal: "/a/c",
            },
            NormpathCase {
                path: "/..",
                normal: "/",
            },
            NormpathCase {
                path: "/../../x",
                normal: "/x",
            },
            NormpathCase {
                path: "a/../..",
                normal: "..",
            },
            NormpathCase {
                path: "../a",
                normal: "../a",
            },
            NormpathCase {
                path: "a/b/..",
                normal: "a",
            },
            NormpathCase {
                path: "a/..",
                normal: ".",
            },
        ];
        for case in cases {
            assert_eq!(normpath(case.path), case.normal, "normpath({:?})", case.path);
        }
    }
}
