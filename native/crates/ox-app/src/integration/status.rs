// SPDX-License-Identifier: AGPL-3.0-only
//! What the Default apps settings show: which app opens each route, what
//! Restore can put back, and who answers Show in folder.
//!
//! Ports the `desktopStatus` branch of `dispatch` and `reveal_status` in
//! `v2.0.0:desktop/winspace.py`, and the status texts of `renderDefaultStatus`
//! in `v2.0.0:desktop/ui/app.js` (INT-010, INT-012, INT-016). Reading the status
//! only reads: `xdg-mime query`, the two session files and the bus
//! daemon, which is asked without starting any service.

use std::collections::BTreeMap;

use gtk::gio;
use gtk::prelude::*;
use ox_core::integration::{DefaultApps, DefaultsStatus, MimeType, RevealRegistration, APP_ID};

use super::{DesktopIntegration, MimeBackend};

/// The default handlers, with the name the desktop gives each one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefaultsReport {
    /// The handlers as `xdg-mime` reports them, and what Restore can put
    /// back.
    pub(crate) status: DefaultsStatus,
    /// The name of each handler's application, by desktop ID, where the
    /// desktop knows one.
    pub(crate) names: BTreeMap<String, String>,
}

impl DefaultsReport {
    /// How the status names the app that opens `mime_type`: `OpenXplorer`
    /// for the app itself, the app's name, its desktop ID when it has no
    /// name, or "Not set" (`renderDefaultStatus`).
    pub(crate) fn handler_label(&self, mime_type: MimeType) -> String {
        let handler = self.status.handler(mime_type);
        if handler.is_empty() {
            return "Not set".to_owned();
        }
        if handler == APP_ID {
            return "OpenXplorer".to_owned();
        }
        self.names
            .get(handler)
            .filter(|name| !name.is_empty())
            .map_or_else(|| handler.to_owned(), Clone::clone)
    }

    /// The line under "ZIP files" (`#zip-status`).
    pub(crate) fn zip_text(&self) -> String {
        if self.status.is_zip_default() {
            return "ZIP opening: OpenXplorer. This is separate from folder defaults.".to_owned();
        }
        let handler = self.status.handler(MimeType::Zip);
        let handler = if handler.is_empty() {
            "the desktop choice".to_owned()
        } else {
            self.handler_label(MimeType::Zip)
        };
        format!("ZIP opening uses {handler}. Opening a download is not Show in folder.")
    }
}

/// Who answers Show in folder (`reveal_status`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ShowInFolderStatus {
    /// The two session files are installed.
    pub(crate) is_enabled: bool,
    /// This app owns `org.freedesktop.FileManager1`.
    pub(crate) is_owned: bool,
    /// The process that owns the name otherwise, when known.
    pub(crate) owner_label: String,
}

impl ShowInFolderStatus {
    /// The status line of Show in folder (`#reveal-status`, INT-016).
    pub(crate) fn text(&self) -> String {
        if self.is_owned {
            return "Show in folder: OpenXplorer owns FileManager1. Browser portal routing is a \
                    separate check."
                .to_owned();
        }
        if !self.is_enabled {
            return "Show in folder: not enabled. Folder associations alone do not control every \
                    browser route."
                .to_owned();
        }
        let owner = if self.owner_label.is_empty() {
            "the current file manager"
        } else {
            &self.owner_label
        };
        format!(
            "Show in folder: enabled, waiting for {owner}. Close other file managers or log out and \
             back in."
        )
    }
}

/// Everything the Default apps page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegrationStatus {
    /// The handlers, or why they could not be read; a status error
    /// replaces the routes with its message (INT-030).
    pub(crate) defaults: Result<DefaultsReport, String>,
    /// Who answers Show in folder.
    pub(crate) show_in_folder: ShowInFolderStatus,
}

impl DesktopIntegration {
    /// Reads the status off the main thread. Only reads (INT-010).
    pub(crate) async fn status(&self) -> IntegrationStatus {
        let services = self.services();
        let defaults = services
            .defaults
            .run_in_background(read_defaults)
            .await
            .map_err(|error| error.to_string());
        let is_enabled = services
            .reveal
            .run_in_background(RevealRegistration::is_enabled)
            .await;
        let bus = self.file_manager_status().await;
        let show_in_folder = ShowInFolderStatus {
            is_enabled,
            is_owned: bus.is_owned_by_openxplorer,
            owner_label: bus.owner_label,
        };
        IntegrationStatus {
            defaults,
            show_in_folder,
        }
    }
}

/// Reads the handlers and the names the desktop gives them.
fn read_defaults(
    defaults: &DefaultApps<MimeBackend>,
) -> Result<DefaultsReport, ox_core::integration::DefaultAppsError> {
    let status = defaults.status()?;
    let handlers: Vec<&String> = status.current.values().collect();
    let names = gio::AppInfo::all()
        .into_iter()
        .filter_map(|application| Some((application.id()?.to_string(), application)))
        .filter(|(id, _)| handlers.contains(&id))
        .map(|(id, application)| (id, application.display_name().to_string()))
        .collect();
    Ok(DefaultsReport { status, names })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(handlers: &[(MimeType, &str)]) -> DefaultsReport {
        let current = handlers
            .iter()
            .map(|(mime_type, handler)| (*mime_type, (*handler).to_owned()))
            .collect();
        let status = DefaultsStatus {
            current,
            can_restore_zip: false,
            can_restore: false,
        };
        let names = BTreeMap::from([("org.gnome.Nautilus.desktop".to_owned(), "Files".to_owned())]);
        DefaultsReport { status, names }
    }

    /// Ported from `renderDefaultStatus` in `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: INT-010
    #[test]
    fn a_route_names_openxplorer_the_app_its_id_or_nothing() {
        let routes = report(&[
            (MimeType::Directory, APP_ID),
            (MimeType::SmbLink, "org.gnome.Nautilus.desktop"),
            (MimeType::Zip, "org.gnome.FileRoller.desktop"),
            (MimeType::XZip, ""),
        ]);
        assert_eq!(routes.handler_label(MimeType::Directory), "OpenXplorer");
        assert_eq!(routes.handler_label(MimeType::SmbLink), "Files");
        assert_eq!(
            routes.handler_label(MimeType::Zip),
            "org.gnome.FileRoller.desktop"
        );
        assert_eq!(routes.handler_label(MimeType::XZip), "Not set");
    }

    /// parity: INT-012
    #[test]
    fn the_zip_line_says_whether_openxplorer_opens_zip_files() {
        let all_zip_types = MimeType::ZIP_TYPES.map(|mime_type| (mime_type, APP_ID));
        assert_eq!(
            report(&all_zip_types).zip_text(),
            "ZIP opening: OpenXplorer. This is separate from folder defaults."
        );
        assert_eq!(
            report(&[(MimeType::Zip, "org.gnome.Nautilus.desktop")]).zip_text(),
            "ZIP opening uses Files. Opening a download is not Show in folder."
        );
        assert_eq!(
            report(&[]).zip_text(),
            "ZIP opening uses the desktop choice. Opening a download is not Show in folder."
        );
    }

    /// A status and the line Show in folder shows for it.
    struct ShowInFolderCase {
        status: ShowInFolderStatus,
        text: &'static str,
    }

    /// Ported from the `#reveal-status` texts of `renderDefaultStatus`.
    ///
    /// parity: INT-016
    #[test]
    fn show_in_folder_says_who_answers_it() {
        let cases = [
            ShowInFolderCase {
                status: ShowInFolderStatus {
                    is_enabled: true,
                    is_owned: true,
                    owner_label: "OpenXplorer".to_owned(),
                },
                text: "Show in folder: OpenXplorer owns FileManager1. Browser portal routing is a \
                       separate check.",
            },
            ShowInFolderCase {
                status: ShowInFolderStatus {
                    is_enabled: true,
                    is_owned: false,
                    owner_label: "nautilus".to_owned(),
                },
                text: "Show in folder: enabled, waiting for nautilus. Close other file managers or log \
                       out and back in.",
            },
            ShowInFolderCase {
                status: ShowInFolderStatus {
                    is_enabled: true,
                    ..ShowInFolderStatus::default()
                },
                text: "Show in folder: enabled, waiting for the current file manager. Close other file \
                       managers or log out and back in.",
            },
            ShowInFolderCase {
                status: ShowInFolderStatus::default(),
                text: "Show in folder: not enabled. Folder associations alone do not control every \
                       browser route.",
            },
        ];
        for case in cases {
            assert_eq!(case.status.text(), case.text, "{:?}", case.status);
        }
    }
}
