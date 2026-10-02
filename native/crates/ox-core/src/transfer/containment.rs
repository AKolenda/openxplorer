// SPDX-License-Identifier: AGPL-3.0-only
//! XFER-016: a folder is never copied or moved into itself or one of its
//! descendants. Ports `guard_destination` in `v2.0.0:desktop/operations.py`.
//!
//! Local paths are compared after resolving symbolic links, so an alias of a
//! descendant is caught. URIs on the same host are compared textually
//! (case-insensitively for SMB, which is conservative). Aliases that cannot
//! be proven identical, such as two host names for one server, are caught
//! during the copy by the staging-name check in `copy.rs`.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use super::error::TransferError;
use super::node::Node;
use crate::location::split_location;

/// Rejects placing the folder `source` inside itself or one of its
/// descendants.
///
/// # Errors
///
/// A refusal when `destination_folder` is `source` or inside it, or when
/// either URI is not a valid location (as `split_location` raises in
/// Python).
pub(crate) fn guard_destination(
    source: &dyn Node,
    destination_folder: &dyn Node,
) -> Result<(), TransferError> {
    if resolved_paths_nest(source, destination_folder) {
        return Err(TransferError::failed(crate::i18n::gettext(
            "Cannot place a folder inside itself (including through a symlink).",
        )));
    }
    if uris_nest(source, destination_folder)? {
        return Err(TransferError::failed(crate::i18n::gettext(
            "Cannot place a folder inside itself.",
        )));
    }
    Ok(())
}

/// True when both items have local paths and `destination_folder`, with
/// links resolved, is `source` or inside it.
fn resolved_paths_nest(source: &dyn Node, destination_folder: &dyn Node) -> bool {
    let (Some(source_path), Some(folder_path)) = (source.path(), destination_folder.path()) else {
        return false;
    };
    // A component-wise prefix: the same folder or a descendant.
    resolve_links(&folder_path).starts_with(resolve_links(&source_path))
}

/// True when `destination_folder`'s URI is `source`'s URI or below it, on
/// the same host.
///
/// # Errors
///
/// Either URI is not a valid location.
fn uris_nest(source: &dyn Node, destination_folder: &dyn Node) -> Result<bool, TransferError> {
    let source_parts = split_location(&source.uri())?;
    let folder_parts = split_location(&destination_folder.uri())?;
    let same_host = source_parts.scheme == folder_parts.scheme
        && source_parts.authority.to_lowercase() == folder_parts.authority.to_lowercase();
    if !same_host {
        return Ok(false);
    }
    let case = PathCase::of_scheme(&source_parts.scheme);
    let source_path = comparable_path(&source_parts.path, case);
    let folder_path = comparable_path(&folder_parts.path, case);
    let descendant_prefix = format!("{source_path}/");
    Ok(folder_path == source_path || folder_path.starts_with(&descendant_prefix))
}

/// Whether a location's paths tell upper and lower case apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathCase {
    /// Linux file systems and most backends.
    Sensitive,
    /// SMB shares, which are usually case-insensitive; comparing folded
    /// paths errs on the side of refusing.
    Insensitive,
}

impl PathCase {
    /// How paths of URIs with `scheme` compare.
    fn of_scheme(scheme: &str) -> Self {
        if scheme == "smb" {
            PathCase::Insensitive
        } else {
            PathCase::Sensitive
        }
    }
}

/// The decoded path without trailing slashes, case-folded where `case`
/// ignores case.
fn comparable_path(escaped: &str, case: PathCase) -> String {
    let decoded = percent_decode_str(escaped).decode_utf8_lossy();
    let trimmed = decoded.trim_end_matches('/');
    match case {
        PathCase::Insensitive => fold_case(trimmed),
        PathCase::Sensitive => trimmed.to_string(),
    }
}

/// Unicode case folding keeps SMB containment conservative for all scripts.
fn fold_case(text: &str) -> String {
    glib::casefold(text).to_string()
}

/// Resolves symbolic links like Python's non-strict `os.path.realpath`: the
/// longest existing ancestor is resolved and the rest is appended as given.
/// A path that cannot be resolved at all (for example a `GVfs` FUSE path
/// whose daemon is gone) is compared as given; the URI comparison still
/// applies.
fn resolve_links(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(resolved) = std::fs::canonicalize(existing) {
            return missing
                .iter()
                .rev()
                .fold(resolved, |joined, part| joined.join(part));
        }
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return path.to_path_buf();
        };
        missing.push(name.to_os_string());
        existing = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: XFER-016
    #[test]
    fn smb_paths_fold_like_python_casefold() {
        assert_eq!(fold_case("Straße/ΣΟΦΟΣ"), "strasse/σοφοσ");
        assert_eq!(fold_case("Oﬃce"), "office");
        assert_eq!(fold_case("ﬓ"), "մն");
        assert_eq!(
            comparable_path("/Share/A%20B/", PathCase::Insensitive),
            "/share/a b"
        );
        assert_eq!(
            comparable_path("/Share/A%20B/", PathCase::Sensitive),
            "/Share/A B"
        );
        assert_eq!(PathCase::of_scheme("smb"), PathCase::Insensitive);
        assert_eq!(PathCase::of_scheme("sftp"), PathCase::Sensitive);
    }

    /// parity: XFER-016
    #[test]
    fn missing_tail_is_appended_to_the_resolved_ancestor() {
        let root = std::env::temp_dir();
        let resolved_root = std::fs::canonicalize(&root).expect("the temp dir exists");
        let missing = root.join("ox-transfer-guard-missing").join("child");
        assert_eq!(
            resolve_links(&missing),
            resolved_root.join("ox-transfer-guard-missing").join("child")
        );
    }
}
