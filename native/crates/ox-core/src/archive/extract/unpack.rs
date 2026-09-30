// SPDX-License-Identifier: AGPL-3.0-only
//! Writing the planned members into the staging folder and publishing it.
//! Ports the loop of `ZipExtractor.extract` in
//! `desktop/zip_extraction.py`.

use std::collections::HashMap;
use std::ffi::OsStr;

use super::limits::WrittenBytes;
use super::plan::{ExtractionPlan, ExtractionSummary, PathKind, PlannedMember};
use super::staging::ExtractionStaging;
use super::ZipExtractor;
use crate::archive::source::OpenedArchive;
use crate::archive::ArchiveError;
use crate::transfer::{Cancellation, Node};

/// The size of the blocks data is copied in (64 KiB).
const BLOCK_BYTES: usize = 64 * 1024;
/// The fraction the progress bar stops at until the folder is published.
const LAST_FRACTION_BEFORE_PUBLISHING: f64 = 0.99;

/// An archive being read for extraction.
pub(super) type SourceArchive = OpenedArchive;

/// One extraction writing into its staging folder.
pub(super) struct Unpacking<'a> {
    extractor: &'a mut ZipExtractor,
    summary: ExtractionSummary,
    cancel: &'a Cancellation,
    written: WrittenBytes,
    files_done: usize,
}

impl<'a> Unpacking<'a> {
    /// An extraction of an archive `summary` describes, by `extractor`.
    pub(super) fn new(
        extractor: &'a mut ZipExtractor,
        summary: ExtractionSummary,
        cancel: &'a Cancellation,
    ) -> Self {
        Self {
            extractor,
            summary,
            cancel,
            written: WrittenBytes::default(),
            files_done: 0,
        }
    }

    /// Makes `staging` private, writes every planned member into it and
    /// publishes it as `target`.
    ///
    /// # Errors
    ///
    /// The first failure; the caller then removes `staging`.
    pub(super) fn run(
        &mut self,
        archive: &mut SourceArchive,
        plan: &ExtractionPlan,
        staging: &mut ExtractionStaging,
        target: &dyn Node,
    ) -> Result<(), ArchiveError> {
        staging.make_private()?;
        let mut folders = StagedFolders::new(staging.folder());
        for member in &plan.members {
            self.cancel.check()?;
            folders.create_folders(member, self.cancel)?;
            if member.kind == PathKind::File {
                let file = folders.file(&member.segments);
                self.write_file(archive, member.index, file.as_ref())?;
            }
        }
        self.cancel.check()?;
        staging.publish(target, self.cancel)
    }

    /// Writes the member at `index` into the new file `file`.
    ///
    /// # Errors
    ///
    /// Damaged data, a write the destination did not fully accept, a
    /// member that yields more or fewer bytes than it declares, or the
    /// backend's error.
    fn write_file(
        &mut self,
        archive: &mut SourceArchive,
        index: usize,
        file: &dyn Node,
    ) -> Result<(), ArchiveError> {
        let member = &archive.members()[index];
        let (name, declared_size) = (member.name.clone(), member.size);
        let mut source = archive.open_member(index)?;
        let mut output = self.extractor.output.create(file, self.cancel)?;
        let mut block = vec![0u8; BLOCK_BYTES];
        self.written.start_member();
        loop {
            self.cancel.check()?;
            let count = source.read_chunk(&mut block)?;
            if count == 0 {
                break;
            }
            self.written.add(count);
            self.extractor.limits.check_written(self.written, declared_size)?;
            let accepted = output.write_block(&block[..count], self.cancel)?;
            // ARC-013: a short write would leave a silently truncated file.
            if accepted != count {
                return Err(ArchiveError::IncompleteWrite);
            }
            self.report_file(&name);
        }
        output.close()?;
        // ARC-017: a member that ends early is damaged or forged.
        if self.written.member != declared_size {
            return Err(ArchiveError::TruncatedMember);
        }
        self.files_done += 1;
        Ok(())
    }

    /// Reports the file being extracted and the share of bytes written.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a progress bar needs far less precision than f64 keeps"
    )]
    fn report_file(&mut self, name: &str) {
        let file_number = self.files_done + 1;
        let file_count = self.summary.file_count;
        let label = format!("Extracting {name} · {file_number}/{file_count} files");
        let share = self.written.total as f64 / self.summary.unpacked_bytes.max(1) as f64;
        self.extractor
            .report(label, share.min(LAST_FRACTION_BEFORE_PUBLISHING));
    }
}

/// The folders created in the staging folder so far, by path.
struct StagedFolders<'a> {
    root: &'a dyn Node,
    /// Created folders, keyed by their segments joined with `/`.
    created: HashMap<String, Box<dyn Node>>,
}

impl<'a> StagedFolders<'a> {
    fn new(root: &'a dyn Node) -> Self {
        Self {
            root,
            created: HashMap::new(),
        }
    }

    /// ARC-019: creates the folders `member`'s path implies, and the member
    /// itself for a folder entry, unless an earlier member created them.
    ///
    /// # Errors
    ///
    /// The backend's error when a folder cannot be created.
    fn create_folders(&mut self, member: &PlannedMember, cancel: &Cancellation) -> Result<(), ArchiveError> {
        let segments = &member.segments;
        let folder_depth = match member.kind {
            PathKind::Folder => segments.len(),
            PathKind::File => segments.len() - 1,
        };
        for depth in 1..=folder_depth {
            let key = segments[..depth].join("/");
            if self.created.contains_key(&key) {
                continue;
            }
            let folder = self.child(&segments[..depth - 1], &segments[depth - 1]);
            folder.create_directory(Some(cancel))?;
            self.created.insert(key, folder);
        }
        Ok(())
    }

    /// The node of the file with path `segments`, whose folders exist.
    fn file(&self, segments: &[String]) -> Box<dyn Node> {
        let (name, folder) = segments
            .split_last()
            .expect("a planned member path has at least one segment");
        self.child(folder, name)
    }

    /// The item `name` in the created folder `folder_segments`, or in the
    /// staging folder itself when that is empty.
    fn child(&self, folder_segments: &[String], name: &str) -> Box<dyn Node> {
        let parent = if folder_segments.is_empty() {
            self.root
        } else {
            self.created
                .get(&folder_segments.join("/"))
                .expect("folders are created parents first")
                .as_ref()
        };
        // ARC-014: `name` is a checked, single path component.
        parent.child(OsStr::new(name))
    }
}
