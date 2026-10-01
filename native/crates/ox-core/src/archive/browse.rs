// SPDX-License-Identifier: AGPL-3.0-only
//! Browsing a ZIP as read-only folders without extracting it. Ports
//! `Archives.list` in `v2.0.0:desktop/archives.py`.
//!
//! A listing reads the central directory only; no member is decompressed
//! and nothing is written (ARC-003). Members with unsafe names, altered
//! names, links and special files are left out and counted (ARC-004).

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use super::member_names::{is_previewable, is_safe_member};
use super::preview::MAX_PREVIEW_BYTES;
use super::source::{open_archive, ArchiveOpener};
use super::worker::on_worker;
use super::zip::{MemberFileType, ZipMember};
use super::ArchiveError;
use crate::transfer::Cancellation;

/// ARC-003: a listing shows at most this many rows.
pub const MAX_LISTED_ENTRIES: usize = 5000;

/// Lists archives as folders and opens single members as private copies.
///
/// Cloning is cheap; clones share the opener.
#[derive(Clone)]
pub struct ArchiveBrowser {
    pub(super) opener: Arc<dyn ArchiveOpener>,
    /// Where opened members are copied to.
    pub(super) preview_root: PathBuf,
}

impl fmt::Debug for ArchiveBrowser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArchiveBrowser")
            .field("preview_root", &self.preview_root)
            .finish_non_exhaustive()
    }
}

/// What a row of the archive browser shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveEntryKind {
    /// A folder: a folder entry, or a folder implied by a member's path.
    Folder,
    /// A file.
    File {
        /// The uncompressed size in bytes.
        size: u64,
        /// The compressed size in bytes.
        compressed_size: u64,
    },
}

/// One row of an archive folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// The file or folder name.
    pub name: String,
    /// The member path to list or open: `Docs/` for a folder, `Docs/a.txt`
    /// for a file.
    pub member: String,
    /// A folder or a file with its sizes.
    pub kind: ArchiveEntryKind,
    /// Seconds since the Unix epoch, like [`Entry::modified`]; `None` for
    /// an impossible date, which the Python app sent as 0.
    ///
    /// [`Entry::modified`]: crate::entry::Entry::modified
    pub modified: Option<u64>,
    /// True when the member is encrypted.
    pub is_encrypted: bool,
    /// ARC-006: true for a file that can be opened as a private copy: not
    /// encrypted, at most 256 MiB, and a document, image or media file.
    pub can_open: bool,
}

/// One folder of an archive, read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveListing {
    /// The archive's URI.
    pub archive_uri: String,
    /// The folder listed: empty for the top, otherwise ending with `/`.
    pub prefix: String,
    /// Folders first, then files, each by name ignoring case.
    pub entries: Vec<ArchiveEntry>,
    /// ARC-004: how many members were hidden for unsafe or altered names,
    /// or for being links or special files.
    pub hidden_unsafe_count: usize,
    /// True when the listing stopped at [`MAX_LISTED_ENTRIES`] rows.
    pub is_truncated: bool,
}

impl ArchiveBrowser {
    /// A browser reading archives with `opener` and copying opened members
    /// into new private folders under `preview_root`, which is created when
    /// needed. The app uses [`default_preview_root`].
    pub fn new(opener: Arc<dyn ArchiveOpener>, preview_root: PathBuf) -> Self {
        Self { opener, preview_root }
    }

    /// Lists the folder `prefix` (empty for the top) of the archive at
    /// `uri`. Blocking; see [`Self::list_in_background`].
    ///
    /// # Errors
    ///
    /// [`ArchiveError::InvalidFolder`] for an unsafe `prefix`, the errors
    /// of reading the archive, or [`ArchiveError::Cancelled`].
    pub fn list(
        &self,
        uri: &str,
        prefix: &str,
        cancel: &Cancellation,
    ) -> Result<ArchiveListing, ArchiveError> {
        let prefix = folder_prefix(prefix)?;
        let archive = open_archive(self.opener.as_ref(), uri, cancel)?;
        let mut rows = ListingRows::new(prefix);
        for member in archive.members() {
            cancel.check()?;
            rows.add(member);
            if rows.is_full() {
                break;
            }
        }
        Ok(rows.into_listing(uri))
    }

    /// [`Self::list`] on a GIO worker thread, for the main loop to await.
    ///
    /// # Errors
    ///
    /// See [`Self::list`].
    pub async fn list_in_background(
        &self,
        uri: String,
        prefix: String,
        cancel: Cancellation,
    ) -> Result<ArchiveListing, ArchiveError> {
        let browser = self.clone();
        on_worker(move || browser.list(&uri, &prefix, &cancel)).await
    }
}

/// Where the app copies opened members: a folder in the user's private
/// runtime directory (`$XDG_RUNTIME_DIR/winspace-archive-previews`), which
/// the session removes at logout.
pub fn default_preview_root() -> PathBuf {
    glib::user_runtime_dir().join("winspace-archive-previews")
}

/// The folder to list as a member prefix: empty, or ending with one `/`.
fn folder_prefix(prefix: &str) -> Result<String, ArchiveError> {
    if prefix.is_empty() {
        return Ok(String::new());
    }
    if !is_safe_member(prefix) {
        return Err(ArchiveError::InvalidFolder);
    }
    Ok(format!("{}/", prefix.trim_end_matches('/')))
}

/// ARC-004: true for a member the browser may show: a safe name the
/// archive records unaltered, and a regular file, a folder or no type.
fn is_listable(member: &ZipMember) -> bool {
    is_safe_member(&member.name)
        && member.has_unaltered_name()
        && member.file_type() != MemberFileType::LinkOrSpecial
}

/// The rows of one folder, collected member by member.
#[derive(Debug)]
struct ListingRows {
    prefix: String,
    entries: Vec<ArchiveEntry>,
    /// The member paths already shown, so a folder appears once however
    /// many members are inside it.
    shown: HashSet<String>,
    hidden_unsafe_count: usize,
}

impl ListingRows {
    fn new(prefix: String) -> Self {
        Self {
            prefix,
            entries: Vec::new(),
            shown: HashSet::new(),
            hidden_unsafe_count: 0,
        }
    }

    /// Adds the row `member` shows in this folder, if any: itself, or the
    /// folder of this folder's that it is inside.
    fn add(&mut self, member: &ZipMember) {
        if !is_listable(member) {
            self.hidden_unsafe_count += 1;
            return;
        }
        let relative = match member.name.strip_prefix(self.prefix.as_str()) {
            Some(relative) if !relative.is_empty() => relative,
            _ => return,
        };
        let entry = entry_for(&self.prefix, relative, member);
        if self.shown.insert(entry.member.clone()) {
            self.entries.push(entry);
        }
    }

    fn is_full(&self) -> bool {
        self.entries.len() >= MAX_LISTED_ENTRIES
    }

    /// The listing, folders first and each group by case-folded name.
    fn into_listing(mut self, archive_uri: &str) -> ArchiveListing {
        let is_truncated = self.is_full();
        self.entries.sort_by_cached_key(|entry| {
            let is_file = matches!(entry.kind, ArchiveEntryKind::File { .. });
            (is_file, glib::casefold(&entry.name))
        });
        ArchiveListing {
            archive_uri: archive_uri.to_owned(),
            prefix: self.prefix,
            entries: self.entries,
            hidden_unsafe_count: self.hidden_unsafe_count,
            is_truncated,
        }
    }
}

/// The row for `member`, whose path below the listed folder is `relative`.
fn entry_for(prefix: &str, relative: &str, member: &ZipMember) -> ArchiveEntry {
    let trimmed = relative.trim_end_matches('/');
    let (name, is_inside_subfolder) = match trimmed.split_once('/') {
        Some((first, _rest)) => (first, true),
        None => (trimmed, false),
    };
    let is_folder = is_inside_subfolder || member.is_directory();
    let kind = if is_folder {
        ArchiveEntryKind::Folder
    } else {
        ArchiveEntryKind::File {
            size: member.size,
            compressed_size: member.compressed_size,
        }
    };
    // ARC-006: the same rules `preview_member` applies before opening.
    let is_openable_file = !is_folder && !member.is_encrypted() && member.size <= MAX_PREVIEW_BYTES;
    let folder_slash = if is_folder { "/" } else { "" };
    ArchiveEntry {
        name: name.to_owned(),
        member: format!("{prefix}{name}{folder_slash}"),
        kind,
        modified: member.modified.to_unix_seconds(),
        is_encrypted: member.is_encrypted(),
        can_open: is_openable_file && is_previewable(name),
    }
}
