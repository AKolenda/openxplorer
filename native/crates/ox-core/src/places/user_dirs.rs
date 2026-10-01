// SPDX-License-Identifier: AGPL-3.0-only
//! Reading `user-dirs.dirs` without ever running it as a shell script.
//!
//! Ports `read_user_dirs` and its `LINE` pattern from
//! `v2.0.0:desktop/folder_locations.py`. Both applications must resolve the same
//! standard folders, because Quick access compares their URIs with the
//! `hiddenQuick` and `quickOrder` entries in the shared `settings.json`.
//!
//! The rules only involve ASCII characters, so a value is checked and
//! decoded as bytes: a home folder that is not UTF-8 passes through
//! unchanged, as Python's `surrogateescape` paths do.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use rustix::fs::OFlags;

use super::known_folders::KnownFolder;
use crate::location::python_strip;
use crate::private_storage::KernelOpenFlags;

/// The standard folders a file configures validly.
pub(super) type UserDirs = HashMap<KnownFolder, PathBuf>;

/// Largest `user-dirs.dirs` read, in bytes.
const SIZE_LIMIT: u64 = 128 * 1024;

/// Why `user-dirs.dirs` was not read.
#[derive(Debug, thiserror::Error)]
pub(super) enum UserDirsError {
    /// Over [`SIZE_LIMIT`], in Python's words.
    #[error("The user-dirs configuration is unexpectedly large.")]
    TooLarge,
    /// Not UTF-8, which Python's `read_text(encoding='utf-8')` refuses.
    #[error("The user-dirs configuration is not valid UTF-8 text.")]
    NotText,
    /// A FIFO, device or directory.
    #[error("The user-dirs configuration is not a regular file.")]
    NotAFile,
    /// The operating system refused to open or read it.
    #[error("The user-dirs configuration could not be read: {0}.")]
    Io(#[from] io::Error),
}

/// Reads the standard folders configured in `path`; a missing file
/// configures none. `$HOME` means `home`.
///
/// # Errors
///
/// A [`UserDirsError`] for a file that is too large, not UTF-8 text, not a
/// regular file, or cannot be opened or read.
pub(super) fn read(path: &Path, home: &Path) -> Result<UserDirs, UserDirsError> {
    // O_NONBLOCK: a FIFO in place of the file must not hang the sidebar.
    let opened = OpenOptions::new()
        .read(true)
        .kernel_flags(OFlags::NONBLOCK)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(UserDirs::new()),
        Err(error) => return Err(error.into()),
    };
    let text = read_limited_text(file)?;
    Ok(parse(&text, home))
}

/// Parses the documented `XDG_<NAME>_DIR="<path>"` lines. Unknown names,
/// other lines and invalid values are skipped; for a repeated name the
/// last valid line wins.
pub(super) fn parse(text: &str, home: &Path) -> UserDirs {
    let mut folders = UserDirs::new();
    for line in text.split(is_python_line_boundary) {
        let Some((key, value)) = split_assignment(line) else {
            continue;
        };
        let Some(folder) = KnownFolder::from_xdg_key(key) else {
            continue;
        };
        if let Some(path) = decode_path(value, home) {
            folders.insert(folder, path);
        }
    }
    folders
}

/// The whole file as UTF-8 text, refusing more than [`SIZE_LIMIT`] bytes
/// even if it grows while it is read.
fn read_limited_text(file: File) -> Result<String, UserDirsError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(UserDirsError::NotAFile);
    }
    if metadata.len() > SIZE_LIMIT {
        return Err(UserDirsError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(SIZE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > SIZE_LIMIT {
        return Err(UserDirsError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| UserDirsError::NotText)
}

/// The characters Python's `str.splitlines()` ends a line at. `\r\n`
/// yields an extra empty line here, which never matches, so it does not
/// need special treatment.
fn is_python_line_boundary(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// The name and raw quoted value of a line matching Python's
/// `^\s*XDG_([A-Z]+)_DIR\s*=\s*"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$`.
///
/// `\s` is Python's Unicode white space, which [`python_strip`] trims;
/// trimming the end as well is harmless because the pattern allows
/// trailing white space anyway.
fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let rest = python_strip(line).strip_prefix("XDG_")?;
    let name_length = rest.bytes().take_while(u8::is_ascii_uppercase).count();
    let (name, rest) = rest.split_at(name_length);
    let rest = rest.strip_prefix("_DIR")?;
    let rest = python_strip(rest).strip_prefix('=')?;
    let quoted = python_strip(rest).strip_prefix('"')?;
    let value_length = quoted_length(quoted)?;
    let (value, rest) = quoted.split_at(value_length);
    let after_value = python_strip(rest.strip_prefix('"')?);
    let is_line_end = after_value.is_empty() || after_value.starts_with('#');
    (!name.is_empty() && is_line_end).then_some((name, value))
}

/// The length of a double-quoted value up to its closing quote, where a
/// backslash always takes the next character with it; `None` without a
/// closing quote.
fn quoted_length(quoted: &str) -> Option<usize> {
    let mut characters = quoted.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => return Some(index),
            '\\' => {
                characters.next()?;
            }
            _ => {}
        }
    }
    None
}

/// The folder a raw value names, or `None` if it is not a valid absolute
/// path. Follows `read_user_dirs` step by step.
fn decode_path(value: &str, home: &Path) -> Option<PathBuf> {
    let raw = expand_home(value.as_bytes(), home);
    if !raw.starts_with(b"/") {
        return None;
    }
    let decoded = unescape(&raw)?;
    // Python's CONTROL check; unescaped control characters were refused
    // already, so only DEL is left to catch.
    if decoded.contains(&0x7f) {
        return None;
    }
    Some(normalise_absolute(Path::new(OsStr::from_bytes(&decoded))))
}

/// Replaces a leading `$HOME` or `${HOME}` that stands alone or before a
/// `/` with `home`, the only variables `read_user_dirs` expands.
fn expand_home(value: &[u8], home: &Path) -> Vec<u8> {
    for variable in [b"$HOME".as_slice(), b"${HOME}".as_slice()] {
        let Some(rest) = value.strip_prefix(variable) else {
            continue;
        };
        if rest.is_empty() || rest.starts_with(b"/") {
            return [home.as_os_str().as_bytes(), rest].concat();
        }
    }
    value.to_vec()
}

/// Decodes the shell's quoted-string escapes `\\`, `\"`, `\$` and `` \` ``;
/// any other backslash stays literal.
///
/// Safety rule "never evaluate the file" (`read_user_dirs` in
/// `folder_locations.py`): an unescaped `$` or backtick would be a variable or
/// command substitution in a shell, and a control character is never part
/// of a folder name, so either makes the value invalid.
fn unescape(raw: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(raw.len());
    let mut bytes = raw.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            let escaped = bytes.next_if(|next| matches!(next, b'\\' | b'"' | b'$' | b'`'));
            decoded.push(escaped.unwrap_or(b'\\'));
            continue;
        }
        if matches!(byte, b'$' | b'`') || byte < 0x20 {
            return None;
        }
        decoded.push(byte);
    }
    Some(decoded)
}

/// Python's `os.path.normpath` for an absolute path: repeated slashes, `.`
/// and trailing slashes go, `..` removes the previous name and stops at
/// the root. Exactly two leading slashes stay two, as POSIX allows.
fn normalise_absolute(path: &Path) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    let keeps_two_slashes = bytes.starts_with(b"//") && !bytes.starts_with(b"///");
    let mut names: Vec<&OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => names.push(name),
            Component::ParentDir => {
                names.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    let root = if keeps_two_slashes { "//" } else { "/" };
    let mut normalised = PathBuf::from(root);
    normalised.extend(names);
    normalised
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::make_fifo;

    const HOME: &str = "/home/demo";

    /// One line of `user-dirs.dirs` and the Desktop folder it configures.
    struct Case {
        line: &'static str,
        desktop: Option<&'static str>,
    }

    fn desktop(text: &str) -> Option<PathBuf> {
        parse(text, Path::new(HOME)).remove(&KnownFolder::Desktop)
    }

    #[test]
    fn values_are_decoded_like_read_user_dirs() {
        let cases = [
            Case {
                line: r#"XDG_DESKTOP_DIR="$HOME/Desk""#,
                desktop: Some("/home/demo/Desk"),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="${HOME}/Desk""#,
                desktop: Some("/home/demo/Desk"),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="$HOME""#,
                desktop: Some("/home/demo"),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="$HOME/""#,
                desktop: Some("/home/demo"),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="/a/\"q\" \\ \$ \` \x""#,
                desktop: Some(r#"/a/"q" \ $ ` \x"#),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="/data//x/./y/../z/""#,
                desktop: Some("/data/x/z"),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="//server/x""#,
                desktop: Some("//server/x"),
            },
            Case {
                line: r#"XDG_DESKTOP_DIR="///x""#,
                desktop: Some("/x"),
            },
            Case {
                line: r#"  XDG_DESKTOP_DIR = "/x"  # moved"#,
                desktop: Some("/x"),
            },
        ];
        for case in cases {
            let expected = case.desktop.map(PathBuf::from);
            assert_eq!(desktop(case.line), expected, "{}", case.line);
        }
    }

    /// The refusals SET-018 describes: shell syntax, control characters
    /// and relative paths fall back to the standard folder.
    ///
    /// parity: SET-018
    #[test]
    fn shell_syntax_control_characters_and_relative_paths_are_refused() {
        let lines = [
            r#"XDG_DESKTOP_DIR="/data/$USER""#,
            r#"XDG_DESKTOP_DIR="/data/`id`""#,
            r#"XDG_DESKTOP_DIR="$HOMEDIR/x""#,
            r#"XDG_DESKTOP_DIR="Desktop""#,
            r#"XDG_DESKTOP_DIR="~/Desktop""#,
            "XDG_DESKTOP_DIR=\"/tab\there\"",
            "XDG_DESKTOP_DIR=\"/del\u{7f}\"",
            r#"XDG_DESKTOP_DIR="/x" trailing"#,
            r#"XDG_DESKTOP_DIR="/unterminated"#,
            r"XDG_DESKTOP_DIR=/unquoted",
            r#"xdg_desktop_dir="/lower""#,
            r#"XDG__DIR="/no-name""#,
        ];
        for line in lines {
            assert_eq!(desktop(line), None, "{line}");
        }
    }

    #[test]
    fn the_last_valid_line_wins() {
        let text = "XDG_DESKTOP_DIR=\"/first\"\nXDG_DESKTOP_DIR=\"/second\"\nXDG_DESKTOP_DIR=\"/in$valid\"\n";
        assert_eq!(desktop(text), Some(PathBuf::from("/second")));
    }

    #[test]
    fn lines_end_where_python_splitlines_ends_them() {
        assert_eq!(desktop("XDG_DESKTOP_DIR=\"/a\u{1c}b\""), None);
        assert_eq!(desktop("XDG_DESKTOP_DIR=\"/a\u{2028}b\""), None);
        assert_eq!(
            desktop("# c\r\nXDG_DESKTOP_DIR=\"/a\"\r\n"),
            Some(PathBuf::from("/a"))
        );
        assert_eq!(
            desktop("\u{a0}XDG_DESKTOP_DIR=\"/nbsp\""),
            Some(PathBuf::from("/nbsp"))
        );
    }

    #[test]
    fn a_home_folder_that_is_not_utf8_is_kept_byte_for_byte() {
        let home = Path::new(OsStr::from_bytes(b"/home/caf\xe9"));
        let folders = parse("XDG_MUSIC_DIR=\"$HOME/Music\"", home);
        let music = folders.get(&KnownFolder::Music).expect("Music is configured");
        assert_eq!(music.as_os_str().as_bytes(), b"/home/caf\xe9/Music");
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("user-dirs.dirs");
        make_fifo(&fifo);
        let result = read(&fifo, Path::new(HOME));
        assert!(matches!(result, Err(UserDirsError::NotAFile)));
    }
}
