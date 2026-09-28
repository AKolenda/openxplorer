// SPDX-License-Identifier: AGPL-3.0-only
//! Text rules shared by the cache and the service: folding for matching,
//! the path shown for a location, and URI containment.
//!
//! Ports `fold`, `display_path` and `below` from `desktop/search_index.py`
//! and the parent rule both of its write paths use.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::location::{split_location, unquote_lossy, LocationKind};

/// Folds text so that matching ignores case and compatibility forms:
/// NFKC normalisation, then Unicode case folding (Python's
/// `unicodedata.normalize('NFKC', value).casefold()`).
///
/// `GLib` does both, and its result equals Python's for every character
/// except the Cherokee capitals, which [`to_cherokee_capital`] corrects.
/// The two apps share the cache, so a name the Python app folded must
/// match a query folded here.
pub(crate) fn fold(text: &str) -> String {
    let composed = glib::normalize(text, glib::NormalizeMode::AllCompose);
    let folded = glib::casefold(composed);
    folded.chars().map(to_cherokee_capital).collect()
}

/// The Cherokee capital a small Cherokee letter folds to; other characters
/// are returned unchanged.
///
/// Unicode case folding maps Cherokee small letters to their capitals, as
/// Python does. `GLib` folds the capitals to small letters instead, so a
/// capital and its small letter would not even match each other after
/// `GLib` alone; mapping every small letter back gives Python's result.
fn to_cherokee_capital(character: char) -> char {
    let code = u32::from(character);
    let capital = match code {
        0xAB70..=0xABBF => code - 0xAB70 + 0x13A0,
        0x13F8..=0x13FD => code - 0x13F8 + 0x13F0,
        _ => return character,
    };
    char::from_u32(capital).unwrap_or(character)
}

/// How a location is shown in search results and as a root's default
/// label: `\\server\share\folder` for SMB, the decoded path otherwise
/// (`display_path` in `search_index.py`).
pub fn display_path(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return unquote_lossy(uri);
    };
    let path = unquote_lossy(&parts.path);
    if parts.kind() != LocationKind::Smb {
        return path;
    }
    let windows_path = path.replace('/', "\\");
    format!("\\\\{}{windows_path}", parts.authority)
}

/// Whether `uri` is `root` or inside it, compared as canonical URIs
/// (`below` in `search_index.py`).
pub(crate) fn is_at_or_below(uri: &str, root: &str) -> bool {
    if uri == root {
        return true;
    }
    let prefix = folder_prefix(root);
    uri.starts_with(&prefix)
}

/// `folder` with exactly one trailing slash, so that a prefix match
/// cannot take `/data2` for a child of `/data`.
pub(crate) fn folder_prefix(folder: &str) -> String {
    format!("{}/", folder.trim_end_matches('/'))
}

/// The folder that holds `uri`: everything before its last `/`, with
/// `file://` corrected to the root `file:///`.
pub(crate) fn parent_uri(uri: &str) -> String {
    let parent = uri.rsplit_once('/').map_or(uri, |(parent, _)| parent);
    if parent == "file://" {
        return "file:///".to_owned();
    }
    parent.to_owned()
}

/// Whether two folder URIs name the same folder, ignoring trailing slashes.
pub(crate) fn is_same_folder(first: &str, second: &str) -> bool {
    first.trim_end_matches('/') == second.trim_end_matches('/')
}

/// At most `limit` characters of `text`, as Python's `text[:limit]`.
pub(crate) fn truncate_chars(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// The decoded local path of a `file://` URI; `None` for other schemes.
pub(crate) fn local_path(uri: &str) -> Option<PathBuf> {
    let parts = split_location(uri).ok()?;
    if parts.kind() != LocationKind::Local {
        return None;
    }
    let file = gio::File::for_uri(uri);
    gio::prelude::FileExt::path(&file)
}

/// The host of a network location, lower-cased; `None` for local folders.
pub(crate) fn host_of(uri: &str) -> Option<String> {
    split_location(uri).ok()?.hostname()
}

/// Seconds since the Unix epoch with a fraction, as Python's `time.time()`,
/// for the timestamps the database shares with the Python app.
pub(crate) fn unix_now() -> f64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    since_epoch.as_secs_f64()
}

/// A stored timestamp as whole seconds since the Unix epoch; `None` for 0
/// ("never"), negative or invalid values. The format module shows times
/// in whole seconds.
pub(crate) fn whole_seconds(timestamp: f64) -> Option<u64> {
    if !timestamp.is_finite() || timestamp < 1.0 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is finite and at least 1, and dropping the fraction is intended"
    )]
    let seconds = timestamp.trunc() as u64;
    Some(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A case of [`fold`] and what Python's
    /// `unicodedata.normalize('NFKC', text).casefold()` returns for it.
    struct FoldCase {
        text: &'static str,
        python: &'static str,
    }

    /// parity: SRCH-008
    #[test]
    fn folding_matches_python() {
        let cases = [
            FoldCase {
                text: "Straße",
                python: "strasse",
            },
            FoldCase {
                text: "ﬁle Ⅻ",
                python: "file xii",
            },
            FoldCase {
                text: "ＦＵＬＬ",
                python: "full",
            },
            FoldCase {
                text: "ΣΑΣ",
                python: "σασ",
            },
            FoldCase {
                text: "İstanbul",
                python: "i\u{307}stanbul",
            },
            FoldCase {
                text: "\u{1c4}",
                python: "d\u{17e}",
            },
            FoldCase {
                text: "ẞ ﬆ",
                python: "ss st",
            },
            FoldCase {
                text: "\u{2126} \u{212a} \u{212b}",
                python: "ω k å",
            },
            FoldCase {
                text: "\u{13a0}\u{ab70} \u{13f0}\u{13f8}",
                python: "\u{13a0}\u{13a0} \u{13f0}\u{13f0}",
            },
        ];
        for case in cases {
            assert_eq!(fold(case.text), case.python, "folding {}", case.text);
        }
    }

    #[test]
    fn smb_locations_show_as_unc_paths() {
        assert_eq!(
            display_path("smb://nas/share/Team%20files"),
            "\\\\nas\\share\\Team files"
        );
        assert_eq!(
            display_path("file:///home/demo/My%20files"),
            "/home/demo/My files"
        );
    }

    #[test]
    fn containment_does_not_confuse_similar_prefixes() {
        assert!(is_at_or_below("file:///data", "file:///data"));
        assert!(is_at_or_below("file:///data/a", "file:///data/"));
        assert!(!is_at_or_below("file:///data2/a", "file:///data"));
        assert!(is_at_or_below("file:///etc", "file:///"));
    }

    #[test]
    fn the_parent_of_a_top_level_item_is_the_file_system_root() {
        assert_eq!(parent_uri("file:///etc"), "file:///");
        assert_eq!(parent_uri("smb://nas/share/a.txt"), "smb://nas/share");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("ééé", 2), "éé");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }

    #[test]
    fn timestamps_before_one_second_mean_never() {
        assert_eq!(whole_seconds(0.0), None);
        assert_eq!(whole_seconds(f64::NAN), None);
        assert_eq!(whole_seconds(1_700_000_000.75), Some(1_700_000_000));
    }
}
