// SPDX-License-Identifier: AGPL-3.0-only
//! Compressing items into a new ZIP (ARC-023).
//!
//! New in the native app, from the Dolphin baseline and Explorer's
//! "Compress to ZIP file"; the Python app could only extract. The safety
//! rules mirror the extractor's:
//!
//! - The new ZIP never replaces anything: a taken name stops before
//!   anything is written, and the finished archive is published by a
//!   rename that never replaces an item another program created meanwhile
//!   ([`Node::publish`]).
//! - It is written under a hidden, random staging name and removed on any
//!   failure or cancellation, so no half-written ZIP is left under the
//!   final name.
//! - Links are never followed and special files never opened; both are
//!   left out and counted. Sources are only read.
//! - The write guard is asked about the destination folder and the new
//!   ZIP, so nothing is written into a previous version (PROP-024).
//!
//! A name ending in `.tar.xz` gives an XZ-compressed TAR instead of a ZIP,
//! with the same rules (Dolphin's "Compress to…" formats).
//!
//! | Module | Responsibility |
//! |---|---|
//! | `zip_writer` | The ZIP format: headers, deflated data, the central directory |
//! | `tar_writer` | The `.tar.xz` format: ustar headers in an XZ stream |

mod tar_writer;
mod zip_writer;

use std::ffi::OsStr;
use std::fmt;

use gio::prelude::*;

use super::worker::on_worker;
use super::ArchiveError;
use crate::gio_node::GioNode;
use crate::location::{is_smb_server, normalise, validate_name};
use crate::random::{random_hex, NAME_BYTES};
use crate::transfer::{Cancellation, Node, NodeKind, Progress, TransferError, WriteGuard};
use tar_writer::TarXzWriter;
use zip_writer::{DosTime, ZipWriter, MAX_ENTRIES};

/// The ending that asks for a `.tar.xz` instead of a ZIP.
const TAR_XZ_ENDING: &str = ".tar.xz";

/// Staging files are `.openxplorer-compress-<32 hex digits>.part`.
const STAGING_PREFIX: &str = ".openxplorer-compress-";
const STAGING_SUFFIX: &str = ".part";

/// What a walk reads about each item.
const WALK_ATTRIBUTES: &str = "standard::name,standard::type,standard::size,time::modified,unix::mode";

/// What to compress, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionRequest {
    /// The items to put in the ZIP, each at its top.
    pub uris: Vec<String>,
    /// The folder the new ZIP goes in.
    pub destination_uri: String,
    /// The new ZIP's name; nothing may have it yet.
    pub archive_name: String,
}

/// A finished compression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedArchive {
    /// The new ZIP.
    pub uri: String,
    /// Its name.
    pub name: String,
    /// The files and folders it holds.
    pub item_count: usize,
    /// Links and special files left out.
    pub skipped_count: usize,
}

/// One entry of the new ZIP.
#[derive(Debug, Clone)]
struct PlannedEntry {
    /// The item to read; `None` for a folder.
    source: Option<gio::File>,
    /// The entry's name: `Photos/`, `Photos/a.jpg`.
    name: String,
    /// When the item was last modified, in seconds since the Unix epoch.
    modified: Option<u64>,
    /// The file's size, for progress.
    size: u64,
    /// The item's `rwx` bits, which a TAR keeps; a ZIP gets fixed ones.
    mode: Option<u32>,
}

/// Everything a compression will write.
#[derive(Debug, Default)]
struct CompressionPlan {
    entries: Vec<PlannedEntry>,
    skipped_count: usize,
    total_bytes: u64,
}

/// The checked destination of a compression.
struct Destination {
    /// The folder the new ZIP goes in.
    folder: Box<dyn Node>,
    /// The new ZIP, which does not exist yet.
    archive: Box<dyn Node>,
}

/// Compresses items into new ZIP archives.
pub struct ZipCompressor {
    /// Receives progress on the worker thread.
    emit: Box<dyn FnMut(Progress) + Send>,
    write_guard: Option<Box<WriteGuard>>,
}

impl fmt::Debug for ZipCompressor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZipCompressor")
            .field("has_write_guard", &self.write_guard.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for ZipCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl ZipCompressor {
    /// A compressor without progress reports or a write guard.
    pub fn new() -> Self {
        Self {
            emit: Box::new(|_| {}),
            write_guard: None,
        }
    }

    /// Receives progress; it is called on the thread the compression runs
    /// on.
    #[must_use]
    pub fn with_progress(mut self, emit: impl FnMut(Progress) + Send + 'static) -> Self {
        self.emit = Box::new(emit);
        self
    }

    /// Rejects writes into protected locations such as snapshot folders.
    #[must_use]
    pub fn with_write_guard(
        mut self,
        guard: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.write_guard = Some(Box::new(guard));
        self
    }

    /// Compresses the request's items into a new ZIP. Blocking; see
    /// [`Self::compress_in_background`].
    ///
    /// # Errors
    ///
    /// [`ArchiveError::NotARealFolder`] for a destination that is not a
    /// folder, [`ArchiveError::ArchiveExists`] for a taken name,
    /// [`ArchiveError::NothingToCompress`],
    /// [`ArchiveError::DuplicateSelectedNames`],
    /// [`ArchiveError::TooLargeToCompress`], the write guard's refusal,
    /// the errors of reading the items or writing the ZIP, or
    /// [`ArchiveError::Cancelled`]. Nothing is left behind on failure.
    pub fn compress(
        &mut self,
        request: &CompressionRequest,
        cancel: &Cancellation,
    ) -> Result<CreatedArchive, ArchiveError> {
        let destination_uri = normalise(&request.destination_uri)?;
        let name = validate_name(&request.archive_name)?.to_owned();
        let destination = self.prepare_destination(&destination_uri, &name, cancel)?;
        self.report(format!("Preparing {name}…"), 0.0);
        let plan = plan(&request.uris, cancel)?;
        let staging = destination.folder.child(OsStr::new(&staging_name()?));
        if let Err(error) = self.write_archive(&plan, staging.as_ref(), is_tar_xz(&name), cancel) {
            remove_staging(staging.as_ref());
            return Err(error.unless_cancelled(cancel));
        }
        // Safety rule ARC-023: the rename never replaces an item another
        // program created under the name meanwhile.
        if let Err(error) = staging.publish(destination.archive.as_ref(), Some(cancel)) {
            remove_staging(staging.as_ref());
            return Err(match error {
                TransferError::Exists(_) => ArchiveError::ArchiveExists,
                other => ArchiveError::from(other).unless_cancelled(cancel),
            });
        }
        self.report(format!("Compressed {name}"), 1.0);
        Ok(CreatedArchive {
            uri: destination.archive.uri(),
            name,
            item_count: plan.entries.len(),
            skipped_count: plan.skipped_count,
        })
    }

    /// [`Self::compress`] on a GIO worker thread, for the main loop to
    /// await.
    ///
    /// # Errors
    ///
    /// See [`Self::compress`].
    pub async fn compress_in_background(
        mut self,
        request: CompressionRequest,
        cancel: Cancellation,
    ) -> Result<CreatedArchive, ArchiveError> {
        on_worker(move || self.compress(&request, &cancel)).await
    }

    /// Checks the destination folder and the new name before anything is
    /// read or written.
    fn prepare_destination(
        &self,
        destination_uri: &str,
        name: &str,
        cancel: &Cancellation,
    ) -> Result<Destination, ArchiveError> {
        self.check_writable(destination_uri)?;
        let folder: Box<dyn Node> = Box::new(GioNode::new(destination_uri));
        let is_folder = folder.info(Some(cancel))?.kind == NodeKind::Directory;
        if is_smb_server(destination_uri) || !is_folder {
            return Err(ArchiveError::NotARealFolder);
        }
        let archive = folder.child(OsStr::new(name));
        self.check_writable(&archive.uri())?;
        // Safety rule ARC-023: an existing item, even a dangling link,
        // keeps its name.
        if archive.exists(Some(cancel)) {
            return Err(ArchiveError::ArchiveExists);
        }
        Ok(Destination { folder, archive })
    }

    /// Writes every planned entry into a new file at `staging`: a
    /// `.tar.xz` with `tar_xz`, else a ZIP.
    fn write_archive(
        &mut self,
        plan: &CompressionPlan,
        staging: &dyn Node,
        tar_xz: bool,
        cancel: &Cancellation,
    ) -> Result<(), ArchiveError> {
        let output = gio::File::for_uri(&staging.uri())
            .create(gio::FileCreateFlags::NONE, Some(cancel.cancellable()))?;
        let stream = output.clone().upcast::<gio::OutputStream>().into_write();
        let mut writer = if tar_xz {
            EntryWriter::TarXz(Box::new(TarXzWriter::new(stream)?))
        } else {
            EntryWriter::Zip(ZipWriter::new(stream))
        };
        let mut done_bytes = 0u64;
        for (index, entry) in plan.entries.iter().enumerate() {
            cancel.check()?;
            let label = format!(
                "Compressing {} ({}/{})",
                entry.name,
                index + 1,
                plan.entries.len()
            );
            self.report(label, fraction(done_bytes, plan.total_bytes));
            let Some(source) = &entry.source else {
                writer.add_folder(entry)?;
                continue;
            };
            let stream = source.read(Some(cancel.cancellable()))?;
            let mut content = stream.upcast::<gio::InputStream>().into_read();
            writer.add_file(entry, &mut content, cancel)?;
            done_bytes += entry.size;
        }
        writer.finish()?;
        // Closed without the cancellable, so the file is flushed and
        // released however the compression ends.
        output.close(gio::Cancellable::NONE)?;
        Ok(())
    }

    /// Asks the write guard, if there is one, about `uri`.
    fn check_writable(&self, uri: &str) -> Result<(), ArchiveError> {
        match &self.write_guard {
            Some(guard) => Ok(guard(uri)?),
            None => Ok(()),
        }
    }

    fn report(&mut self, label: String, fraction: f64) {
        (self.emit)(Progress { label, fraction });
    }
}

/// `done` of `total`, between 0 and 1.
#[expect(clippy::cast_precision_loss, reason = "a progress bar needs no exact bytes")]
fn fraction(done: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    done as f64 / total as f64
}

/// Every entry the items at `uris` give, folders before their contents.
fn plan(uris: &[String], cancel: &Cancellation) -> Result<CompressionPlan, ArchiveError> {
    let mut plan = CompressionPlan::default();
    let mut top_names: Vec<String> = Vec::new();
    for uri in uris {
        let file = gio::File::for_uri(&normalise(uri)?);
        let Some(name) = file.basename().and_then(|name| name.to_str().map(str::to_owned)) else {
            plan.skipped_count += 1;
            continue;
        };
        if top_names.contains(&name) {
            return Err(ArchiveError::DuplicateSelectedNames);
        }
        top_names.push(name.clone());
        add_item(&mut plan, &file, name, cancel)?;
    }
    if plan.entries.is_empty() {
        return Err(ArchiveError::NothingToCompress);
    }
    Ok(plan)
}

/// Adds `file`, named `name` in the ZIP, and everything inside it.
fn add_item(
    plan: &mut CompressionPlan,
    file: &gio::File,
    name: String,
    cancel: &Cancellation,
) -> Result<(), ArchiveError> {
    let info = file.query_info(
        WALK_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    let modified = modified_seconds(&info);
    let mode = info
        .has_attribute("unix::mode")
        .then(|| info.attribute_uint32("unix::mode") & 0o777);
    match info.file_type() {
        gio::FileType::Regular => {
            let size = u64::try_from(info.size()).unwrap_or(0);
            plan.total_bytes += size;
            push_entry(plan, Some(file.clone()), name, (modified, mode), size)
        }
        gio::FileType::Directory => {
            push_entry(plan, None, format!("{name}/"), (modified, mode), 0)?;
            add_folder_contents(plan, file, &name, cancel)
        }
        // Links are never followed and special files never opened.
        _ => {
            plan.skipped_count += 1;
            Ok(())
        }
    }
}

/// Adds the items inside the folder `folder`, named `name` in the ZIP.
fn add_folder_contents(
    plan: &mut CompressionPlan,
    folder: &gio::File,
    name: &str,
    cancel: &Cancellation,
) -> Result<(), ArchiveError> {
    let children = folder.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    while let Some(info) = children.next_file(Some(cancel.cancellable()))? {
        cancel.check()?;
        let child = children.child(&info);
        let Some(child_name) = info.name().to_str().map(str::to_owned) else {
            plan.skipped_count += 1;
            continue;
        };
        add_item(plan, &child, format!("{name}/{child_name}"), cancel)?;
    }
    Ok(())
}

/// Adds one entry, within the entry limit of a ZIP without ZIP64.
fn push_entry(
    plan: &mut CompressionPlan,
    source: Option<gio::File>,
    name: String,
    (modified, mode): (Option<u64>, Option<u32>),
    size: u64,
) -> Result<(), ArchiveError> {
    if plan.entries.len() >= MAX_ENTRIES {
        return Err(ArchiveError::TooLargeToCompress);
    }
    plan.entries.push(PlannedEntry {
        source,
        name,
        modified,
        size,
        mode,
    });
    Ok(())
}

/// The format a compression writes.
enum EntryWriter<W: std::io::Write> {
    Zip(ZipWriter<W>),
    TarXz(Box<TarXzWriter<W>>),
}

impl<W: std::io::Write> EntryWriter<W> {
    fn add_folder(&mut self, entry: &PlannedEntry) -> Result<(), ArchiveError> {
        match self {
            Self::Zip(writer) => writer.add_folder(&entry.name, DosTime::from_unix_seconds(entry.modified)),
            Self::TarXz(writer) => writer.add_folder(
                &entry.name,
                entry.modified.unwrap_or(0),
                entry.mode.unwrap_or(0o755),
            ),
        }
    }

    fn add_file(
        &mut self,
        entry: &PlannedEntry,
        content: &mut dyn std::io::Read,
        cancel: &Cancellation,
    ) -> Result<(), ArchiveError> {
        match self {
            Self::Zip(writer) => {
                let modified = DosTime::from_unix_seconds(entry.modified);
                writer.add_file(&entry.name, modified, content, cancel, &mut |_| {})
            }
            Self::TarXz(writer) => writer.add_file(
                &entry.name,
                (entry.modified.unwrap_or(0), entry.mode.unwrap_or(0o644)),
                entry.size,
                content,
                cancel,
            ),
        }
    }

    fn finish(self) -> Result<W, ArchiveError> {
        match self {
            Self::Zip(writer) => writer.finish(),
            Self::TarXz(writer) => writer.finish(),
        }
    }
}

/// Whether `name` asks for a `.tar.xz`.
fn is_tar_xz(name: &str) -> bool {
    name.len() >= TAR_XZ_ENDING.len()
        && name
            .get(name.len() - TAR_XZ_ENDING.len()..)
            .is_some_and(|ending| ending.eq_ignore_ascii_case(TAR_XZ_ENDING))
}

/// `time::modified`, in seconds since the Unix epoch.
fn modified_seconds(info: &gio::FileInfo) -> Option<u64> {
    info.has_attribute("time::modified")
        .then(|| info.attribute_uint64("time::modified"))
}

/// A new staging name: hidden, random and unpredictable, like the
/// extractor's.
fn staging_name() -> Result<String, ArchiveError> {
    let digits = random_hex(NAME_BYTES).map_err(|error| {
        TransferError::failed(format!("Could not reserve a private staging name. {error}"))
    })?;
    Ok(format!("{STAGING_PREFIX}{digits}{STAGING_SUFFIX}"))
}

/// Removes a staging file this compression created. The first error is
/// the one the user needs; the hidden leftover names itself.
fn remove_staging(staging: &dyn Node) {
    let _ = staging.delete();
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::archive::{ArchiveBrowser, GioArchiveOpener};
    use crate::location::file_uri;

    /// A folder `Photos` with two files and a link, and a file beside it.
    fn sources(root: &Path) -> Vec<String> {
        let photos = root.join("Photos");
        fs::create_dir(&photos).unwrap();
        fs::write(photos.join("a.jpg"), b"first picture").unwrap();
        fs::write(photos.join("b.jpg"), b"second picture").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", photos.join("link")).unwrap();
        fs::write(root.join("notes.txt"), b"notes").unwrap();
        vec![file_uri(&photos), file_uri(&root.join("notes.txt"))]
    }

    /// Compress `uris` into `name` in `root`.
    fn request(root: &Path, uris: Vec<String>, name: &str) -> CompressionRequest {
        CompressionRequest {
            uris,
            destination_uri: file_uri(root),
            archive_name: name.to_owned(),
        }
    }

    /// The names in `root`.
    fn names_in(root: &Path) -> Vec<String> {
        let items = fs::read_dir(root).unwrap().filter_map(Result::ok);
        items
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// parity: ARC-023
    #[test]
    fn items_are_compressed_into_a_new_zip_without_following_links() {
        let root = tempfile::tempdir().unwrap();
        let uris = sources(root.path());
        let cancel = Cancellation::new();

        let created = ZipCompressor::new()
            .compress(&request(root.path(), uris, "Photos.zip"), &cancel)
            .unwrap();

        assert_eq!(created.name, "Photos.zip");
        assert_eq!(created.item_count, 4);
        assert_eq!(created.skipped_count, 1);
        let browser = ArchiveBrowser::new(Arc::new(GioArchiveOpener), root.path().join("previews"));
        let top = browser.list(&created.uri, "", &cancel).unwrap();
        let top_names: Vec<&str> = top.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(top_names, ["Photos", "notes.txt"]);
        let inside = browser.list(&created.uri, "Photos/", &cancel).unwrap();
        let inside_names: Vec<&str> = inside.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(inside_names, ["a.jpg", "b.jpg"]);
        let is_staging = |name: &String| name.starts_with(STAGING_PREFIX);
        assert!(!names_in(root.path()).iter().any(is_staging));

        // A `.tar.xz` name writes an XZ-compressed TAR with the same items.
        let long_name = format!("{}.jpg", "n".repeat(120));
        fs::write(root.path().join("Photos").join(&long_name), b"long").unwrap();
        for (name, mode) in [("a.jpg", 0o750), ("b.jpg", 0o640)] {
            let permissions = std::os::unix::fs::PermissionsExt::from_mode(mode);
            fs::set_permissions(root.path().join("Photos").join(name), permissions).unwrap();
        }
        let uris = vec![file_uri(&root.path().join("Photos"))];
        let tar = ZipCompressor::new()
            .compress(&request(root.path(), uris, "Photos.tar.xz"), &cancel)
            .unwrap();
        let inside = browser.list(&tar.uri, "Photos/", &cancel).unwrap();
        let inside_names: Vec<&str> = inside.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(inside_names, ["a.jpg", "b.jpg", long_name.as_str()]);
        let copy = browser.preview_member(&tar.uri, "Photos/b.jpg", &cancel).unwrap();
        assert_eq!(fs::read(copy.path).unwrap(), b"second picture");
        // A TAR keeps each item's permissions, so a program stays runnable.
        let archive = super::super::tar::TarArchive::open(
            Box::new(fs::File::open(root.path().join("Photos.tar.xz")).unwrap()),
            super::super::tar::TarCompression::Xz,
            100,
            &cancel,
        )
        .unwrap();
        let mode_of = |name: &str| {
            let member = archive
                .members()
                .iter()
                .find(|member| member.name == name)
                .unwrap();
            (member.external_attributes >> 16) & 0o777
        };
        assert_eq!(mode_of("Photos/a.jpg"), 0o750);
        assert_eq!(mode_of("Photos/b.jpg"), 0o640);
    }

    /// parity: ARC-023
    #[test]
    fn an_existing_name_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let uris = sources(root.path());
        fs::write(root.path().join("Photos.zip"), b"keep me").unwrap();

        let refused =
            ZipCompressor::new().compress(&request(root.path(), uris, "Photos.zip"), &Cancellation::new());

        assert_eq!(refused, Err(ArchiveError::ArchiveExists));
        assert_eq!(fs::read(root.path().join("Photos.zip")).unwrap(), b"keep me");
    }

    /// parity: ARC-023, PROP-024
    #[test]
    fn the_write_guard_keeps_the_zip_out_of_protected_folders() {
        let root = tempfile::tempdir().unwrap();
        let uris = sources(root.path());
        let refuse = |_: &str| Err(TransferError::failed("Previous-version locations are read-only."));
        let mut compressor = ZipCompressor::new().with_write_guard(refuse);

        let refused = compressor.compress(&request(root.path(), uris, "Photos.zip"), &Cancellation::new());

        assert!(refused.is_err());
        assert!(!root.path().join("Photos.zip").exists());
    }

    /// parity: ARC-023
    #[test]
    fn a_cancelled_compression_leaves_nothing_behind() {
        let root = tempfile::tempdir().unwrap();
        let uris = sources(root.path());
        let cancel = Cancellation::new();
        cancel.cancel();

        let refused = ZipCompressor::new().compress(&request(root.path(), uris, "Photos.zip"), &cancel);

        assert!(refused.is_err());
        let is_output = |name: &String| {
            let is_zip = Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "zip");
            is_zip || name.starts_with(STAGING_PREFIX)
        };
        assert!(!names_in(root.path()).iter().any(is_output));
    }
}
