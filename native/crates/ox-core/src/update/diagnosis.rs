// SPDX-License-Identifier: AGPL-3.0-only
//! The report `openxplorer --diagnose` prints: which build is installed
//! and which runs, the folder and ZIP associations, and who answers Show
//! in folder.
//!
//! Ports the `args.diagnose` branch of `main` in `desktop/winspace.py`,
//! key for key: `Session.status` from `runtime_guard.py`, with
//! `associations` (`DesktopIntegration.status`) and `showInFolder`
//! (`FileManagerBus.status` plus whether the session files are enabled).
//! It holds no file names or credentials, so it can be pasted into a bug
//! report.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{InstanceStatus, RuntimeIdentity};
use crate::integration::{BusStatus, DefaultsStatus, MimeType, APP_ID};

/// The whole report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    /// The installed build.
    pub installed: RuntimeIdentity,
    /// The unique bus name of the running instance, if one runs.
    pub owner: Option<String>,
    /// What the running instance reports, if anything.
    pub running: Option<RuntimeIdentity>,
    /// Whether the running instance is the installed build; `None` when
    /// nothing runs.
    pub matches: Option<bool>,
    /// An instance runs that reports no identity (a release from before
    /// `runtime-info`).
    pub legacy_process: bool,
    /// The default handlers, or why they could not be read.
    pub associations: Associations,
    /// Who answers Show in folder.
    pub show_in_folder: ShowInFolder,
}

/// The associations part: `DesktopIntegration.status()` of the Python app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Associations {
    /// The handlers as the desktop reports them.
    Read(AssociationStatus),
    /// Why they could not be read.
    Failed {
        /// The reason, in the app's words.
        error: String,
    },
}

/// The handlers and what Restore previous could put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssociationStatus {
    /// Each MIME type's current handler; empty when none.
    pub current: BTreeMap<&'static str, String>,
    /// The app opens folders.
    pub is_default: bool,
    /// The app opens folders and SMB links.
    pub all_default: bool,
    /// The app opens every ZIP type.
    pub zip_default: bool,
    /// A previous ZIP handler is recorded.
    pub can_restore_zip: bool,
    /// Any previous handler is recorded.
    pub can_restore: bool,
    /// The desktop ID the app registers under.
    pub app_id: &'static str,
}

impl From<&DefaultsStatus> for AssociationStatus {
    fn from(status: &DefaultsStatus) -> Self {
        Self {
            current: MimeType::ALL
                .into_iter()
                .map(|mime_type| (mime_type.as_str(), status.handler(mime_type).to_owned()))
                .collect(),
            is_default: status.is_default(),
            all_default: status.is_default_for_folder_types(),
            zip_default: status.is_zip_default(),
            can_restore_zip: status.can_restore_zip,
            can_restore: status.can_restore,
            app_id: APP_ID,
        }
    }
}

/// The Show in folder part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowInFolder {
    /// The unique bus name that owns `org.freedesktop.FileManager1`.
    pub owner: Option<String>,
    /// The owning process's name, when it can be read.
    pub owner_label: String,
    /// This app owns the service.
    pub owned_by_open_xplorer: bool,
    /// The session files that start the service are installed.
    pub enabled: bool,
}

impl ShowInFolder {
    /// The report of `bus`, with whether the session files are `enabled`.
    pub fn new(bus: BusStatus, enabled: bool) -> Self {
        Self {
            owner: bus.owner,
            owner_label: bus.owner_label,
            owned_by_open_xplorer: bus.is_owned_by_openxplorer,
            enabled,
        }
    }
}

impl Diagnosis {
    /// The report of `instance`, `associations` and `show_in_folder`.
    pub fn new(instance: InstanceStatus, associations: Associations, show_in_folder: ShowInFolder) -> Self {
        Self {
            matches: instance.matches(),
            legacy_process: instance.is_legacy_process(),
            installed: instance.installed,
            owner: instance.owner,
            running: instance.running,
            associations,
            show_in_folder,
        }
    }

    /// The report as indented JSON, as `json.dumps(report, indent=2)`
    /// printed it.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("the report is plain data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(build: &str) -> RuntimeIdentity {
        RuntimeIdentity {
            version: "2.1.0".to_owned(),
            protocol: super::super::RUNTIME_PROTOCOL,
            build: build.to_owned(),
        }
    }

    /// The report keeps the Python keys, compares the builds and names
    /// no file.
    ///
    /// parity: UPD-010
    #[test]
    fn the_report_keeps_the_python_keys_and_compares_the_builds() {
        let instance = InstanceStatus {
            installed: identity("new"),
            owner: Some(":1.42".to_owned()),
            running: Some(identity("old")),
        };
        let mut current = BTreeMap::new();
        current.insert(MimeType::Directory, APP_ID.to_owned());
        let defaults = DefaultsStatus {
            current,
            can_restore_zip: false,
            can_restore: true,
        };
        let bus = BusStatus {
            owner: Some(":1.7".to_owned()),
            owner_label: "nautilus".to_owned(),
            is_owned_by_openxplorer: false,
        };
        let report = Diagnosis::new(
            instance,
            Associations::Read(AssociationStatus::from(&defaults)),
            ShowInFolder::new(bus, true),
        );

        let json: serde_json::Value = serde_json::from_str(&report.to_json()).expect("valid JSON");
        assert_eq!(json["matches"], false);
        assert_eq!(json["legacyProcess"], false);
        assert_eq!(json["running"]["build"], "old");
        assert_eq!(json["associations"]["current"]["inode/directory"], APP_ID);
        assert_eq!(json["associations"]["isDefault"], true);
        assert_eq!(json["associations"]["canRestore"], true);
        assert_eq!(json["showInFolder"]["ownerLabel"], "nautilus");
        assert_eq!(json["showInFolder"]["ownedByOpenXplorer"], false);
        assert_eq!(json["showInFolder"]["enabled"], true);
    }
}
