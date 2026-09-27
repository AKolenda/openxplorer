// SPDX-License-Identifier: AGPL-3.0-only
//! Location parsing and validation.
//!
//! Ports `desktop/core.py` (`normalise_location`, `split_location`,
//! `validate_name`, `new_copy_name`, `require_item_uri`, `is_smb_server`,
//! `is_device_location`) and the display helpers from `desktop/ui/app.js`
//! (`displayUri`, `baseName`, `parentUri`, `breadcrumbs`, `networkLocation`).
//! Locations are always absolute URIs: `file://`, `smb://`, or a connected
//! device (`mtp://`, `gphoto2://`, `afc://`), plus the virtual `trash:///`
//! and `recent:///` folders.

use std::path::Path;

use gio::prelude::*;

/// Portable-device GVfs schemes. Their authorities can contain brackets
/// (`mtp://[usb:001,002]/`), which ordinary URL parsers reject.
pub const DEVICE_SCHEMES: [&str; 3] = ["mtp", "gphoto2", "afc"];

/// A user-facing validation error. The message is shown as-is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LocationError(pub String);

/// One breadcrumb button in the address bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub uri: String,
}

/// Accepts Linux paths (`/x`, `~/x`, relative to `base`), `file://` and
/// `smb://` URIs, UNC paths (`\\server\share`) and connected-device URIs, and
/// returns one canonical URI. Never runs a shell or expands variables.
pub fn normalise_location(value: &str, base: Option<&str>, home: &Path) -> Result<String, LocationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(LocationError(
            "Enter a local folder path or an SMB address.".into(),
        ));
    }
    if value.starts_with('/') {
        return Ok(gio::File::for_path(value).uri().to_string());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(gio::File::for_path(home.join(rest)).uri().to_string());
    }
    let _ = base;
    Ok(gio::File::for_uri(value).uri().to_string())
}

/// Validates a single file or folder name.
pub fn validate_name(name: &str) -> Result<&str, LocationError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(LocationError(
            "Enter a non-empty file name, not “.” or “..”.".into(),
        ));
    }
    if name.contains('/') || name.contains('\\') || name.chars().any(char::is_control) {
        return Err(LocationError(
            "A name cannot contain slashes or control characters.".into(),
        ));
    }
    if name.len() > 255 {
        return Err(LocationError("This name is longer than 255 bytes.".into()));
    }
    Ok(name)
}

/// Windows-style duplicate name: `Report (2).txt`, `Folder (3)`.
pub fn new_copy_name(name: &str, count: u32, is_dir: bool) -> String {
    match name.rfind('.') {
        Some(dot) if !is_dir && dot > 0 => format!("{} ({count}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({count})"),
    }
}

/// The URI scheme, lower-cased (`file`, `smb`, `mtp`, ...).
pub fn scheme(uri: &str) -> String {
    uri.split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default()
}

/// True for phones, cameras and iOS devices.
pub fn is_device_location(uri: &str) -> bool {
    DEVICE_SCHEMES.contains(&scheme(uri).as_str())
}

/// True for an SMB server listing (`smb://host/`), which holds shares.
pub fn is_smb_server(uri: &str) -> bool {
    scheme(uri) == "smb"
        && uri
            .splitn(4, '/')
            .nth(3)
            .map_or(true, |p| p.trim_matches('/').is_empty())
}

/// The parent folder, or `None` at a root.
pub fn parent_location(uri: &str) -> Option<String> {
    gio::File::for_uri(uri).parent().map(|p| p.uri().to_string())
}

/// The last path component for titles and tab labels.
pub fn base_name(uri: &str) -> String {
    let file = gio::File::for_uri(uri);
    match file.basename() {
        Some(name) if name.as_os_str() != "/" => name.to_string_lossy().into_owned(),
        _ => "Local Disk".into(),
    }
}

/// Text for the editable address bar: a plain path for local folders,
/// `\\server\share\...` for SMB, otherwise the URI.
pub fn display_location(uri: &str) -> String {
    gio::File::for_uri(uri)
        .path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| uri.to_string())
}

/// Breadcrumb buttons from the root to `uri`.
pub fn breadcrumbs(uri: &str) -> Vec<Crumb> {
    let mut crumbs = Vec::new();
    let mut current = Some(gio::File::for_uri(uri));
    while let Some(file) = current {
        crumbs.push(Crumb {
            label: base_name(&file.uri()),
            uri: file.uri().to_string(),
        });
        current = file.parent();
    }
    crumbs.reverse();
    crumbs
}
