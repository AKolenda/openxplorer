// SPDX-License-Identifier: AGPL-3.0-only
//! Default apps: which app opens folders, SMB links and ZIP files, and
//! Show in folder from browsers.
//!
//! Ports the "Default file explorer" card of `renderSettingsPage`,
//! `renderDefaultStatus` and the Show in folder and ZIP controls of
//! `appendV07Settings` in `desktop/ui/app.js` (INT-030, SET-009). Which
//! app opens each route is read from GIO now, off the main thread, and
//! again with "Refresh status" and each time Settings opens. Changing the
//! defaults waits for the desktop integration milestone; every Python
//! control is shown with its wording, the rarely used undo actions in
//! Advanced. The Zorin and Brave guide opens as a page of its own.

use gtk::prelude::*;
use gtk::{gio, glib};

use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::parts;
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use super::SettingsPage;
use crate::icons::Icon;
use crate::window::{ButtonStyle, Milestone};

/// The Python app's desktop file, which the status calls `OpenXplorer`.
const OPENXPLORER_DESKTOP_ID: &str = "io.winspace.Development.desktop";

/// What a route shows before GIO has answered (`default-status`).
const CHECKING: &str = "Checking the current default…";

/// A kind of item other apps open, and the row that says which app does.
#[derive(Debug, Clone, Copy)]
struct Route {
    /// The content type GIO keeps the default app of.
    content_type: &'static str,
    /// The row's text.
    text: RowText,
}

/// The routes of `renderDefaultStatus`, in its order.
const ROUTES: [Route; 3] = [
    Route {
        content_type: "inode/directory",
        text: RowText {
            title: "Folders",
            description: "Double-clicking a folder in other apps.",
            keywords: "default file explorer manager nautilus dolphin zorin",
        },
    },
    Route {
        content_type: "x-scheme-handler/smb",
        text: RowText {
            title: "SMB links",
            description: "smb:// links from browsers and chat apps.",
            keywords: "default network links share",
        },
    },
    Route {
        content_type: "application/zip",
        text: RowText {
            title: "ZIP files",
            description: "Opening a ZIP file from a browser or another app.",
            keywords: "default archive zip file roller",
        },
    },
];

const INCLUDE_SHOW_IN_FOLDER: RowText = RowText {
    title: "Include Show in folder",
    description: "Brave and other apps' Show in folder opens OpenXplorer (per-user, starts at \
                  login).",
    keywords: "brave reveal filemanager1 download integration",
};

const ALSO_OPEN_ZIPS: RowText = RowText {
    title: "Also open ZIP files in OpenXplorer",
    description: "Changes the archive association too.",
    keywords: "zip archive default",
};

const USE_FOR_ZIPS: RowText = RowText {
    title: "Use OpenXplorer for ZIPs",
    description: "Separate from folder defaults. Opening a download is not Show in folder.",
    keywords: "zip archive default association",
};

const SHOW_IN_FOLDER: RowText = RowText {
    title: "Show in folder",
    description: "Folder associations alone do not control every browser route. Test checks \
                  FileManager1, not Brave.",
    keywords: "brave reveal filemanager1 download zorin",
};

const TROUBLESHOOTING: RowText = RowText {
    title: "Zorin + Brave setup and troubleshooting",
    description: "Setup steps, and how to undo each change.",
    keywords: "help guide brave zorin portal flatpak snap",
};

const RESTORE_PREVIOUS: RowText = RowText {
    title: "Restore previous",
    description: "Restores the recorded file handlers and removes OpenXplorer's Show in folder \
                  files.",
    keywords: "undo default file explorer",
};

const RESTORE_ZIP_HANDLER: RowText = RowText {
    title: "Restore ZIP handler",
    description: "Gives ZIP files back to the app that opened them before.",
    keywords: "undo zip archive",
};

const DISABLE_SHOW_IN_FOLDER: RowText = RowText {
    title: "Disable Show in folder",
    description: "OpenXplorer stops answering Show in folder requests.",
    keywords: "undo brave reveal filemanager1",
};

/// The steps of the Python app's "Zorin + Brave setup and
/// troubleshooting", numbered again (the Python list had two fourth
/// steps) and pointing at where the controls are now.
const GUIDE: [&str; 7] = [
    "1. Click Make OpenXplorer default with Include Show in folder on. Both folder/SMB handlers and \
     the optional FileManager1 service are configured for your account.",
    "2. Close other file-manager windows. If Show in folder says waiting, log out of Zorin and log \
     back in. OpenXplorer does not terminate Files or Dolphin.",
    "3. Clicking a ZIP filename in Brave opens its ZIP handler, not your folder handler. Use \
     OpenXplorer for ZIPs in Default apps to change that association. Restart Brave after \
     changing it.",
    "4. Downloads → Show in folder is a different action. Test Show in folder checks FileManager1, \
     not Brave or its portal. The status must show OpenXplorer as the owner, not just enabled.",
    "5. Flatpak/Snap Brave or a remembered portal choice may still use another handler. In a \
     chooser, select OpenXplorer. Do not disable your desktop portal: file-picker dialogs remain \
     system dialogs.",
    "6. Pin the installed OpenXplorer folder icon to your Zorin panel. Right-click it → Open \
     windows… lists existing windows; New window creates another. Super+E is a separate system \
     keyboard shortcut.",
    "Restore previous removes OpenXplorer's unmodified per-user reveal/autostart files and restores \
     recorded file handlers. No system packages are removed.",
];

/// The Default apps page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::DefaultApps;
    let default_apps = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    let pending = Availability::Unported(Milestone::DesktopIntegration);
    default_apps.append_card(&status_card());
    default_apps.append_group(&routes_group(page));
    default_apps.append_group(&make_default_options_group(pending));
    default_apps.append_group(&button_group(
        "ZIP files",
        USE_FOR_ZIPS,
        &["Use OpenXplorer for ZIPs"],
        pending,
    ));
    let show_in_folder = ["Test Show in folder", "Enable Show in folder"];
    default_apps.append_group(&button_group(
        "Show in folder from browsers",
        SHOW_IN_FOLDER,
        &show_in_folder,
        pending,
    ));
    default_apps.append_group(&help_group(page));
    default_apps.append_group(&advanced_group(pending));
    default_apps
}

/// The Default file explorer card with its main action.
fn status_card() -> StatusCard {
    let pending = Milestone::DesktopIntegration.notice();
    let make_default = parts::button("Make OpenXplorer default", ButtonStyle::Accent);
    make_default.set_sensitive(false);
    make_default.set_tooltip_text(Some(&pending));
    let status = StatusText {
        glyph: Icon::Apps,
        title: "Default file explorer",
        text: "Open local folders and SMB links in OpenXplorer. System file-picker dialogs are \
               unchanged.",
        notice: Some(&pending),
    };
    StatusCard::new(status, &[make_default.upcast()])
}

/// Which app opens each route now, with "Refresh status".
fn routes_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("What opens where");
    let mut values = Vec::new();
    for route in ROUTES {
        let row = SettingRow::new(route.text);
        let value = parts::value_label(CHECKING);
        row.add_control(&value, ControlName::RowTitle);
        group.add_row(&row);
        values.push(value.downgrade());
    }
    let status = RouteStatus { values };
    status.read();
    let refresh = parts::button_with_glyph("Refresh status", Icon::ArrowClockwise);
    let read_on_click = status.clone();
    refresh.connect_clicked(move |_| read_on_click.read());
    page.when_opened(move || status.read());
    group.add_heading_action(&refresh);
    group
}

/// The labels that say which app opens each route, one per route of
/// [`ROUTES`].
#[derive(Debug, Clone)]
struct RouteStatus {
    values: Vec<glib::WeakRef<gtk::Label>>,
}

impl RouteStatus {
    /// Reads each route's default app off the main thread, where reading
    /// `mimeapps.list` cannot hold up the window, and shows it.
    fn read(&self) {
        let values = self.values.clone();
        glib::spawn_future_local(async move {
            let reading = gio::spawn_blocking(|| ROUTES.map(|route| default_app_id(route.content_type)));
            let Ok(desktop_ids) = reading.await else {
                return;
            };
            for (value, desktop_id) in values.iter().zip(desktop_ids) {
                if let Some(value) = value.upgrade() {
                    value.set_text(&handler_label(desktop_id.as_deref()));
                }
            }
        });
    }
}

/// The desktop file of the app that opens `content_type`, as GIO reads
/// `mimeapps.list`.
fn default_app_id(content_type: &str) -> Option<String> {
    let app = gio::AppInfo::default_for_type(content_type, false)?;
    app.id().map(String::from)
}

/// How the status names an app: `OpenXplorer` for the Python app, the
/// desktop file for any other, "Not set" without one
/// (`renderDefaultStatus`).
fn handler_label(desktop_id: Option<&str>) -> String {
    match desktop_id {
        None => "Not set".to_owned(),
        Some(OPENXPLORER_DESKTOP_ID) => "OpenXplorer".to_owned(),
        Some(desktop_id) => desktop_id.to_owned(),
    }
}

/// The two choices "Make `OpenXplorer` default" applies, starting as the
/// Python checkboxes do: Show in folder on, ZIP files off.
fn make_default_options_group(pending: Availability) -> SettingsGroup {
    let group = SettingsGroup::pending("When you make OpenXplorer the default", pending);
    let show_in_folder = parts::switch();
    show_in_folder.set_active(true);
    group.add_row(&pending_row(
        INCLUDE_SHOW_IN_FOLDER,
        &show_in_folder,
        ControlName::RowTitle,
        pending,
    ));
    let zip_files = parts::switch();
    group.add_row(&pending_row(
        ALSO_OPEN_ZIPS,
        &zip_files,
        ControlName::RowTitle,
        pending,
    ));
    group
}

/// A row saying `text` with `control`, waiting for `pending`.
fn pending_row(
    text: RowText,
    control: &impl IsA<gtk::Widget>,
    name: ControlName,
    pending: Availability,
) -> SettingRow {
    let row = SettingRow::new(text);
    row.add_control(control, name);
    row.set_availability(pending);
    row
}

/// A group of one row with `buttons`, all waiting for `pending`.
fn button_group(title: &str, text: RowText, buttons: &[&str], pending: Availability) -> SettingsGroup {
    let group = SettingsGroup::pending(title, pending);
    let row = SettingRow::new(text);
    for label in buttons {
        row.add_control(
            &parts::button(label, ButtonStyle::Bordered),
            ControlName::OwnLabel,
        );
    }
    row.set_availability(pending);
    group.add_row(&row);
    group
}

/// The row that opens the Zorin and Brave guide.
fn help_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Help");
    let row = SettingRow::new(TROUBLESHOOTING);
    let open = parts::page_button("Open");
    open.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.show_view(SettingsView::Subpage(Subpage::Troubleshooting))
    ));
    row.add_control(&open, ControlName::OwnLabel);
    group.add_row(&row);
    group
}

/// The undo actions, rarely needed, at the bottom.
fn advanced_group(pending: Availability) -> SettingsGroup {
    let group = SettingsGroup::pending("Advanced", pending);
    for text in [RESTORE_PREVIOUS, RESTORE_ZIP_HANDLER, DISABLE_SHOW_IN_FOLDER] {
        let row = SettingRow::new(text);
        row.add_control(
            &parts::button(text.title, ButtonStyle::Bordered),
            ControlName::OwnLabel,
        );
        row.set_availability(pending);
        group.add_row(&row);
    }
    group
}

/// The Zorin + Brave setup and troubleshooting page.
pub(super) fn build_troubleshooting() -> SettingsSection {
    let guide = SettingsSection::new(
        TROUBLESHOOTING.title,
        "Make OpenXplorer Zorin's file explorer and Brave's Show in folder, and undo it.",
        PageKind::Subpage,
    );
    let (restore, steps) = GUIDE.split_last().expect("the guide has steps");
    for step in steps {
        guide.append_text(&parts::paragraph(step));
    }
    guide.append_text(&parts::note(Icon::Info, restore));
    guide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_names_openxplorer_and_says_when_nothing_is_set() {
        assert_eq!(
            handler_label(Some("io.winspace.Development.desktop")),
            "OpenXplorer"
        );
        assert_eq!(
            handler_label(Some("org.gnome.Nautilus.desktop")),
            "org.gnome.Nautilus.desktop"
        );
        assert_eq!(handler_label(None), "Not set");
    }

    #[test]
    fn the_routes_are_folders_smb_links_and_zip_files() {
        let content_types = ROUTES.map(|route| route.content_type);
        assert_eq!(
            content_types,
            ["inode/directory", "x-scheme-handler/smb", "application/zip"]
        );
    }
}
