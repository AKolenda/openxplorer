// SPDX-License-Identifier: AGPL-3.0-only
//! The HTTPS client for GitHub, on libsoup. Ports `open_url` and
//! `TrustedRedirect` in `desktop/updater.py`.
//!
//! libsoup is GNOME's HTTP library: it follows the desktop's proxy
//! settings, checks certificates against the system's store through
//! glib-networking, and takes a [`gio::Cancellable`]. Each fetch makes its
//! own session on the worker thread that reads it, so nothing is shared
//! between threads.
//!
//! Redirects are followed by [`follow_redirects`], which knows nothing of
//! libsoup, so the trust rule for redirects is tested without a network.

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

/// HTTP statuses that redirect a GET, as urllib follows them.
const REDIRECT_STATUSES: [u32; 5] = [301, 302, 303, 307, 308];

/// What a request asks the server to answer, its `Accept` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaType {
    /// GitHub's JSON, from the "latest release" endpoint.
    GitHubJson,
    /// Raw bytes, for a download.
    OctetStream,
}

impl MediaType {
    /// What to ask `url` to answer.
    fn for_url(url: &TrustedUrl) -> Self {
        if url.is_latest_release() {
            Self::GitHubJson
        } else {
            Self::OctetStream
        }
    }

    /// The `Accept` header's value.
    fn as_str(self) -> &'static str {
        match self {
            Self::GitHubJson => "application/vnd.github+json",
            Self::OctetStream => "application/octet-stream",
        }
    }
}

/// One answer on the way to a fetch's final answer.
#[derive(Debug)]
enum Hop<Body> {
    /// A redirect status, with the `Location` header if the answer had one.
    Redirect { status: u32, location: Option<String> },
    /// Any other status, with the answer's body.
    Final { status: u32, body: Body },
}

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
        let send = |target: &TrustedUrl, media_type| send_once(&session, target, media_type, cancel);
        let stream = follow_redirects(url, send)?;
        Ok(Box::new(ResponseBody {
            stream,
            cancellable: cancel.cancellable().clone(),
        }))
    }
}

/// Fetches `first`, following at most [`MAX_REDIRECTS`] redirects. `send`
/// makes one request and answers with its [`Hop`].
///
/// Safety rule "redirects cross the same trust boundary"
/// (`TrustedRedirect` in `desktop/updater.py`): every target passes
/// [`TrustedUrl::redirect`] before `send` sees it, so an untrusted address
/// is never contacted.
///
/// As urllib does, every request asks for the first request's media type,
/// and only a 2xx final status is success.
///
/// # Errors
///
/// Whatever `send` returns; [`FetchError::Untrusted`] for an untrusted
/// target; [`FetchError::Status`] for a final status outside 2xx, for a
/// redirect without a `Location`, and for a redirect past the tenth, with
/// that answer's status.
fn follow_redirects<Body>(
    first: &TrustedUrl,
    mut send: impl FnMut(&TrustedUrl, MediaType) -> Result<Hop<Body>, FetchError>,
) -> Result<Body, FetchError> {
    // Redirects keep the first request's headers, as urllib's do.
    let media_type = MediaType::for_url(first);
    let mut target = first.clone();
    let mut last_status = 0;
    for _ in 0..=MAX_REDIRECTS {
        let (status, location) = match send(&target, media_type)? {
            Hop::Final { status, body } => return success_body(status, body),
            Hop::Redirect { status, location } => (status, location),
        };
        let location = location.ok_or(FetchError::Status(status))?;
        target = target.redirect(&location).map_err(|_| FetchError::Untrusted)?;
        last_status = status;
    }
    // urllib reports a redirect loop as an HTTP error with the last
    // redirect's status.
    Err(FetchError::Status(last_status))
}

/// The body of a final answer, or its status as an error. As urllib, only
/// a 2xx status is success.
fn success_body<Body>(status: u32, body: Body) -> Result<Body, FetchError> {
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(FetchError::Status(status))
    }
}

/// Sends one GET for `url` over `session`.
fn send_once(
    session: &soup::Session,
    url: &TrustedUrl,
    media_type: MediaType,
    cancel: &Cancellation,
) -> Result<Hop<gio::InputStream>, FetchError> {
    let message = get_request(url, media_type);
    let body = session
        .send(&message, Some(cancel.cancellable()))
        .map_err(|_| FetchError::interrupted(cancel))?;
    let status = message.status_code();
    if !REDIRECT_STATUSES.contains(&status) {
        return Ok(Hop::Final { status, body });
    }
    let location = response_header(&message, "Location");
    Ok(Hop::Redirect { status, location })
}

/// A GET for `url` that libsoup must not redirect by itself.
fn get_request(url: &TrustedUrl, media_type: MediaType) -> soup::Message {
    let message = soup::Message::from_uri("GET", url.as_uri());
    message.set_flags(soup::MessageFlags::NO_REDIRECT);
    if let Some(headers) = message.request_headers() {
        headers.replace("Accept", media_type.as_str());
    }
    message
}

/// A header of the answer to `message`.
fn response_header(message: &soup::Message, name: &str) -> Option<String> {
    let headers = message.response_headers()?;
    headers.one(name).map(String::from)
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

    /// The body the scripted server's final answers carry.
    const BODY: &str = "Fictional body";

    /// A request as it reached the scripted server.
    #[derive(Debug, PartialEq, Eq)]
    struct SentRequest {
        /// The parsed address the HTTP client would be given.
        address: String,
        /// The media type asked for.
        media_type: MediaType,
    }

    /// What a fetch against the scripted server did.
    struct Fetch {
        result: Result<&'static str, FetchError>,
        sent: Vec<SentRequest>,
    }

    /// Fetches `first` from a server whose answer to the request numbered
    /// `n` (from 0) is `answer(n)`, and records every request it received.
    fn fetch(first: &TrustedUrl, answer: impl Fn(usize) -> Hop<&'static str>) -> Fetch {
        let mut sent = Vec::new();
        let result = follow_redirects(first, |target, media_type| {
            let hop = answer(sent.len());
            sent.push(SentRequest {
                address: target.as_uri().to_str().to_string(),
                media_type,
            });
            Ok(hop)
        });
        Fetch { result, sent }
    }

    fn redirect(status: u32, location: &str) -> Hop<&'static str> {
        Hop::Redirect {
            status,
            location: Some(location.to_owned()),
        }
    }

    fn final_answer(status: u32) -> Hop<&'static str> {
        Hop::Final { status, body: BODY }
    }

    fn download() -> TrustedUrl {
        TrustedUrl::parse("https://github.com/fixture").unwrap()
    }

    /// Ported from `desktop/tests/test_updater.py::UrlTests::test_http_opener_sets_timeout_and_uses_validating_redirect_handler`
    ///
    /// The request side: the check asks for GitHub's JSON, downloads ask
    /// for bytes, and every session has the 30-second timeout and the
    /// app's user agent. The redirect side is tested below, through
    /// [`follow_redirects`].
    /// parity: UPD-002
    #[test]
    fn requests_carry_the_media_type_timeout_and_user_agent() {
        let latest = TrustedUrl::latest_release();
        let session = GitHubReleases::new(ReleaseVersion::new(1, 2, 3)).session();

        let request = get_request(&latest, MediaType::for_url(&latest));

        assert_eq!(
            MediaType::for_url(&download()).as_str(),
            "application/octet-stream"
        );
        let headers = request.request_headers().unwrap();
        assert_eq!(headers.one("Accept").unwrap(), "application/vnd.github+json");
        assert!(request.flags().contains(soup::MessageFlags::NO_REDIRECT));
        assert_eq!(session.timeout(), 30);
        assert_eq!(session.user_agent().unwrap(), "OpenXplorer/1.2.3");
    }

    /// A redirect to an untrusted host ends the fetch before that host is
    /// contacted.
    /// parity: UPD-002
    #[test]
    fn an_untrusted_redirect_target_is_never_contacted() {
        let fetched = fetch(&download(), |_| redirect(302, "https://example.invalid/fixture"));

        assert_eq!(fetched.result, Err(FetchError::Untrusted));
        assert_eq!(fetched.sent.len(), 1);
        assert_eq!(fetched.sent[0].address, "https://github.com/fixture");
    }

    /// Ten redirects are followed; the eleventh is an HTTP error with its
    /// own status, as urllib reports it.
    /// parity: UPD-002
    #[test]
    fn a_redirect_past_the_tenth_is_an_http_error_with_its_status() {
        let answer = |request| {
            let status = if request == MAX_REDIRECTS { 308 } else { 301 };
            redirect(status, "/next")
        };

        let fetched = fetch(&download(), answer);

        assert_eq!(fetched.result, Err(FetchError::Status(308)));
        assert_eq!(fetched.sent.len(), MAX_REDIRECTS + 1);
    }

    /// Ten redirects, then a final answer, still succeed.
    /// parity: UPD-002
    #[test]
    fn a_final_answer_after_ten_redirects_is_read() {
        let answer = |request| {
            if request < MAX_REDIRECTS {
                redirect(302, "/next")
            } else {
                final_answer(200)
            }
        };

        let fetched = fetch(&download(), answer);

        assert_eq!(fetched.result, Ok(BODY));
        assert_eq!(fetched.sent.len(), MAX_REDIRECTS + 1);
    }

    /// A redirect without a `Location` is an HTTP error with its status.
    /// parity: UPD-002
    #[test]
    fn a_redirect_without_a_location_is_an_http_error() {
        let answer = |_| Hop::Redirect {
            status: 302,
            location: None,
        };

        let fetched = fetch(&download(), answer);

        assert_eq!(fetched.result, Err(FetchError::Status(302)));
        assert_eq!(fetched.sent.len(), 1);
    }

    /// Only a 2xx final status is success.
    /// parity: UPD-002
    #[test]
    fn only_a_2xx_final_status_is_success() {
        struct StatusCase {
            status: u32,
            expected: Result<&'static str, FetchError>,
        }
        let cases = [
            StatusCase {
                status: 200,
                expected: Ok(BODY),
            },
            StatusCase {
                status: 206,
                expected: Ok(BODY),
            },
            StatusCase {
                status: 304,
                expected: Err(FetchError::Status(304)),
            },
            StatusCase {
                status: 404,
                expected: Err(FetchError::Status(404)),
            },
            StatusCase {
                status: 500,
                expected: Err(FetchError::Status(500)),
            },
        ];

        for case in cases {
            let fetched = fetch(&download(), |_| final_answer(case.status));

            assert_eq!(fetched.result, case.expected, "HTTP {}", case.status);
        }
    }

    /// Regression: GitHub's signed download address reached the client
    /// percent-decoded, and GitHub answered 400.
    /// parity: UPD-002
    #[test]
    fn a_percent_encoded_location_reaches_the_client_unchanged() {
        let location = "https://release-assets.githubusercontent.com/a?sig=Ab%2Bc%3D\
                        &rscd=attachment%3B%20filename%3Dx.deb&x=a%26b";
        let answer = |request| {
            if request == 0 {
                redirect(302, location)
            } else {
                final_answer(206)
            }
        };

        let fetched = fetch(&download(), answer);

        assert_eq!(fetched.result, Ok(BODY));
        assert_eq!(fetched.sent[1].address, location);
        let target = download().redirect(location).unwrap();
        let request = get_request(&target, MediaType::OctetStream);
        let requested = request.uri().unwrap().to_str();
        assert_eq!(requested, location, "libsoup's request keeps it too");
    }

    /// Every request of a fetch asks for the first request's media type,
    /// even where the redirect target alone would ask for another.
    /// parity: UPD-002
    #[test]
    fn redirects_keep_the_first_requests_media_type() {
        let answer = |request| {
            if request == 0 {
                redirect(301, "/repositories/1/releases/latest")
            } else {
                final_answer(200)
            }
        };

        let fetched = fetch(&TrustedUrl::latest_release(), answer);

        assert_eq!(fetched.result, Ok(BODY));
        let expected = [
            SentRequest {
                address: "https://api.github.com/repos/AKolenda/openxplorer/releases/latest".to_owned(),
                media_type: MediaType::GitHubJson,
            },
            SentRequest {
                address: "https://api.github.com/repositories/1/releases/latest".to_owned(),
                media_type: MediaType::GitHubJson,
            },
        ];
        assert_eq!(fetched.sent, expected);
    }
}
