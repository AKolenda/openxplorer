// SPDX-License-Identifier: AGPL-3.0-only
//! Checks that run before anything is changed: self/descendant transfers,
//! protected (read-only) locations anywhere in an affected tree, and the
//! nesting depth limit.
//!
//! Ports `guard_destination` and `TransferEngine._check_write_tree` in
//! `desktop/operations.py`.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use super::error::TransferError;
use super::names::child_node;
use super::node::{Cancellation, Node, NodeKind, WriteGuard};
use crate::location::split_location;

/// The deepest folder nesting the engine walks. Deeper trees are refused
/// before anything is changed (preflight) or while copying, so a runaway
/// tree (or a backend that reports a loop) cannot recurse without bound.
pub const MAX_DEPTH: usize = 128;

/// The error for a tree deeper than [`MAX_DEPTH`].
pub(crate) fn nesting_error() -> TransferError {
    TransferError::failed(format!(
        "Folder nesting exceeds this build’s safety limit ({MAX_DEPTH})."
    ))
}

/// Rejects placing a folder inside itself or one of its descendants.
///
/// Local paths are compared after resolving symbolic links, so an alias of
/// a descendant is caught. URIs on the same host are compared textually
/// (case-insensitively for SMB, which is conservative). Aliases that cannot
/// be proven identical, such as two host names for one server, are caught
/// during the copy by the staging-name check in the copier.
///
/// # Errors
///
/// A refusal when `directory` is `source` or inside it, or when either URI
/// is not a valid location (as `split_location` raises in Python).
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
    let source_parts = split_location(&source.uri())?;
    let directory_parts = split_location(&directory.uri())?;
    let same_host = source_parts.scheme == directory_parts.scheme
        && source_parts.netloc.to_lowercase() == directory_parts.netloc.to_lowercase();
    if !same_host {
        return Ok(());
    }
    let case = PathCase::of_scheme(&source_parts.scheme);
    let source_path = comparable_path(&source_parts.path, case);
    let directory_path = comparable_path(&directory_parts.path, case);
    let inside = directory_path == source_path || directory_path.starts_with(&format!("{source_path}/"));
    if inside {
        return Err(TransferError::failed("Cannot place a folder inside itself."));
    }
    Ok(())
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
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// What an operation does to its source tree, which decides whether the
/// write guard is asked about the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChange {
    /// The source stays as it is (copy).
    Kept,
    /// The source is moved, renamed, trashed or deleted.
    Changed,
}

/// Preflight for one top-level item: asks `guard` about every affected URI
/// before anything changes, so a protected descendant (for example a
/// `.snapshot` folder deep inside a selected folder) stops the whole item.
///
/// The source tree is checked when `source_change` is
/// [`SourceChange::Changed`]; `destination` is checked in parallel with the
/// source tree (copy, move). Nothing is followed through symbolic links and
/// nothing is modified. Without a guard this does nothing, exactly like the
/// Python engine. The Python app also runs it before a rename (`rename_item`
/// in `desktop/gio_backend.py`).
///
/// # Errors
///
/// The guard's refusal for the first protected URI, the nesting limit,
/// [`TransferError::Cancelled`], or a failure to inspect or list the tree.
pub fn check_write_tree(
    guard: Option<&WriteGuard>,
    source: &dyn Node,
    destination: Option<&dyn Node>,
    cancel: &Cancellation,
    source_change: SourceChange,
) -> Result<(), TransferError> {
    let Some(guard) = guard else {
        return Ok(());
    };
    let check = TreeCheck {
        guard,
        cancel,
        source_change,
    };
    check.check_tree(source, destination, 0)
}

/// One preflight walk.
struct TreeCheck<'a> {
    guard: &'a WriteGuard,
    cancel: &'a Cancellation,
    source_change: SourceChange,
}

impl TreeCheck<'_> {
    /// Checks `source` (at nesting `depth`), its counterpart in the
    /// destination, and everything below them.
    fn check_tree(
        &self,
        source: &dyn Node,
        destination: Option<&dyn Node>,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(nesting_error());
        }
        if self.source_change == SourceChange::Changed {
            (self.guard)(&source.uri())?;
        }
        if let Some(destination) = destination {
            (self.guard)(&destination.uri())?;
        }
        // Inspected without following links: a link to a protected folder is
        // checked as the link itself, and its target is never walked.
        if source.info(Some(self.cancel))?.kind != NodeKind::Directory {
            return Ok(());
        }
        for child in source.children(Some(self.cancel))? {
            let child_destination = match destination {
                Some(folder) => Some(child_node(folder, child.name())?),
                None => None,
            };
            self.check_tree(child.as_ref(), child_destination.as_deref(), depth + 1)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_error_names_the_limit() {
        assert_eq!(
            nesting_error(),
            TransferError::failed("Folder nesting exceeds this build’s safety limit (128).")
        );
    }

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
