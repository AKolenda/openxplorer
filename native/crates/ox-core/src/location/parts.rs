// SPDX-License-Identifier: AGPL-3.0-only
//! Splitting a location into scheme, authority and path.
//!
//! Ports `split_location` from `desktop/core.py` together with the parts of
//! Python's `urllib.parse.urlsplit` it relies on, including the `hostname`
//! and `port` properties and the bracketed-IPv6 checks. Portable-device URIs
//! (`mtp://[usb:001,002]/`) use their own narrow parser because their
//! bracketed bus identifiers are not IPv6 addresses.

use std::borrow::Cow;
use std::net::{Ipv4Addr, Ipv6Addr};

use super::text::is_python_space;
use super::{LocationError, DEVICE_SCHEMES};

/// A location split into its URI components, like Python's `SplitResult`.
///
/// Nothing is decoded: `path` keeps its percent escapes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocationParts {
    /// Lower-cased scheme, or empty for a plain path.
    pub scheme: String,
    /// Everything between `//` and the path, including any user and port.
    pub netloc: String,
    /// The escaped path. Device locations always have at least `/`.
    pub path: String,
    /// Text after `?`, without the `?`.
    pub query: String,
    /// Text after `#`, without the `#`.
    pub fragment: String,
}

impl LocationParts {
    /// True when the authority carries a user name or password (`user@`).
    pub fn has_credentials(&self) -> bool {
        self.netloc.contains('@')
    }

    /// The host name, lower-cased, without brackets, user or port; `None`
    /// when empty. Mirrors Python's `SplitResult.hostname`.
    pub fn hostname(&self) -> Option<String> {
        let (host, _) = self.host_and_port();
        if host.is_empty() {
            return None;
        }
        // An IPv6 zone (`%eth0`) keeps its case, as in Python.
        let (address, zone) = match host.split_once('%') {
            Some((address, zone)) => (address, Some(zone)),
            None => (host, None),
        };
        let mut hostname = address.to_lowercase();
        if let Some(zone) = zone {
            hostname.push('%');
            hostname.push_str(zone);
        }
        Some(hostname)
    }

    /// The port, or `None` when absent or empty. Mirrors Python's
    /// `SplitResult.port`: ASCII digits only, at most 65535.
    ///
    /// # Errors
    ///
    /// Python's `ValueError` wording for a port that is not all digits or
    /// is above 65535; callers show their own message instead.
    pub fn port(&self) -> Result<Option<u16>, LocationError> {
        let (_, port) = self.host_and_port();
        let Some(port) = port else {
            return Ok(None);
        };
        let invalid = || LocationError::new(format!("Port could not be cast to integer value as {port:?}"));
        if !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        // Leading zeros are allowed, so parse wider than u16 first.
        let value: u128 = port.parse().map_err(|_| invalid())?;
        u16::try_from(value)
            .map(Some)
            .map_err(|_| LocationError::new("Port out of range 0-65535"))
    }

    /// Python's `_hostinfo`: the host (brackets removed) and the non-empty
    /// port text.
    fn host_and_port(&self) -> (&str, Option<&str>) {
        let host_info = after_user_info(&self.netloc);
        let (host, port) = match host_info.split_once('[') {
            Some((_, bracketed)) => {
                let (host, after_bracket) = bracketed.split_once(']').unwrap_or((bracketed, ""));
                let port = after_bracket.split_once(':').map_or("", |(_, port)| port);
                (host, port)
            }
            None => host_info.split_once(':').unwrap_or((host_info, "")),
        };
        let port = if port.is_empty() { None } else { Some(port) };
        (host, port)
    }
}

/// Splits a location, including the non-RFC USB authorities of GIO's
/// device URIs.
///
/// `mtp://[usb:001,002]/DCIM` splits into scheme `mtp`, authority
/// `[usb:001,002]` and path `/DCIM`; a device URI without a path gets `/`.
/// Everything else follows Python's `urlsplit`.
///
/// # Errors
///
/// A [`LocationError`] with Python's wording for unbalanced or invalid
/// bracketed hosts.
pub fn split_location(value: &str) -> Result<LocationParts, LocationError> {
    if let Some(device) = DeviceMatch::parse(value) {
        let scheme = device.scheme.to_ascii_lowercase();
        if DEVICE_SCHEMES.contains(&scheme.as_str()) {
            return Ok(LocationParts {
                scheme,
                netloc: device.authority.to_string(),
                path: device.path.to_string(),
                query: String::new(),
                fragment: String::new(),
            });
        }
    }
    urlsplit(value)
}

/// A match of `DEVICE_URI` in `core.py`:
/// `^([A-Za-z][A-Za-z0-9+.-]*)://([^/?#]+)(/[^?#]*)?$`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceMatch<'a> {
    /// The scheme as written (not lower-cased).
    pub scheme: &'a str,
    /// The non-empty authority, for example `[usb:001,002]`.
    pub authority: &'a str,
    /// The path; `/` when the URI has none.
    pub path: &'a str,
}

impl<'a> DeviceMatch<'a> {
    /// Matches any scheme; callers check it against [`DEVICE_SCHEMES`].
    pub fn parse(value: &'a str) -> Option<Self> {
        let (scheme, rest) = value.split_once("://")?;
        if !is_scheme(scheme) {
            return None;
        }
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let (authority, path) = rest.split_at(authority_end);
        if authority.is_empty() || authority.contains(['?', '#']) || path.contains(['?', '#']) {
            return None;
        }
        let path = if path.is_empty() { "/" } else { path };
        Some(Self {
            scheme,
            authority,
            path,
        })
    }
}

/// True for text matching `[A-Za-z][A-Za-z0-9+.-]*`.
pub(crate) fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    starts_with_letter && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The scheme of `value` by `urlsplit`'s rule, lower-cased, and the text
/// after its colon; `None` for a plain path.
pub(crate) fn url_scheme(value: &str) -> Option<(String, &str)> {
    let (scheme, rest) = value.split_once(':')?;
    is_scheme(scheme).then(|| (scheme.to_ascii_lowercase(), rest))
}

/// Python's `urllib.parse.urlsplit`, as shipped with Python 3.12.
///
/// Like Python, it first drops leading C0 controls and spaces and removes
/// every tab, carriage return and line feed (the WHATWG rules), then checks
/// bracketed hosts and rejects non-ASCII authorities that NFKC
/// normalisation turns into URL delimiters (`℀` becomes `a/c`).
pub(crate) fn urlsplit(value: &str) -> Result<LocationParts, LocationError> {
    let cleaned = strip_whatwg_noise(value);
    let (scheme, mut rest) = match url_scheme(&cleaned) {
        Some((scheme, rest)) => (scheme, rest),
        None => (String::new(), cleaned.as_ref()),
    };
    let mut netloc = "";
    if let Some(after_slashes) = rest.strip_prefix("//") {
        let end = after_slashes.find(['/', '?', '#']).unwrap_or(after_slashes.len());
        netloc = &after_slashes[..end];
        rest = &after_slashes[end..];
        check_brackets(netloc)?;
    }
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    check_nfkc_authority(netloc)?;
    Ok(LocationParts {
        scheme,
        netloc: netloc.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        fragment: fragment.to_string(),
    })
}

/// Characters `urlsplit` removes wherever they occur.
const UNSAFE_URL_CHARACTERS: [char; 3] = ['\t', '\r', '\n'];

/// Drops leading C0 controls and spaces and removes tabs, carriage returns
/// and line feeds anywhere, as `urlsplit` does before parsing.
fn strip_whatwg_noise(value: &str) -> Cow<'_, str> {
    let trimmed = value.trim_start_matches(|c: char| c <= ' ');
    if trimmed.contains(UNSAFE_URL_CHARACTERS) {
        Cow::Owned(trimmed.replace(UNSAFE_URL_CHARACTERS, ""))
    } else {
        Cow::Borrowed(trimmed)
    }
}

/// Python's `_checknetloc`: a non-ASCII authority must not gain `/`, `?`,
/// `#`, `@` or `:` under NFKC normalisation, because it would then split
/// differently once a client converts it to ASCII.
fn check_nfkc_authority(netloc: &str) -> Result<(), LocationError> {
    if netloc.is_ascii() {
        return Ok(());
    }
    let without_delimiters: String = netloc
        .chars()
        .filter(|c| !matches!(c, '@' | ':' | '#' | '?'))
        .collect();
    let normalised = glib::normalize(&without_delimiters, glib::NormalizeMode::AllCompose);
    let gains_delimiter = normalised.contains(['/', '?', '#', '@', ':']);
    if normalised.as_str() == without_delimiters || !gains_delimiter {
        return Ok(());
    }
    Err(LocationError::new(format!(
        "The server name “{netloc}” contains characters that are not allowed in an address."
    )))
}

/// `urlsplit`'s bracket rules: balanced brackets, nothing before `[`,
/// only `:port` after `]`, and an IPv6 address or an RFC 3986 future
/// address (`v1.x`) inside.
fn check_brackets(netloc: &str) -> Result<(), LocationError> {
    let has_open = netloc.contains('[');
    let has_close = netloc.contains(']');
    if has_open != has_close {
        return Err(invalid_ipv6());
    }
    if !has_open {
        return Ok(());
    }
    let host_info = after_user_info(netloc);
    let host = match host_info.split_once('[') {
        Some((before, bracketed)) => {
            if !before.is_empty() {
                return Err(invalid_ipv6());
            }
            let (host, after) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            if !after.is_empty() && !after.starts_with(':') {
                return Err(invalid_ipv6());
            }
            host
        }
        // The only `[` is in the user part: Python checks the host text
        // before the port as if it were bracketed.
        None => host_info.split_once(':').map_or(host_info, |(host, _)| host),
    };
    check_bracketed_host(host)
}

/// Python's `_check_bracketed_host`.
fn check_bracketed_host(host: &str) -> Result<(), LocationError> {
    if let Some(future) = host.strip_prefix('v') {
        let valid = future.split_once('.').is_some_and(|(version, rest)| {
            !version.is_empty() && version.bytes().all(|b| b.is_ascii_hexdigit()) && !rest.is_empty()
        });
        return if valid {
            Ok(())
        } else {
            Err(LocationError::new("IPvFuture address is invalid"))
        };
    }
    if host.parse::<Ipv4Addr>().is_ok() {
        return Err(LocationError::new("An IPv4 address cannot be in brackets"));
    }
    // Python accepts a non-empty scope such as `fe80::1%eth0`.
    let address = match host.split_once('%') {
        Some((address, zone)) if !zone.is_empty() && !zone.contains('%') => address,
        Some(_) => return Err(invalid_ipv6_address(host)),
        None => host,
    };
    address
        .parse::<Ipv6Addr>()
        .map(|_| ())
        .map_err(|_| invalid_ipv6_address(host))
}

/// The host and port part of an authority: the text after the last `@`.
fn after_user_info(netloc: &str) -> &str {
    netloc.rsplit_once('@').map_or(netloc, |(_, host)| host)
}

fn invalid_ipv6() -> LocationError {
    LocationError::new("Invalid IPv6 URL")
}

fn invalid_ipv6_address(host: &str) -> LocationError {
    LocationError::new(format!("{host:?} does not appear to be an IPv4 or IPv6 address"))
}

/// True if `text` contains a character Python's `str.isspace()` accepts.
pub(crate) fn contains_python_space(text: &str) -> bool {
    text.chars().any(is_python_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(scheme: &str, netloc: &str, path: &str) -> LocationParts {
        LocationParts {
            scheme: scheme.into(),
            netloc: netloc.into(),
            path: path.into(),
            ..LocationParts::default()
        }
    }

    #[test]
    fn device_authorities_with_brackets_split() {
        assert_eq!(
            split_location("mtp://[usb:001,010]/Internal%20storage/DCIM"),
            Ok(parts("mtp", "[usb:001,010]", "/Internal%20storage/DCIM"))
        );
        assert_eq!(
            split_location("GPhoto2://[usb:001,002]"),
            Ok(parts("gphoto2", "[usb:001,002]", "/"))
        );
        // Not a device scheme: the IPv6 check rejects the brackets.
        assert!(split_location("smb://[usb:001,002]/").is_err());
    }

    #[test]
    fn urlsplit_matches_python() {
        assert_eq!(
            split_location("smb://NAS/Team%20files"),
            Ok(parts("smb", "NAS", "/Team%20files"))
        );
        assert_eq!(split_location("file:///tmp/x"), Ok(parts("file", "", "/tmp/x")));
        assert_eq!(split_location("/tmp/a:b"), Ok(parts("", "", "/tmp/a:b")));
        assert_eq!(split_location("C:\\Windows"), Ok(parts("c", "", "\\Windows")));
        let with_query = split_location("smb://nas/a?b#c").expect("valid URL");
        assert_eq!((with_query.path.as_str(), with_query.query.as_str()), ("/a", "b"));
        assert_eq!(with_query.fragment, "c");
        assert_eq!(split_location("mtp:foo"), Ok(parts("mtp", "", "foo")));
    }

    #[test]
    fn bracketed_hosts_must_be_ipv6() {
        assert!(split_location("smb://[fe80::1]/share").is_ok());
        assert!(split_location("smb://[fe80::1%eth0]:445/share").is_ok());
        assert!(split_location("smb://[v1.x]/share").is_ok());
        for bad in [
            "smb://[nas/share",
            "smb://nas]/",
            "smb://[1.2.3.4]/",
            "smb://x[::1]/",
            "smb://[::1]x/",
        ] {
            assert!(split_location(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn hostname_and_port_follow_python() {
        let smb = split_location("smb://User@NAS:0445/share").expect("valid URL");
        assert!(smb.has_credentials());
        assert_eq!(smb.hostname().as_deref(), Some("nas"));
        assert_eq!(smb.port(), Ok(Some(445)));
        let ipv6 = split_location("smb://[FE80::1]:139/").expect("valid URL");
        assert_eq!(ipv6.hostname().as_deref(), Some("fe80::1"));
        assert_eq!(ipv6.port(), Ok(Some(139)));
        assert_eq!(
            split_location("smb://nas:/x").expect("valid URL").port(),
            Ok(None)
        );
        assert!(split_location("smb://nas:99999/")
            .expect("valid URL")
            .port()
            .is_err());
        assert!(split_location("smb://nas:4x/")
            .expect("valid URL")
            .port()
            .is_err());
        assert_eq!(
            split_location("smb:///share").expect("valid URL").hostname(),
            None
        );
    }
}
