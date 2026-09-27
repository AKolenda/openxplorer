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

use super::{LocationError, DEVICE_SCHEMES};

/// Characters `urlsplit` removes wherever they occur (Python's
/// `_UNSAFE_URL_BYTES_TO_REMOVE`).
const UNSAFE_URL_CHARACTERS: [char; 3] = ['\t', '\r', '\n'];

/// What kind of place the scheme of a location names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocationKind {
    /// `file:`: a folder on this computer.
    Local,
    /// `smb:`: a Windows or Samba server, share or shared folder.
    Smb,
    /// `mtp:`, `gphoto2:` or `afc:`: a phone, camera or iOS device.
    Device,
    /// Any other scheme, or a plain path without one.
    Other,
}

impl LocationKind {
    /// The kind a lower-case `scheme` names.
    pub(crate) fn from_scheme(scheme: &str) -> Self {
        match scheme {
            "file" => Self::Local,
            "smb" => Self::Smb,
            _ if DEVICE_SCHEMES.contains(&scheme) => Self::Device,
            _ => Self::Other,
        }
    }
}

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
        let hostname = match host.split_once('%') {
            Some((address, zone)) => format!("{}%{zone}", address.to_lowercase()),
            None => host.to_lowercase(),
        };
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
        let not_an_integer =
            || LocationError::new(format!("Port could not be cast to integer value as {port:?}"));
        if !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(not_an_integer());
        }
        // Leading zeros are allowed, so parse wider than u16 first.
        let value: u128 = port.parse().map_err(|_| not_an_integer())?;
        u16::try_from(value)
            .map(Some)
            .map_err(|_| LocationError::new("Port out of range 0-65535"))
    }

    /// What kind of place the scheme names.
    pub(crate) fn kind(&self) -> LocationKind {
        LocationKind::from_scheme(&self.scheme)
    }

    /// True for `file:` locations.
    pub(crate) fn is_local(&self) -> bool {
        self.kind() == LocationKind::Local
    }

    /// True for `smb:` locations.
    pub(crate) fn is_smb(&self) -> bool {
        self.kind() == LocationKind::Smb
    }

    /// True for phones, cameras and iOS devices: `mtp:`, `gphoto2:` and
    /// `afc:` locations.
    pub(crate) fn is_device(&self) -> bool {
        self.kind() == LocationKind::Device
    }

    /// The number of non-empty path components: 0 at a local, server or
    /// device root, 1 for an SMB share.
    pub(crate) fn path_depth(&self) -> usize {
        self.path
            .split('/')
            .filter(|component| !component.is_empty())
            .count()
    }

    /// Python's `_hostinfo`: the host (brackets removed) and the non-empty
    /// port text.
    fn host_and_port(&self) -> (&str, Option<&str>) {
        let host_info = after_user_info(&self.netloc);
        let (host, port) = match host_info.split_once('[') {
            Some((_, bracketed)) => {
                let (host, after_bracket) = partition(bracketed, ']');
                let (_, port) = partition(after_bracket, ':');
                (host, port)
            }
            None => partition(host_info, ':'),
        };
        (host, (!port.is_empty()).then_some(port))
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
    let device_parts = DeviceUriMatch::parse(value).map(DeviceUriMatch::to_parts);
    match device_parts {
        Some(parts) if parts.is_device() => Ok(parts),
        _ => split_url(value),
    }
}

/// A match of `DEVICE_URI` in `core.py`:
/// `^([A-Za-z][A-Za-z0-9+.-]*)://([^/?#]+)(/[^?#]*)?$`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceUriMatch<'a> {
    /// The scheme as written (not lower-cased).
    pub(crate) scheme: &'a str,
    /// The non-empty authority, for example `[usb:001,002]`.
    pub(crate) authority: &'a str,
    /// The path; `/` when the URI has none.
    pub(crate) path: &'a str,
}

impl<'a> DeviceUriMatch<'a> {
    /// Matches any scheme; callers check it against [`DEVICE_SCHEMES`].
    pub(crate) fn parse(value: &'a str) -> Option<Self> {
        let (scheme, after_scheme) = value.split_once("://")?;
        if !is_scheme(scheme) || after_scheme.contains(['?', '#']) {
            return None;
        }
        let (authority, path) = match after_scheme.find('/') {
            Some(slash) => after_scheme.split_at(slash),
            None => (after_scheme, "/"),
        };
        if authority.is_empty() {
            return None;
        }
        Some(Self {
            scheme,
            authority,
            path,
        })
    }

    /// The match as [`LocationParts`] with the scheme lower-cased, as
    /// `split_location` in `core.py` builds its `SplitResult`.
    pub(crate) fn to_parts(self) -> LocationParts {
        LocationParts {
            scheme: self.scheme.to_ascii_lowercase(),
            netloc: self.authority.to_string(),
            path: self.path.to_string(),
            query: String::new(),
            fragment: String::new(),
        }
    }
}

/// The scheme of `value` by `urlsplit`'s rule, lower-cased, and the text
/// after its colon; `None` for a plain path.
pub(crate) fn split_scheme(value: &str) -> Option<(String, &str)> {
    let (scheme, after_scheme) = value.split_once(':')?;
    is_scheme(scheme).then(|| (scheme.to_ascii_lowercase(), after_scheme))
}

/// True for text matching `[A-Za-z][A-Za-z0-9+.-]*`.
fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    starts_with_letter && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Python's `urllib.parse.urlsplit`, as shipped with Python 3.12.
///
/// Like Python, it first drops leading C0 controls and spaces and removes
/// every tab, carriage return and line feed (the WHATWG rules), then checks
/// bracketed hosts and rejects non-ASCII authorities that NFKC
/// normalisation turns into URL delimiters (`℀` becomes `a/c`).
pub(crate) fn split_url(value: &str) -> Result<LocationParts, LocationError> {
    let cleaned = strip_ignored_url_characters(value);
    let (scheme, after_scheme) = match split_scheme(&cleaned) {
        Some((scheme, after_scheme)) => (scheme, after_scheme),
        None => (String::new(), cleaned.as_ref()),
    };
    let (netloc, after_netloc) = split_netloc(after_scheme);
    check_brackets(netloc)?;
    let (before_fragment, fragment) = partition(after_netloc, '#');
    let (path, query) = partition(before_fragment, '?');
    check_nfkc_netloc(netloc)?;
    Ok(LocationParts {
        scheme,
        netloc: netloc.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        fragment: fragment.to_string(),
    })
}

/// Drops leading C0 controls and spaces and removes tabs, carriage returns
/// and line feeds anywhere, as `urlsplit` does before parsing.
fn strip_ignored_url_characters(value: &str) -> Cow<'_, str> {
    let trimmed = value.trim_start_matches(|c: char| c <= ' ');
    if trimmed.contains(UNSAFE_URL_CHARACTERS) {
        Cow::Owned(trimmed.replace(UNSAFE_URL_CHARACTERS, ""))
    } else {
        Cow::Borrowed(trimmed)
    }
}

/// Python's `_splitnetloc`: the authority after a leading `//`, up to the
/// first `/`, `?` or `#`, and the text after it. Without `//` the
/// authority is empty.
fn split_netloc(text: &str) -> (&str, &str) {
    let Some(after_slashes) = text.strip_prefix("//") else {
        return ("", text);
    };
    let end = after_slashes.find(['/', '?', '#']).unwrap_or(after_slashes.len());
    after_slashes.split_at(end)
}

/// `urlsplit`'s bracket rules: balanced brackets, and then
/// [`check_bracketed_netloc`].
fn check_brackets(netloc: &str) -> Result<(), LocationError> {
    let has_open = netloc.contains('[');
    let has_close = netloc.contains(']');
    if has_open != has_close {
        return Err(invalid_ipv6_url());
    }
    if !has_open {
        return Ok(());
    }
    check_bracketed_netloc(netloc)
}

/// Python's `_check_bracketed_netloc`: nothing before `[`, only `:port`
/// after `]`, and an IPv6 address or an RFC 3986 future address (`v1.x`)
/// inside.
fn check_bracketed_netloc(netloc: &str) -> Result<(), LocationError> {
    let host_info = after_user_info(netloc);
    let host = match host_info.split_once('[') {
        Some((before_bracket, bracketed)) => {
            let (host, after_bracket) = partition(bracketed, ']');
            let has_only_port_after = after_bracket.is_empty() || after_bracket.starts_with(':');
            if !before_bracket.is_empty() || !has_only_port_after {
                return Err(invalid_ipv6_url());
            }
            host
        }
        // The only `[` is in the user part: Python checks the host text
        // before the port as if it were bracketed.
        None => partition(host_info, ':').0,
    };
    check_bracketed_host(host)
}

/// Python's `_check_bracketed_host`.
fn check_bracketed_host(host: &str) -> Result<(), LocationError> {
    if host.starts_with('v') {
        if !is_ip_future_address(host) {
            return Err(LocationError::new("IPvFuture address is invalid"));
        }
        return Ok(());
    }
    if host.parse::<Ipv4Addr>().is_ok() {
        return Err(LocationError::new("An IPv4 address cannot be in brackets"));
    }
    if !is_ipv6_address(host) {
        return Err(LocationError::new(format!(
            "{host:?} does not appear to be an IPv4 or IPv6 address"
        )));
    }
    Ok(())
}

/// `\Av[a-fA-F0-9]+\..+\z`: an RFC 3986 future address such as `v1.x`.
fn is_ip_future_address(host: &str) -> bool {
    let Some((version, rest)) = host.strip_prefix('v').and_then(|future| future.split_once('.')) else {
        return false;
    };
    let is_hex_version = !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_hexdigit());
    is_hex_version && !rest.is_empty()
}

/// Python's `ipaddress.IPv6Address`, which accepts a non-empty zone such as
/// `fe80::1%eth0`.
fn is_ipv6_address(host: &str) -> bool {
    let address = match host.split_once('%') {
        Some((address, zone)) if !zone.is_empty() && !zone.contains('%') => address,
        Some(_) => return false,
        None => host,
    };
    address.parse::<Ipv6Addr>().is_ok()
}

/// Python's `_checknetloc`: a non-ASCII authority must not gain `/`, `?`,
/// `#`, `@` or `:` under NFKC normalisation, because it would then split
/// differently once a client converts it to ASCII.
fn check_nfkc_netloc(netloc: &str) -> Result<(), LocationError> {
    if netloc.is_ascii() {
        return Ok(());
    }
    let without_delimiters: String = netloc
        .chars()
        .filter(|c| !matches!(c, '@' | ':' | '#' | '?'))
        .collect();
    let normalised = glib::normalize(&without_delimiters, glib::NormalizeMode::AllCompose);
    // The authority holds no `/`, `?` or `#` (they end it) and `@` and `:`
    // were removed, so any delimiter now present came from NFKC.
    if !normalised.contains(['/', '?', '#', '@', ':']) {
        return Ok(());
    }
    Err(LocationError::new(format!(
        "The server name “{netloc}” contains characters that are not allowed in an address."
    )))
}

/// The host and port part of an authority: the text after the last `@`.
fn after_user_info(netloc: &str) -> &str {
    netloc.rsplit_once('@').map_or(netloc, |(_, host)| host)
}

/// Python's `str.partition` without the separator: the text before and
/// after the first `separator`, or all of `text` and an empty string.
fn partition(text: &str, separator: char) -> (&str, &str) {
    text.split_once(separator).unwrap_or((text, ""))
}

/// Python's wording for unbalanced or misplaced brackets.
fn invalid_ipv6_url() -> LocationError {
    LocationError::new("Invalid IPv6 URL")
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

    /// Splits a location the test knows to be valid.
    fn split(value: &str) -> LocationParts {
        split_location(value).expect("valid URL")
    }

    /// parity: DEV-005
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
    fn urls_are_split_like_python_urlsplit() {
        assert_eq!(
            split_location("smb://NAS/Team%20files"),
            Ok(parts("smb", "NAS", "/Team%20files"))
        );
        assert_eq!(split_location("file:///tmp/x"), Ok(parts("file", "", "/tmp/x")));
        assert_eq!(split_location("/tmp/a:b"), Ok(parts("", "", "/tmp/a:b")));
        assert_eq!(split_location("C:\\Windows"), Ok(parts("c", "", "\\Windows")));
        let with_query = split("smb://nas/a?b#c");
        assert_eq!(with_query.path, "/a");
        assert_eq!(with_query.query, "b");
        assert_eq!(with_query.fragment, "c");
        assert_eq!(split_location("mtp:foo"), Ok(parts("mtp", "", "foo")));
    }

    #[test]
    fn location_kinds_follow_the_scheme() {
        assert_eq!(split("file:///tmp").kind(), LocationKind::Local);
        assert_eq!(split("SMB://nas/share").kind(), LocationKind::Smb);
        for device in ["mtp://[usb:001,010]/", "gphoto2://[usb:001,002]/", "afc://id/"] {
            assert_eq!(split(device).kind(), LocationKind::Device, "{device}");
        }
        assert_eq!(split("trash:///").kind(), LocationKind::Other);
        assert_eq!(split("/tmp").kind(), LocationKind::Other);
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
        let smb = split("smb://User@NAS:0445/share");
        assert!(smb.has_credentials());
        assert_eq!(smb.hostname().as_deref(), Some("nas"));
        assert_eq!(smb.port(), Ok(Some(445)));
        let ipv6 = split("smb://[FE80::1]:139/");
        assert_eq!(ipv6.hostname().as_deref(), Some("fe80::1"));
        assert_eq!(ipv6.port(), Ok(Some(139)));
        assert_eq!(split("smb://nas:/x").port(), Ok(None));
        assert!(split("smb://nas:99999/").port().is_err());
        assert!(split("smb://nas:4x/").port().is_err());
        assert_eq!(split("smb:///share").hostname(), None);
    }

    #[test]
    fn ipv6_zones_keep_their_case() {
        let parts = split("smb://[FE80::1%Eth0]/share");
        assert_eq!(parts.hostname().as_deref(), Some("fe80::1%Eth0"));
    }

    #[test]
    fn path_depth_counts_folders_below_the_root() {
        assert_eq!(parts("smb", "nas", "/").path_depth(), 0);
        assert_eq!(parts("smb", "nas", "//").path_depth(), 0);
        assert_eq!(parts("smb", "nas", "/share/").path_depth(), 1);
        assert_eq!(parts("file", "", "/a//b").path_depth(), 2);
    }
}
