// SPDX-License-Identifier: AGPL-3.0-only
//! Turning typed or stored addresses into one canonical URI.
//!
//! Ports `normalise_location`, `_normalise_device_location`,
//! `require_share`, `require_item_uri` and `is_smb_server` from
//! `desktop/core.py`. The output is byte-for-byte what the Python app
//! produces, because both apps store these URIs in the shared
//! `settings.json` and compare them as strings.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use percent_encoding::percent_encode;

use super::parts::{split_location, url_scheme, urlsplit, DeviceMatch};
use super::text::{
    contains_python_space, has_control_character, normpath, python_strip, quote_component, quote_path,
    unquote_lossy, unquote_without_controls, PYTHON_PATH_SAFE,
};
use super::{LocationError, DEVICE_SCHEMES};

/// Longest accepted device authority, in characters (Python's `len()`).
const MAX_DEVICE_AUTHORITY_CHARS: usize = 512;

/// Accepts Linux paths (`/x`, `~`, `~/x`, or relative to `base`), `file://`
/// and `smb://` URIs, UNC paths (`\\server\share` or `//server/share`) and
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
pub fn normalise_location(value: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let value = python_strip(value);
    if value.is_empty() {
        return Err(LocationError::new("Enter a local folder path or an SMB address."));
    }
    if has_control_character(value) {
        return Err(LocationError::new(
            "Control characters are not allowed in an address.",
        ));
    }
    let unc;
    let value = if value.starts_with("\\\\") || value.starts_with("//") {
        unc = unc_to_smb(value)?;
        unc.as_str()
    } else {
        value
    };
    if is_windows_drive_path(value) {
        return Err(LocationError::new(
            "Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.",
        ));
    }
    match url_scheme(value) {
        None => normalise_plain_path(value, base, home),
        Some((scheme, _)) if DEVICE_SCHEMES.contains(&scheme.as_str()) => {
            normalise_device_location(value, &scheme)
        }
        Some(_) => normalise_url(value),
    }
}

/// [`normalise_location`] without a base, relative to the user's home
/// folder. The equivalent of calling the Python function with one argument.
///
/// # Errors
///
/// As [`normalise_location`].
pub fn normalise(value: &str) -> Result<String, LocationError> {
    normalise_location(value, None, &glib::home_dir())
}

/// The canonical `file://` URI of an absolute local path, exactly as
/// Python's `Path(path).as_uri()`. Unlike `gio::File::uri()`, it escapes
/// `(`, `)`, `!`, `'` and the other sub-delimiters, so use it (or
/// [`normalise`]) before comparing a GIO URI with a stored one.
pub fn file_uri(path: &Path) -> String {
    let escaped = percent_encode(path.as_os_str().as_bytes(), PYTHON_PATH_SAFE);
    format!("file://{escaped}")
}

/// Normalises a shared folder, rejecting a bare server (`smb://nas/`) and
/// anything that is not SMB.
///
/// # Errors
///
/// As [`normalise`], and a message asking for a shared folder such as
/// `\\nas\Projects` for anything but an SMB shared folder.
pub fn require_share(value: &str) -> Result<String, LocationError> {
    let uri = normalise(value)?;
    let parts = split_location(&uri)?;
    if parts.scheme != "smb" || parts.path.trim_matches('/').is_empty() {
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
    let segment_count = parts
        .path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count();
    if parts.scheme == "smb" && segment_count <= 1 {
        return Err(LocationError::new(
            "Open the network share first, then select files or folders inside it. The share itself \
             cannot be renamed, moved, copied or trashed here.",
        ));
    }
    let is_device = DEVICE_SCHEMES.contains(&parts.scheme.as_str());
    if is_device && parts.path.trim_matches('/').is_empty() {
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
    match split_location(&canonical) {
        Ok(parts) => parts.scheme == "smb" && parts.path.trim_matches('/').is_empty(),
        Err(_) => false,
    }
}

/// `\\NAS\Team files\Q3 #1` to `smb://NAS/Team%20files/Q3%20%231`. The host
/// is lower-cased later with every other SMB URL.
fn unc_to_smb(value: &str) -> Result<String, LocationError> {
    let forward = value.replace('\\', "/");
    let mut components = forward.trim_start_matches('/').split('/');
    let server = components.next().unwrap_or_default();
    if server.is_empty() || server.contains(['@', ':']) || has_control_character(server) {
        return Err(LocationError::new(
            "Use a server name without credentials, for example \\\\nas\\share.",
        ));
    }
    let path: Vec<String> = components.map(quote_component).collect();
    Ok(format!("smb://{server}/{}", path.join("/")))
}

/// `^[A-Za-z]:[\\/]`, for example `C:\Windows`.
fn is_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/')
}

/// A value without a scheme: `~`, an absolute path, or a path relative to
/// `base` (SMB, device or local) or to `home`.
fn normalise_plain_path(value: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let expanded = expand_home(value, home);
    if expanded.starts_with('/') {
        return local_path_uri(&expanded);
    }
    if let Some(base) = base.filter(|base| is_remote_base(base)) {
        // Append the escaped relative path and validate the whole URI again.
        let joined = format!("{}/{}", base.trim_end_matches('/'), quote_path(&expanded));
        return normalise_location(&joined, None, home);
    }
    let base_path = match base {
        Some(base) if base.starts_with("file:") => unquote_lossy(&urlsplit(base)?.path),
        _ => home.to_string_lossy().into_owned(),
    };
    local_path_uri(&join_path(&base_path, &expanded))
}

/// Expands `~` and `~/rest` like `str(home)` and `str(home / rest)`.
fn expand_home(value: &str, home: &Path) -> String {
    if value == "~" {
        return home.to_string_lossy().into_owned();
    }
    match value.strip_prefix("~/") {
        Some(rest) => join_path(&home.to_string_lossy(), rest),
        None => value.to_string(),
    }
}

/// True when relative paths should be appended to `base` as a URL: SMB
/// and connected-device folders.
fn is_remote_base(base: &str) -> bool {
    match split_location(base) {
        Ok(parts) => parts.scheme == "smb" || DEVICE_SCHEMES.contains(&parts.scheme.as_str()),
        Err(_) => false,
    }
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
    let normal = normpath(path);
    let absolute = if normal.starts_with('/') {
        normal
    } else {
        // Only reachable with a relative home or base; resolve it like
        // `os.path.abspath` against the working directory.
        let cwd = std::env::current_dir()
            .map_err(|error| LocationError::new(format!("Could not resolve the current folder: {error}")))?;
        normpath(&join_path(&cwd.to_string_lossy(), &normal))
    };
    Ok(format!("file://{}", quote_path(&absolute)))
}

/// A `file:` or `smb:` URL; any other scheme is rejected.
fn normalise_url(value: &str) -> Result<String, LocationError> {
    let parts = urlsplit(value)?;
    if parts.scheme != "file" && parts.scheme != "smb" {
        return Err(LocationError::new(
            "Only local paths, smb:// locations and connected devices are supported in this build.",
        ));
    }
    if parts.has_credentials() {
        return Err(LocationError::new(
            "Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.",
        ));
    }
    if !parts.query.is_empty() || !parts.fragment.is_empty() {
        return Err(LocationError::new(
            "In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.",
        ));
    }
    let decoded = unquote_without_controls(&parts.path)?;
    if parts.scheme == "file" {
        normalise_file_url(&parts.netloc, &decoded)
    } else {
        normalise_smb_url(&parts, &decoded)
    }
}

fn normalise_file_url(netloc: &str, decoded_path: &str) -> Result<String, LocationError> {
    if !netloc.is_empty() && !netloc.eq_ignore_ascii_case("localhost") {
        return Err(LocationError::new(
            "For network folders, use smb://server/share rather than file://server/…",
        ));
    }
    if !decoded_path.starts_with('/') {
        return Err(LocationError::new("A file URL must contain an absolute path."));
    }
    Ok(format!("file://{}", quote_path(&normpath(decoded_path))))
}

fn normalise_smb_url(parts: &super::LocationParts, decoded_path: &str) -> Result<String, LocationError> {
    if parts.netloc.contains('%') || has_control_character(&parts.netloc) {
        return Err(LocationError::new(
            "Use an unescaped server name without credentials or control characters.",
        ));
    }
    let hostname = parts.hostname().filter(|host| !contains_python_space(host));
    let Some(hostname) = hostname else {
        return Err(LocationError::new(
            "Enter an SMB server name, for example smb://nas/Projects.",
        ));
    };
    let port = parts
        .port()
        .map_err(|_| LocationError::new("Invalid SMB port."))?;
    let host = if hostname.contains(':') {
        format!("[{hostname}]")
    } else {
        hostname
    };
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    // SMB uses / in a URI; literal backslashes are path separators.
    let forward = decoded_path.replace('\\', "/");
    let path = normpath(&format!("/{}", forward.trim_start_matches('/')));
    Ok(format!("smb://{authority}{}", quote_path(&path)))
}

/// Ports `_normalise_device_location`: keeps the device authority as
/// written (GIO needs it exactly) and canonicalises the path.
fn normalise_device_location(value: &str, scheme: &str) -> Result<String, LocationError> {
    let device = DeviceMatch::parse(value).filter(|device| device.scheme.eq_ignore_ascii_case(scheme));
    let Some(device) = device else {
        return Err(LocationError::new(
            "A connected-device address must include a device identifier and path.",
        ));
    };
    check_device_authority(device.authority)?;
    let decoded = unquote_without_controls(device.path)?;
    let path = normpath(&format!("/{}", decoded.trim_start_matches('/')));
    Ok(format!("{scheme}://{}{}", device.authority, quote_path(&path)))
}

/// Rejects credentials, escapes, whitespace, controls, over-long
/// identifiers and brackets other than one enclosing pair.
fn check_device_authority(authority: &str) -> Result<(), LocationError> {
    let invalid = || LocationError::new("Invalid connected-device identifier.");
    let too_long = authority.chars().count() > MAX_DEVICE_AUTHORITY_CHARS;
    if too_long
        || authority.contains(['@', '%'])
        || contains_python_space(authority)
        || has_control_character(authority)
    {
        return Err(invalid());
    }
    if authority.contains(['[', ']']) {
        let inner = authority
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'));
        let one_enclosing_pair = inner.is_some_and(|inner| !inner.contains(['[', ']']));
        if !one_enclosing_pair {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/test")
    }

    fn normal(value: &str) -> Result<String, LocationError> {
        normalise_location(value, None, &home())
    }

    #[test]
    fn plain_paths_are_escaped_like_python() {
        assert_eq!(
            normal("/tmp/Été #1?.txt").as_deref(),
            Ok("file:///tmp/%C3%89t%C3%A9%20%231%3F.txt")
        );
        assert_eq!(normal("/tmp/a(1)!").as_deref(), Ok("file:///tmp/a%281%29%21"));
        assert_eq!(normal("  /tmp/x/../y/  ").as_deref(), Ok("file:///tmp/y"));
        assert_eq!(normal("/").as_deref(), Ok("file:///"));
        assert_eq!(normal("~").as_deref(), Ok("file:///home/test"));
        assert_eq!(normal("~//etc").as_deref(), Ok("file:///etc"));
        assert_eq!(normal("~user").as_deref(), Ok("file:///home/test/~user"));
    }

    #[test]
    fn relative_paths_join_the_right_base() {
        let base = |value: &str, base: &str| normalise_location(value, Some(base), &home());
        assert_eq!(
            base("Plans", "file:///home/a").as_deref(),
            Ok("file:///home/a/Plans")
        );
        assert_eq!(
            base("x", "file:///tmp/a%20b/").as_deref(),
            Ok("file:///tmp/a%20b/x")
        );
        assert_eq!(base("..", "file:///home/a").as_deref(), Ok("file:///home"));
        assert_eq!(
            base("Next plan", "smb://nas/share").as_deref(),
            Ok("smb://nas/share/Next%20plan")
        );
        assert_eq!(base("../../x", "smb://nas/share/").as_deref(), Ok("smb://nas/x"));
        assert_eq!(
            base("a#b", "smb://nas/share").as_deref(),
            Ok("smb://nas/share/a%23b")
        );
        // Virtual and unknown bases fall back to the home folder.
        assert_eq!(base("Docs", "trash:///").as_deref(), Ok("file:///home/test/Docs"));
        assert_eq!(base("Docs", "ox:pc").as_deref(), Ok("file:///home/test/Docs"));
    }

    #[test]
    fn smb_urls_are_canonical() {
        assert_eq!(normal("SMB://NAS/Projects/").as_deref(), Ok("smb://nas/Projects"));
        assert_eq!(normal("smb://nas").as_deref(), Ok("smb://nas/"));
        assert_eq!(normal("smb://nas:0445/a").as_deref(), Ok("smb://nas:445/a"));
        assert_eq!(normal("smb://[FE80::1]/a").as_deref(), Ok("smb://[fe80::1]/a"));
        assert_eq!(normal("smb://nas/a%5Cb").as_deref(), Ok("smb://nas/a/b"));
        assert_eq!(normal("smb://nas/a?").as_deref(), Ok("smb://nas/a"));
        assert!(normal("smb://nas:x/a").is_err());
        assert!(normal("smb://my nas/a").is_err());
    }

    #[test]
    fn unc_paths_become_smb() {
        assert_eq!(
            normal("\\\\NAS\\Team files\\Q3 #1").as_deref(),
            Ok("smb://nas/Team%20files/Q3%20%231")
        );
        assert_eq!(normal("\\\\nas").as_deref(), Ok("smb://nas/"));
        assert_eq!(normal("\\\\nas\\").as_deref(), Ok("smb://nas/"));
        for bad in ["\\\\", "\\\\u:p@nas\\share", "\\\\nas:445\\share"] {
            assert!(normal(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn device_uris_keep_their_authority() {
        assert_eq!(
            normal("MTP://[usb:001,010]").as_deref(),
            Ok("mtp://[usb:001,010]/")
        );
        assert_eq!(
            normal("afc://Device-ID/a/../b/").as_deref(),
            Ok("afc://Device-ID/b")
        );
        for bad in [
            "mtp://[usb]x/",
            "mtp://[[usb]]/",
            "mtp://a b/",
            "mtp://a%20b/",
            "mtp:///x",
        ] {
            assert!(normal(bad).is_err(), "{bad} should be rejected");
        }
        assert!(normal(&format!("mtp://{}/", "x".repeat(513))).is_err());
        assert!(normal(&format!("mtp://{}/", "x".repeat(512))).is_ok());
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
    fn server_detection() {
        assert!(is_smb_server("smb://nas/"));
        assert!(is_smb_server("\\\\nas"));
        assert!(!is_smb_server("smb://nas/work"));
        assert!(!is_smb_server("file:///"));
        assert!(!is_smb_server("http://nas/"));
    }
}
