// SPDX-License-Identifier: AGPL-3.0-only
//! A row of the "Indexed folders" list: one folder the search index keeps,
//! with its state and what can be done with it.
//!
//! Ports the enabled rows of `renderSettingsCache` in `v2.0.0:desktop/ui/app.js`
//! (SET-006, SRCH-022, SRCH-023) in the look of the settings mockup: the
//! folder's picture, its name with a "Pinned" tag for a folder pinning
//! added (SRCH-040), its path, how many names it holds and how it keeps
//! current, a status chip, and buttons to refresh it (or stop a running
//! scan), clear its cached names, and stop indexing it.

use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::search::{IndexRoot, RootOrigin, RootStatus};

use super::commands::IndexCommand;
use crate::icons::{self, Art, ArtImage, Icon};
use crate::settings_page::{parts, SettingsPage};

/// A folder's picture in the list (`folderIcon(24)` in app.js).
const FOLDER_ART: i32 = 28;
/// The glyphs of the row's buttons and tag.
const BUTTON_GLYPH: i32 = 16;
/// The pin in the "Pinned" tag.
const TAG_GLYPH: i32 = 12;

/// How a status chip is drawn (`.chip.ok`, `.run`, `.warn`, `.off` in the
/// mockup).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipTone {
    /// The last scan completed.
    Ready,
    /// A scan is running.
    Running,
    /// The last scan could not read everything, or was interrupted.
    Warning,
    /// Nothing is cached.
    Idle,
}

impl ChipTone {
    /// The tone of `status`.
    pub(crate) const fn of(status: RootStatus) -> Self {
        match status {
            RootStatus::Ready => ChipTone::Ready,
            RootStatus::Indexing | RootStatus::Queued => ChipTone::Running,
            RootStatus::Incomplete | RootStatus::Interrupted => ChipTone::Warning,
            RootStatus::NotIndexed | RootStatus::Disabled => ChipTone::Idle,
        }
    }

    /// The chip's CSS class (`resources/skin/settings.css`).
    const fn css_class(self) -> &'static str {
        match self {
            ChipTone::Ready => "ready",
            ChipTone::Running => "running",
            ChipTone::Warning => "warning",
            ChipTone::Idle => "idle",
        }
    }
}

/// What the row says next to the chip: "8,420 names · Live local events"
/// (`${count} names · ${update_mode}` in `renderSettingsCache`).
pub(crate) fn names_and_updates(root: &IndexRoot) -> String {
    let names = grouped_number(root.entry_count);
    format!("{names} names · {}", root.update_mode.as_str())
}

/// The status's tooltip: why checks or the scan failed, or when the last
/// full refresh finished.
pub(crate) fn status_tooltip(root: &IndexRoot) -> String {
    if let Some(problem) = root.watch_error.as_ref().or(root.error.as_ref()) {
        return problem.clone();
    }
    match root.updated {
        Some(updated) => format!("Last full refresh: {}", format::date_time_text(Some(updated))),
        None => "Not yet scanned".to_owned(),
    }
}

/// `count` with commas between thousands, as `toLocaleString` writes it
/// in English.
pub(crate) fn grouped_number(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (position, digit) in digits.chars().enumerate() {
        let remaining = digits.len() - position;
        if position > 0 && remaining.is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// The row of the indexed folder `root`, shown at `path`, whose buttons
/// hand their command to `page`.
pub(super) fn root_row(root: &IndexRoot, path: &str, is_network: bool, page: &SettingsPage) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(14)
        .css_classes(["setting-row", "folder-row", "indexed-folder"])
        .build();
    let art = if is_network { Art::SHARE } else { Art::Folder };
    row.append(&ArtImage::new(art, FOLDER_ART));
    row.append(&folder_texts(root, path));
    let meta = parts::value_label(&names_and_updates(root));
    meta.set_tooltip_text(Some(&status_tooltip(root)));
    row.append(&meta);
    row.append(&status_chip(root.status));
    for button in root_buttons(root, page) {
        row.append(&button);
    }
    row
}

/// The folder's name, with a "Pinned" tag for a folder pinning added, and
/// its path under it.
fn folder_texts(root: &IndexRoot, path: &str) -> gtk::Box {
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    // One line that ends in "…": a wrapping label beside the tag would
    // ask for a different height at each width.
    let label = gtk::Label::builder()
        .label(&root.label)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["setting-title"])
        .build();
    title.append(&label);
    if root.origin == RootOrigin::Pin {
        title.append(&pinned_tag());
    }
    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    texts.append(&title);
    texts.append(&parts::wrapped_label(path, "setting-description"));
    texts
}

/// The "Pinned" tag of a folder pinning added (SRCH-040).
fn pinned_tag() -> gtk::Box {
    let tag = gtk::Box::builder()
        .spacing(4)
        .valign(gtk::Align::Center)
        .css_classes(["pinned-tag"])
        .build();
    tag.append(&icons::image(Icon::Pin, TAG_GLYPH));
    tag.append(&gtk::Label::new(Some("Pinned")));
    tag
}

/// The chip that shows `status` in its words.
fn status_chip(status: RootStatus) -> gtk::Label {
    let tone = ChipTone::of(status);
    gtk::Label::builder()
        .label(status.as_str())
        .valign(gtk::Align::Center)
        .css_classes(["status-chip", tone.css_class()])
        .build()
}

/// Refresh, or Stop while indexing; Clear; and stop indexing the folder.
fn root_buttons(root: &IndexRoot, page: &SettingsPage) -> [gtk::Button; 3] {
    let uri = &root.uri;
    let scan = if root.status == RootStatus::Indexing {
        FolderAction {
            glyph: Icon::Dismiss,
            name: "Stop indexing",
            command: IndexCommand::Stop(uri.clone()),
        }
    } else {
        FolderAction {
            glyph: Icon::ArrowClockwise,
            name: "Refresh cache",
            command: IndexCommand::Refresh(uri.clone()),
        }
    };
    let clear = FolderAction {
        glyph: Icon::ArrowReset,
        name: "Clear cached names only",
        command: IndexCommand::Clear(uri.clone()),
    };
    let remove = FolderAction {
        glyph: Icon::Delete,
        name: "Remove from index",
        command: IndexCommand::StopIndexing(uri.clone()),
    };
    [scan, clear, remove].map(|action| action.into_button(&root.label, page))
}

/// A button of a folder's row.
#[derive(Debug)]
struct FolderAction {
    /// Its glyph.
    glyph: Icon,
    /// What it does, as its tooltip says.
    name: &'static str,
    /// What it asks the page to do.
    command: IndexCommand,
}

impl FolderAction {
    /// A flat glyph button that runs the command through `page`; a screen
    /// reader hears its name and the folder's `label`.
    fn into_button(self, label: &str, page: &SettingsPage) -> gtk::Button {
        let button = gtk::Button::builder()
            .child(&icons::image(self.glyph, BUTTON_GLYPH))
            .tooltip_text(self.name)
            .valign(gtk::Align::Center)
            .css_classes(["folder-action"])
            .build();
        let name = format!("{} {label}", self.name);
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
        let command = self.command;
        button.connect_clicked(glib::clone!(
            #[weak]
            page,
            move |_| page.run_index_command(command.clone())
        ));
        button
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::test_roots::enabled_root;

    /// parity: SET-006, SRCH-022
    #[test]
    fn a_folder_says_how_many_names_it_holds_and_how_it_keeps_current() {
        let mut root = enabled_root("file:///home/demo/Work");
        root.entry_count = 8420;

        assert_eq!(names_and_updates(&root), "8,420 names · Live local events");
        assert!(status_tooltip(&root).starts_with("Last full refresh: "));
        root.error = Some("Some folders could not be read.".to_owned());
        assert_eq!(status_tooltip(&root), "Some folders could not be read.");
        root.error = None;
        root.updated = None;
        assert_eq!(status_tooltip(&root), "Not yet scanned");
    }

    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(grouped_number(0), "0");
        assert_eq!(grouped_number(999), "999");
        assert_eq!(grouped_number(1000), "1,000");
        assert_eq!(grouped_number(1_234_567), "1,234,567");
    }

    #[test]
    fn each_status_has_the_chip_of_its_kind() {
        assert_eq!(ChipTone::of(RootStatus::Ready), ChipTone::Ready);
        assert_eq!(ChipTone::of(RootStatus::Indexing), ChipTone::Running);
        assert_eq!(ChipTone::of(RootStatus::Incomplete), ChipTone::Warning);
        assert_eq!(ChipTone::of(RootStatus::Interrupted), ChipTone::Warning);
        assert_eq!(ChipTone::of(RootStatus::NotIndexed), ChipTone::Idle);
    }
}
