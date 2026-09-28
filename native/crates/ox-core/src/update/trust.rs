// SPDX-License-Identifier: AGPL-3.0-only
//! The addresses the updater may contact. Ports `REPOSITORY`,
//! `REPOSITORIES`, `LATEST`, `HOSTS`, `trusted_url` and `TrustedRedirect`
//! in `desktop/updater.py`.

use std::fmt;

use super::UpdateError;

/// The repository that publishes the app's releases.
pub const REPOSITORY: &str = "https://github.com/AKolenda/openxplorer";

/// The repositories whose release installers are accepted: the original
/// one and its new home in the openxplorer organisation. GitHub redirects
/// the old API endpoint after the move, but the release assets then carry
/// the new owner.
pub const RELEASE_REPOSITORIES: [&str; 2] = [REPOSITORY, "https://github.com/openxplorer/openxplorer"];

/// GitHub's "latest release" API endpoint: the only address an update
/// check asks.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/AKolenda/openxplorer/releases/latest";

/// The hosts the updater may contact, including GitHub's download hosts
/// that release assets redirect to.
pub const TRUSTED_HOSTS: [&str; 4] = [
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// The standard HTTPS port, the only port a trusted address may name.
const HTTPS_PORT: i32 = 443;

/// An address the updater may contact.
///
/// Safety rule "only fixed upstream HTTPS endpoints" (`trusted_url` in
/// `desktop/updater.py`): the scheme is `https`, the host is one of
/// [`TRUSTED_HOSTS`], there are no credentials and the port is the
/// standard one. The only way to get a `TrustedUrl` is to pass this check,
/// and the HTTP client accepts nothing else, so no address from the UI or
/// the network is contacted unchecked.
#[derive(Clone)]
pub struct TrustedUrl {
    /// The address as it was checked, which is the address requested.
    text: String,
    /// `text`, parsed once, so the check and the request cannot disagree
    /// about which host it names.
    uri: glib::Uri,
}

impl TrustedUrl {
    /// Checks `url`.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UntrustedLocation`] if it breaks the rule above or is
    /// not a URL at all.
    pub fn parse(url: &str) -> Result<Self, UpdateError> {
        refuse_control_characters(url)?;
        let uri = glib::Uri::parse(url, glib::UriFlags::NONE).map_err(|_| UpdateError::UntrustedLocation)?;
        Self::checked(url.to_owned(), uri)
    }

    /// The "latest release" endpoint, [`LATEST_RELEASE_URL`].
    ///
    /// # Panics
    ///
    /// Never: the endpoint is on a trusted host.
    pub fn latest_release() -> Self {
        Self::parse(LATEST_RELEASE_URL).expect("the latest-release endpoint is on a trusted host")
    }

    /// Where a redirect from this address to `location` leads. A relative
    /// `location` is resolved against this address, as a browser would.
    ///
    /// Safety rule "redirects cross the same trust boundary"
    /// (`TrustedRedirect` in `desktop/updater.py`): the target must pass
    /// the same check as the address that was asked.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UntrustedLocation`] if the target is not trusted.
    pub fn redirect(&self, location: &str) -> Result<Self, UpdateError> {
        refuse_control_characters(location)?;
        let target = self
            .uri
            .parse_relative(location, glib::UriFlags::NONE)
            .map_err(|_| UpdateError::UntrustedLocation)?;
        let text = target.to_str().to_string();
        Self::checked(text, target)
    }

    /// Whether this is the "latest release" endpoint, which answers JSON
    /// rather than a file.
    pub fn is_latest_release(&self) -> bool {
        self.text == LATEST_RELEASE_URL
    }

    /// The address as text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The parsed address, for the HTTP client.
    pub(crate) fn as_uri(&self) -> &glib::Uri {
        &self.uri
    }

    /// Applies the trust rule to a parsed address.
    fn checked(text: String, uri: glib::Uri) -> Result<Self, UpdateError> {
        let host = uri.host().map(|host| host.to_ascii_lowercase());
        let is_trusted_host = host.is_some_and(|host| TRUSTED_HOSTS.contains(&host.as_str()));
        let has_standard_port = uri.port() == -1 || uri.port() == HTTPS_PORT;
        // Python's `urlsplit` keeps an empty user name ("https://@host/")
        // and treats it as none; any user part is refused here.
        let has_credentials = uri.userinfo().is_some();
        if uri.scheme() != "https" || !is_trusted_host || has_credentials || !has_standard_port {
            return Err(UpdateError::UntrustedLocation);
        }
        Ok(Self { text, uri })
    }
}

/// Refuses an address with a control character. `GLib` cannot take a NUL
/// byte, and urllib refuses to send any control character, so no such
/// address could have been fetched by the Python app either.
fn refuse_control_characters(url: &str) -> Result<(), UpdateError> {
    if url.chars().any(char::is_control) {
        Err(UpdateError::UntrustedLocation)
    } else {
        Ok(())
    }
}

impl PartialEq for TrustedUrl {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for TrustedUrl {}

impl fmt::Debug for TrustedUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("TrustedUrl").field(&self.text).finish()
    }
}

impl fmt::Display for TrustedUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_are_compared_without_case() {
        let url = TrustedUrl::parse("https://GitHub.com/fixture").unwrap();

        assert_eq!(url.as_str(), "https://GitHub.com/fixture");
    }

    #[test]
    fn an_empty_user_name_is_still_a_user_name() {
        assert!(TrustedUrl::parse("https://@github.com/fixture").is_err());
    }

    /// Regression: a NUL byte made `GLib`'s string conversion panic.
    #[test]
    fn a_control_character_is_refused_rather_than_a_panic() {
        let latest = TrustedUrl::latest_release();

        assert!(TrustedUrl::parse("https://github.com/\0").is_err());
        assert!(TrustedUrl::parse("https://github.com/\t").is_err());
        assert!(latest.redirect("/fixture\0").is_err());
    }

    #[test]
    fn a_relative_redirect_stays_on_the_same_host() {
        let url = TrustedUrl::latest_release();

        let target = url.redirect("/repositories/1/releases/latest").unwrap();

        assert_eq!(
            target.as_str(),
            "https://api.github.com/repositories/1/releases/latest"
        );
        assert!(url.is_latest_release());
        assert!(!target.is_latest_release());
    }
}
