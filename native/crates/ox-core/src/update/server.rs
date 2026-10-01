// SPDX-License-Identifier: AGPL-3.0-only
//! Fetching from the update server, and reading the release answer. Ports
//! `open_url` and the reading part of `Updater.check` in
//! `v2.0.0:desktop/updater.py`.

use std::io::Read;

use serde_json::Value;

use super::{TrustedUrl, UpdateError};
use crate::transfer::Cancellation;

/// The largest release answer read: 2 MiB.
const MAX_RELEASE_ANSWER: u64 = 2 * 1024 * 1024;

/// Where the updater fetches from: [`GitHubReleases`](super::GitHubReleases)
/// in the app, fixtures in tests.
pub trait ReleaseServer: Send + Sync {
    /// Opens `url` for reading, following redirects only to trusted
    /// addresses. The reader belongs to the calling thread.
    ///
    /// # Errors
    ///
    /// [`FetchError`] says why nothing could be read.
    fn open(&self, url: &TrustedUrl, cancel: &Cancellation) -> Result<Box<dyn Read>, FetchError>;
}

/// Why a fetch failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The server answered with this HTTP status, after redirects.
    #[error("HTTP {0}")]
    Status(u32),
    /// No connection, a timeout, or a broken transfer.
    #[error("the update server could not be reached")]
    Unreachable,
    /// A redirect led outside the trusted hosts.
    #[error("the update server redirected to an untrusted location")]
    Untrusted,
    /// The fetch was cancelled.
    #[error("the fetch was cancelled")]
    Cancelled,
}

impl FetchError {
    /// The user-facing error of a failed check, with the Python wording.
    pub(super) fn into_check_error(self) -> UpdateError {
        match self {
            Self::Status(status) => UpdateError::CheckRefused { status },
            Self::Unreachable => UpdateError::Unreachable,
            Self::Untrusted => UpdateError::UntrustedLocation,
            Self::Cancelled => UpdateError::Cancelled,
        }
    }

    /// The user-facing error of a failed installer download.
    pub(super) fn into_download_error(self) -> UpdateError {
        match self {
            Self::Status(status) => UpdateError::DownloadRefused { status },
            other => other.into_check_error(),
        }
    }

    /// A read that failed part-way: cancelled if the user cancelled,
    /// otherwise the connection broke.
    pub(super) fn interrupted(cancel: &Cancellation) -> Self {
        if cancel.is_cancelled() {
            Self::Cancelled
        } else {
            Self::Unreachable
        }
    }
}

/// Asks the "latest release" endpoint and parses its JSON answer.
///
/// Safety rule "bounded answers" (`Updater.check` in
/// `v2.0.0:desktop/updater.py`): at most [`MAX_RELEASE_ANSWER`] bytes are read,
/// and a longer answer is refused rather than cut.
///
/// # Errors
///
/// A failed fetch in the check's wording (for example
/// [`UpdateError::CheckRefused`]), [`UpdateError::ResponseTooLarge`] or
/// [`UpdateError::InvalidResponse`].
pub(super) fn read_release_answer(
    server: &dyn ReleaseServer,
    cancel: &Cancellation,
) -> Result<Value, UpdateError> {
    let url = TrustedUrl::latest_release();
    let body = server.open(&url, cancel).map_err(FetchError::into_check_error)?;
    let mut answer = Vec::new();
    body.take(MAX_RELEASE_ANSWER + 1)
        .read_to_end(&mut answer)
        .map_err(|_| FetchError::interrupted(cancel).into_check_error())?;
    if answer.len() as u64 > MAX_RELEASE_ANSWER {
        return Err(UpdateError::ResponseTooLarge);
    }
    serde_json::from_slice(&answer).map_err(|_| UpdateError::InvalidResponse)
}
