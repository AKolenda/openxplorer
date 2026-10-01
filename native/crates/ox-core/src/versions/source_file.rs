// SPDX-License-Identifier: AGPL-3.0-only
//! The saved snapshot sources: `snapshot-sources.json` in the settings
//! directory, shared with the Python app.
//!
//! Ports `PreviousVersions.sources` and the saving half of
//! `PreviousVersions.configure` in `v2.0.0:desktop/previous_versions.py`, built on
//! the checks and the atomic replace in `crate::private_storage`.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use rustix::fs::OFlags;

use super::{SnapshotSource, VersionsError};
use crate::private_storage::{
    private_directory, read_limited_text, replace_file_atomically, KernelOpenFlags, StorageError,
    StorageRefusal, WithPath,
};

/// The most sources that are read or saved.
pub const MAX_SOURCES: usize = 64;

/// The name of the sources file inside the settings directory.
const FILE_NAME: &str = "snapshot-sources.json";

/// The start of the temporary file a save writes before renaming it.
const TEMPORARY_PREFIX: &str = ".versions-";

/// The largest sources file that is read. 64 sources of the longest
/// locations stay far below it.
const READ_LIMIT: u64 = 4 * 1024 * 1024;

/// The sources file of one settings directory.
#[derive(Debug)]
pub(crate) struct SourceFile {
    /// The settings directory holding the file.
    directory: PathBuf,
    /// `snapshot-sources.json` inside `directory`.
    path: PathBuf,
    /// Serialises the read-change-write of [`SourceFile::update`] between
    /// the threads of this app, like the `RLock` of `PreviousVersions`.
    update_lock: Mutex<()>,
}

impl SourceFile {
    /// The sources file in `settings_directory`.
    pub(crate) fn new(settings_directory: &Path) -> Self {
        Self {
            directory: settings_directory.to_path_buf(),
            path: settings_directory.join(FILE_NAME),
            update_lock: Mutex::new(()),
        }
    }

    /// The saved sources. A missing, unreadable or malformed file holds no
    /// sources, and invalid entries are dropped, as in the Python app.
    /// Only the first [`MAX_SOURCES`] entries of the file are considered.
    pub(crate) fn read(&self) -> Vec<SnapshotSource> {
        let Ok(text) = read_saved_text(&self.path) else {
            return Vec::new();
        };
        let Ok(serde_json::Value::Array(saved)) = serde_json::from_str(&text) else {
            return Vec::new();
        };
        saved
            .iter()
            .take(MAX_SOURCES)
            .filter_map(SnapshotSource::from_saved)
            .collect()
    }

    /// Reads the sources, lets `change` edit them, saves the result and
    /// returns it. Nothing is saved when `change` fails.
    ///
    /// # Errors
    ///
    /// The error of `change`, and [`VersionsError::Refused`] or
    /// [`VersionsError::Io`] when the file cannot be saved.
    pub(crate) fn update(
        &self,
        change: impl FnOnce(&mut Vec<SnapshotSource>) -> Result<(), VersionsError>,
    ) -> Result<Vec<SnapshotSource>, VersionsError> {
        // The guarded data is `()`, so a panic in another update cannot
        // have left anything half-changed.
        let _serialised = self.update_lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut sources = self.read();
        change(&mut sources)?;
        self.save(&sources)?;
        Ok(sources)
    }

    /// Saves `sources` atomically in a private file, so readers see the
    /// old or the new sources, never a mix. Unlike `settings.json`, an
    /// existing sources file is replaced whatever it is, even a link, as
    /// Python's `os.replace` does; the link's target is never written.
    fn save(&self, sources: &[SnapshotSource]) -> Result<(), VersionsError> {
        let contents = serde_json::to_vec(sources)
            .expect("sources are strings and layout names, which always serialise");
        private_directory(&self.directory)?;
        replace_file_atomically(&self.path, TEMPORARY_PREFIX, &contents)?;
        Ok(())
    }
}

/// Reads the sources file as text, following a symlinked file as Python's
/// `read_text` does (a dotfile manager may link it).
///
/// Safety rule "reading settings never hangs": `O_NONBLOCK` keeps a FIFO
/// put in place of the file from blocking, and anything but a regular file
/// is refused before it is read. The size is bounded by [`READ_LIMIT`].
fn read_saved_text(path: &Path) -> Result<String, StorageError> {
    let file = OpenOptions::new()
        .read(true)
        .kernel_flags(OFlags::NONBLOCK)
        .open(path)
        .with_path(path)?;
    if !file.metadata().with_path(path)?.is_file() {
        return Err(StorageError::refused(path, StorageRefusal::NotPrivateFile));
    }
    read_limited_text(file, path, READ_LIMIT)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::test_support::{make_fifo, permission_bits};
    use crate::versions::SnapshotLayout;

    /// A source that maps `/srv/<name>` to `/srv/history/<name>`.
    fn source(name: &str) -> SnapshotSource {
        let live = format!("/srv/{name}");
        let collection = format!("/srv/history/{name}");
        SnapshotSource::new(&live, &collection, SnapshotLayout::Direct).expect("a valid source")
    }

    /// The names in `directory`, sorted.
    fn names_in(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .expect("list the directory")
            .map(|entry| {
                entry
                    .expect("read an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// parity: PROP-023
    #[test]
    fn saving_makes_a_private_file_in_a_private_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("winspace");
        let file = SourceFile::new(&directory);

        let saved = file.update(|sources| {
            sources.push(source("data"));
            Ok(())
        });

        assert_eq!(saved.expect("saved"), [source("data")]);
        assert_eq!(permission_bits(&directory), 0o700);
        assert_eq!(permission_bits(&directory.join(FILE_NAME)), 0o600);
        assert_eq!(names_in(&directory), [FILE_NAME]);
        assert_eq!(file.read(), [source("data")]);
    }

    /// parity: PROP-023
    #[test]
    fn a_failed_save_leaves_no_temporary_file_and_keeps_the_old_one() {
        let directory = tempfile::tempdir().unwrap();
        let file = SourceFile::new(directory.path());
        // A directory in place of the file makes the final rename fail.
        fs::create_dir(directory.path().join(FILE_NAME)).unwrap();

        let saved = file.update(|sources| {
            sources.push(source("data"));
            Ok(())
        });

        assert!(matches!(saved, Err(VersionsError::Io { .. })), "{saved:?}");
        assert_eq!(names_in(directory.path()), [FILE_NAME]);
        assert!(directory.path().join(FILE_NAME).is_dir());
    }

    /// A dotfile manager may link the sources file. Saving replaces the
    /// link with a private file, as Python's `os.replace` does, and never
    /// writes to the file the link points to.
    ///
    /// parity: PROP-023
    #[test]
    fn a_linked_sources_file_is_replaced_not_written_through() {
        let directory = tempfile::tempdir().unwrap();
        let elsewhere = directory.path().join("dotfiles-sources.json");
        fs::write(&elsewhere, "[]").unwrap();
        symlink(&elsewhere, directory.path().join(FILE_NAME)).unwrap();
        let file = SourceFile::new(directory.path());

        let saved = file.update(|sources| {
            sources.push(source("data"));
            Ok(())
        });

        assert_eq!(saved.expect("saved"), [source("data")]);
        assert!(!directory.path().join(FILE_NAME).is_symlink());
        assert_eq!(permission_bits(&directory.path().join(FILE_NAME)), 0o600);
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "[]");
    }

    #[test]
    fn a_refused_change_saves_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let file = SourceFile::new(directory.path());

        let saved = file.update(|_| Err(VersionsError::TooManySources));

        assert!(matches!(saved, Err(VersionsError::TooManySources)));
        assert!(names_in(directory.path()).is_empty());
    }

    /// A FIFO in place of the sources file must not block the read that
    /// every protection check makes.
    ///
    /// parity: PROP-024
    #[test]
    fn a_fifo_in_place_of_the_file_holds_no_sources_and_does_not_block() {
        let directory = tempfile::tempdir().unwrap();
        make_fifo(&directory.path().join(FILE_NAME));

        assert_eq!(SourceFile::new(directory.path()).read(), []);
    }

    /// parity: PROP-023
    #[test]
    fn a_malformed_file_holds_no_sources() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        for contents in ["", "not json", "{\"live\": \"/srv\"}", "[1, 2", "\u{feff}[]"] {
            fs::write(&path, contents).unwrap();

            assert_eq!(SourceFile::new(directory.path()).read(), [], "{contents:?}");
        }
    }

    /// parity: PROP-023
    #[test]
    fn only_the_first_sixty_four_saved_entries_are_read() {
        let directory = tempfile::tempdir().unwrap();
        let saved: Vec<SnapshotSource> = (0..=MAX_SOURCES)
            .map(|number| source(&number.to_string()))
            .collect();
        let contents = serde_json::to_vec(&saved).unwrap();
        fs::write(directory.path().join(FILE_NAME), contents).unwrap();

        let sources = SourceFile::new(directory.path()).read();

        assert_eq!(sources, saved[..MAX_SOURCES]);
    }
}
