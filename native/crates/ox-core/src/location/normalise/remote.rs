// SPDX-License-Identifier: AGPL-3.0-only
//! SFTP, FTP, WebDAV and NFS addresses (NET-029), which the Python app did
//! not browse.

use super::super::parts::{canonical_remote_scheme, split_url, LocationParts};
use super::super::text::{
    contains_python_space, has_control_character, quote_path, unquote_without_controls,
};
use super::super::LocationError;
use super::{absolute_normal_path, server_authority};

/// An SFTP, FTP, WebDAV or NFS URL (NET-029): the scheme canonical (`ssh`
/// is `sftp`, `webdav` is `dav`), the host lower-cased, the port kept, and
/// the path canonical as for SMB.
///
/// Unlike SMB, a user name may name the account (`sftp://anna@build/`), as
/// Dolphin, Files and `GVfs` expect for SSH: it is not a secret. A password
/// never enters the address (SAFE-010), and NFS takes no user at all.
pub(super) fn normalise_remote_url(address: &str) -> Result<String, LocationError> {
    let parts = split_url(address)?;
    let scheme = canonical_remote_scheme(&parts.scheme);
    if !parts.query.is_empty() || !parts.fragment.is_empty() {
        return Err(LocationError::query_or_fragment());
    }
    let user = remote_user(&parts, scheme)?;
    let decoded = unquote_without_controls(&parts.path)?;
    let server = LocationParts {
        authority: after_user(&parts.authority).to_owned(),
        ..parts.clone()
    };
    let authority = server_authority(
        &server,
        "Enter a server name, for example sftp://server/folder.",
        "Invalid port.",
    )?;
    let path = absolute_normal_path(&decoded);
    let user = user.map(|user| format!("{user}@")).unwrap_or_default();
    Ok(format!("{scheme}://{user}{authority}{}", quote_path(&path)))
}

/// `uri` without the user name of an SFTP, FTP or WebDAV address
/// (`sftp://anna@build/srv` is `sftp://build/srv`), for what leaves the
/// session: settings.json, tab titles, the clipboard and GTK's recent
/// servers, which never hold a user name, as for SMB (SAFE-010). Other
/// addresses are returned as they are.
pub fn without_user(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_owned();
    };
    if !matches!(scheme, "sftp" | "ftp" | "ftps" | "dav" | "davs") {
        return uri.to_owned();
    }
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    format!("{scheme}://{}{path}", after_user(authority))
}

/// The user name of a remote URL, if any.
///
/// # Errors
///
/// The sign-in message for a password, an escaped or empty user name, or
/// any user name on NFS.
fn remote_user<'a>(parts: &'a LocationParts, scheme: &str) -> Result<Option<&'a str>, LocationError> {
    let Some((user, _)) = parts.authority.rsplit_once('@') else {
        return Ok(None);
    };
    let is_plain_name = !user.is_empty()
        && !user.contains([':', '%', '@'])
        && !has_control_character(user)
        && !contains_python_space(user);
    // Safety rule (SAFE-010): a password never reaches settings.json, a
    // tab title or the clipboard.
    if !is_plain_name || scheme == "nfs" {
        return Err(LocationError::new(
            "Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.",
        ));
    }
    Ok(Some(user))
}

/// The authority without its `user@`.
fn after_user(authority: &str) -> &str {
    authority.rsplit_once('@').map_or(authority, |(_, host)| host)
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

    /// The canonical URI of `name` typed while `folder` is open.
    fn resolve(name: &str, folder: &str) -> Result<String, LocationError> {
        normalise_location(name, Some(folder), Path::new("/home/test"))
    }

    /// Dolphin's other network protocols are typed and browsed like SMB.
    ///
    /// parity: NET-029, NET-030, NET-031, NET-032, NET-033
    #[test]
    fn sftp_ftp_webdav_and_nfs_urls_are_canonical() {
        let cases = [
            ("SFTP://Build-Host/home/anna/", "sftp://build-host/home/anna"),
            ("ssh://anna@build:2222/srv/../etc", "sftp://anna@build:2222/etc"),
            ("ftp://mirror.example", "ftp://mirror.example/"),
            (
                "ftps://files.example/pub/Q3 %231",
                "ftps://files.example/pub/Q3%20%231",
            ),
            ("webdav://cloud/remote.php/dav", "dav://cloud/remote.php/dav"),
            ("davs://anna@cloud.example/", "davs://anna@cloud.example/"),
            (
                "webdavs://cloud.example:8443/files",
                "davs://cloud.example:8443/files",
            ),
            ("nfs://NAS/export/media", "nfs://nas/export/media"),
        ];
        for (typed, expected) in cases {
            assert_eq!(canonical(typed).as_deref(), Ok(expected), "{typed}");
        }
        assert_eq!(
            resolve("Q3 plans", "sftp://build/srv").as_deref(),
            Ok("sftp://build/srv/Q3%20plans")
        );
        // Safety rule (SAFE-010): a password never enters an address.
        for bad in [
            "sftp://anna:secret@build/",
            "ftp://@host/",
            "dav://a%40b@cloud/",
            "nfs://anna@nas/export",
            "sftp:///home",
            "ftp://host/?x",
        ] {
            assert!(canonical(bad).is_err(), "{bad} should be rejected");
        }
        let refusal = |address: &str| canonical(address).map_err(|error| error.to_string());
        assert_eq!(refusal("sftp://build:abc/"), Err("Invalid port.".to_owned()));
        assert_eq!(
            refusal("ftp:///pub"),
            Err("Enter a server name, for example sftp://server/folder.".to_owned())
        );
        assert_eq!(
            require_share("sftp://build").as_deref(),
            Ok("sftp://build/"),
            "a server's root is a folder"
        );
        assert!(require_share("nfs://nas/").is_err(), "NFS needs an export");
        assert!(require_item_uri("sftp://build/").is_err());
        assert!(require_item_uri("sftp://build/a.txt").is_ok());
    }
}
