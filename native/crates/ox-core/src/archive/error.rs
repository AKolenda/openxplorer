// SPDX-License-Identifier: AGPL-3.0-only
//! Why an archive could not be listed, previewed, checked or extracted.
//!
//! Every message is the Python app's, word for word: the `ValueError`,
//! `FileExistsError`, `OSError` and `RuntimeError` texts of
//! `v2.0.0:desktop/archives.py`, `v2.0.0:desktop/zip_extraction.py` and the archive
//! branches of `dispatch` in `v2.0.0:desktop/winspace.py`.

use crate::location::LocationError;
use crate::transfer::{Cancellation, TransferError};

use super::zip::ZipFormatError;

/// An archive failure. `Display` is the user-facing message.
///
/// Most refusals end with "Nothing was extracted." or "Use an archive
/// manager.": the archive is never changed and a refused extraction leaves
/// nothing behind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveError {
    /// The user cancelled.
    #[error("{}", crate::i18n::gettext("Operation cancelled."))]
    Cancelled,
    /// The archive is damaged or uses a ZIP feature the reader does not
    /// support.
    #[error(transparent)]
    Format(#[from] ZipFormatError),
    /// Reading the archive or writing the destination failed, or the write
    /// guard refused a protected location. [`TransferError::NotMounted`]
    /// asks the caller to mount the location and try again, as the Python
    /// app's `mount_retry` does.
    #[error(transparent)]
    Backend(TransferError),
    /// A location or folder name that is not valid.
    #[error(transparent)]
    Location(#[from] LocationError),

    /// A TAR archive whose headers or compressed data are damaged, or a
    /// compressed file that holds no TAR.
    #[error(
        "{}",
        crate::i18n::gettext("This archive is damaged or is not a TAR archive. Use an archive manager.")
    )]
    DamagedArchive,

    // Reading: v2.0.0:desktop/archives.py and v2.0.0:desktop/native_opening.py.
    /// ARC-005: the central directory is over 32 MiB.
    #[error(
        "{}",
        crate::i18n::gettext("ZIP directory is too large for the built-in viewer. Use an archive manager.")
    )]
    DirectoryTooLarge,
    /// ARC-005: the archive has more than 100,000 members.
    #[error(
        "{}",
        crate::i18n::gettext("The archive has more than 100,000 members. Use an archive manager.")
    )]
    TooManyMembers,
    /// ARC-005: a TAR's member names add up to more than 32 MiB, the size
    /// of the largest ZIP directory the viewer reads.
    #[error(
        "{}",
        crate::i18n::gettext(
            "The archive's file names are too long for the built-in viewer. Use an archive manager."
        )
    )]
    TarNamesTooLarge,
    /// ARC-007: the share cannot seek, which reading a ZIP needs.
    #[error(
        "{}",
        crate::i18n::gettext(
            "This share does not support seekable ZIP reading. Mount it locally or use an archive manager."
        )
    )]
    NotSeekable,

    // Browsing and previewing: v2.0.0:desktop/archives.py.
    /// The folder to list is not a safe member path.
    #[error("{}", crate::i18n::gettext("Invalid archive folder."))]
    InvalidFolder,
    /// The member to open is a folder or not a safe member path.
    #[error("{}", crate::i18n::gettext("Choose a regular archive member."))]
    NotARegularMember,
    /// ARC-006: only documents, images and media are opened from a ZIP.
    #[error(
        "{}",
        crate::i18n::gettext(
            "This file type cannot be previewed safely. Use an archive manager to extract it intentionally."
        )
    )]
    UnsafePreviewType,
    /// The member to open is missing, or several members have its name.
    #[error(
        "{}",
        crate::i18n::gettext("This archive member is missing or duplicated. Use an archive manager.")
    )]
    MissingOrDuplicatedMember,
    /// The member to open is encrypted.
    #[error(
        "{}",
        crate::i18n::gettext("Encrypted ZIP members require an archive manager.")
    )]
    EncryptedMember,
    /// The member to open is a link or special file, has an altered name,
    /// is over 256 MiB or compressed more than 1,000 times.
    #[error(
        "{}",
        crate::i18n::gettext(
            "This archive member is a link, too large, or exceeds the decompression safety limit."
        )
    )]
    MemberNotPreviewable,
    /// The member yielded more than 256 MiB while being opened.
    #[error("{}", crate::i18n::gettext("Decompression safety limit reached."))]
    PreviewLimitReached,

    // Checking members before extraction: v2.0.0:desktop/zip_extraction.py.
    /// ARC-014: a member name is empty, over 4,096 characters, or hides a
    /// NUL or a different Unicode name.
    #[error(
        "{}",
        crate::i18n::gettext("The archive contains an invalid or overlong member name.")
    )]
    InvalidMemberName,
    /// ARC-014: a member path is absolute or has backslashes or control
    /// characters.
    #[error(
        "{}",
        crate::i18n::gettext("The archive contains an unsafe member path. Nothing was extracted.")
    )]
    UnsafeMemberPath,
    /// ARC-017: a member is more than 128 folders deep.
    #[error(
        "{}",
        crate::i18n::gettext("The archive's nesting exceeds the 128-level safety limit.")
    )]
    NestingTooDeep,
    /// ARC-014: a path segment is empty, `.`, `..`, has a colon, ends in a
    /// space or dot, or is over 255 bytes.
    #[error(
        "{}",
        crate::i18n::gettext(
            "The archive contains a path unsafe for local/SMB extraction. Nothing was extracted."
        )
    )]
    PathUnsafeForShares,
    /// ARC-014: a path segment is a Windows device name such as `CON`.
    #[error(
        "{}",
        crate::i18n::gettext(
            "The archive contains a reserved device filename. Use an archive manager to inspect it."
        )
    )]
    ReservedDeviceName,
    /// ARC-016: a member is a symbolic link, FIFO, device or socket.
    #[error("{}", crate::i18n::gettext("The archive contains a symbolic link or special file. Nothing was extracted. Use an archive manager."))]
    LinkOrSpecialFile,
    /// ARC-016: a member is encrypted.
    #[error(
        "{}",
        crate::i18n::gettext("Password-protected ZIPs need an external archive manager in this release.")
    )]
    PasswordProtected,
    /// ARC-016: a member uses a method other than stored, deflate, bzip2
    /// or LZMA.
    #[error(
        "{}",
        crate::i18n::gettext("This ZIP compression method needs an external archive manager.")
    )]
    UnsupportedCompression,
    /// ARC-016: a folder entry carries data.
    #[error(
        "{}",
        crate::i18n::gettext("The archive contains inconsistent size metadata.")
    )]
    InconsistentSizes,
    /// ARC-017: a member is over the per-file size or compression ratio
    /// limit.
    #[error(
        "{}",
        crate::i18n::gettext(
            "The archive exceeds the per-file decompression safety limit. Use an archive manager."
        )
    )]
    MemberTooLarge,
    /// ARC-017: the archive has more entries than the extractor accepts.
    #[error(
        "{}",
        crate::i18n::gettext("The archive has too many entries for the built-in extractor.")
    )]
    TooManyEntries,
    /// ARC-015: two members have the same name.
    #[error(
        "{}",
        crate::i18n::gettext("The archive contains duplicate filenames. Nothing was extracted.")
    )]
    DuplicateNames,
    /// ARC-015: a path is both a file and a folder, or two paths differ only
    /// in case or Unicode normalisation.
    #[error(
        "{}",
        crate::i18n::gettext("The archive has conflicting or case-ambiguous paths. Nothing was extracted.")
    )]
    AmbiguousPaths,
    /// ARC-017: the members and their implied folders are too many paths.
    #[error(
        "{}",
        crate::i18n::gettext("The archive has too many paths for the built-in extractor.")
    )]
    TooManyPaths,
    /// ARC-017: the members add up to more than the total size limit.
    #[error(
        "{}",
        crate::i18n::gettext("The archive exceeds the 20 GiB extraction limit. Use an archive manager.")
    )]
    ArchiveTooLarge,

    // Extracting: v2.0.0:desktop/zip_extraction.py and v2.0.0:desktop/winspace.py.
    /// The destination is an SMB server's list of shares.
    #[error(
        "{}",
        crate::i18n::gettext("Open a network share before choosing it as an extraction destination.")
    )]
    ServerListingDestination,
    /// ARC-012: the destination is a link or not a folder.
    #[error(
        "{}",
        crate::i18n::gettext("Choose a real destination folder, not a link or server listing.")
    )]
    NotARealFolder,
    /// ARC-012: the new folder's name is taken, by anything.
    #[error(
        "{}",
        crate::i18n::gettext(
            "The destination already exists. Choose a new folder name; existing files are never overwritten."
        )
    )]
    DestinationExists,
    /// ARC-017: a member yielded more than it declared or the limits allow.
    #[error(
        "{}",
        crate::i18n::gettext("The archive exceeded its declared size or the extraction safety limit.")
    )]
    SizeLimitExceeded,
    /// ARC-013: the destination accepted only part of a block.
    #[error(
        "{}",
        crate::i18n::gettext("The destination did not accept all extracted bytes.")
    )]
    IncompleteWrite,
    /// ARC-017: a member yielded less than it declared.
    #[error(
        "{}",
        crate::i18n::gettext("This archive member has a truncated size. Extraction stopped.")
    )]
    TruncatedMember,
    // Compressing: new in the native app (ARC-023).
    /// Compress was given nothing it can put in a ZIP.
    #[error("{}", crate::i18n::gettext("Select files or folders to compress."))]
    NothingToCompress,
    /// ARC-023: the new ZIP would need ZIP64, which the writer does not
    /// write.
    #[error(
        "{}",
        crate::i18n::gettext("ZIP files over 4 GiB or with more than 65,535 items need an archive manager.")
    )]
    TooLargeToCompress,
    /// ARC-023: two selected items have the same name, which one ZIP folder
    /// cannot hold.
    #[error(
        "{}",
        crate::i18n::gettext("Two selected items have the same name. Compress them from one folder.")
    )]
    DuplicateSelectedNames,
    /// ARC-023: an item already has the new ZIP's name; it is never
    /// replaced.
    #[error(
        "{}",
        crate::i18n::gettext("An item with this name already exists. Nothing was replaced.")
    )]
    ArchiveExists,
    /// ARC-023: a file held more or fewer bytes when it was written into
    /// a `.tar.xz` than when it was measured.
    #[error(
        "{}",
        crate::i18n::gettext("An item changed while it was compressed. Nothing was created.")
    )]
    SourceChanged,

    /// ARC-013: the extraction failed and its staging folder could not be
    /// removed either.
    #[error(
        "{}",
        crate::i18n::format_message(
            "{cause}\nIncomplete extraction remains at {staging_uri}. Inspect it before removing it. {cleanup}",
            &[
                ("cause", &cause.to_string()),
                ("staging_uri", staging_uri),
                ("cleanup", &cleanup.to_string()),
            ]
        )
    )]
    StagingLeftBehind {
        /// Why the extraction stopped.
        cause: Box<ArchiveError>,
        /// Where the staging folder is.
        staging_uri: String,
        /// Why it could not be removed.
        cleanup: TransferError,
    },
}

impl ArchiveError {
    /// True for a user cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, ArchiveError::Cancelled)
    }

    /// True when the archive or destination is on a share or device that
    /// must be mounted first; mount it and try again.
    pub fn is_not_mounted(&self) -> bool {
        matches!(self, ArchiveError::Backend(TransferError::NotMounted(_)))
    }

    /// This error, or [`ArchiveError::Cancelled`] once `cancel` was
    /// cancelled: GIO aborts the call in progress with an error of its own
    /// then, which is not what went wrong.
    pub(crate) fn unless_cancelled(self, cancel: &Cancellation) -> Self {
        if cancel.is_cancelled() {
            ArchiveError::Cancelled
        } else {
            self
        }
    }
}

/// A backend failure. The user's cancellation stays a cancellation.
impl From<TransferError> for ArchiveError {
    fn from(error: TransferError) -> Self {
        match error {
            TransferError::Cancelled => ArchiveError::Cancelled,
            other => ArchiveError::Backend(other),
        }
    }
}

/// A read or write failure with the system's message, sorted like the
/// transfer engine's (missing, name taken, other).
impl From<std::io::Error> for ArchiveError {
    fn from(error: std::io::Error) -> Self {
        ArchiveError::Backend(error.into())
    }
}

/// A GIO failure, keeping "not mounted" so the caller can mount and retry.
impl From<glib::Error> for ArchiveError {
    fn from(error: glib::Error) -> Self {
        ArchiveError::Backend(error.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_left_behind_staging_folder_is_reported_with_its_location() {
        let error = ArchiveError::StagingLeftBehind {
            cause: Box::new(ArchiveError::IncompleteWrite),
            staging_uri: "file:///tmp/.openxplorer-extract-1.part".to_owned(),
            cleanup: TransferError::failed("Permission denied"),
        };

        assert_eq!(
            error.to_string(),
            "The destination did not accept all extracted bytes.\nIncomplete extraction remains at \
             file:///tmp/.openxplorer-extract-1.part. Inspect it before removing it. Permission denied"
        );
    }

    #[test]
    fn backend_errors_keep_cancellation_and_not_mounted() {
        assert!(ArchiveError::from(TransferError::Cancelled).is_cancelled());
        let unmounted = glib::Error::new(gio::IOErrorEnum::NotMounted, "Not mounted");
        assert!(ArchiveError::from(unmounted).is_not_mounted());
    }

    #[test]
    fn a_failure_after_cancelling_is_the_cancellation() {
        let cancel = Cancellation::new();
        assert_eq!(
            ArchiveError::NotSeekable.unless_cancelled(&cancel),
            ArchiveError::NotSeekable
        );

        cancel.cancel();

        assert!(ArchiveError::NotSeekable.unless_cancelled(&cancel).is_cancelled());
    }
}
