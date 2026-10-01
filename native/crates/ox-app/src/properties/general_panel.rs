// SPDX-License-Identifier: AGPL-3.0-only
//! The General and Permissions tabs of the Properties dialog.
//!
//! Ports the part of `propertiesDialog` in `v2.0.0:desktop/ui/app.js` that runs
//! once the item's properties are read (PROP-003, PROP-006): the icon and
//! name, then Type, Location, Full path, Size, Opens with, Created,
//! Modified and Accessed, the Change app…, Copy full path and Calculate
//! folder size buttons, and the read-only permissions the backend reports.
//! A local folder also gets Analyse disk usage where an analyser is
//! installed.

use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::integration::DiskTool;
use ox_core::location::{is_smb_server, LocationContext};
use ox_core::versions::{is_conventional_snapshot, snapshot_location};

use super::folder_sizes::{FolderSizeState, NOT_SCANNED};
use super::metadata::{ItemProperties, MountFacts};
use crate::dialog::{note, quiet_text, DialogFrame, PropertyGrid};
use crate::icons::{self, Art, ArtImage, Icon};
use crate::integration;
use crate::window::{is_disk_tool_installed, ButtonStyle, WindowAction};

/// The size of the item's picture at the top of the General tab
/// (`fileIcon(current, 48)`).
const HEADER_ART_SIZE: i32 = 48;

/// The glyph size in the tab's buttons.
const BUTTON_GLYPH: i32 = 16;

/// Under Calculate folder size: what the measured size is, and is not.
const SIZE_EXPLANATION: &str = "Logical file bytes, measured on demand. Skips links, nested mounts and \
                                snapshot collections. The result may be partial; it is not ZFS compressed \
                                or snapshot usage.";

/// Under the permissions: what the tab does not do.
const PERMISSIONS_NOTE: &str = "These are the permissions reported by Linux/GIO. This page does not edit \
                                Windows ACLs, take ownership, or change server permissions.";

/// The Permissions tab when the item could not be read.
const METADATA_UNREADABLE: &str = "Metadata could not be read.";

/// A file's Opens with row without a default application.
const NO_DEFAULT_APP: &str = "No default application";

/// The toast after Copy full path.
const PATH_COPIED: &str = "Full path copied.";

/// What the General tab shows besides the item's own properties.
#[derive(Debug, Clone, Copy)]
pub(super) struct GeneralFacts<'a> {
    /// The item, as read.
    pub properties: &'a ItemProperties,
    /// Display names for the home folder and devices.
    pub locations: &'a LocationContext,
    /// The folder's measured size, if any.
    pub folder_size: Option<&'a FolderSizeState>,
    /// The snapshot collections known now, whose items are read-only.
    pub snapshot_roots: &'a [String],
    /// The name can be edited to rename the item (PROP-005): not a
    /// standard folder, a share, a page or a previous version.
    pub can_rename: bool,
}

/// The values of a folder's General tab that a folder-size scan updates.
#[derive(Debug, Clone)]
pub(super) struct FolderRows {
    /// The Size value.
    pub size: gtk::Label,
    /// The Contains value: how many files and folders it holds.
    pub contains: gtk::Label,
}

impl FolderRows {
    /// Shows the folder's measured size and counts.
    pub(super) fn show(&self, state: &FolderSizeState) {
        self.size.set_text(&state.size_text());
        self.size.set_tooltip_text(Some(&state.summary_tooltip()));
        self.contains.set_text(&state.contains_text());
    }
}

/// Fills the General tab, replacing "Reading file properties…". Returns
/// a folder's Size and Contains values, which a folder-size scan updates.
pub(super) fn fill_general(panel: &gtk::Box, facts: &GeneralFacts<'_>) -> Option<FolderRows> {
    clear(panel);
    let properties = facts.properties;
    let entry = &properties.entry;
    panel.append(&header(properties, facts.can_rename));
    let grid = PropertyGrid::new();
    let container = properties.parent_uri.as_deref().unwrap_or(&entry.uri);
    grid.add_row("Type", &entry.type_label);
    grid.add_row("Location", &facts.locations.display_location(container));
    grid.add_row("Full path", &facts.locations.display_location(&entry.uri));
    let size_value = grid.add_row("Size", &size_text(facts));
    if let Some(state) = facts.folder_size.filter(|_| entry.is_dir) {
        size_value.set_tooltip_text(Some(&state.summary_tooltip()));
    }
    let contains = entry.is_dir.then(|| {
        let text = facts
            .folder_size
            .map_or_else(|| NOT_SCANNED.to_owned(), FolderSizeState::contains_text);
        grid.add_row("Contains", &text)
    });
    if let Some(target) = &properties.link_target {
        grid.add_row("Points to", target);
    }
    if let Some((width, height)) = properties.dimensions {
        grid.add_row("Dimensions", &format!("{width} × {height} pixels"));
    }
    if !entry.is_dir {
        let app = properties.default_app.as_deref().unwrap_or(NO_DEFAULT_APP);
        grid.add_row("Opens with", app);
    }
    grid.add_row("Created", &format::date_time_text(properties.created));
    grid.add_row("Modified", &format::date_time_text(entry.modified));
    grid.add_row("Accessed", &format::date_time_text(properties.accessed));
    if let Some(mount) = &properties.mount {
        add_mount_rows(&grid, mount);
    }
    panel.append(grid.widget());
    panel.append(&buttons(facts));
    if entry.is_dir && !is_smb_server(&entry.uri) {
        panel.append(&quiet_text(SIZE_EXPLANATION));
    }
    contains.map(|contains| FolderRows {
        size: size_value,
        contains,
    })
}

/// Mounted on, Mounted from, File system and the free space with its bar,
/// for a mount point (PROP-004, as Dolphin's General tab). They come last,
/// so the bar under the free space ends the grid.
fn add_mount_rows(grid: &PropertyGrid, mount: &MountFacts) {
    grid.add_row("Mounted on", &mount.mounted_on);
    grid.add_row("Mounted from", &mount.mounted_from);
    grid.add_row("File system", &mount.filesystem);
    let Some((free, total)) = mount.space.filter(|(_, total)| *total > 0) else {
        return;
    };
    let text = format!(
        "{} free of {}",
        format::pretty_bytes(free),
        format::pretty_bytes(total)
    );
    let value = grid.add_row("Free space", &text);
    let bar = gtk::LevelBar::builder()
        .min_value(0.0)
        .max_value(1.0)
        .hexpand(true)
        .build();
    #[expect(clippy::cast_precision_loss, reason = "a bar's fraction needs no exact bytes")]
    bar.set_value(1.0 - free as f64 / total as f64);
    bar.update_property(&[gtk::accessible::Property::Label(&text)]);
    // The bar goes under the text; it is the grid's last line.
    let (column, line, _, _) = grid.widget().query_child(&value);
    grid.widget().attach(&bar, column, line + 1, 1, 1);
}

/// The item's picture and name (`.property-file`). The name is a field
/// when the item can be renamed: Enter renames it, as OK does in
/// Explorer's and Dolphin's Properties.
fn header(properties: &ItemProperties, can_rename: bool) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["property-file"])
        .build();
    header.append(&ArtImage::new(Art::for_entry(&properties.entry), HEADER_ART_SIZE));
    if can_rename {
        header.append(&name_field(&properties.entry.uri, &properties.entry.name));
        return header;
    }
    let name = gtk::Label::builder()
        .label(&properties.entry.name)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .css_classes(["property-name-heading"])
        .build();
    header.append(&name);
    header
}

/// The editable name of the item at `uri`; Enter asks the window to
/// rename it, and the dialog closes once it is renamed.
fn name_field(uri: &str, name: &str) -> gtk::Entry {
    let field = gtk::Entry::builder()
        .text(name)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    field.update_property(&[gtk::accessible::Property::Label("Name")]);
    let uri = uri.to_owned();
    let original = name.to_owned();
    field.connect_activate(move |field| {
        let name = field.text().to_string();
        let window = field.root().and_downcast::<crate::window::BrowserWindow>();
        let (false, Some(window)) = (name == original, window) else {
            return;
        };
        let frame = field
            .ancestor(DialogFrame::static_type())
            .and_downcast::<DialogFrame>();
        let uri = uri.clone();
        glib::spawn_future_local(async move {
            let renamed = window.rename_item_at(&uri, &name).await;
            match (renamed, frame) {
                (Ok(()), Some(frame)) => frame.close(),
                (Err(message), Some(frame)) => frame.show_error(&message),
                (_, None) => {}
            }
        });
    });
    field
}

/// The Size value: a file's size, or a folder's measured size or "Not
/// scanned".
fn size_text(facts: &GeneralFacts<'_>) -> String {
    let entry = &facts.properties.entry;
    if !entry.is_dir {
        return entry.size.map(format::pretty_bytes).unwrap_or_default();
    }
    facts
        .folder_size
        .map_or_else(|| NOT_SCANNED.to_owned(), FolderSizeState::size_text)
}

/// Change app…, Copy full path and, for a folder, Calculate folder size.
fn buttons(facts: &GeneralFacts<'_>) -> gtk::Box {
    let entry = &facts.properties.entry;
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["property-buttons"])
        .build();
    let is_read_only =
        snapshot_location(&entry.uri, facts.snapshot_roots).is_some() || is_conventional_snapshot(&entry.uri);
    if !entry.is_dir && !is_read_only {
        row.append(&change_app_button(&entry.uri));
        let content_type = entry.content_type.as_deref();
        if let Some(content_type) =
            content_type.filter(|content_type| !integration::is_protected(content_type))
        {
            row.append(&type_applications_button(content_type));
        }
    }
    row.append(&copy_path_button(facts.locations.display_location(&entry.uri)));
    if entry.is_dir && !is_smb_server(&entry.uri) {
        row.append(&calculate_size_button(&entry.uri));
    }
    if entry.uri.starts_with("file:") && !is_read_only {
        super::custom_icon::icon_buttons(&row, &entry.uri, facts.properties.has_custom_icon);
    }
    let is_local_folder = entry.is_dir && entry.uri.starts_with("file:");
    if is_local_folder && is_disk_tool_installed(DiskTool::AnalyseUsage) {
        row.append(&analyse_usage_button(&entry.uri));
    }
    row
}

/// Analyse disk usage: a disk-usage analyser at the folder at `uri`, as
/// Dolphin's "Explore in Filelight" (PROP-015).
fn analyse_usage_button(uri: &str) -> gtk::Button {
    let button = glyph_button("Analyse disk usage", Icon::HardDrive);
    WindowAction::AnalyseDiskUsage.assign_with_target_to(&button, &uri.to_variant());
    button
}

/// A bordered button with `glyph` and `label`.
pub(super) fn glyph_button(label: &str, glyph: Icon) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icons::image(glyph, BUTTON_GLYPH));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class(ButtonStyle::Bordered.css_class());
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// Change app…: the Open with dialog for the file at `uri`.
fn change_app_button(uri: &str) -> gtk::Button {
    let button = glyph_button("Change app…", Icon::Grid);
    WindowAction::ChangeApp.assign_with_target_to(&button, &uri.to_variant());
    button
}

/// Apps for this type…: the applications associated with the file's
/// type (OPEN-026).
fn type_applications_button(content_type: &str) -> gtk::Button {
    let button = glyph_button("Apps for this type…", Icon::Apps);
    WindowAction::TypeApplications.assign_with_target_to(&button, &content_type.to_variant());
    button
}

/// Copy full path: puts `path` on the clipboard and says so.
fn copy_path_button(path: String) -> gtk::Button {
    let button = glyph_button("Copy full path", Icon::Copy);
    button.connect_clicked(move |button| {
        button.clipboard().set_text(&path);
        if let Some(window) = button.root().and_downcast::<crate::window::BrowserWindow>() {
            window.show_message(PATH_COPIED);
        }
    });
    button
}

/// Calculate folder size: measures the folder at `uri`, which shows in
/// this tab, the details pane and the Size column.
fn calculate_size_button(uri: &str) -> gtk::Button {
    let button = glyph_button("Calculate folder size", Icon::HardDrive);
    WindowAction::CalculateFolderSizeOf.assign_with_target_to(&button, &uri.to_variant());
    button
}

/// Fills the Permissions tab; `editor` changes them, where the user may.
pub(super) fn fill_permissions(panel: &gtk::Box, properties: &ItemProperties, editor: Option<gtk::Box>) {
    clear(panel);
    let grid = PropertyGrid::new();
    grid.add_row("Owner", properties.owner.as_deref().unwrap_or_default());
    grid.add_row("Group", properties.group.as_deref().unwrap_or_default());
    grid.add_row("POSIX mode", &properties.mode_text().unwrap_or_default());
    let access = properties.access;
    grid.add_row("Readable", access_text(access.readable));
    grid.add_row("Writable", access_text(access.writable));
    grid.add_row("Executable", access_text(access.executable));
    panel.append(grid.widget());
    if let Some(editor) = editor {
        panel.append(&editor);
    }
    panel.append(&note(PERMISSIONS_NOTE));
}

/// `Yes`, `No` or `Not reported by this backend`.
fn access_text(allowed: Option<bool>) -> &'static str {
    match allowed {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Not reported by this backend",
    }
}

/// Shows why the item could not be read on the General tab, and that
/// there is no metadata on the Permissions tab.
pub(super) fn show_read_failure(general: &gtk::Box, permissions: &gtk::Box, message: &str) {
    clear(general);
    general.append(&note(message));
    clear(permissions);
    permissions.append(&quiet_text(METADATA_UNREADABLE));
}

/// Removes every child of `panel`.
fn clear(panel: &gtk::Box) {
    while let Some(child) = panel.first_child() {
        panel.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Properties button runs the analyser on the folder.
    ///
    /// parity: PROP-015
    #[gtk::test]
    fn the_analyse_button_runs_the_analyser_on_the_folder() {
        let button = analyse_usage_button("file:///srv/media");
        assert_eq!(button.action_name().as_deref(), Some("win.analyse-disk-usage"));
        assert_eq!(
            button.action_target_value(),
            Some("file:///srv/media".to_variant())
        );
    }
}
