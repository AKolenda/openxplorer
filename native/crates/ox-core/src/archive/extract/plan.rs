// SPDX-License-Identifier: AGPL-3.0-only
//! Checking every member before anything is extracted. Ports
//! `member_parts` and `plan` of `v2.0.0:desktop/zip_extraction.py`.
//!
//! All of these rules are checked before the staging folder exists, so a
//! refused archive leaves nothing behind:
//!
//! - ARC-014: a member path is relative and every segment is a name that
//!   local disks, SMB shares and Windows can all store.
//! - ARC-015: no two members share a path, and no path is used as both a
//!   file and a folder or spelled differently in case or Unicode
//!   normalisation, so a case-insensitive share ends up with exactly the
//!   archive's contents.
//! - ARC-016: links, special files, encrypted members, exotic compression
//!   and folders carrying data are refused.
//! - ARC-017: the [`ExtractionLimits`] against ZIP bombs.
//!
//! Copying items out of a ZIP (ARC-026) checks only the members it
//! copies, with the same rules ([`plan_selected`]): a link or a broken
//! name elsewhere in the archive does not stop the copy, and one the
//! archive browser hides inside a copied folder is left out and counted.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use super::limits::ExtractionLimits;
use crate::archive::zip::{MemberFileType, ZipMember};
use crate::archive::ArchiveError;
use crate::transfer::Cancellation;

/// The longest member name, in characters.
const MAX_NAME_CHARS: usize = 4096;
/// The longest path segment, in UTF-8 bytes: the limit of Linux file
/// systems and SMB.
const MAX_SEGMENT_BYTES: usize = 255;

/// What an archive holds, as the Extract dialog summarises it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtractionSummary {
    /// The files to extract.
    pub file_count: usize,
    /// The folders to create, including those only implied by member
    /// paths.
    pub folder_count: usize,
    /// The uncompressed size of all files.
    pub unpacked_bytes: u64,
    /// The members of the archive, folder entries included.
    pub entry_count: usize,
    /// Members inside the items copied out of the archive that were left
    /// out because the archive browser hides them: links, special files
    /// and names that are not safe (ARC-026).
    pub left_out: usize,
}

/// Whether a path is a file or a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathKind {
    /// A file, written from the member's data.
    File,
    /// A folder: a folder entry, or a folder a member path implies.
    Folder,
}

impl PathKind {
    /// What `member` is: a folder entry, or a file.
    fn of(member: &ZipMember) -> Self {
        if member.is_directory() {
            PathKind::Folder
        } else {
            PathKind::File
        }
    }
}

/// One member that passed every check, with its path split into segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlannedMember {
    /// Its index in the archive's member list.
    pub(super) index: usize,
    /// Its path segments; each is a safe name (ARC-014).
    pub(super) segments: Vec<String>,
    /// A folder entry or a file.
    pub(super) kind: PathKind,
}

/// An archive that may be extracted: its members in archive order and its
/// summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExtractionPlan {
    pub(super) members: Vec<PlannedMember>,
    pub(super) summary: ExtractionSummary,
}

/// Checks every member of an archive and summarises it (`plan`).
///
/// # Errors
///
/// The first refusal of the rules in the module documentation, or
/// [`ArchiveError::Cancelled`].
pub(super) fn plan(
    members: &[ZipMember],
    limits: &ExtractionLimits,
    cancel: &Cancellation,
) -> Result<ExtractionPlan, ArchiveError> {
    if members.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries);
    }
    let mut paths = ArchivePaths::default();
    let mut planned = Vec::with_capacity(members.len());
    let mut summary = ExtractionSummary {
        entry_count: members.len(),
        ..ExtractionSummary::default()
    };
    for (index, member) in members.iter().enumerate() {
        cancel.check()?;
        let segments = member_segments(member, limits)?;
        let kind = PathKind::of(member);
        paths.add(&segments, kind)?;
        if paths.count() > limits.max_paths {
            return Err(ArchiveError::TooManyPaths);
        }
        if kind == PathKind::File {
            count_file(&mut summary, member.size, limits)?;
        }
        planned.push(PlannedMember {
            index,
            segments,
            kind,
        });
    }
    summary.folder_count = paths.folder_count();
    Ok(ExtractionPlan {
        members: planned,
        summary,
    })
}

/// Checks the members at or below one of `selected` (path segments, as
/// the archive browser names them) and summarises them, for copying items
/// out of the archive (ARC-026). Members elsewhere are not looked at; a
/// member inside the selection that the browser hides (a link, a special
/// file, an unsafe name) is left out and counted; every other selected
/// member must pass every rule of [`plan`], or the copy is refused.
///
/// # Errors
///
/// The first refusal of the rules in the module documentation for a
/// copied member, or [`ArchiveError::Cancelled`].
pub(super) fn plan_selected(
    members: &[ZipMember],
    selected: &[Vec<String>],
    limits: &ExtractionLimits,
    cancel: &Cancellation,
) -> Result<ExtractionPlan, ArchiveError> {
    if members.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries);
    }
    let mut paths = ArchivePaths::default();
    let mut planned = Vec::new();
    let mut summary = ExtractionSummary::default();
    for (index, member) in members.iter().enumerate() {
        cancel.check()?;
        if !is_selected(&member.name, selected) {
            continue;
        }
        if !crate::archive::browse::is_listable(member) {
            summary.left_out += 1;
            continue;
        }
        let segments = member_segments(member, limits)?;
        let kind = PathKind::of(member);
        paths.add(&segments, kind)?;
        if paths.count() > limits.max_paths {
            return Err(ArchiveError::TooManyPaths);
        }
        if kind == PathKind::File {
            count_file(&mut summary, member.size, limits)?;
        }
        planned.push(PlannedMember {
            index,
            segments,
            kind,
        });
    }
    summary.entry_count = planned.len();
    summary.folder_count = paths.folder_count();
    Ok(ExtractionPlan {
        members: planned,
        summary,
    })
}

/// Whether the member `name` is at or below one of `selected`, compared
/// segment by segment as the archive browser splits names.
fn is_selected(name: &str, selected: &[Vec<String>]) -> bool {
    let segments: Vec<&str> = name.trim_end_matches('/').split('/').collect();
    selected.iter().any(|chosen| {
        chosen.len() <= segments.len() && chosen.iter().zip(&segments).all(|(want, have)| want == have)
    })
}

/// ARC-017: adds a file of `size` bytes to the summary, refusing an archive
/// over the total size limit.
fn count_file(
    summary: &mut ExtractionSummary,
    size: u64,
    limits: &ExtractionLimits,
) -> Result<(), ArchiveError> {
    summary.file_count += 1;
    summary.unpacked_bytes = summary.unpacked_bytes.saturating_add(size);
    if summary.unpacked_bytes > limits.max_total_bytes {
        return Err(ArchiveError::ArchiveTooLarge);
    }
    Ok(())
}

/// The path segments of `member`, after checking its name, type and sizes
/// (`member_parts`).
///
/// # Errors
///
/// The refusal of the first rule `member` breaks.
pub(super) fn member_segments(
    member: &ZipMember,
    limits: &ExtractionLimits,
) -> Result<Vec<String>, ArchiveError> {
    check_name(member)?;
    let segments: Vec<String> = member
        .name
        .trim_end_matches('/')
        .split('/')
        .map(str::to_owned)
        .collect();
    // ARC-017: deep nesting exhausts path lengths and recursive tools.
    if segments.len() > limits.max_depth {
        return Err(ArchiveError::NestingTooDeep);
    }
    for segment in &segments {
        check_segment(segment)?;
    }
    check_type(member)?;
    check_size(member, limits)?;
    Ok(segments)
}

/// ARC-014: the member name as a whole.
fn check_name(member: &ZipMember) -> Result<(), ArchiveError> {
    let name = &member.name;
    // A NUL cuts the name Python's `zipfile` shows short of the name the
    // archive records: the member would be extracted under another name.
    let is_overlong = name.chars().count() > MAX_NAME_CHARS;
    if !member.has_unaltered_name() || name.is_empty() || is_overlong {
        return Err(ArchiveError::InvalidMemberName);
    }
    let has_control_character = name.chars().any(|character| character.is_ascii_control());
    // Absolute and Windows-style paths could escape the new folder.
    if name.starts_with('/') || name.contains('\\') || has_control_character {
        return Err(ArchiveError::UnsafeMemberPath);
    }
    Ok(())
}

/// ARC-014: one path segment must be a plain name that local disks and SMB
/// shares store unchanged.
fn check_segment(segment: &str) -> Result<(), ArchiveError> {
    // `..` and `.` would leave or re-enter a folder; an empty segment means
    // `//`. SMB and Windows drop trailing spaces and dots and read a colon
    // as a stream name.
    let is_relative = matches!(segment, "" | "." | "..");
    let is_altered_by_smb = segment.contains(':') || segment.ends_with([' ', '.']);
    if is_relative || is_altered_by_smb || segment.len() > MAX_SEGMENT_BYTES {
        return Err(ArchiveError::PathUnsafeForShares);
    }
    if crate::location::is_reserved_device_name(segment) {
        return Err(ArchiveError::ReservedDeviceName);
    }
    Ok(())
}

/// ARC-016: only plain files and folders, unencrypted, with a compression
/// method the reader supports.
fn check_type(member: &ZipMember) -> Result<(), ArchiveError> {
    let is_link_or_special = match member.file_type() {
        MemberFileType::Unrecorded | MemberFileType::Regular => false,
        MemberFileType::Directory => !member.is_directory(),
        MemberFileType::LinkOrSpecial => true,
    };
    if is_link_or_special {
        return Err(ArchiveError::LinkOrSpecialFile);
    }
    if member.is_encrypted() {
        return Err(ArchiveError::PasswordProtected);
    }
    if !member.method.is_supported() {
        return Err(ArchiveError::UnsupportedCompression);
    }
    Ok(())
}

/// ARC-016 and ARC-017: a folder carries no data, and a file is within the
/// per-member size and compression ratio limits.
fn check_size(member: &ZipMember, limits: &ExtractionLimits) -> Result<(), ArchiveError> {
    if member.is_directory() && member.size != 0 {
        return Err(ArchiveError::InconsistentSizes);
    }
    let ratio_limit = member.compressed_size.max(1).saturating_mul(limits.max_ratio);
    if member.size > limits.max_member_bytes || member.size > ratio_limit {
        return Err(ArchiveError::MemberTooLarge);
    }
    Ok(())
}

/// The position of a path in [`ArchivePaths::paths`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PathId(usize);

/// How a path is found: the folder it is in, and its last segment keyed
/// the way a case-insensitive share compares names.
#[derive(Debug, PartialEq, Eq, Hash)]
struct PathKey {
    /// The folder the path is in; `None` at the top of the archive.
    folder: Option<PathId>,
    /// The [`share_key`] of the path's last segment.
    segment_key: String,
}

/// What one path of the archive is, and how the archive spells its last
/// segment. The segments above it are spelled by its folders.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KnownPath {
    kind: PathKind,
    segment: String,
}

/// ARC-015: every path an extraction would create, keyed the way a
/// case-insensitive share compares names.
///
/// ARC-017: a path is stored once, as its last segment and a link to its
/// folder, so the memory this takes grows with the length of the member
/// names and not with their depth times their length. Python's `plan`
/// keeps one copy of each segment too: its path tuples share the segment
/// strings. Storing every folder's whole path instead would let a crafted
/// archive of deep, long names within every limit cost gigabytes just to
/// be checked.
#[derive(Debug, Default)]
struct ArchivePaths {
    /// The member paths themselves: their segments' share keys, joined by
    /// `/`, which no segment contains.
    members: HashSet<String>,
    /// Every path's position in `paths`.
    ids: HashMap<PathKey, PathId>,
    /// Every path, including the folders member paths imply.
    paths: Vec<KnownPath>,
}

impl ArchivePaths {
    /// Adds the member of `kind` with path `segments`.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::DuplicateNames`] for a member path that is already
    /// taken, and [`ArchiveError::AmbiguousPaths`] for a path that another
    /// member uses as the other kind or spells differently.
    fn add(&mut self, segments: &[String], kind: PathKind) -> Result<(), ArchiveError> {
        let keys: Vec<String> = segments.iter().map(|segment| share_key(segment)).collect();
        // The whole path is compared first, as Python does: a member
        // repeated in another case is a duplicate, not an ambiguous path.
        if !self.members.insert(keys.join("/")) {
            return Err(ArchiveError::DuplicateNames);
        }
        let mut folder = None;
        for (depth, (segment, segment_key)) in segments.iter().zip(keys).enumerate() {
            // Every path above the member itself is a folder.
            let is_member_itself = depth + 1 == segments.len();
            let path = KnownPath {
                kind: if is_member_itself { kind } else { PathKind::Folder },
                segment: segment.clone(),
            };
            let key = PathKey { folder, segment_key };
            folder = Some(self.record(key, path)?);
        }
        Ok(())
    }

    /// Records `path` under `key` and returns its id, refusing a different
    /// earlier record. The folders above `path` were compared before it, so
    /// comparing its last segment compares its whole spelling.
    fn record(&mut self, key: PathKey, path: KnownPath) -> Result<PathId, ArchiveError> {
        match self.ids.entry(key) {
            Entry::Occupied(known) => {
                let id = *known.get();
                if self.paths[id.0] == path {
                    Ok(id)
                } else {
                    Err(ArchiveError::AmbiguousPaths)
                }
            }
            Entry::Vacant(new) => {
                let id = PathId(self.paths.len());
                self.paths.push(path);
                new.insert(id);
                Ok(id)
            }
        }
    }

    /// The number of paths, implied folders included.
    fn count(&self) -> usize {
        self.paths.len()
    }

    /// The number of folders, implied ones included.
    fn folder_count(&self) -> usize {
        self.paths
            .iter()
            .filter(|path| path.kind == PathKind::Folder)
            .count()
    }
}

/// `segment` as a case-insensitive share compares it: composed Unicode
/// (NFC), then case-folded, as `v2.0.0:desktop/zip_extraction.py` does.
fn share_key(segment: &str) -> String {
    // `DefaultCompose` is NFC.
    let composed = glib::normalize(segment, glib::NormalizeMode::DefaultCompose);
    glib::casefold(composed).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path keys of `segments`.
    fn keys(segments: &[&str]) -> Vec<String> {
        segments.iter().map(|segment| share_key(segment)).collect()
    }

    #[test]
    fn share_keys_ignore_case_and_unicode_composition() {
        assert_eq!(keys(&["Café", "NOTE.txt"]), keys(&["cafe\u{301}", "note.TXT"]));
        // Full case folding, like Python's `str.casefold`.
        assert_eq!(keys(&["Straße", "ﬁle"]), keys(&["STRASSE", "FILE"]));
        // Compatibility forms stay distinct: the normalisation is NFC.
        assert_ne!(keys(&["x²"]), keys(&["x2"]));
        assert_ne!(keys(&["a"]), keys(&["b"]));
    }

    /// parity: ARC-014
    #[test]
    fn reserved_device_names_are_refused_with_any_extension_and_case() {
        for segment in ["CON", "nul.txt", "Com1.tar.gz", "lpt9"] {
            assert_eq!(
                check_segment(segment),
                Err(ArchiveError::ReservedDeviceName),
                "{segment}"
            );
        }
        for segment in ["console", "com10", "nulled.txt", "lpt"] {
            assert_eq!(check_segment(segment), Ok(()), "{segment}");
        }
    }

    /// `segments` as the owned segments of a member path.
    fn owned(segments: &[&str]) -> Vec<String> {
        segments.iter().map(|segment| (*segment).to_owned()).collect()
    }

    /// parity: ARC-015, ARC-019
    #[test]
    fn implied_folders_count_once_and_a_parent_after_its_child_agrees() {
        let mut paths = ArchivePaths::default();

        paths
            .add(&owned(&["a", "b.txt"]), PathKind::File)
            .expect("a new file");
        paths
            .add(&owned(&["a"]), PathKind::Folder)
            .expect("its folder, listed after it");

        assert_eq!((paths.count(), paths.folder_count()), (2, 1));
    }

    /// Python compares whole member paths before their folders, so a member
    /// repeated in another case is a duplicate even though its folder is
    /// spelled differently too.
    ///
    /// parity: ARC-015
    #[test]
    fn a_member_repeated_in_another_case_is_a_duplicate_not_an_ambiguous_folder() {
        let mut paths = ArchivePaths::default();
        paths
            .add(&owned(&["Docs", "a.txt"]), PathKind::File)
            .expect("a new file");

        let repeated = paths.add(&owned(&["docs", "A.TXT"]), PathKind::File);
        let beside_it = paths.add(&owned(&["docs", "b.txt"]), PathKind::File);

        assert_eq!(repeated, Err(ArchiveError::DuplicateNames));
        assert_eq!(beside_it, Err(ArchiveError::AmbiguousPaths));
    }

    /// ARC-017: a deep member costs one stored segment per level, not one
    /// stored path per level.
    ///
    /// parity: ARC-017
    #[test]
    fn a_deep_member_stores_each_segment_once() {
        let segments: Vec<String> = (0..128).map(|level| format!("{level:031}")).collect();
        let mut paths = ArchivePaths::default();

        paths.add(&segments, PathKind::File).expect("a new deep file");

        let stored_bytes: usize = paths.paths.iter().map(|path| path.segment.len()).sum();
        assert_eq!(stored_bytes, 128 * 31);
        assert_eq!((paths.count(), paths.folder_count()), (128, 127));
    }
}
