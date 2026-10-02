// SPDX-License-Identifier: AGPL-3.0-only
//! The keyboard shortcuts window: every bound key, grouped by task, with
//! a search field (CMD-032).
//!
//! Ctrl+? and More options > Keyboard shortcuts open it, from any tab,
//! Settings included, as Nautilus's Keyboard Shortcuts window. Its keys
//! are read from the live bindings: the application's accelerators as
//! GTK holds them, and the key tables the window installs its shortcut
//! controllers from. Only the names and groups are written here, so the
//! list cannot disagree with the keys that work.

use std::collections::BTreeMap;

use gtk::glib;
use gtk::prelude::*;

use super::context_menu::CONTEXT_MENU_KEYS;
use super::file_ops::file_key_bindings;
use super::focus_regions::region_key_bindings;
use super::navigation_buttons::navigation_key_bindings;
use super::selection_keys::selection_key_bindings;
use super::window_keys::window_key_bindings;
use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::Dialog;
use crate::folder_view::grid::IconSize;

/// The tasks the window groups shortcuts by, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Group {
    Navigation,
    TabsAndWindows,
    Selection,
    Files,
    View,
    Search,
    TextSize,
}

impl Group {
    /// Every group, in the window's order.
    const ALL: [Group; 7] = [
        Group::Navigation,
        Group::TabsAndWindows,
        Group::Selection,
        Group::Files,
        Group::View,
        Group::Search,
        Group::TextSize,
    ];

    /// The group's heading.
    const fn label(self) -> &'static str {
        match self {
            Group::Navigation => "Navigation",
            Group::TabsAndWindows => "Tabs and windows",
            Group::Selection => "Selection",
            Group::Files => "Files",
            Group::View => "View",
            Group::Search => "Search",
            Group::TextSize => "Text size",
        }
    }
}

/// The group and name of the command a binding runs, by the action's
/// detailed name; `None` for a name the window does not know.
fn describe(name: &str) -> Option<(Group, String)> {
    if let Some(number) = name.strip_prefix("win.show-tab-number::") {
        let label = match number {
            "0" => "Go to the last tab".to_owned(),
            number => format!("Go to tab {number}"),
        };
        return Some((Group::TabsAndWindows, label));
    }
    if let Some(view) = name.strip_prefix("win.view::") {
        let label = match view {
            "details" => "Details",
            "compact" => "List",
            key => IconSize::from_key(key)?.label()?,
        };
        return Some((Group::View, label.to_owned()));
    }
    let (group, label) = match name {
        "win.back" => (Group::Navigation, "Back"),
        "win.forward" => (Group::Navigation, "Forward"),
        "win.up" => (Group::Navigation, "Up to the containing folder"),
        "win.home" => (Group::Navigation, "Home folder"),
        "win.refresh" => (Group::Navigation, "Refresh"),
        "win.location" => (Group::Navigation, "Type an address"),
        "win.address-history" => (Group::Navigation, "Address history"),
        "region-next" => (Group::Navigation, "Next part of the window"),
        "region-previous" => (Group::Navigation, "Previous part of the window"),
        "app.new-window" => (Group::TabsAndWindows, "New window"),
        "win.new-tab" => (Group::TabsAndWindows, "New tab"),
        "win.close-tab" => (Group::TabsAndWindows, "Close tab"),
        "win.reopen-closed-tab" => (Group::TabsAndWindows, "Reopen closed tab"),
        "win.next-tab" => (Group::TabsAndWindows, "Next tab"),
        "win.previous-tab" => (Group::TabsAndWindows, "Previous tab"),
        "win.settings" => (Group::TabsAndWindows, "Settings"),
        "win.keyboard-shortcuts" => (Group::TabsAndWindows, "Keyboard shortcuts"),
        "win.help" => (Group::TabsAndWindows, "Help"),
        "app.quit" => (Group::TabsAndWindows, "Quit"),
        "win.select-all" => (Group::Selection, "Select all"),
        "win.select-none" => (Group::Selection, "Clear the selection"),
        "win.context-menu" => (Group::Selection, "Context menu"),
        "win.cut" => (Group::Files, "Cut"),
        "win.copy" => (Group::Files, "Copy"),
        "win.paste" => (Group::Files, "Paste"),
        "win.copy-path" => (Group::Files, "Copy path"),
        "win.rename" => (Group::Files, "Rename"),
        "win.trash" => (Group::Files, "Delete"),
        "win.delete-permanently" => (Group::Files, "Delete permanently"),
        "win.new-folder" => (Group::Files, "New folder"),
        "win.undo" => (Group::Files, "Undo"),
        "win.redo" => (Group::Files, "Redo"),
        "win.properties" => (Group::Files, "Properties"),
        "win.open-terminal" => (Group::Files, "Open Terminal"),
        "win.open-terminal-here" => (Group::Files, "Open Terminal here"),
        "win.hidden" => (Group::View, "Show hidden files"),
        "win.details-pane" => (Group::View, "Details pane"),
        "win.sidebar" => (Group::View, "Navigation pane"),
        "win.search" => (Group::Search, "Search"),
        "win.search-tool" => (Group::Search, "Search tool"),
        "win.text-larger" => (Group::TextSize, "Larger text"),
        "win.text-smaller" => (Group::TextSize, "Smaller text"),
        "win.text-reset" => (Group::TextSize, "Reset text size"),
        _ => return None,
    };
    Some((group, label.to_owned()))
}

/// What the keys of `trigger`, alternatives split at `|`, read as: GTK's
/// own labels, without the keypad's and `ISO_Left_Tab`'s duplicates.
fn key_labels(trigger: &str) -> Vec<String> {
    trigger
        .split('|')
        .filter(|keys| !keys.contains("KP_") && !keys.contains("ISO_Left_Tab"))
        .filter_map(gtk::accelerator_parse)
        .map(|(key, modifiers)| gtk::accelerator_get_label(key, modifiers).to_string())
        .collect()
}

/// Every binding: the action's detailed name and its keys as GTK parses
/// them, the application's accelerators read from `app`.
fn bindings(app: &gtk::Application) -> Vec<(String, String)> {
    let mut bindings: Vec<(String, String)> = Vec::new();
    for name in app.list_action_descriptions() {
        for keys in app.accels_for_action(&name) {
            bindings.push((name.to_string(), keys.to_string()));
        }
    }
    let tables = window_key_bindings()
        .chain(file_key_bindings())
        .chain(selection_key_bindings())
        .chain(navigation_key_bindings())
        .chain(region_key_bindings())
        .chain([("win.context-menu".to_owned(), CONTEXT_MENU_KEYS)]);
    bindings.extend(tables.map(|(name, keys)| (name, keys.to_owned())));
    bindings
}

/// One line of the window: a command and its keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShortcutRow {
    pub(super) group: Group,
    pub(super) label: String,
    pub(super) keys: Vec<String>,
}

/// The window's lines, grouped and in binding order; the names a binding
/// runs that [`describe`] does not know are returned too, for tests.
fn shortcut_rows(app: &gtk::Application) -> (Vec<ShortcutRow>, Vec<String>) {
    let mut rows: Vec<ShortcutRow> = Vec::new();
    let mut unknown = Vec::new();
    for (name, keys) in bindings(app) {
        let Some((group, label)) = describe(&name) else {
            unknown.push(name);
            continue;
        };
        let index = match rows
            .iter()
            .position(|row| row.label == label && row.group == group)
        {
            Some(index) => index,
            None => {
                rows.push(ShortcutRow {
                    group,
                    label,
                    keys: Vec::new(),
                });
                rows.len() - 1
            }
        };
        for key in key_labels(&keys) {
            if !rows[index].keys.contains(&key) {
                rows[index].keys.push(key);
            }
        }
    }
    rows.sort_by_key(|row| row.group);
    (rows, unknown)
}

impl BrowserWindow {
    /// The shortcuts the window lists, for its dialog and tests.
    pub(super) fn shortcut_rows(&self) -> (Vec<ShortcutRow>, Vec<String>) {
        self.application()
            .map(|app| shortcut_rows(&app))
            .unwrap_or_default()
    }

    /// Opens the keyboard shortcuts window.
    pub(super) fn show_keyboard_shortcuts(&self) {
        let dialog = self.shortcuts_dialog();
        dialog.open();
        glib::spawn_future_local(async move {
            dialog.next_response().await;
            dialog.finish();
        });
    }

    /// The keyboard shortcuts window: a search field, then one list per
    /// group, which the search narrows to the lines that match.
    pub(super) fn shortcuts_dialog(&self) -> Dialog {
        let dialog = Dialog::new(self, "Keyboard shortcuts", "");
        let search = dialog.add_text_field("Search shortcuts", "");
        let (rows, _) = self.shortcut_rows();
        let mut groups: BTreeMap<Group, Vec<&ShortcutRow>> = BTreeMap::new();
        for row in &rows {
            groups.entry(row.group).or_default().push(row);
        }
        let mut lists = Vec::new();
        for group in Group::ALL {
            let Some(rows) = groups.get(&group) else {
                continue;
            };
            let grid = gtk::Grid::builder().column_spacing(24).row_spacing(6).build();
            let mut lines = Vec::new();
            for (line, row) in (0..).zip(rows.iter()) {
                let name = gtk::Label::builder()
                    .label(&row.label)
                    .xalign(0.0)
                    .hexpand(true)
                    .build();
                let keys = row.keys.join(", ");
                let key_label = gtk::Label::builder()
                    .label(&keys)
                    .xalign(1.0)
                    .css_classes(["dialog-hint"])
                    .build();
                grid.attach(&name, 0, line, 1, 1);
                grid.attach(&key_label, 1, line, 1, 1);
                let haystack = format!("{} {keys}", row.label).to_lowercase();
                lines.push((name, key_label, haystack));
            }
            dialog.add_labelled(group.label(), &grid);
            lists.push((grid, lines));
        }
        search.connect_changed(move |search| {
            let query = search.text().to_lowercase();
            for (grid, lines) in &lists {
                let mut any = false;
                for (name, keys, haystack) in lines {
                    let matches = haystack.contains(query.trim());
                    name.set_visible(matches);
                    keys.set_visible(matches);
                    any |= matches;
                }
                Dialog::set_field_visible(grid, any);
            }
        });
        dialog.add_button("Close", ButtonStyle::Accent);
        dialog
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{descendants, Fixture, TestWindow};

    /// The window lists every bound key under a known name, Copy with
    /// Ctrl+C among them, and the search narrows the list.
    ///
    /// parity: CMD-032
    #[gtk::test]
    fn the_shortcuts_window_lists_every_live_binding_and_searches_them() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());

        let (rows, unknown) = test.window.shortcut_rows();
        let dialog = test.window.shortcuts_dialog();
        let search = descendants::<gtk::Entry>(&dialog).remove(0);
        search.set_text("paste");
        let is_shown = |text: &str| {
            descendants::<gtk::Label>(&dialog)
                .iter()
                .filter(|label| label.text() == text)
                .any(|label| label.get_visible() && label.parent().is_some_and(|grid| grid.get_visible()))
        };

        assert!(unknown.is_empty(), "every binding has a name: {unknown:?}");
        let copy = rows
            .iter()
            .find(|row| row.label == "Copy")
            .expect("Copy is listed");
        assert_eq!(copy.group, Group::Files);
        assert_eq!(copy.keys, ["Ctrl+C"]);
        assert!(rows.iter().any(|row| row.label == "Keyboard shortcuts"));
        assert!(rows.iter().any(|row| row.label == "Help" && row.keys == ["F1"]));
        assert!(is_shown("Paste"));
        assert!(!is_shown("Rename"), "the search hides the other lines");
        dialog.finish();
    }
}
