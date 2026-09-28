// SPDX-License-Identifier: AGPL-3.0-only
//! Shared by the `archive_*` integration tests: the extraction fixture of
//! `desktop/tests/test_zip_extract.py`, test doubles for the extraction
//! output, archive openers and the ZIP writer of `archive_zip_writer.rs`.
//! Include it with `#[path = "archive_support.rs"] mod support;`. Cargo
//! also builds this file as a test executable of its own, which has no
//! tests.
//!
//! Destinations go through the production [`GioNode`]; tests that need the
//! overridable local provider of `transfer_support/` include it themselves.
#![allow(
    dead_code,
    unused_imports,
    reason = "each test executable that includes this module uses a different part of it"
)]

#[path = "archive_zip_writer.rs"]
mod zip_writer;

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use gio::prelude::*;
use ox_core::archive::{
    ArchiveError, ArchiveOpener, ArchiveStream, ExtractedFolder, ExtractionOutput, ExtractionRequest,
    GioArchiveOpener, OutputFile, ZipExtractor,
};
use ox_core::gio_node::GioNode;
use ox_core::transfer::{Cancellation, Node, NodeFactory, Progress, TransferError};

pub use zip_writer::{
    file_type, first_member_data_offset, write_archive_declaring_directory, zip_bytes, zip_bytes_with,
    ArchiveLayout, Compression, EndRecords, TestMember, ENCRYPTED_FLAG, UTF8_NAME_FLAG,
};

/// Writes an archive with one member `name` holding `data` at `path`
/// using Python's `zipfile` with the method `zipfile_method` (such as
/// `ZIP_LZMA`), the writer of the Python app's tests.
///
/// # Panics
///
/// When Python 3 is missing or fails.
pub fn write_zip_with_python(path: &Path, zipfile_method: &str, name: &str, data: &str) {
    let script = "import sys, zipfile\n\
                  with zipfile.ZipFile(sys.argv[1], 'w', getattr(zipfile, sys.argv[2])) as archive:\n\
                  \x20   archive.writestr(sys.argv[3], sys.argv[4])\n";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(path)
        .args([zipfile_method, name, data])
        .output()
        .expect("Python 3 is required to write reference archives");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `length` bytes that do not compress, from a fixed seed.
pub fn incompressible_bytes(length: usize) -> Vec<u8> {
    let mut state: u32 = 0x9e37_79b9;
    let mut bytes = Vec::with_capacity(length);
    for _ in 0..length {
        // xorshift32
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        bytes.push(state.to_le_bytes()[0]);
    }
    bytes
}

/// Opens every URI as `archive`, held in memory, like the `BytesIO`
/// openers of the Python tests.
pub fn memory_opener(archive: Vec<u8>) -> Arc<dyn ArchiveOpener> {
    let opener = move |_uri: &str, cancel: &Cancellation| -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        cancel.check()?;
        Ok(Box::new(std::io::Cursor::new(archive.clone())))
    };
    Arc::new(opener)
}

/// Opens archives with the production opener.
pub fn opener() -> Arc<GioArchiveOpener> {
    Arc::new(GioArchiveOpener)
}

/// Resolves every URI with the production [`GioNode`].
pub fn gio_factory() -> NodeFactory {
    Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>))
}

/// The `file://` URI of `path`, as GIO and [`GioNode`] write it.
pub fn file_uri(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}

/// The permission bits of `path`, without following a link.
///
/// # Panics
///
/// When `path` cannot be inspected.
pub fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("inspect the item")
        .permissions()
        .mode()
        & 0o7777
}

/// What an extraction double does when a block is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBehaviour {
    /// Writes every block, like `writer_open` in `test_zip_extract.py`.
    Complete,
    /// Writes two bytes of the first block, then fails with "Disk full".
    DiskFull,
    /// Accepts only the first byte of every block.
    Short,
}

/// How an extraction double creates files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedMode {
    /// Owner-only (`0600`), like `writer_open`.
    OwnerOnly,
    /// Whatever the umask gives: a device without Unix modes, like
    /// `device_writer` in `test_zip_extract.py`.
    Unchanged,
}

/// Writes extracted files to the local path behind each node, whatever
/// its URI scheme.
#[derive(Debug, Clone, Copy)]
pub struct LocalFileOutput {
    /// How new files are created.
    pub created_mode: CreatedMode,
    /// What happens when a block is written.
    pub behaviour: WriteBehaviour,
}

impl LocalFileOutput {
    /// `writer_open`: new owner-only files that take every block.
    pub fn owner_only() -> Self {
        Self {
            created_mode: CreatedMode::OwnerOnly,
            behaviour: WriteBehaviour::Complete,
        }
    }

    /// Owner-only files whose writes behave as `behaviour` says.
    pub fn failing(behaviour: WriteBehaviour) -> Self {
        Self {
            behaviour,
            ..Self::owner_only()
        }
    }
}

impl ExtractionOutput for LocalFileOutput {
    fn create(&self, file: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn OutputFile>, TransferError> {
        cancel.check()?;
        let path = file.path().expect("test destinations are local folders");
        let created = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        if self.created_mode == CreatedMode::OwnerOnly {
            created.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        Ok(Box::new(LocalOutputFile {
            file: created,
            behaviour: self.behaviour,
        }))
    }
}

/// A local file an extraction double writes.
struct LocalOutputFile {
    file: fs::File,
    behaviour: WriteBehaviour,
}

impl OutputFile for LocalOutputFile {
    fn write_block(&mut self, block: &[u8], _cancel: &Cancellation) -> Result<usize, TransferError> {
        match self.behaviour {
            WriteBehaviour::Complete => {
                self.file.write_all(block)?;
                Ok(block.len())
            }
            WriteBehaviour::DiskFull => {
                self.file.write_all(&block[..2])?;
                Err(TransferError::failed("Disk full"))
            }
            WriteBehaviour::Short => {
                self.file.write_all(&block[..1])?;
                Ok(1)
            }
        }
    }

    fn close(self: Box<Self>) -> Result<(), TransferError> {
        self.file.sync_all()?;
        Ok(())
    }
}

/// The temporary folders and recorded progress of one extraction test:
/// `setUp` of `ZipExtractTests` in `desktop/tests/test_zip_extract.py`.
pub struct ExtractionFixture {
    _temporary: tempfile::TempDir,
    /// The temporary folder holding everything else.
    pub root: PathBuf,
    /// `sample.zip`, the archive tests extract.
    pub archive: PathBuf,
    /// The empty folder extractions go into.
    pub destination: PathBuf,
    /// The user's cancellation.
    pub cancel: Cancellation,
    events: Arc<Mutex<Vec<Progress>>>,
}

impl ExtractionFixture {
    /// A fixture with an empty destination folder and no archive yet.
    ///
    /// # Panics
    ///
    /// When the temporary folders cannot be created.
    pub fn new() -> Self {
        let temporary = tempfile::tempdir().expect("create a temporary folder");
        let root = temporary.path().to_path_buf();
        let destination = root.join("destination");
        fs::create_dir(&destination).expect("create the destination folder");
        Self {
            archive: root.join("sample.zip"),
            root,
            destination,
            _temporary: temporary,
            cancel: Cancellation::new(),
            events: Arc::default(),
        }
    }

    /// `make_zip`: writes `members` as the archive.
    ///
    /// # Panics
    ///
    /// When the archive cannot be written.
    pub fn write_zip(&self, members: &[TestMember]) {
        fs::write(&self.archive, zip_bytes(members)).expect("write the archive");
    }

    /// `make_zip()` without items: a folder, a file in it and an empty
    /// file.
    pub fn write_sample_zip(&self) {
        self.write_zip(&[
            TestMember::folder("Docs/"),
            TestMember::file("Docs/Guide.txt", b"hello world"),
            TestMember::file("Zero.bin", b""),
        ]);
    }

    /// The archive's URI.
    pub fn archive_uri(&self) -> String {
        file_uri(&self.archive)
    }

    /// The destination folder's URI.
    pub fn destination_uri(&self) -> String {
        file_uri(&self.destination)
    }

    /// The extractor of `setUp`: local folders through the production
    /// [`GioNode`], owner-only output files and recorded progress.
    pub fn extractor(&self) -> ZipExtractor {
        self.extractor_with(gio_factory(), Arc::new(LocalFileOutput::owner_only()))
    }

    /// An extractor resolving destinations with `factory` and writing with
    /// `output`, recording progress.
    ///
    /// # Panics
    ///
    /// The extractor panics when a progress report panicked earlier.
    pub fn extractor_with(&self, factory: NodeFactory, output: Arc<dyn ExtractionOutput>) -> ZipExtractor {
        let events = Arc::clone(&self.events);
        ZipExtractor::new(opener(), factory, output).with_progress(move |progress| {
            events.lock().expect("progress events").push(progress);
        })
    }

    /// `run_extract`: extracts the archive into the destination folder as
    /// `name` with the extractor of `setUp`.
    ///
    /// # Errors
    ///
    /// The extraction's error.
    pub fn extract(&self, name: &str) -> Result<ExtractedFolder, ArchiveError> {
        self.extract_with(&mut self.extractor(), name)
    }

    /// `run_extract` with `extractor`.
    ///
    /// # Errors
    ///
    /// The extraction's error.
    pub fn extract_with(
        &self,
        extractor: &mut ZipExtractor,
        name: &str,
    ) -> Result<ExtractedFolder, ArchiveError> {
        let request = ExtractionRequest {
            archive_uri: self.archive_uri(),
            destination_uri: self.destination_uri(),
            folder_name: name.to_owned(),
        };
        extractor.extract(&request, &self.cancel)
    }

    /// The progress reported so far.
    ///
    /// # Panics
    ///
    /// When a progress report panicked while recording.
    pub fn events(&self) -> Vec<Progress> {
        self.events.lock().expect("progress events").clone()
    }

    /// The names in the destination folder.
    ///
    /// # Panics
    ///
    /// When the destination folder cannot be listed.
    pub fn destination_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.destination)
            .expect("list the destination")
            .map(|entry| {
                entry
                    .expect("a folder entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// `assert_no_output`: the destination folder is still empty.
    ///
    /// # Panics
    ///
    /// When it is not.
    pub fn assert_no_output(&self) {
        assert_eq!(self.destination_names(), Vec::<String>::new());
    }
}

impl Default for ExtractionFixture {
    fn default() -> Self {
        Self::new()
    }
}
