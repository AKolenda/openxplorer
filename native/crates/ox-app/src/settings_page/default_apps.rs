// SPDX-License-Identifier: AGPL-3.0-only
//! Default apps: which app opens folders, SMB links and ZIP files, and
//! Show in folder from browsers.
//!
//! Ports the "Default file explorer" card of `renderSettingsPage`,
//! `renderDefaultStatus` and the Show in folder and ZIP controls of
//! `appendV07Settings` in `desktop/ui/app.js` (INT-030, SET-009), laid out
//! as the settings mockup's Default apps page: a status card that says
//! whether `OpenXplorer` is the default file explorer, what opens each
//! route, one row for Show in folder, and the guide as a page of its own
//! ([`super::troubleshooting`]).
//! Which app opens each route is read from GIO each time Settings opens,
//! off the main thread, and again with "Refresh status".
//!
//! Changing the defaults waits for the desktop integration milestone, so
//! those controls are shown disabled. The Python page's options of the
//! main button become controls of their own: "Also open ZIP files" and
//! "Use `OpenXplorer` for ZIPs" are the ZIP files switch, and "Include Show
//! in folder" is Enable in Show in folder from browsers; the Python labels
//! stay among the rows' keywords, so a search for them finds the rows. The
//! rarely used undo actions sit in Advanced.

use gtk::prelude::*;
use gtk::{gio, glib};

use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::parts;
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use super::troubleshooting::GUIDE_TITLE;
use super::SettingsPage;
use crate::icons::Icon;
use crate::window::{ButtonStyle, Milestone};

/// The Python app's desktop file, which the status calls `OpenXplorer`.
const OPENXPLORER_DESKTOP_ID: &str = "io.winspace.Development.desktop";

/// What a route shows before GIO has answered (`default-status`).
const CHECKING: &str = "Checking the current default…";

/// What the status card says before GIO has answered.
const CHECKING_TITLE: &str = "Checking the default file explorer…";

/// A kind of item other apps open, and the row that says which app does.
#[derive(Debug, Clone, Copy)]
struct Route {
    /// The content type GIO keeps the default app of.
    content_type: &'static str,
    /// The row's text.
    text: RowText,
}

const FOLDERS: Route = Route {
    content_type: "inode/directory",
    text: RowText {
        title: "Folders",
        description: "Double-clicking a folder in other apps.",
        keywords: "default file explorer manager nautilus dolphin zorin",
    },
};

const SMB_LINKS: Route = Route {
    content_type: "x-scheme-handler/smb",
    text: RowText {
        title: "SMB links",
        description: "smb:// links from browsers and chat apps.",
        keywords: "default network links share",
    },
};

const ZIP_FILES: Route = Route {
    content_type: "application/zip",
    text: RowText {
        title: "ZIP files",
        description: "Open ZIP archives in OpenXplorer instead of the archive manager.",
        keywords: "default archive zip file roller association use openxplorer for zips also open \
                   zip files in openxplorer changes the archive association separate from folder \
                   defaults opening a download is not show in folder",
    },
};

const BRAVE_AND_OTHER_APPS: RowText = RowText {
    title: "Brave and other apps",
    description: "\"Show in folder\" opens OpenXplorer. Runs for your user at login.",
    keywords: "include show in folder integration per-user starts at login reveal filemanager1 \
               download zorin test show in folder enable show in folder folder associations alone \
               do not control every browser route test checks filemanager1 not brave",
};

const TROUBLESHOOTING: RowText = RowText {
    title: "Troubleshooting",
    description: "Zorin and Brave setup steps, and how to undo each change.",
    keywords: "zorin + brave setup and troubleshooting help guide portal flatpak snap",
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

/// The Default apps page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::DefaultApps;
    let default_apps = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    let pending = Availability::Unported(Milestone::DesktopIntegration);
    let card = status_card();
    default_apps.append_card(&card);
    let (routes, handlers) = routes_group(&card, pending);
    default_apps.append_group(&routes);
    default_apps.append_group(&show_in_folder_group(pending));
    default_apps.append_group(&troubleshooting_group(page));
    default_apps.append_group(&advanced_group(pending));
    // Read when Settings opens, not when the window is built: most windows
    // never open Settings.
    let refresh = handlers.clone();
    page.when_opened(move || refresh.read());
    routes.add_heading_action(&refresh_button(handlers));
    default_apps
}

/// Whether `OpenXplorer` is the default file explorer, with the button
/// that makes it so.
fn status_card() -> StatusCard {
    let pending = Milestone::DesktopIntegration.notice();
    let make_default = parts::button("Make OpenXplorer default", ButtonStyle::Accent);
    make_default.set_sensitive(false);
    make_default.set_tooltip_text(Some(&pending));
    let status = StatusText {
        glyph: Icon::Apps,
        title: CHECKING_TITLE,
        text: "Open local folders and SMB links in OpenXplorer. System file-picker dialogs are \
               unchanged.",
        notice: Some(&pending),
    };
    StatusCard::new(status, &[make_default.upcast()])
}

/// Which app opens each route now, and where it is shown.
fn routes_group(card: &StatusCard, pending: Availability) -> (SettingsGroup, HandlerDisplay) {
    let group = SettingsGroup::new("What opens where");
    let (folders_row, folders) = route_row(FOLDERS);
    group.add_row(&folders_row);
    let (smb_links_row, smb_links) = route_row(SMB_LINKS);
    group.add_row(&smb_links_row);
    let zip_row = SettingRow::new(ZIP_FILES.text);
    zip_row.set_description(CHECKING);
    let zip_switch = parts::switch();
    zip_row.add_control(&zip_switch, ControlName::RowTitle);
    zip_row.set_availability(pending);
    group.add_row(&zip_row);
    let display = HandlerDisplay {
        card: card.downgrade(),
        folders: folders.downgrade(),
        smb_links: smb_links.downgrade(),
        zip_row: zip_row.downgrade(),
        zip_switch: zip_switch.downgrade(),
    };
    (group, display)
}

/// The row of `route` and the label that names the app opening it.
fn route_row(route: Route) -> (SettingRow, gtk::Label) {
    let row = SettingRow::new(route.text);
    let value = parts::value_label(CHECKING);
    row.add_control(&value, ControlName::RowTitle);
    (row, value)
}

/// "Refresh status", which reads the routes again.
fn refresh_button(handlers: HandlerDisplay) -> gtk::Button {
    let refresh = parts::button_with_glyph("Refresh status", Icon::ArrowClockwise);
    refresh.connect_clicked(move |_| handlers.read());
    refresh
}

/// The app GIO names as the default for a content type.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DefaultApp {
    /// Its desktop file, such as `org.gnome.Nautilus.desktop`.
    id: String,
    /// Its name, such as "Files".
    name: String,
}

impl DefaultApp {
    /// The default app of `content_type`, as GIO reads `mimeapps.list`.
    /// It reads files, so it runs off the main thread.
    fn of(content_type: &str) -> Option<Self> {
        let app = gio::AppInfo::default_for_type(content_type, false)?;
        let id = app.id()?.to_string();
        let name = app.display_name().to_string();
        Some(Self { id, name })
    }

    /// Whether it is the Python app, which the status calls `OpenXplorer`.
    fn is_openxplorer(&self) -> bool {
        self.id == OPENXPLORER_DESKTOP_ID
    }
}

/// How the status names the app opening a route: `OpenXplorer` for the
/// Python app, its name for any other (its desktop file when it has no
/// name), "Not set" without one (`renderDefaultStatus`).
fn handler_label(app: Option<&DefaultApp>) -> String {
    match app {
        None => "Not set".to_owned(),
        Some(app) if app.is_openxplorer() => "OpenXplorer".to_owned(),
        Some(app) if app.name.is_empty() => app.id.clone(),
        Some(app) => app.name.clone(),
    }
}

/// The app that opens each route now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Handlers {
    folders: Option<DefaultApp>,
    smb_links: Option<DefaultApp>,
    zip_files: Option<DefaultApp>,
}

impl Handlers {
    /// Reads them from GIO; blocking, so it runs off the main thread.
    fn read() -> Self {
        Self {
            folders: DefaultApp::of(FOLDERS.content_type),
            smb_links: DefaultApp::of(SMB_LINKS.content_type),
            zip_files: DefaultApp::of(ZIP_FILES.content_type),
        }
    }

    /// Whether `OpenXplorer` opens folders and SMB links, which is what
    /// the card's main button sets.
    fn is_default_file_explorer(&self) -> bool {
        let explorer_routes = [&self.folders, &self.smb_links];
        explorer_routes
            .into_iter()
            .all(|app| app.as_ref().is_some_and(DefaultApp::is_openxplorer))
    }

    /// The status card's title: the result, as the mockup states it.
    fn status_title(&self) -> &'static str {
        if self.is_default_file_explorer() {
            "OpenXplorer is your default file explorer"
        } else {
            "OpenXplorer isn't your default file explorer yet"
        }
    }

    /// The line under "ZIP files", naming the app that opens them now.
    fn zip_description(&self) -> String {
        match &self.zip_files {
            Some(app) if app.is_openxplorer() => "ZIP archives open in OpenXplorer.".to_owned(),
            Some(app) => format!(
                "Open ZIP archives in OpenXplorer instead of {}.",
                handler_label(Some(app))
            ),
            None => "Open ZIP archives in OpenXplorer; no app opens them now.".to_owned(),
        }
    }
}

/// The widgets that show which app opens each route. The page owns them,
/// so they are held weakly.
#[derive(Debug, Clone)]
struct HandlerDisplay {
    card: glib::WeakRef<StatusCard>,
    folders: glib::WeakRef<gtk::Label>,
    smb_links: glib::WeakRef<gtk::Label>,
    zip_row: glib::WeakRef<SettingRow>,
    zip_switch: glib::WeakRef<gtk::Switch>,
}

impl HandlerDisplay {
    /// Reads each route's default app off the main thread, where reading
    /// `mimeapps.list` cannot hold up the window, and shows them.
    fn read(&self) {
        let display = self.clone();
        glib::spawn_future_local(async move {
            let Ok(handlers) = gio::spawn_blocking(Handlers::read).await else {
                return;
            };
            display.show(&handlers);
        });
    }

    /// Shows `handlers` in the card and the rows still on screen.
    fn show(&self, handlers: &Handlers) {
        if let Some(card) = self.card.upgrade() {
            card.set_title(handlers.status_title());
        }
        if let Some(folders) = self.folders.upgrade() {
            folders.set_text(&handler_label(handlers.folders.as_ref()));
        }
        if let Some(smb_links) = self.smb_links.upgrade() {
            smb_links.set_text(&handler_label(handlers.smb_links.as_ref()));
        }
        if let Some(zip_row) = self.zip_row.upgrade() {
            zip_row.set_description(&handlers.zip_description());
        }
        if let Some(zip_switch) = self.zip_switch.upgrade() {
            let opens_zips = handlers
                .zip_files
                .as_ref()
                .is_some_and(DefaultApp::is_openxplorer);
            zip_switch.set_active(opens_zips);
        }
    }
}

/// "Brave and other apps", with Test and Enable (`revealTest`,
/// `revealEnable`). The mockup's status chip needs the owner of
/// `FileManager1`, which desktop integration reads, so it is left out
/// rather than guessed.
fn show_in_folder_group(pending: Availability) -> SettingsGroup {
    let group = SettingsGroup::pending("Show in folder from browsers", pending);
    let row = SettingRow::new(BRAVE_AND_OTHER_APPS);
    let test = parts::button("Test", ButtonStyle::Bordered);
    row.add_control(&test, ControlName::OwnLabel);
    let enable = parts::button("Enable", ButtonStyle::Accent);
    row.add_control(&enable, ControlName::OwnLabel);
    row.set_availability(pending);
    group.add_row(&row);
    group
}

/// The row whose chevron opens the Zorin and Brave guide.
fn troubleshooting_group(page: &SettingsPage) -> SettingsGroup {
    // The row names itself, as in the mockup.
    let group = SettingsGroup::new("");
    let row = SettingRow::new(TROUBLESHOOTING);
    let open = parts::chevron_button(GUIDE_TITLE);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn nautilus() -> DefaultApp {
        DefaultApp {
            id: "org.gnome.Nautilus.desktop".to_owned(),
            name: "Files".to_owned(),
        }
    }

    fn openxplorer() -> DefaultApp {
        DefaultApp {
            id: OPENXPLORER_DESKTOP_ID.to_owned(),
            name: "OpenXplorer (development)".to_owned(),
        }
    }

    #[test]
    fn the_status_names_openxplorer_and_says_when_nothing_is_set() {
        assert_eq!(handler_label(Some(&openxplorer())), "OpenXplorer");
        assert_eq!(handler_label(Some(&nautilus())), "Files");
        let unnamed = DefaultApp {
            name: String::new(),
            ..nautilus()
        };
        assert_eq!(handler_label(Some(&unnamed)), "org.gnome.Nautilus.desktop");
        assert_eq!(handler_label(None), "Not set");
    }

    #[test]
    fn the_card_says_openxplorer_is_the_default_only_for_folders_and_smb_links() {
        let everything = Handlers {
            folders: Some(openxplorer()),
            smb_links: Some(openxplorer()),
            zip_files: Some(nautilus()),
        };
        assert_eq!(
            everything.status_title(),
            "OpenXplorer is your default file explorer"
        );
        let folders_only = Handlers {
            smb_links: Some(nautilus()),
            ..everything
        };
        assert_eq!(
            folders_only.status_title(),
            "OpenXplorer isn't your default file explorer yet"
        );
        assert_eq!(
            Handlers::default().status_title(),
            "OpenXplorer isn't your default file explorer yet"
        );
    }

    #[test]
    fn the_zip_row_names_the_app_that_opens_zip_files_now() {
        let by_files = Handlers {
            zip_files: Some(nautilus()),
            ..Handlers::default()
        };
        assert_eq!(
            by_files.zip_description(),
            "Open ZIP archives in OpenXplorer instead of Files."
        );
        let by_openxplorer = Handlers {
            zip_files: Some(openxplorer()),
            ..Handlers::default()
        };
        assert_eq!(
            by_openxplorer.zip_description(),
            "ZIP archives open in OpenXplorer."
        );
    }

    #[test]
    fn the_routes_are_folders_smb_links_and_zip_files() {
        let content_types = [FOLDERS, SMB_LINKS, ZIP_FILES].map(|route| route.content_type);
        assert_eq!(
            content_types,
            ["inode/directory", "x-scheme-handler/smb", "application/zip"]
        );
    }
}
