// SPDX-License-Identifier: AGPL-3.0-only
//! Turning typed or stored addresses into one canonical URI.
//!
//! Ports `normalise_location`, `_normalise_device_location`,
//! `require_share`, `require_item_uri` and `is_smb_server` from
//! `desktop/core.py`. The output is byte-for-byte what the Python app
//! produces, because both apps store these URIs in the shared
//! `settings.json` and compare them as strings.

use std::borrow::Cow;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use percent_encoding::percent_encode;

mod remote;

use remote::normalise_remote_url;

use super::device_uri::DeviceUriMatch;
use super::parts::{split_location, split_scheme, split_url, LocationKind, LocationParts};
use super::text::{
    contains_python_space, has_control_character, normalise_posix_path, python_strip, quote_component,
    quote_path, unquote_lossy, unquote_without_controls, PYTHON_PATH_SAFE,
};
use super::LocationError;

/// Longest accepted device authority, in characters (Python's `len()`).
const MAX_DEVICE_AUTHORITY_CHARS: usize = 512;

/// Accepts Linux paths (`/x`, `~`, `~/x`, or relative to `base`), `file://`
/// and `smb://` URIs, UNC paths (`\\server\share` or `//server/share`), the
/// other network protocols of [`REMOTE_SCHEMES`](super::REMOTE_SCHEMES) and
/// connected-device URIs (`mtp://`, `gphoto2://`, `afc://`), and returns one
/// canonical URI.
///
/// Canonical means: lower-case scheme and SMB host, no credentials, query
/// or fragment, `.`/`..` resolved, no trailing slash except at a root, and
/// the path escaped exactly as Python's `quote(path, safe='/')`. Escaped
/// `#`, `?` and `%` in a URL stay escaped; a plain path may contain them
/// literally.
///
/// A relative path joins onto an SMB or device `base`, onto the path of a
/// `file:` base, and otherwise onto `home`. Never runs a shell, expands
/// variables or maps Windows drive letters. Virtual places (`trash:///`,
/// `ox:home`, ...) are rejected here; see
/// [`normalise_navigation`](super::normalise_navigation).
///
/// # Errors
///
/// A [`LocationError`] with the Python app's message for empty input,
/// control characters, credentials, Windows drive letters, unsupported
/// schemes, a query or fragment in a URL, a `file://` URL with a host or a
/// relative path, an invalid SMB server name or port, or a malformed device
/// address.
pub fn normalise_location(address: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let address = python_strip(address);
    if address.is_empty() {
        return Err(LocationError::new("Enter a local folder path or an SMB address."));
    }
    // Safety rule (`core.py`: `CONTROL.search(value)`): no control
    // character in an address reaches GIO, a file name or settings.json.
    if has_control_character(address) {
        return Err(LocationError::new(
            "Control characters are not allowed in an address.",
        ));
    }
    let address = if is_unc_path(address) {
        Cow::Owned(unc_to_smb(address)?)
    } else {
        Cow::Borrowed(address)
    };
    if is_windows_drive_path(&address) {
        return Err(LocationError::new(
            "Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.",
        ));
    }
    let Some((scheme, _)) = split_scheme(&address) else {
        return normalise_plain_path(&address, base, home);
    };
    match LocationKind::from_scheme(&scheme) {
        LocationKind::Device => normalise_device_location(&address, &scheme),
        LocationKind::Remote => normalise_remote_url(&address),
        LocationKind::Local | LocationKind::Smb | LocationKind::Other => normalise_url(&address),
    }
}

/// [`normalise_location`] without a base, relative to the user's home
/// folder. The equivalent of calling the Python function with one argument.
///
/// # Errors
///
/// As [`normalise_location`].
pub fn normalise(address: &str) -> Result<String, LocationError> {
    normalise_location(address, None, &glib::home_dir())
}

/// The canonical `file://` URI of an absolute local path, exactly as
/// Python's `Path(path).as_uri()`. Unlike `gio::File::uri()`, it escapes
/// `(`, `)`, `!`, `'` and the other sub-delimiters, so use it (or
/// [`normalise`]) before comparing a GIO URI with a stored one.
pub fn file_uri(path: &Path) -> String {
    let escaped = percent_encode(path.as_os_str().as_bytes(), PYTHON_PATH_SAFE);
    format!("file://{escaped}")
}

/// Normalises a network folder for Map network location: an SMB shared
/// folder (not a bare server such as `smb://nas/`), a folder on an SFTP,
/// FTP or WebDAV server (its root included) or an NFS export.
///
/// # Errors
///
/// As [`normalise`], and a message asking for a shared folder such as
/// `\\nas\Projects` for anything else.
pub fn require_share(address: &str) -> Result<String, LocationError> {
    let uri = normalise(address)?;
    let parts = split_location(&uri)?;
    let is_folder = match parts.kind() {
        LocationKind::Smb => parts.path_depth() > 0,
        LocationKind::Remote => parts.scheme != "nfs" || parts.path_depth() > 0,
        LocationKind::Local | LocationKind::Device | LocationKind::Other => false,
    };
    if !is_folder {
        return Err(LocationError::new(
            "Enter a shared folder such as \\\\nas\\Projects, not only the server name.",
        ));
    }
    Ok(uri)
}

/// Normalises a location that is about to be renamed, moved, copied,
/// trashed or put on the clipboard. A whole SMB server or share, or a
/// device root, is not such an item.
///
/// # Errors
///
/// As [`normalise`], and a message telling the user to open the share or
/// the device storage first for a whole SMB server or share or a device
/// root.
pub fn require_item_uri(uri: &str) -> Result<String, LocationError> {
    let uri = normalise(uri)?;
    let parts = split_location(&uri)?;
    // Safety rule (`core.py::require_item_uri`): a whole server, share or
    // device is never renamed, moved, copied or trashed as if it were a
    // folder.
    if parts.is_smb() && parts.path_depth() <= 1 {
        return Err(LocationError::new(
            "Open the network share first, then select files or folders inside it. The share itself \
             cannot be renamed, moved, copied or trashed here.",
        ));
    }
    if parts.is_remote() && parts.path_depth() == 0 {
        return Err(LocationError::new(
            "Open a folder on the server first, then select files or folders inside it. The server itself \
             cannot be renamed, moved, copied or trashed here.",
        ));
    }
    if parts.is_device() && parts.path_depth() == 0 {
        return Err(LocationError::new(
            "Open the device storage first, then select files or folders inside it. The device itself \
             cannot be moved or copied.",
        ));
    }
    Ok(uri)
}

/// True for an SMB server listing (`smb://host/`): it holds shares, not
/// files users can create or delete. Invalid addresses are not servers.
pub fn is_smb_server(uri: &str) -> bool {
    let Ok(canonical) = normalise(uri) else {
        return false;
    };
    split_location(&canonical).is_ok_and(|parts| parts.is_smb() && parts.path_depth() == 0)
}

/// `\\server\share` or `//server/share`.
fn is_unc_path(address: &str) -> bool {
    address.starts_with("\\\\") || address.starts_with("//")
}

/// `\\NAS\Team files\Q3 #1` to `smb://NAS/Team%20files/Q3%20%231`. The host
/// is lower-cased later with every other SMB URL.
fn unc_to_smb(address: &str) -> Result<String, LocationError> {
    let forward = address.replace('\\', "/");
    let mut components = forward.trim_start_matches('/').split('/');
    let server = components.next().unwrap_or_default();
    // Safety rule (SAFE-010): credentials never enter an address, so they
    // cannot reach settings.json, a tab title or the clipboard.
    if server.is_empty() || server.contains(['@', ':']) || has_control_character(server) {
        return Err(LocationError::new(
            "Use a server name without credentials, for example \\\\nas\\share.",
        ));
    }
    let escaped_components: Vec<String> = components.map(quote_component).collect();
    Ok(format!("smb://{server}/{}", escaped_components.join("/")))
}

/// `^[A-Za-z]:[\\/]`, for example `C:\Windows`.
fn is_windows_drive_path(address: &str) -> bool {
    match address.as_bytes() {
        [letter, b':', b'\\' | b'/', ..] => letter.is_ascii_alphabetic(),
        _ => false,
    }
}

/// An address without a scheme: `~`, an absolute path, or a path relative
/// to `base` (SMB, device or local) or to `home`.
fn normalise_plain_path(address: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let expanded = expand_home(address, home);
    if expanded.starts_with('/') {
        return local_path_uri(&expanded);
    }
    if let Some(base) = base.filter(|base| is_remote_base(base)) {
        // Append the escaped relative path and validate the whole URI again.
        let joined = format!("{}/{}", base.trim_end_matches('/'), quote_path(&expanded));
        return normalise_location(&joined, None, home);
    }
    let base_path = match base {
        Some(base) if base.starts_with("file:") => {
            let base_parts = split_url(base)?;
            unquote_lossy(&base_parts.path)
        }
        _ => home.to_string_lossy().into_owned(),
    };
    local_path_uri(&join_path(&base_path, &expanded))
}

/// Expands `~` and `~/rest` like `str(home)` and `str(home / rest)`.
fn expand_home(address: &str, home: &Path) -> String {
    if address == "~" {
        return home.to_string_lossy().into_owned();
    }
    match address.strip_prefix("~/") {
        Some(rest) => join_path(&home.to_string_lossy(), rest),
        None => address.to_string(),
    }
}

/// True when relative paths should be appended to `base` as a URL: SMB
/// and connected-device folders.
fn is_remote_base(base: &str) -> bool {
    split_location(base).is_ok_and(|parts| parts.is_smb() || parts.is_remote() || parts.is_device())
}

/// `os.path.join(base, path)`: an absolute `path` replaces `base`.
fn join_path(base: &str, path: &str) -> String {
    if path.starts_with('/') || base.is_empty() {
        path.to_string()
    } else if base.ends_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// `Path(os.path.abspath(os.path.normpath(path))).as_uri()`.
fn local_path_uri(path: &str) -> Result<String, LocationError> {
    let normal = normalise_posix_path(path);
    let absolute = if normal.starts_with('/') {
        normal
    } else {
        // Only reachable with a relative home or base; resolve it like
        // `os.path.abspath` against the working directory.
        let current_folder = std::env::current_dir()
            .map_err(|error| LocationError::new(format!("Could not resolve the current folder: {error}")))?;
        normalise_posix_path(&join_path(&current_folder.to_string_lossy(), &normal))
    };
    Ok(format!("file://{}", quote_path(&absolute)))
}

/// A `file:` or `smb:` URL; any other scheme is rejected.
fn normalise_url(address: &str) -> Result<String, LocationError> {
    let parts = split_url(address)?;
    if !parts.is_local() && !parts.is_smb() {
        return Err(LocationError::new(
            "Only local paths, smb:// locations and connected devices are supported in this build.",
        ));
    }
    // Safety rule (SAFE-010): credentials never enter an address, so they
    // cannot reach settings.json, a tab title or the clipboard. Users sign
    // in through the sign-in dialog instead.
    if parts.has_credentials() {
        return Err(LocationError::new(
            "Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.",
        ));
    }
    if !parts.query.is_empty() || !parts.fragment.is_empty() {
        return Err(LocationError::query_or_fragment());
    }
    let decoded = unquote_without_controls(&parts.path)?;
    if parts.is_smb() {
        normalise_smb_url(&parts, &decoded)
    } else {
        normalise_file_url(&parts.authority, &decoded)
    }
}

/// A `file:` URL: no host but `localhost`, an absolute path, and the path
/// canonical.
fn normalise_file_url(authority: &str, decoded_path: &str) -> Result<String, LocationError> {
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return Err(LocationError::new(
            "For network folders, use smb://server/share rather than file://server/…",
        ));
    }
    if !decoded_path.starts_with('/') {
        return Err(LocationError::new("A file URL must contain an absolute path."));
    }
    let path = normalise_posix_path(decoded_path);
    Ok(format!("file://{}", quote_path(&path)))
}

/// An `smb:` URL with its authority and path canonical.
fn normalise_smb_url(parts: &LocationParts, decoded_path: &str) -> Result<String, LocationError> {
    let authority = smb_authority(parts)?;
    // SMB uses / in a URI; literal backslashes are path separators.
    let path = absolute_normal_path(&decoded_path.replace('\\', "/"));
    Ok(format!("smb://{authority}{}", quote_path(&path)))
}

/// The canonical `host[:port]` of an SMB URL: the host lower-cased, an
/// IPv6 host in brackets and an explicit port kept without leading zeros.
fn smb_authority(parts: &LocationParts) -> Result<String, LocationError> {
    server_authority(
        parts,
        "Enter an SMB server name, for example smb://nas/Projects.",
        "Invalid SMB port.",
    )
}

/// The canonical `host[:port]` of a server URL, as [`smb_authority`];
/// `no_host` and `bad_port` are the messages for a missing host and for
/// a port that is not a number.
fn server_authority(parts: &LocationParts, no_host: &str, bad_port: &str) -> Result<String, LocationError> {
    // Safety rule (`core.py`: `'%' in u.netloc or CONTROL.search(u.netloc)`):
    // an escaped server name could hide credentials (`u%40nas` is `u@nas`)
    // or a control character from the checks on the decoded address.
    if parts.authority.contains('%') || has_control_character(&parts.authority) {
        return Err(LocationError::new(
            "Use an unescaped server name without credentials or control characters.",
        ));
    }
    let Some(hostname) = parts.hostname().filter(|host| !contains_python_space(host)) else {
        return Err(LocationError::new(no_host));
    };
    let port = parts.port().map_err(|_| LocationError::new(bad_port))?;
    let host = if hostname.contains(':') {
        format!("[{hostname}]")
    } else {
        hostname
    };
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    Ok(authority)
}

/// Ports `_normalise_device_location`: keeps the device authority as
/// written (GIO needs it exactly) and canonicalises the path.
fn normalise_device_location(address: &str, scheme: &str) -> Result<String, LocationError> {
    let device = DeviceUriMatch::parse(address).filter(|device| device.scheme.eq_ignore_ascii_case(scheme));
    let Some(device) = device else {
        return Err(LocationError::new(
            "A connected-device address must include a device identifier and path.",
        ));
    };
    check_device_authority(device.authority)?;
    let decoded = unquote_without_controls(device.path)?;
    let path = absolute_normal_path(&decoded);
    Ok(format!("{scheme}://{}{}", device.authority, quote_path(&path)))
}

/// Rejects over-long identifiers, credentials, escapes, whitespace,
/// controls and brackets other than one enclosing pair.
fn check_device_authority(authority: &str) -> Result<(), LocationError> {
    let is_too_long = authority.chars().count() > MAX_DEVICE_AUTHORITY_CHARS;
    // `@` would carry credentials (SAFE-010).
    let has_forbidden_character = authority.contains(['@', '%'])
        || contains_python_space(authority)
        || has_control_character(authority);
    let has_stray_bracket = authority.contains(['[', ']']) && !is_one_bracketed_identifier(authority);
    if is_too_long || has_forbidden_character || has_stray_bracket {
        return Err(LocationError::new("Invalid connected-device identifier."));
    }
    Ok(())
}

/// `[usb:001,002]`: one `[` at the start, one `]` at the end, none inside.
fn is_one_bracketed_identifier(authority: &str) -> bool {
    let inner = authority
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'));
    inner.is_some_and(|inner| !inner.contains(['[', ']']))
}

/// `posixpath.normpath('/' + path.lstrip('/'))`: the canonical absolute
/// form of a decoded URL path.
fn absolute_normal_path(decoded_path: &str) -> String {
    normalise_posix_path(&format!("/{}", decoded_path.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/test")
    }

    /// The canonical URI of an address typed without a current folder.
    fn canonical(address: &str) -> Result<String, LocationError> {
        normalise_location(address, None, &home())
    }

    /// The canonical URI of `name` typed while `folder` is open.
    fn resolve(name: &str, folder: &str) -> Result<String, LocationError> {
        normalise_location(name, Some(folder), &home())
    }

    /// An address and the message that refuses it.
    struct RefusalCase {
        address: &'static str,
        message: &'static str,
    }

    /// Checks that [`canonical`] refuses each address with its message.
    fn assert_each_refused(cases: &[RefusalCase]) {
        for case in cases {
            let result = canonical(case.address);
            let refusal = result.as_deref().map_err(ToString::to_string);
            assert_eq!(refusal, Err(case.message.to_owned()), "{:?}", case.address);
        }
    }

    /// parity: NAV-034
    #[test]
    fn plain_paths_are_escaped_like_python() {
        assert_eq!(
            canonical("/tmp/Été #1?.txt").as_deref(),
            Ok("file:///tmp/%C3%89t%C3%A9%20%231%3F.txt")
        );
        assert_eq!(canonical("/tmp/a(1)!").as_deref(), Ok("file:///tmp/a%281%29%21"));
        assert_eq!(canonical("  /tmp/x/../y/  ").as_deref(), Ok("file:///tmp/y"));
        assert_eq!(canonical("/").as_deref(), Ok("file:///"));
        assert_eq!(canonical("~").as_deref(), Ok("file:///home/test"));
        assert_eq!(canonical("~//etc").as_deref(), Ok("file:///etc"));
        assert_eq!(canonical("~user").as_deref(), Ok("file:///home/test/~user"));
    }

    /// parity: NAV-034
    #[test]
    fn relative_paths_join_the_right_base() {
        assert_eq!(
            resolve("Plans", "file:///home/a").as_deref(),
            Ok("file:///home/a/Plans")
        );
        assert_eq!(
            resolve("x", "file:///tmp/a%20b/").as_deref(),
            Ok("file:///tmp/a%20b/x")
        );
        assert_eq!(resolve("..", "file:///home/a").as_deref(), Ok("file:///home"));
        assert_eq!(
            resolve("Next plan", "smb://nas/share").as_deref(),
            Ok("smb://nas/share/Next%20plan")
        );
        assert_eq!(
            resolve("../../x", "smb://nas/share/").as_deref(),
            Ok("smb://nas/x")
        );
        assert_eq!(
            resolve("a#b", "smb://nas/share").as_deref(),
            Ok("smb://nas/share/a%23b")
        );
        // Virtual and unknown bases fall back to the home folder.
        assert_eq!(
            resolve("Docs", "trash:///").as_deref(),
            Ok("file:///home/test/Docs")
        );
        assert_eq!(resolve("Docs", "ox:pc").as_deref(), Ok("file:///home/test/Docs"));
    }

    /// parity: NAV-034
    #[test]
    fn smb_urls_are_canonical() {
        assert_eq!(
            canonical("SMB://NAS/Projects/").as_deref(),
            Ok("smb://nas/Projects")
        );
        assert_eq!(canonical("smb://nas").as_deref(), Ok("smb://nas/"));
        assert_eq!(canonical("smb://nas:0445/a").as_deref(), Ok("smb://nas:445/a"));
        assert_eq!(canonical("smb://[FE80::1]/a").as_deref(), Ok("smb://[fe80::1]/a"));
        assert_eq!(canonical("smb://nas/a%5Cb").as_deref(), Ok("smb://nas/a/b"));
        assert_eq!(canonical("smb://nas/a?").as_deref(), Ok("smb://nas/a"));
        assert!(canonical("smb://nas:x/a").is_err());
        assert!(canonical("smb://my nas/a").is_err());
    }

    /// parity: NAV-034
    #[test]
    fn unc_paths_become_smb() {
        assert_eq!(
            canonical("\\\\NAS\\Team files\\Q3 #1").as_deref(),
            Ok("smb://nas/Team%20files/Q3%20%231")
        );
        assert_eq!(canonical("\\\\nas").as_deref(), Ok("smb://nas/"));
        assert_eq!(canonical("\\\\nas\\").as_deref(), Ok("smb://nas/"));
        for bad in ["\\\\", "\\\\u:p@nas\\share", "\\\\nas:445\\share"] {
            assert!(canonical(bad).is_err(), "{bad} should be rejected");
        }
    }

    /// The refusal of a scheme the app cannot open.
    const UNSUPPORTED: &str =
        "Only local paths, smb:// locations and connected devices are supported in this build.";
    /// The refusal of a `?` or `#` in a URL.
    const QUERY_OR_FRAGMENT: &str =
        "In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.";
    /// The refusal of credentials in a URL, which points to the sign-in dialog.
    const SIGN_IN: &str =
        "Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.";
    /// The refusal of credentials or a port in the server of a UNC path.
    const UNC_CREDENTIALS: &str = "Use a server name without credentials, for example \\\\nas\\share.";

    /// Each kind of invalid address and the Python app's guidance for it.
    const GUIDANCE: [RefusalCase; 13] = [
        RefusalCase {
            address: "   ",
            message: "Enter a local folder path or an SMB address.",
        },
        RefusalCase {
            address: "/tmp/a\u{0}",
            message: "Control characters are not allowed in an address.",
        },
        RefusalCase {
            address: "smb://nas/a%00b",
            message: "Encoded control characters are not allowed.",
        },
        RefusalCase {
            address: "C:\\Windows",
            message: "Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.",
        },
        RefusalCase {
            address: "http://example.org",
            message: UNSUPPORTED,
        },
        RefusalCase {
            address: "javascript:alert(1)",
            message: UNSUPPORTED,
        },
        RefusalCase {
            address: "smb://nas/a?b",
            message: QUERY_OR_FRAGMENT,
        },
        RefusalCase {
            address: "file:///tmp/x#y",
            message: QUERY_OR_FRAGMENT,
        },
        RefusalCase {
            address: "file://nas/share",
            message: "For network folders, use smb://server/share rather than file://server/…",
        },
        RefusalCase {
            address: "file:tmp",
            message: "A file URL must contain an absolute path.",
        },
        RefusalCase {
            address: "smb:///share",
            message: "Enter an SMB server name, for example smb://nas/Projects.",
        },
        RefusalCase {
            address: "smb://nas:x/a",
            message: "Invalid SMB port.",
        },
        RefusalCase {
            address: "afc:///DCIM",
            message: "A connected-device address must include a device identifier and path.",
        },
    ];

    /// A user name or password in each address form, and its refusal.
    const CREDENTIALS: [RefusalCase; 7] = [
        RefusalCase {
            address: "smb://u:p@nas/share",
            message: SIGN_IN,
        },
        RefusalCase {
            address: "smb://u@nas/share",
            message: SIGN_IN,
        },
        RefusalCase {
            address: "file://user@localhost/x",
            message: SIGN_IN,
        },
        RefusalCase {
            address: "\\\\u:p@nas\\share",
            message: UNC_CREDENTIALS,
        },
        RefusalCase {
            address: "//u@nas/share",
            message: UNC_CREDENTIALS,
        },
        RefusalCase {
            address: "smb://u%40nas/share",
            message: "Use an unescaped server name without credentials or control characters.",
        },
        RefusalCase {
            address: "mtp://user@device/DCIM",
            message: "Invalid connected-device identifier.",
        },
    ];

    /// Each kind of invalid address gets the Python app's guidance.
    ///
    /// parity: NAV-034, NAV-035, DEV-005
    #[test]
    fn invalid_addresses_are_refused_with_specific_guidance() {
        assert_each_refused(&GUIDANCE);
    }

    /// Every address form refuses a user name or password with the Python
    /// app's message, including a relative name typed inside a folder whose
    /// address carries one.
    ///
    /// parity: SAFE-010
    #[test]
    fn credentials_are_refused_in_every_address_form() {
        assert_each_refused(&CREDENTIALS);
        let inside_signed_in_folder = resolve("x", "smb://u@nas/a");
        let refusal = inside_signed_in_folder.as_deref().map_err(ToString::to_string);
        assert_eq!(refusal, Err(SIGN_IN.to_owned()));
    }

    /// parity: DEV-005
    #[test]
    fn device_uris_keep_their_authority() {
        assert_eq!(
            canonical("MTP://[usb:001,010]").as_deref(),
            Ok("mtp://[usb:001,010]/")
        );
        assert_eq!(
            canonical("afc://Device-ID/a/../b/").as_deref(),
            Ok("afc://Device-ID/b")
        );
        for bad in [
            "mtp://[usb]x/",
            "mtp://[[usb]]/",
            "mtp://a b/",
            "mtp://a%20b/",
            "mtp:///x",
            "mtp://user@device/DCIM",
            "mtp://[usb:001,002]/DCIM?mode=write",
            "mtp://[usb:001,002]/DCIM#top",
            "gphoto2://camera/a%01b",
        ] {
            assert!(canonical(bad).is_err(), "{bad} should be rejected");
        }
        assert!(canonical(&format!("mtp://{}/", "x".repeat(513))).is_err());
        assert!(canonical(&format!("mtp://{}/", "x".repeat(512))).is_ok());
    }

    #[test]
    fn file_uri_matches_path_as_uri() {
        assert_eq!(
            file_uri(Path::new("/tmp/Été #1?.txt")),
            "file:///tmp/%C3%89t%C3%A9%20%231%3F.txt"
        );
        assert_eq!(file_uri(Path::new("/")), "file:///");
        let invalid_utf8 = std::ffi::OsStr::from_bytes(b"/tmp/\xff");
        assert_eq!(file_uri(Path::new(invalid_utf8)), "file:///tmp/%FF");
    }

    #[test]
    fn smb_server_listings_are_detected() {
        assert!(is_smb_server("smb://nas/"));
        assert!(is_smb_server("\\\\nas"));
        assert!(!is_smb_server("smb://nas/work"));
        assert!(!is_smb_server("file:///"));
        assert!(!is_smb_server("http://nas/"));
    }
}
