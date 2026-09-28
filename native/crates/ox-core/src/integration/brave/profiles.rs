// SPDX-License-Identifier: AGPL-3.0-only
//! Finding the native Brave profiles of the user, and noticing Flatpak
//! and Snap installs of Brave, which only Brave's own settings can change.
//!
//! Ports `FLAVORS`, `PROFILE`, `BraveIntegration.profiles` and the
//! sandbox detection of `BraveIntegration.status` in
//! `desktop/brave_integration.py` (INT-019).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::preferences::read_preferences;

/// The folder of Brave's channels in the user's configuration folder.
const BRAVE_FOLDER: &str = "BraveSoftware";

/// The file of a profile's preferences.
const PREFERENCES_FILE: &str = "Preferences";

/// The longest profile name kept, in characters.
const MAX_PROFILE_NAME_CHARS: usize = 160;

/// The longest download folder kept, in characters.
const MAX_DOWNLOAD_PATH_CHARS: usize = 4096;

/// A release channel of Brave, each with its own profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraveChannel {
    /// The stable release.
    Stable,
    /// The beta release.
    Beta,
    /// The nightly release.
    Nightly,
}

impl BraveChannel {
    /// Every channel, in the order the Python app listed them.
    pub const ALL: [Self; 3] = [Self::Stable, Self::Beta, Self::Nightly];

    /// The channel's folder, which the dialog also shows as its flavour.
    pub fn folder_name(self) -> &'static str {
        match self {
            Self::Stable => "Brave-Browser",
            Self::Beta => "Brave-Browser-Beta",
            Self::Nightly => "Brave-Browser-Nightly",
        }
    }
}

/// A detected native Brave profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraveProfile {
    /// `<channel folder>:<profile folder>`, for example
    /// `Brave-Browser:Default`.
    pub id: String,
    /// The name the user gave the profile, or its folder name.
    pub name: String,
    /// The channel the profile belongs to.
    pub channel: BraveChannel,
    /// The profile's folder.
    pub directory: PathBuf,
    /// The download folder the profile uses; empty for Brave's default.
    pub download_path: String,
}

impl BraveProfile {
    /// The profile's preference file.
    pub fn preferences_path(&self) -> PathBuf {
        self.directory.join(PREFERENCES_FILE)
    }
}

/// A Brave install that runs in a sandbox, whose settings only Brave
/// itself can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxedBrave {
    /// Brave from Flathub (`com.brave.Browser`).
    Flatpak,
    /// Brave from the Snap Store.
    Snap,
}

impl SandboxedBrave {
    /// The name the dialog shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::Snap => "Snap",
        }
    }
}

/// The native profiles under `config_home`, channel by channel, each
/// channel's profiles in folder-name order.
///
/// Safety rule "only real, readable profiles are offered" (`profiles` in
/// `brave_integration.py`): symlinked channel or profile folders are
/// skipped, and so is a profile whose preferences are refused or
/// malformed.
pub(super) fn detect_profiles(config_home: &Path) -> Vec<BraveProfile> {
    let brave_folder = config_home.join(BRAVE_FOLDER);
    BraveChannel::ALL
        .into_iter()
        .flat_map(|channel| profiles_of(channel, &brave_folder.join(channel.folder_name())))
        .collect()
}

/// The sandboxed Brave installs in `home`.
pub(super) fn sandboxed_installs(home: &Path) -> Vec<SandboxedBrave> {
    let installs = [
        (SandboxedBrave::Flatpak, home.join(".var/app/com.brave.Browser")),
        (SandboxedBrave::Snap, home.join("snap/brave")),
    ];
    installs
        .into_iter()
        .filter(|(_, folder)| folder.exists())
        .map(|(install, _)| install)
        .collect()
}

/// The profiles in one channel's folder `root`.
fn profiles_of(channel: BraveChannel, root: &Path) -> Vec<BraveProfile> {
    if !is_real_folder(root) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|folder| is_profile_folder(folder))
        .collect();
    folders.sort();
    folders
        .iter()
        .filter_map(|folder| read_profile(channel, folder))
        .collect()
}

/// The profile in `folder`, or `None` if its preferences are unusable.
fn read_profile(channel: BraveChannel, folder: &Path) -> Option<BraveProfile> {
    let folder_name = folder.file_name()?.to_string_lossy().into_owned();
    let preferences = read_preferences(&folder.join(PREFERENCES_FILE)).ok()?;
    let name = optional_setting(&preferences.data, "profile", "name")
        .ok()?
        .map_or_else(|| folder_name.clone(), text_of);
    let download_path = optional_setting(&preferences.data, "download", "default_directory")
        .ok()?
        .map_or_else(String::new, text_of);
    Some(BraveProfile {
        id: format!("{}:{folder_name}", channel.folder_name()),
        name: name.chars().take(MAX_PROFILE_NAME_CHARS).collect(),
        channel,
        directory: folder.to_owned(),
        download_path: download_path.chars().take(MAX_DOWNLOAD_PATH_CHARS).collect(),
    })
}

/// A preference group that is not an object, which makes the profile
/// unusable, as Python's `AttributeError` did.
struct GroupNotAnObject;

/// `data[group][key]`, or `None` when the group or key is missing.
fn optional_setting<'a>(
    data: &'a serde_json::Map<String, Value>,
    group: &str,
    key: &str,
) -> Result<Option<&'a Value>, GroupNotAnObject> {
    let Some(group) = data.get(group) else {
        return Ok(None);
    };
    let group = group.as_object().ok_or(GroupNotAnObject)?;
    Ok(group.get(key))
}

/// A preference as text: a string as it is, anything else as JSON.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// True for a real (not symlinked) folder named `Default` or
/// `Profile <number>`.
fn is_profile_folder(folder: &Path) -> bool {
    let is_profile_name = folder
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_profile_name);
    is_profile_name && is_real_folder(folder)
}

/// `PROFILE` in `brave_integration.py`: `Default` or `Profile ` followed by
/// ASCII digits.
fn is_profile_name(name: &str) -> bool {
    if name == "Default" {
        return true;
    }
    name.strip_prefix("Profile ")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

/// True for a folder that is not a symlink.
fn is_real_folder(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_default_and_numbered_profiles_are_profile_names() {
        for name in ["Default", "Profile 1", "Profile 12"] {
            assert!(is_profile_name(name), "{name}");
        }
        for name in [
            "System Profile",
            "Guest Profile",
            "Profile ",
            "Profile x",
            "Profile 1 ",
            "default",
        ] {
            assert!(!is_profile_name(name), "{name}");
        }
    }
}
