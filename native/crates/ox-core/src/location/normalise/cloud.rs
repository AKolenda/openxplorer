// SPDX-License-Identifier: AGPL-3.0-only
//! Google Drive and `OneDrive` addresses (DEV-013), which the Python app did
//! not browse.
//!
//! GNOME Online Accounts names each drive by its account
//! (`google-drive://alice@example.com/`), and `GVfs` mounts the drive of
//! the account it finds by that name. Unlike an SFTP user name, the
//! account is the drive's address, so it stays in the address wherever
//! the address goes, as GNOME Files keeps it in its bookmarks. It is not a
//! secret: GNOME Online Accounts signs in, never the address.

use super::super::parts::{split_url, LocationParts};
use super::super::text::{
    contains_python_space, has_control_character, quote_path, unquote_without_controls,
};
use super::super::LocationError;
use super::absolute_normal_path;

/// A `google-drive:` or `onedrive:` URL: the scheme lower-case, the
/// account kept exactly as written, since `GVfs` looks the account up by
/// that text, and the path canonical as for SMB.
pub(super) fn normalise_cloud_url(address: &str) -> Result<String, LocationError> {
    let parts = split_url(address)?;
    if !parts.query.is_empty() || !parts.fragment.is_empty() {
        return Err(LocationError::query_or_fragment());
    }
    let account = cloud_account(&parts)?;
    let decoded = unquote_without_controls(&parts.path)?;
    let path = absolute_normal_path(&decoded);
    Ok(format!("{}://{account}{}", parts.scheme, quote_path(&path)))
}

/// The account of a cloud URL, `user@domain`, as GNOME Online Accounts
/// writes it.
///
/// # Errors
///
/// A message asking for the account when the authority is not a plain
/// `user@domain`: missing, escaped, with a password, a port, a space or a
/// control character.
fn cloud_account(parts: &LocationParts) -> Result<&str, LocationError> {
    let authority = parts.authority.as_str();
    let is_plain = |text: &str| {
        !text.is_empty()
            && !text.contains([':', '%', '@', '[', ']'])
            && !has_control_character(text)
            && !contains_python_space(text)
    };
    match authority.split_once('@') {
        Some((user, domain)) if is_plain(user) && is_plain(domain) => Ok(authority),
        _ => Err(LocationError::new(crate::i18n::gettext(
            "Enter the account of the drive, for example google-drive://alice@example.com/.",
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::{normalise_location, require_item_uri, require_share};
    use super::*;

    /// The canonical URI of an address typed without a current folder.
    fn canonical(address: &str) -> Result<String, LocationError> {
        normalise_location(address, None, Path::new("/home/test"))
    }

    /// A drive's address keeps its account exactly, so `GVfs` finds the
    /// account GNOME Online Accounts signed in.
    ///
    /// parity: DEV-013
    #[test]
    fn cloud_urls_keep_their_account() {
        let cases = [
            (
                "google-drive://alice@example.com/",
                "google-drive://alice@example.com/",
            ),
            (
                "Google-Drive://alice@example.com",
                "google-drive://alice@example.com/",
            ),
            (
                "google-drive://alice@example.com/0AIbX-_1/1cE//./x",
                "google-drive://alice@example.com/0AIbX-_1/1cE/x",
            ),
            (
                "onedrive://Alice.B+files@Example.com/Documents/",
                "onedrive://Alice.B+files@Example.com/Documents",
            ),
            (
                "onedrive://alice@example.com/My files/Q3 %231",
                "onedrive://alice@example.com/My%20files/Q3%20%231",
            ),
        ];
        for (typed, expected) in cases {
            assert_eq!(canonical(typed).as_deref(), Ok(expected), "{typed}");
        }
        for bad in [
            "google-drive:///",
            "google-drive://example.com/",
            "google-drive://@example.com/",
            "google-drive://alice@/",
            "google-drive://alice:secret@example.com/",
            "google-drive://alice@example.com:443/",
            "google-drive://a%40b@example.com/",
            "onedrive://alice@example.com/?x",
        ] {
            assert!(canonical(bad).is_err(), "{bad} should be rejected");
        }
        assert!(require_item_uri("google-drive://alice@example.com/").is_err());
        assert!(require_item_uri("onedrive://alice@example.com/Documents/a.txt").is_ok());
        assert!(
            require_share("onedrive://alice@example.com/Documents").is_err(),
            "a drive is not mapped as a network folder"
        );
    }
}
