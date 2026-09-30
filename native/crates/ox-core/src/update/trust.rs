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

/// How addresses are parsed: the path, query and fragment stay
/// percent-encoded, as libsoup's own `SOUP_HTTP_URI_FLAGS` keep them.
///
/// Safety rule "the address checked is the address requested": decoded,
/// a signed download address would reach GitHub with `%2B` sent as `+`,
/// `%20` as a raw space and `%26` splitting one parameter in two, and the
/// download host answers 400. Python's urllib sends a redirect's
/// `Location` unchanged. The host is still decoded, so the host check and
/// the request name the same host.
const HTTP_URI_FLAGS: glib::UriFlags = glib::UriFlags::ENCODED_PATH
    .union(glib::UriFlags::ENCODED_QUERY)
    .union(glib::UriFlags::ENCODED_FRAGMENT);

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
        let uri = glib::Uri::parse(url, HTTP_URI_FLAGS).map_err(|_| UpdateError::UntrustedLocation)?;
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
            .parse_relative(location, HTTP_URI_FLAGS)
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

    /// Only HTTPS to the four GitHub hosts, on the default port and
    /// without a user name, is contacted; a redirect elsewhere is refused
    /// with the Python app's message.
    ///
    /// parity: SAFE-016
    #[test]
    fn only_https_to_the_github_hosts_is_trusted() {
        for host in TRUSTED_HOSTS {
            assert!(
                TrustedUrl::parse(&format!("https://{host}/fixture")).is_ok(),
                "{host}"
            );
            assert!(
                TrustedUrl::parse(&format!("https://{host}:443/fixture")).is_ok(),
                "{host}"
            );
        }
        let refused = [
            "http://github.com/fixture",
            "https://github.com:8443/fixture",
            "https://user:secret@github.com/fixture",
            "https://example.com/fixture",
            "https://github.com.example.com/fixture",
            "file:///etc/passwd",
            "not a url",
        ];
        for url in refused {
            assert!(TrustedUrl::parse(url).is_err(), "{url}");
        }
        let redirect = TrustedUrl::latest_release().redirect("https://example.com/installer.deb");
        let error = redirect.expect_err("an untrusted redirect is refused");
        assert_eq!(
            error.to_string(),
            "The update server returned an untrusted download location."
        );
    }

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

    /// Regression: addresses were percent-decoded, so libsoup sent GitHub's
    /// signed download redirect with `+`, a raw space and a split
    /// parameter, and every installer download failed with HTTP 400.
    /// parity: UPD-002
    #[test]
    fn a_redirect_keeps_its_percent_encoding_byte_for_byte() {
        let latest = TrustedUrl::latest_release();
        let location = "https://release-assets.githubusercontent.com/a?sig=Ab%2Bc%3D\
                        &rscd=attachment%3B%20filename%3Dx.deb&x=a%26b";

        let target = latest.redirect(location).unwrap();

        assert_eq!(target.as_str(), location);
        assert_eq!(target.as_uri().to_str(), location);
    }

    /// The same rule for an address checked directly, such as a release's
    /// installer address.
    /// parity: UPD-002
    #[test]
    fn a_checked_address_keeps_its_percent_encoding_byte_for_byte() {
        let address = "https://github.com/a%20b/c?name=x%2By%26z#part%3D1";

        let url = TrustedUrl::parse(address).unwrap();

        assert_eq!(url.as_str(), address);
        assert_eq!(url.as_uri().to_str(), address);
    }
}
