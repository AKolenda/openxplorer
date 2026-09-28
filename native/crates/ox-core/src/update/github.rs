// SPDX-License-Identifier: AGPL-3.0-only
//! The HTTPS client for GitHub, on libsoup. Ports `open_url` and
//! `TrustedRedirect` in `desktop/updater.py`.
//!
//! libsoup is GNOME's HTTP library: it follows the desktop's proxy
//! settings, checks certificates against the system's store through
//! glib-networking, and takes a [`gio::Cancellable`]. Each fetch makes its
//! own session on the worker thread that reads it, so nothing is shared
//! between threads.

use std::io::{self, Read};

use gio::prelude::*;
use soup::prelude::*;

use super::{FetchError, ReleaseServer, ReleaseVersion, TrustedUrl};
use crate::transfer::Cancellation;

/// Seconds a connection or read may stall before it fails, as Python's
/// `timeout=30`.
const NETWORK_TIMEOUT_SECONDS: u32 = 30;

/// How many redirects one fetch follows, as urllib's
/// `HTTPRedirectHandler.max_redirections`.
const MAX_REDIRECTS: usize = 10;

/// What the "latest release" endpoint is asked to answer.
const RELEASE_MEDIA_TYPE: &str = "application/vnd.github+json";

/// What a download is asked to answer.
const DOWNLOAD_MEDIA_TYPE: &str = "application/octet-stream";

/// HTTP statuses that redirect a GET, as urllib follows them.
const REDIRECT_STATUSES: [u32; 5] = [301, 302, 303, 307, 308];

/// Fetches from GitHub over HTTPS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubReleases {
    /// `OpenXplorer/<version>`, sent with every request.
    user_agent: String,
}

impl GitHubReleases {
    /// A client that introduces itself as `OpenXplorer/<app_version>`.
    pub fn new(app_version: ReleaseVersion) -> Self {
        Self {
            user_agent: format!("OpenXplorer/{app_version}"),
        }
    }

    /// A session with the user agent and the 30-second timeout.
    fn session(&self) -> soup::Session {
        soup::Session::builder()
            .user_agent(self.user_agent.as_str())
            .timeout(NETWORK_TIMEOUT_SECONDS)
            .build()
    }
}

impl ReleaseServer for GitHubReleases {
    /// Sends a GET for `url`. Redirects are followed here rather than by
    /// libsoup, so that every target passes [`TrustedUrl::redirect`]
    /// before it is contacted.
    fn open(&self, url: &TrustedUrl, cancel: &Cancellation) -> Result<Box<dyn Read>, FetchError> {
        let session = self.session();
        let media_type = accepted_media_type(url);
        let mut target = url.clone();
        let mut last_status = 0;
        for _ in 0..=MAX_REDIRECTS {
            let message = get_request(&target, media_type);
            let stream = session
                .send(&message, Some(cancel.cancellable()))
                .map_err(|_| FetchError::interrupted(cancel))?;
            let status = message.status_code();
            if !REDIRECT_STATUSES.contains(&status) {
                return response_body(status, stream, cancel);
            }
            let location = response_header(&message, "Location").ok_or(FetchError::Status(status))?;
            target = target.redirect(&location).map_err(|_| FetchError::Untrusted)?;
            last_status = status;
        }
        // urllib reports a redirect loop as an HTTP error with the last
        // redirect's status.
        Err(FetchError::Status(last_status))
    }
}

/// What to ask `url` to answer. Redirects keep the first request's
/// headers, as urllib's do.
fn accepted_media_type(url: &TrustedUrl) -> &'static str {
    if url.is_latest_release() {
        RELEASE_MEDIA_TYPE
    } else {
        DOWNLOAD_MEDIA_TYPE
    }
}

/// A GET for `url` that libsoup must not redirect by itself.
fn get_request(url: &TrustedUrl, media_type: &str) -> soup::Message {
    let message = soup::Message::from_uri("GET", url.as_uri());
    message.set_flags(soup::MessageFlags::NO_REDIRECT);
    if let Some(headers) = message.request_headers() {
        headers.replace("Accept", media_type);
    }
    message
}

/// A header of the answer to `message`.
fn response_header(message: &soup::Message, name: &str) -> Option<String> {
    let headers = message.response_headers()?;
    headers.one(name).map(String::from)
}

/// The body of a final answer, or its status as an error. As urllib, only
/// a 2xx status is success.
fn response_body(
    status: u32,
    stream: gio::InputStream,
    cancel: &Cancellation,
) -> Result<Box<dyn Read>, FetchError> {
    if !(200..300).contains(&status) {
        return Err(FetchError::Status(status));
    }
    Ok(Box::new(ResponseBody {
        stream,
        cancellable: cancel.cancellable().clone(),
    }))
}

/// A response body that stops reading as soon as the fetch is cancelled.
struct ResponseBody {
    stream: gio::InputStream,
    cancellable: gio::Cancellable,
}

impl Read for ResponseBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream
            .read(buffer, Some(&self.cancellable))
            .map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `desktop/tests/test_updater.py::UrlTests::test_http_opener_sets_timeout_and_uses_validating_redirect_handler`
    ///
    /// The request side: the check asks for GitHub's JSON, downloads ask
    /// for bytes, and every session has the 30-second timeout and the
    /// app's user agent. That redirects are validated and untrusted URLs
    /// never reach the client is the type rule of [`TrustedUrl`], tested
    /// in `tests/update_release.rs`.
    /// parity: UPD-002
    #[test]
    fn requests_carry_the_media_type_timeout_and_user_agent() {
        let latest = TrustedUrl::latest_release();
        let download = TrustedUrl::parse("https://github.com/fixture").unwrap();
        let session = GitHubReleases::new(ReleaseVersion::new(1, 2, 3)).session();

        let request = get_request(&latest, accepted_media_type(&latest));

        assert_eq!(accepted_media_type(&download), "application/octet-stream");
        let headers = request.request_headers().unwrap();
        assert_eq!(headers.one("Accept").unwrap(), "application/vnd.github+json");
        assert!(request.flags().contains(soup::MessageFlags::NO_REDIRECT));
        assert_eq!(session.timeout(), 30);
        assert_eq!(session.user_agent().unwrap(), "OpenXplorer/1.2.3");
    }
}
