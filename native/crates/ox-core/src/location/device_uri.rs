// SPDX-License-Identifier: AGPL-3.0-only
//! Matching GIO's portable-device URIs (`mtp://[usb:001,002]/DCIM`), whose
//! bracketed bus identifiers ordinary URL parsers refuse as malformed IPv6
//! hosts.
//!
//! Ports `DEVICE_URI` in `desktop/core.py`, which `split_location`,
//! `normalise_location` and the device labels try before the ordinary URL
//! rules.

use super::parts::{is_scheme, LocationParts};

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
    /// Matches any scheme; callers check that the scheme names a
    /// [`LocationKind::Device`].
    pub(crate) fn parse(uri: &'a str) -> Option<Self> {
        let (scheme, after_scheme) = uri.split_once("://")?;
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
            authority: self.authority.to_string(),
            path: self.path.to_string(),
            query: String::new(),
            fragment: String::new(),
        }
    }
}
