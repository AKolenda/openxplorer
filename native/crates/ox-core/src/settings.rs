// SPDX-License-Identifier: AGPL-3.0-only
//! Shared settings in `$XDG_CONFIG_HOME/winspace/settings.json`.
//!
//! Ports `Settings` in `desktop/core.py`. The Python application and this one
//! use the same file, so the protocol must match exactly: every mutation
//! takes an exclusive `flock` on `settings.lock`, re-reads the file, applies
//! the change, and atomically replaces the file with a private (0600) copy.
//! Reading validates against a whitelist and never fails: unreadable input
//! yields safe defaults plus a warning. The `winspace` directory name is a
//! compatibility contract; do not rename it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub uri: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// `details` or `grid`.
    pub view: String,
    /// Details pane visible.
    pub details: bool,
    pub show_hidden: bool,
    /// Percent: 80, 90, 100, 110, 125, 150, 175 or 200.
    pub text_size: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<u32>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            view: "details".into(),
            details: true,
            show_hidden: false,
            text_size: 100,
            sidebar_width: None,
        }
    }
}

/// The validated contents of `settings.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsData {
    pub pins: Vec<Bookmark>,
    pub shares: Vec<Bookmark>,
    pub hidden_quick: Vec<String>,
    pub quick_order: Vec<String>,
    pub preferences: Preferences,
}

pub struct Settings {
    directory: PathBuf,
    data: SettingsData,
    warning: Option<String>,
}

impl Settings {
    /// The default directory: `$XDG_CONFIG_HOME/winspace` or `~/.config/winspace`.
    pub fn default_directory() -> PathBuf {
        glib::user_config_dir().join("winspace")
    }

    /// Loads settings; never fails.
    pub fn open(directory: &Path) -> Self {
        let mut settings = Self {
            directory: directory.to_path_buf(),
            data: SettingsData::default(),
            warning: None,
        };
        settings.reload();
        settings
    }

    /// Re-reads the file.
    pub fn reload(&mut self) {
        let path = self.directory.join("settings.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => {
                let bookmarks = |key: &str| -> Vec<Bookmark> {
                    value[key]
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|i| serde_json::from_value(i.clone()).ok())
                                .collect()
                        })
                        .unwrap_or_default()
                };
                self.data.pins = bookmarks("pins");
                self.data.shares = bookmarks("shares");
                if let Ok(prefs) = serde_json::from_value::<Preferences>(value["preferences"].clone()) {
                    self.data.preferences = prefs;
                }
            }
            Err(err) => {
                self.warning = Some(format!(
                    "Could not fully read settings; using safe defaults. {err}"
                ))
            }
        }
    }

    pub fn data(&self) -> &SettingsData {
        &self.data
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
