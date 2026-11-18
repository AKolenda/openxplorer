// SPDX-License-Identifier: AGPL-3.0-only
//! The headers of the 7z archives browsed last, so that opening folder
//! after folder of one archive does not read and unpack its header each
//! time (ARC-003).
//!
//! An entry is found by the archive's location, its length and its first
//! 32 bytes, which hold the checksum of the last header: a changed archive
//! has a different header, so it is read again.

use std::sync::{Arc, Mutex, PoisonError};

use super::Parsed;

/// How many archives are kept.
const KEPT: usize = 4;

/// What identifies one version of an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Key {
    uri: String,
    length: u64,
    start: [u8; 32],
}

impl Key {
    pub(super) fn new(uri: &str, length: u64, start: [u8; 32]) -> Self {
        Self {
            uri: uri.to_owned(),
            length,
            start,
        }
    }
}

/// The archives kept, the one used last at the end.
static KEPT_HEADERS: Mutex<Vec<(Key, Arc<Parsed>)>> = Mutex::new(Vec::new());

/// The headers kept for `key`, if any.
pub(super) fn find(key: &Key) -> Option<Arc<Parsed>> {
    let mut kept = KEPT_HEADERS.lock().unwrap_or_else(PoisonError::into_inner);
    let position = kept.iter().position(|(candidate, _)| candidate == key)?;
    let entry = kept.remove(position);
    let parsed = Arc::clone(&entry.1);
    kept.push(entry);
    Some(parsed)
}

/// Keeps `parsed` for `key`, forgetting the archive used longest ago.
pub(super) fn keep(key: Key, parsed: Arc<Parsed>) {
    let mut kept = KEPT_HEADERS.lock().unwrap_or_else(PoisonError::into_inner);
    kept.retain(|(candidate, _)| candidate.uri != key.uri);
    if kept.len() >= KEPT {
        kept.remove(0);
    }
    kept.push((key, parsed));
}
