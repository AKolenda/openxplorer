// SPDX-License-Identifier: AGPL-3.0-only
//! The lock that elects one index owner among all `OpenXplorer` processes.
//!
//! Ports `owner_fd`, `elect` and `_release` in `desktop/index_service.py`
//! (SRCH-027). The Python app takes the same `flock`, so a Python and a
//! Rust process never both crawl.

use std::fs::File;
use std::mem;
use std::path::Path;

use super::error::SearchError;
use crate::private_storage::{private_file, PrivateFileOptions};

/// The lock file inside the cache directory.
const LOCK_FILE: &str = "index-owner.lock";

/// Whether this process owns the index.
#[derive(Debug)]
pub(super) enum Ownership {
    /// Another process owns the index; the lock file stays open to try
    /// again later.
    Candidate(File),
    /// This process holds the exclusive lock.
    Owner {
        /// Never read: it stays open because closing it releases the lock.
        _lock_file: File,
    },
    /// The service closed and gave the lock up for good.
    Released,
}

impl Ownership {
    /// Opens the lock file in `directory` without taking the lock.
    ///
    /// # Errors
    ///
    /// The private-storage errors for the lock file, for example when it is
    /// a symlink.
    pub(super) fn open(directory: &Path) -> Result<Self, SearchError> {
        let options = PrivateFileOptions {
            create: true,
            writable: true,
            allow_unlinked: false,
        };
        let file = private_file(&directory.join(LOCK_FILE), options)?;
        Ok(Self::Candidate(file))
    }

    /// Whether this process owns the index.
    pub(super) fn is_owner(&self) -> bool {
        matches!(self, Self::Owner { .. })
    }

    /// Tries to take the lock without waiting; true when this call made
    /// this process the owner.
    pub(super) fn try_acquire(&mut self) -> bool {
        let Self::Candidate(file) = self else {
            return false;
        };
        // `File::try_lock` is `flock(fd, LOCK_EX | LOCK_NB)` on Linux, the
        // lock `elect` takes in Python. Another owner, or any other failure,
        // leaves this process a candidate that tries again on the next tick.
        if file.try_lock().is_err() {
            return false;
        }
        if let Self::Candidate(file) = mem::replace(self, Self::Released) {
            *self = Self::Owner { _lock_file: file };
        }
        true
    }

    /// Closes the lock file, which releases the lock, and stops trying.
    pub(super) fn release(&mut self) {
        *self = Self::Released;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SRCH-027
    #[test]
    fn only_one_candidate_becomes_owner_until_it_releases() {
        let directory = tempfile::tempdir().unwrap();
        let mut first = Ownership::open(directory.path()).unwrap();
        let mut second = Ownership::open(directory.path()).unwrap();

        assert!(first.try_acquire());
        assert!(!second.try_acquire());
        assert!(!first.try_acquire(), "an owner does not become owner again");

        first.release();

        assert!(!first.is_owner());
        assert!(second.try_acquire());
        assert!(second.is_owner());
    }
}
