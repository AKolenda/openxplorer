// SPDX-License-Identifier: AGPL-3.0-only
//! The keyboard shortcuts window: every bound key, grouped by task, with
//! a search field (CMD-032).
//!
//! Ctrl+? and Settings > About > Keyboard shortcuts open it, from any tab,
//! Settings included, as Nautilus's Keyboard Shortcuts window. Its keys
//! are read from the live bindings: the application's accelerators as
//! GTK holds them, and the key tables the window installs its shortcut
//! controllers from. Only the names and groups are written here, so the
//! list cannot disagree with the keys that work.

use std::collections::BTreeMap;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::context_menu::{CLASSIC_MENU_KEY, MENU_KEY};
use super::file_ops::file_key_bindings;
use super::focus_regions::region_key_bindings;
use super::folder_tree::tree_key_bindings;
use super::navigation_buttons::navigation_key_bindings;
use super::selection_keys::selection_key_bindings;
use super::window_keys::{window_key_bindings, REVEAL_KEY_NAME};
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
    fn label(self) -> &'static str {
        match self {
            Group::Navigation => ox_core::i18n::gettext_static("Navigation"),
            Group::TabsAndWindows => ox_core::i18n::gettext_static("Tabs and windows"),
            Group::Selection => ox_core::i18n::gettext_static("Selection"),
            Group::Files => ox_core::i18n::gettext_static("Files"),
            Group::View => ox_core::i18n::gettext_static("View"),
            Group::Search => ox_core::i18n::gettext_static("Search"),
            Group::TextSize => ox_core::i18n::gettext_static("Text size"),
        }
    }
}

/// The group and name of a View command's binding, as [`describe`].
fn describe_view(name: &str) -> Option<(Group, String)> {
    if let Some(view) = name.strip_prefix("win.view::") {
        let label = match view {
            "details" => ox_core::i18n::gettext_static("Details"),
            "compact" => ox_core::i18n::gettext_static("List"),
            "tiles" => ox_core::i18n::gettext_static("Tiles"),
            "content" => ox_core::i18n::gettext_static("Content"),
            key => IconSize::from_key(key)?.label()?,
        };
        return Some((Group::View, label.to_owned()));
    }
    let label = match name {
        "win.split-view" => ox_core::i18n::gettext_static("Split view"),
        "win.folder-tree" => ox_core::i18n::gettext_static("Folder tree"),
        "win.hidden" => ox_core::i18n::gettext_static("Show hidden files"),
        "win.details-pane" => ox_core::i18n::gettext_static("Details pane"),
        "win.preview-pane" => ox_core::i18n::gettext_static("Preview pane"),
        "win.sidebar" => ox_core::i18n::gettext_static("Navigation pane"),
        "win.full-screen" => ox_core::i18n::gettext_static("Full screen"),
        "win.fit-all-columns" => ox_core::i18n::gettext_static("Size all columns to fit"),
        _ => return None,
    };
    Some((Group::View, label.to_owned()))
}

/// The group and name of the command a binding runs, by the action's
/// detailed name; `None` for a name the window does not know.
fn describe(name: &str) -> Option<(Group, String)> {
    if let Some(number) = name.strip_prefix("win.show-tab-number::") {
        let label = match number {
            "0" => ox_core::i18n::gettext_static("Go to the last tab").to_owned(),
            number => ox_core::i18n::format_message("Go to tab {number}", &[("number", number)]),
        };
        return Some((Group::TabsAndWindows, label));
    }
    if let Some(found) = describe_view(name).or_else(|| describe_navigation_pane_key(name)) {
        return Some(found);
    }
    let (group, label) = match name {
        "win.back" => (Group::Navigation, ox_core::i18n::gettext_static("Back")),
        "win.forward" => (Group::Navigation, ox_core::i18n::gettext_static("Forward")),
        "win.up" => (
            Group::Navigation,
            ox_core::i18n::gettext_static("Up to the containing folder"),
        ),
        "win.home" => (Group::Navigation, ox_core::i18n::gettext_static("Home folder")),
        "win.refresh" => (Group::Navigation, ox_core::i18n::gettext_static("Refresh")),
        "win.location" => (
            Group::Navigation,
            ox_core::i18n::gettext_static("Type an address"),
        ),
        "win.address-history" => (
            Group::Navigation,
            ox_core::i18n::gettext_static("Address history"),
        ),
        "region-next" => (
            Group::Navigation,
            ox_core::i18n::gettext_static("Next part of the window"),
        ),
        "region-previous" => (
            Group::Navigation,
            ox_core::i18n::gettext_static("Previous part of the window"),
        ),
        "app.new-window" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("New window")),
        "win.new-tab" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("New tab")),
        "win.close-tab" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("Close tab")),
        "win.reopen-closed-tab" => (
            Group::TabsAndWindows,
            ox_core::i18n::gettext_static("Reopen closed tab"),
        ),
        "win.next-tab" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("Next tab")),
        "win.previous-tab" => (
            Group::TabsAndWindows,
            ox_core::i18n::gettext_static("Previous tab"),
        ),
        "win.settings" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("Settings")),
        "win.keyboard-shortcuts" => (
            Group::TabsAndWindows,
            ox_core::i18n::gettext_static("Keyboard shortcuts"),
        ),
        "win.help" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("Help")),
        "app.quit" => (Group::TabsAndWindows, ox_core::i18n::gettext_static("Quit")),
        "win.select-all" => (Group::Selection, ox_core::i18n::gettext_static("Select all")),
        "win.select-none" => (
            Group::Selection,
            ox_core::i18n::gettext_static("Clear the selection"),
        ),
        // The Menu key, which runs no action of its own.
        "menu-key" => (Group::Selection, ox_core::i18n::gettext_static("Context menu")),
        "win.context-menu" => (
            Group::Selection,
            ox_core::i18n::gettext_static("Show more options"),
        ),
        "win.cut" => (Group::Files, ox_core::i18n::gettext_static("Cut")),
        "win.copy" => (Group::Files, ox_core::i18n::gettext_static("Copy")),
        "win.paste" => (Group::Files, ox_core::i18n::gettext_static("Paste")),
        "win.copy-path" => (Group::Files, ox_core::i18n::gettext_static("Copy as path")),
        "win.rename" => (Group::Files, ox_core::i18n::gettext_static("Rename")),
        "win.trash" => (Group::Files, ox_core::i18n::gettext_static("Delete")),
        "win.delete-permanently" => (Group::Files, ox_core::i18n::gettext_static("Delete permanently")),
        "win.new-folder" => (Group::Files, ox_core::i18n::gettext_static("New folder")),
        "win.undo" => (Group::Files, ox_core::i18n::gettext_static("Undo")),
        "win.redo" => (Group::Files, ox_core::i18n::gettext_static("Redo")),
        "win.properties" => (Group::Files, ox_core::i18n::gettext_static("Properties")),
        "win.open-terminal" => (Group::Files, ox_core::i18n::gettext_static("Open Terminal")),
        "win.open-terminal-here" => (Group::Files, ox_core::i18n::gettext_static("Open Terminal here")),
        "win.search" => (Group::Search, ox_core::i18n::gettext_static("Search")),
        "win.search-tool" => (Group::Search, ox_core::i18n::gettext_static("Search tool")),
        "win.text-larger" => (Group::TextSize, ox_core::i18n::gettext_static("Larger text")),
        "win.text-smaller" => (Group::TextSize, ox_core::i18n::gettext_static("Smaller text")),
        "win.text-reset" => (Group::TextSize, ox_core::i18n::gettext_static("Reset text size")),
        _ => return None,
    };
    Some((group, label.to_owned()))
}

/// The navigation pane's keys of CMD-039, which run no action of their
/// own: Ctrl+Shift+E and the keypad's plus, minus and asterisk.
fn describe_navigation_pane_key(name: &str) -> Option<(Group, String)> {
    let label = match name {
        REVEAL_KEY_NAME => crate::i18n::message_id("Open the navigation pane to the current folder"),
        "tree-open" => crate::i18n::message_id("Expand the selected folder in the navigation pane"),
        "tree-close" => crate::i18n::message_id("Collapse the selected folder in the navigation pane"),
        "tree-open-all" => crate::i18n::message_id("Expand every folder under the selected one"),
        _ => return None,
    };
    Some((Group::Navigation, ox_core::i18n::gettext_static(label).to_owned()))
}

/// What the keys of a command's `triggers`, alternatives split at `|`,
/// read as: GTK's own labels, without `ISO_Left_Tab`'s duplicates. Keypad
/// keys are left out as duplicates while the command has another key, and
/// kept when it has none, as Size all columns to fit, which only the
/// keypad's plus runs (VIEW-072).
fn key_labels<'a>(triggers: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let keys: Vec<&str> = triggers
        .into_iter()
        .flat_map(|trigger| trigger.split('|'))
        .filter(|keys| !keys.contains("ISO_Left_Tab"))
        .collect();
    let has_main_key = keys.iter().any(|keys| !keys.contains("KP_"));
    let mut labels: Vec<String> = Vec::new();
    let shown = keys
        .into_iter()
        .filter(|keys| !has_main_key || !keys.contains("KP_"));
    for (key, modifiers) in shown.filter_map(gtk::accelerator_parse) {
        let label = key_label(key, modifiers);
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

/// What a key reads as: GTK's label, except Ctrl with the keypad's plus,
/// which reads as Windows lists it, "Ctrl+Plus (numeric keypad)", and the
/// navigation pane's keypad plus, minus and asterisk alone, which read
/// "Num +", "Num -" and "Num *", as Windows names them after Num Lock
/// (CMD-039).
fn key_label(key: gdk::Key, modifiers: gdk::ModifierType) -> String {
    if key == gdk::Key::KP_Add && modifiers == gdk::ModifierType::CONTROL_MASK {
        return ox_core::i18n::gettext_static("Ctrl+Plus (numeric keypad)").to_owned();
    }
    let keypad = [gdk::Key::KP_Add, gdk::Key::KP_Subtract, gdk::Key::KP_Multiply];
    if let (true, Some(symbol)) = (modifiers.is_empty() && keypad.contains(&key), key.to_unicode()) {
        return ox_core::i18n::format_message("Num {key}", &[("key", &symbol.to_string())]);
    }
    gtk::accelerator_get_label(key, modifiers).to_string()
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
        .chain(tree_key_bindings())
        .chain([
            ("menu-key".to_owned(), MENU_KEY),
            ("win.context-menu".to_owned(), CLASSIC_MENU_KEY),
        ]);
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
    let mut triggers: Vec<Vec<String>> = Vec::new();
    let mut unknown = Vec::new();
    for (name, keys) in bindings(app) {
        let Some((group, label)) = describe(&name) else {
            unknown.push(name);
            continue;
        };
        let index = if let Some(index) = rows
            .iter()
            .position(|row| row.label == label && row.group == group)
        {
            index
        } else {
            rows.push(ShortcutRow {
                group,
                label,
                keys: Vec::new(),
            });
            triggers.push(Vec::new());
            rows.len() - 1
        };
        triggers[index].push(keys);
    }
    for (row, triggers) in rows.iter_mut().zip(&triggers) {
        row.keys = key_labels(triggers.iter().map(String::as_str));
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
        let dialog = Dialog::new(self, ox_core::i18n::gettext_static("Keyboard shortcuts"), "");
        let search = dialog.add_text_field(ox_core::i18n::gettext_static("Search shortcuts"), "");
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
        dialog.add_button(ox_core::i18n::gettext_static("Close"), ButtonStyle::Accent);
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
        assert_eq!(copy.keys, ["Ctrl+C", "Ctrl+Insert"]);
        assert!(rows.iter().any(|row| row.label == "Keyboard shortcuts"));
        assert!(rows.iter().any(|row| row.label == "Help" && row.keys == ["F1"]));
        let fit = rows
            .iter()
            .find(|row| row.label == "Size all columns to fit")
            .expect("Size all columns to fit is listed");
        assert_eq!(fit.group, Group::View);
        assert_eq!(
            fit.keys,
            ["Ctrl+Plus (numeric keypad)"],
            "its only key is the keypad's, named as Windows names it"
        );
        let larger = rows
            .iter()
            .find(|row| row.label == "Larger text")
            .expect("Larger text");
        assert!(
            larger
                .keys
                .iter()
                .all(|key| !key.contains("KP") && !key.contains("keypad")),
            "Larger text has no keypad plus: {:?}",
            larger.keys
        );
        let reset = rows
            .iter()
            .find(|row| row.label == "Reset text size")
            .expect("Reset text size");
        assert_eq!(reset.keys, ["Ctrl+0"], "keypad 0 stays a hidden duplicate");
        assert!(is_shown("Paste"));
        assert!(!is_shown("Rename"), "the search hides the other lines");
        dialog.finish();
    }

    /// Explorer's Ctrl+E, F3, Ctrl+D, F11 and Alt+P are listed beside the keys
    /// they match, and the search finds them by name and by key.
    ///
    /// parity: CMD-032, CMD-036
    #[gtk::test]
    fn the_shortcuts_window_lists_explorers_further_keys() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());

        let (rows, _) = test.window.shortcut_rows();
        let keys_of = |label: &str| {
            rows.iter()
                .find(|row| row.label == label)
                .map_or_else(|| panic!("{label} is listed"), |row| row.keys.clone())
        };
        let dialog = test.window.shortcuts_dialog();
        let search = descendants::<gtk::Entry>(&dialog).remove(0);
        let shown_after = |query: &str| {
            search.set_text(query);
            descendants::<gtk::Label>(&dialog)
                .iter()
                .filter(|label| label.get_visible() && label.parent().is_some_and(|grid| grid.get_visible()))
                .filter(|label| !label.has_css_class("dialog-hint"))
                .map(|label| label.text().to_string())
                .collect::<Vec<_>>()
        };

        assert_eq!(keys_of("Search"), ["Ctrl+F", "Ctrl+E", "F3"]);
        assert_eq!(keys_of("Delete"), ["Delete", "Ctrl+D"]);
        assert_eq!(keys_of("Details pane"), ["Shift+Alt+P"]);
        assert_eq!(keys_of("Preview pane"), ["Alt+P"]);
        assert_eq!(keys_of("Full screen"), ["F11"]);
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "Full screen")
                .map(|row| row.group),
            Some(Group::View)
        );
        assert!(shown_after("full").contains(&"Full screen".to_owned()));
        assert!(shown_after("ctrl+d").contains(&"Delete".to_owned()));
        assert!(shown_after("ctrl+e").contains(&"Search".to_owned()));
        assert!(shown_after("f3").contains(&"Search".to_owned()));
        assert!(shown_after("alt+p").contains(&"Details pane".to_owned()));
        dialog.finish();
    }

    /// The keys of CMD-039 are listed: Ctrl+1 beside Alt+1, Ctrl+F5 for
    /// Refresh, Ctrl+Insert and Shift+Insert, Ctrl+Shift+E and the
    /// navigation pane's keypad keys, each under its own name.
    ///
    /// parity: CMD-032, CMD-039
    #[gtk::test]
    fn the_shortcuts_window_lists_the_navigation_pane_and_tab_keys() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());

        let (rows, unknown) = test.window.shortcut_rows();
        let keys_of = |label: &str| {
            rows.iter()
                .find(|row| row.label == label)
                .map_or_else(|| panic!("{label} is listed"), |row| row.keys.clone())
        };

        assert!(unknown.is_empty(), "every binding has a name: {unknown:?}");
        assert_eq!(keys_of("Go to tab 1"), ["Alt+1", "Ctrl+1"]);
        assert_eq!(keys_of("Go to tab 9"), ["Alt+9", "Ctrl+9"]);
        assert_eq!(keys_of("Go to the last tab"), ["Alt+0"]);
        assert_eq!(keys_of("Refresh"), ["F5", "Ctrl+R", "Ctrl+F5"]);
        assert_eq!(keys_of("Paste"), ["Ctrl+V", "Shift+Insert"]);
        assert_eq!(
            keys_of("Open the navigation pane to the current folder"),
            ["Shift+Ctrl+E"]
        );
        assert_eq!(
            keys_of("Expand the selected folder in the navigation pane"),
            ["Num +"]
        );
        assert_eq!(
            keys_of("Collapse the selected folder in the navigation pane"),
            ["Num -"]
        );
        assert_eq!(keys_of("Expand every folder under the selected one"), ["Num *"]);
    }
}
