// SPDX-License-Identifier: AGPL-3.0-only
//! Checks that run before anything is changed: self/descendant transfers,
//! protected (read-only) locations anywhere in an affected tree, and the
//! nesting depth limit.
//!
//! Ports `guard_destination` and `TransferEngine._check_write_tree` in
//! `desktop/operations.py`, with the URI splitting of `split_location` in
//! `desktop/core.py`.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use super::error::TransferError;
use super::names::child_node;
use super::node::{Cancellation, Node, NodeKind, WriteGuard};

/// The deepest folder nesting the engine walks. Deeper trees are refused
/// before anything is changed (preflight) or while copying, so a runaway
/// tree (or a backend that reports a loop) cannot recurse without bound.
pub const MAX_DEPTH: usize = 128;

/// The error for a tree deeper than [`MAX_DEPTH`].
pub(crate) fn nesting_error() -> TransferError {
    TransferError::failed("Folder nesting exceeds this build’s safety limit (128).")
}

/// A URI split into the parts the guard compares.
#[derive(Debug, PartialEq, Eq)]
struct SplitUri {
    /// Lower-cased scheme.
    scheme: String,
    /// The authority (host, or a device identifier such as
    /// `[usb:001,002]`), exactly as written.
    authority: String,
    /// The still-escaped path, without query or fragment.
    path: String,
}

/// Splits `scheme://authority/path?query#fragment` like Python's `urlsplit`,
/// but also accepts GVfs device authorities in brackets, which `urlsplit`
/// rejects as malformed IPv6 addresses (see `split_location` in `core.py`).
fn split_location(uri: &str) -> SplitUri {
    let (scheme, rest) = uri.split_once(':').unwrap_or(("", uri));
    let (authority, remainder) = match rest.strip_prefix("//") {
        Some(after) => {
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            (&after[..end], &after[end..])
        }
        None => ("", rest),
    };
    let path_end = remainder.find(['?', '#']).unwrap_or(remainder.len());
    SplitUri {
        scheme: scheme.to_ascii_lowercase(),
        authority: authority.to_string(),
        path: remainder[..path_end].to_string(),
    }
}

/// Rejects placing a folder inside itself or one of its descendants.
///
/// Local paths are compared after resolving symbolic links, so an alias of
/// a descendant is caught. URIs on the same host are compared textually
/// (case-insensitively for SMB, which is conservative). Aliases that cannot
/// be proven identical, such as two host names for one server, are caught
/// during the copy by the staging-name check in the copier.
pub fn guard_destination(source: &dyn Node, directory: &dyn Node) -> Result<(), TransferError> {
    if let (Some(source_path), Some(directory_path)) = (source.path(), directory.path()) {
        let resolved_source = resolve_links(&source_path);
        let resolved_directory = resolve_links(&directory_path);
        // Component-wise prefix: the same folder or a descendant.
        if resolved_directory.starts_with(&resolved_source) {
            return Err(TransferError::failed(
                "Cannot place a folder inside itself (including through a symlink).",
            ));
        }
    }
    let source_uri = split_location(&source.uri());
    let directory_uri = split_location(&directory.uri());
    let same_host = source_uri.scheme == directory_uri.scheme
        && source_uri.authority.to_lowercase() == directory_uri.authority.to_lowercase();
    if !same_host {
        return Ok(());
    }
    let is_smb = source_uri.scheme == "smb";
    let source_path = comparable_path(&source_uri.path, is_smb);
    let directory_path = comparable_path(&directory_uri.path, is_smb);
    let inside = directory_path == source_path || directory_path.starts_with(&format!("{source_path}/"));
    if inside {
        return Err(TransferError::failed("Cannot place a folder inside itself."));
    }
    Ok(())
}

/// The decoded path without trailing slashes, case-folded for SMB.
fn comparable_path(escaped: &str, fold: bool) -> String {
    let decoded = percent_decode_str(escaped).decode_utf8_lossy();
    let trimmed = decoded.trim_end_matches('/');
    if fold {
        fold_case(trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Unicode case folding keeps SMB containment conservative for all scripts.
fn fold_case(text: &str) -> String {
    glib::casefold(text).to_string()
}

/// Resolves symbolic links like Python's non-strict `os.path.realpath`: the
/// longest existing ancestor is resolved and the rest is appended as given.
/// A path that cannot be resolved at all (for example a GVfs FUSE path whose
/// daemon is gone) is compared as given; the URI comparison still applies.
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
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Preflight for one top-level item: asks `guard` about every affected URI
/// before anything changes, so a protected descendant (for example a
/// `.snapshot` folder deep inside a selected folder) stops the whole item.
///
/// `source_writable` also checks the source tree (move, Trash, delete,
/// rename); `destination` is checked in parallel with the source tree (copy,
/// move). Nothing is followed through symbolic links and nothing is
/// modified. Without a guard this does nothing, exactly like the Python
/// engine. Also used by rename (`rename_item` in `desktop/gio_backend.py`).
pub fn check_write_tree(
    guard: Option<&WriteGuard>,
    source: &dyn Node,
    destination: Option<&dyn Node>,
    cancel: &Cancellation,
    source_writable: bool,
) -> Result<(), TransferError> {
    match guard {
        Some(guard) => check_tree(guard, source, destination, cancel, source_writable, 0),
        None => Ok(()),
    }
}

fn check_tree(
    guard: &WriteGuard,
    source: &dyn Node,
    destination: Option<&dyn Node>,
    cancel: &Cancellation,
    source_writable: bool,
    depth: usize,
) -> Result<(), TransferError> {
    cancel.check()?;
    if depth > MAX_DEPTH {
        return Err(nesting_error());
    }
    if source_writable {
        guard(&source.uri())?;
    }
    if let Some(destination) = destination {
        guard(&destination.uri())?;
    }
    // Inspected without following links: a link to a protected folder is
    // checked as the link itself, and its target is never walked.
    if source.info(Some(cancel))?.kind != NodeKind::Directory {
        return Ok(());
    }
    for child in source.children(Some(cancel))? {
        let child_destination = match destination {
            Some(folder) => Some(child_node(folder, child.name())?),
            None => None,
        };
        check_tree(
            guard,
            child.as_ref(),
            child_destination.as_deref(),
            cancel,
            source_writable,
            depth + 1,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_authorities_split_without_error() {
        let parts = split_location("mtp://[usb:001,002]/Internal%20storage/x?y#z");
        assert_eq!(parts.scheme, "mtp");
        assert_eq!(parts.authority, "[usb:001,002]");
        assert_eq!(parts.path, "/Internal%20storage/x");
        let local = split_location("file:///home/demo");
        assert_eq!(
            (local.authority.as_str(), local.path.as_str()),
            ("", "/home/demo")
        );
    }

    #[test]
    fn smb_paths_fold_like_python_casefold() {
        assert_eq!(fold_case("Straße/ΣΟΦΟΣ"), "strasse/σοφοσ");
        assert_eq!(fold_case("Oﬃce"), "office");
        assert_eq!(fold_case("ﬓ"), "մն");
        assert_eq!(comparable_path("/Share/A%20B/", true), "/share/a b");
        assert_eq!(comparable_path("/Share/A%20B/", false), "/Share/A B");
    }

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
