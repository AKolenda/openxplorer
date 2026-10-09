// SPDX-License-Identifier: AGPL-3.0-only
//! Default apps: which app opens folders, SMB links and ZIP files, making
//! `OpenXplorer` the default file explorer and undoing it, and Show in
//! folder from browsers.
//!
//! Ports the "Default file explorer" card of `renderSettingsPage`,
//! `renderDefaultStatus`, `changeDefault`, `changeZipDefault` and the Show
//! in folder and ZIP controls of `appendV07Settings` in
//! `v2.0.0:desktop/ui/app.js` (INT-008 to INT-016, INT-030, SET-009), laid out as
//! the settings mockup's Default apps page: a status card that says
//! whether `OpenXplorer` is the default file explorer, with "Make
//! `OpenXplorer` default"; the two options that go with it; what opens each
//! route, Show in folder and the file dialogs with their status lines;
//! Brave's downloads ([`super::brave`]); and, folded away, the undo
//! actions and the guide, a page of its own ([`super::troubleshooting`]).
//! The ZIP route is shown on the ZIP & archives page.
//!
//! The status is read each time Settings opens, after every change, when
//! "Refresh status" is clicked and when the `FileManager1` name changes
//! owner, always off the main thread ([`view`]). Nothing changes until
//! the user clicks: reading only reads (INT-010).

#[cfg(test)]
mod tests;
mod view;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use super::troubleshooting::GUIDE_TITLE;
use super::{SettingsPage, SharedHandler};
use crate::icons::Icon;
use crate::window::ButtonStyle;

use view::DefaultAppsView;

/// What a route shows before the desktop has answered (`default-status`).
const CHECKING: &str = crate::i18n::message_id("Checking the current default…");

/// What the status card says before the desktop has answered.
const CHECKING_TITLE: &str = crate::i18n::message_id("Checking the default file explorer…");

const INCLUDE_SHOW_IN_FOLDER: RowText = RowText {
    title: "Also handle “Show in folder”",
    description: "Brave's and other apps' Show in folder opens OpenXplorer. Per-user, starts at login.",
    keywords: "include show in folder include brave / other apps’ show in folder integration (per-user, starts at login) make \
               default option",
};

const ALSO_ZIP_FILES: RowText = RowText {
    title: "Also open ZIP files",
    description: "Make default also opens ZIP archives in OpenXplorer (changes the archive association).",
    keywords: "also open zip files in openxplorer (changes the archive association) make default option",
};

const FOLDERS: RowText = RowText {
    title: "Folders",
    description: "Double-clicking a folder in other apps.",
    keywords: "default file explorer manager nautilus dolphin zorin",
};

const SMB_LINKS: RowText = RowText {
    title: "Network (SMB) links",
    description: "smb:// links from browsers and chat apps.",
    keywords: "smb links default network links share",
};

const ZIP_FILES: RowText = RowText {
    title: "Open ZIP files from other apps with OpenXplorer",
    description: "Open ZIP archives in OpenXplorer instead of the archive manager.",
    keywords: "zip files default archive zip file roller association use openxplorer for zips changes the \
               archive association separate from folder defaults opening a download is not show in \
               folder",
};

const BRAVE_AND_OTHER_APPS: RowText = RowText {
    title: "“Show in folder” from browsers and apps",
    description: "\"Show in folder\" opens OpenXplorer. Runs for your user at login.",
    keywords: "brave and other apps show in folder from browsers include show in folder \
               integration per-user starts at login reveal filemanager1 \
               download zorin test show in folder enable show in folder folder associations alone \
               do not control every browser route test checks filemanager1 not brave",
};

const FILE_DIALOGS: RowText = RowText {
    title: "Other apps' Open and Save dialogs",
    description: "Other apps choose and save files in an OpenXplorer window.",
    keywords: "apps' open and save dialogs file picker file chooser open save save as upload download dialog portal \
               xdg-desktop-portal filechooser chrome firefox use openxplorer for open and save \
               dialogs apply now restart portal",
};

const TROUBLESHOOTING: RowText = RowText {
    title: "Setup help for Zorin and Brave",
    description: "Zorin and Brave setup steps, and how to undo each change.",
    keywords: "troubleshooting zorin + brave setup and troubleshooting help guide portal flatpak \
               snap",
};

const RESTORE_PREVIOUS: RowText = RowText {
    title: "Restore the previous file handlers",
    description: "Restores the recorded file handlers and removes OpenXplorer's Show in folder \
                  files.",
    keywords: "restore previous undo default file explorer",
};

const RESTORE_ZIP_HANDLER: RowText = RowText {
    title: "Give ZIP files back to the previous app",
    description: "Gives ZIP files back to the app that opened them before.",
    keywords: "restore zip handler undo zip archive",
};

const RESTORE_FILE_DIALOGS: RowText = RowText {
    title: "Turn off OpenXplorer's Open and Save dialogs",
    description: "Gives other apps' Open and Save dialogs back to the desktop.",
    keywords: "restore open and save dialogs undo file picker file chooser portal save dialog",
};

const DISABLE_SHOW_IN_FOLDER: RowText = RowText {
    title: "Turn off Show in folder",
    description: "OpenXplorer stops answering Show in folder requests.",
    keywords: "disable show in folder undo brave reveal filemanager1",
};

/// The controls the page updates when the status is read, made before
/// the rows that hold them.
#[derive(Debug)]
struct Controls {
    card: StatusCard,
    make_default: gtk::Button,
    include_show_in_folder: gtk::Switch,
    include_zip: gtk::Switch,
    folders: gtk::Label,
    smb_links: gtk::Label,
    zip_files: gtk::Label,
    zip_row: SettingRow,
    use_for_zips: gtk::Button,
    show_in_folder_row: SettingRow,
    test_show_in_folder: gtk::Button,
    enable_show_in_folder: gtk::Button,
    restore_previous: gtk::Button,
    restore_zip: gtk::Button,
    disable_show_in_folder: gtk::Button,
    file_dialogs_row: SettingRow,
    enable_file_dialogs: gtk::Button,
    apply_file_dialogs: gtk::Button,
    restore_file_dialogs: gtk::Button,
}

impl Controls {
    fn new() -> Self {
        let make_default = parts::button(
            &ox_core::i18n::gettext("Make OpenXplorer default"),
            ButtonStyle::Accent,
        );
        let status = StatusText {
            glyph: Icon::Apps,
            title: ox_core::i18n::gettext_static(CHECKING_TITLE),
            text: "Open local folders and SMB links in OpenXplorer. System file-picker dialogs are \
                   unchanged.",
            notice: None,
        };
        let card = StatusCard::new(status, &[make_default.clone().upcast()]);
        let zip_row = SettingRow::new(ZIP_FILES);
        zip_row.set_description(ox_core::i18n::gettext_static(CHECKING));
        Self {
            card,
            make_default,
            // Show in folder on and ZIP files off, as the Python check
            // boxes started (INT-009).
            include_show_in_folder: switch_starting(true),
            include_zip: switch_starting(false),
            folders: parts::value_label(ox_core::i18n::gettext_static(CHECKING)),
            smb_links: parts::value_label(ox_core::i18n::gettext_static(CHECKING)),
            zip_files: parts::value_label(ox_core::i18n::gettext_static(CHECKING)),
            zip_row,
            use_for_zips: parts::button(
                &ox_core::i18n::gettext("Use OpenXplorer for ZIPs"),
                ButtonStyle::Bordered,
            ),
            show_in_folder_row: SettingRow::new(BRAVE_AND_OTHER_APPS),
            test_show_in_folder: parts::button(&ox_core::i18n::gettext("Test"), ButtonStyle::Bordered),
            enable_show_in_folder: parts::button(&ox_core::i18n::gettext("Enable"), ButtonStyle::Accent),
            restore_previous: parts::button(
                &ox_core::i18n::gettext("Restore previous"),
                ButtonStyle::Bordered,
            ),
            restore_zip: parts::button(
                &ox_core::i18n::gettext("Restore ZIP handler"),
                ButtonStyle::Bordered,
            ),
            disable_show_in_folder: parts::button(
                &ox_core::i18n::gettext("Disable Show in folder"),
                ButtonStyle::Bordered,
            ),
            file_dialogs_row: SettingRow::new(FILE_DIALOGS),
            enable_file_dialogs: parts::button("Enable", ButtonStyle::Accent),
            apply_file_dialogs: parts::button("Apply now", ButtonStyle::Bordered),
            restore_file_dialogs: parts::button(
                &ox_core::i18n::gettext("Restore Open and Save dialogs"),
                ButtonStyle::Bordered,
            ),
        }
    }
}

/// A settings switch that starts `active`.
fn switch_starting(active: bool) -> gtk::Switch {
    let switch = parts::switch();
    switch.set_active(active);
    switch
}

/// The Default apps page, and the group with the app that opens ZIP
/// files, which the ZIP & archives page shows.
pub(super) fn build(page: &SettingsPage) -> (SettingsSection, SettingsGroup) {
    let category = Category::DefaultApps;
    let section = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    let controls = Controls::new();
    section.append_card(&controls.card);
    section.append_group(&options_group(&controls));
    let routes = routes_group(&controls);
    section.append_group(&routes);
    section.append_group(&browsers_group(page));
    section.append_group(&undo_group(page, &controls));
    let zip_files = zip_files_group(&controls);
    let view = DefaultAppsView::new(page, &controls);
    routes.add_heading_action(&refresh_button(&view));
    follow_integration(page, &view);
    (section, zip_files)
}

/// Reads the status when Settings opens (most windows never open it) and
/// whenever the `FileManager1` name changes owner.
fn follow_integration(page: &SettingsPage, view: &DefaultAppsView) {
    let opened = view.clone();
    page.when_opened(move || opened.read_status());
    let integration = page.context().desktop_integration();
    let changed = view.clone();
    let id = integration.connect_changed(move || changed.read_status());
    page.imp().handlers.borrow_mut().push(SharedHandler {
        object: integration.clone().upcast(),
        id,
    });
}

/// The options sent with Make `OpenXplorer` default.
fn options_group(controls: &Controls) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("When making OpenXplorer default"));
    for (text, switch) in [
        (INCLUDE_SHOW_IN_FOLDER, &controls.include_show_in_folder),
        (ALSO_ZIP_FILES, &controls.include_zip),
    ] {
        let row = SettingRow::new(text);
        row.add_control(switch, ControlName::RowTitle);
        group.add_row(&row);
    }
    group
}

/// Which app opens each route now: folders, SMB links, Show in folder
/// with Test and Enable (`revealTest`, `revealEnable`) and other apps'
/// Open and Save dialogs with Apply now and Enable (INT-032). The last
/// two rows say their status under their names.
fn routes_group(controls: &Controls) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("What opens where"));
    for (text, value) in [(FOLDERS, &controls.folders), (SMB_LINKS, &controls.smb_links)] {
        let row = SettingRow::new(text);
        row.add_control(value, ControlName::RowTitle);
        group.add_row(&row);
    }
    let show_in_folder = &controls.show_in_folder_row;
    show_in_folder.add_control(&controls.test_show_in_folder, ControlName::OwnLabel);
    show_in_folder.add_control(&controls.enable_show_in_folder, ControlName::OwnLabel);
    group.add_row(show_in_folder);
    let dialogs = &controls.file_dialogs_row;
    dialogs.add_control(&controls.apply_file_dialogs, ControlName::OwnLabel);
    dialogs.add_control(&controls.enable_file_dialogs, ControlName::OwnLabel);
    group.add_row(dialogs);
    group
}

/// The app that opens ZIP files from other apps, with "Use `OpenXplorer`
/// for ZIPs".
fn zip_files_group(controls: &Controls) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("From other apps"));
    let zip_row = &controls.zip_row;
    zip_row.add_control(&controls.zip_files, ControlName::RowTitle);
    zip_row.add_control(&controls.use_for_zips, ControlName::OwnLabel);
    group.add_row(zip_row);
    group
}

/// Brave's download folder.
fn browsers_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Browsers"));
    group.add_row(&super::brave::downloads_row(page));
    group
}

/// "Refresh status", which reads the status again.
fn refresh_button(view: &DefaultAppsView) -> gtk::Button {
    let refresh = parts::button_with_glyph(&ox_core::i18n::gettext("Refresh status"), Icon::ArrowClockwise);
    let view = view.clone();
    refresh.connect_clicked(move |_| view.read_status());
    refresh
}

/// The undo actions and the Zorin and Brave guide, rarely needed, folded
/// away at the bottom.
fn undo_group(page: &SettingsPage, controls: &Controls) -> SettingsGroup {
    let group = SettingsGroup::new_folded(&ox_core::i18n::gettext("Undo and troubleshooting"));
    for (text, button) in [
        (RESTORE_PREVIOUS, &controls.restore_previous),
        (RESTORE_ZIP_HANDLER, &controls.restore_zip),
        (DISABLE_SHOW_IN_FOLDER, &controls.disable_show_in_folder),
        (RESTORE_FILE_DIALOGS, &controls.restore_file_dialogs),
    ] {
        let row = SettingRow::new(text);
        row.add_control(button, ControlName::OwnLabel);
        group.add_row(&row);
    }
    let guide = SettingRow::new(TROUBLESHOOTING);
    let open = parts::chevron_button(GUIDE_TITLE);
    open.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.show_view(SettingsView::Subpage(Subpage::Troubleshooting))
    ));
    guide.add_control(&open, ControlName::OwnLabel);
    group.add_row(&guide);
    group
}
