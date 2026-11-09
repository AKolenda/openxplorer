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
        REVEAL_KEY_NAME => {
            crate::i18n::message_id("Open the navigation pane to the current folder, while it is shown")
        }
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
    /// group, which the search narrows to the lines that match
    /// ([`query_matches`]) as you type. Names and keys wrap, so the window
    /// keeps its usual width however long they are, and the list keeps
    /// clear of the scroll bar.
    pub(super) fn shortcuts_dialog(&self) -> Dialog {
        let dialog = Dialog::new(self, ox_core::i18n::gettext_static("Keyboard shortcuts"), "");
        dialog.keep_clear_of_scroll_bar();
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
            let (grid, lines) = shortcut_grid(group, rows);
            dialog.add_labelled(group.label(), &grid);
            lists.push((grid, lines));
        }
        search.connect_changed(move |search| {
            let query = search.text();
            for (grid, lines) in &lists {
                let mut any = false;
                for line in lines {
                    let matches = line.matches(&query);
                    line.name.set_visible(matches);
                    line.keys.set_visible(matches);
                    any |= matches;
                }
                Dialog::set_field_visible(grid, any);
            }
        });
        dialog.add_button(ox_core::i18n::gettext_static("Close"), ButtonStyle::Accent);
        dialog
    }
}

/// One line of the shortcuts window and what its search looks at.
struct ShortcutLine {
    name: gtk::Label,
    keys: gtk::Label,
    label: String,
    section: String,
    combinations: Vec<String>,
}

impl ShortcutLine {
    /// Whether the search `query` shows this line.
    fn matches(&self, query: &str) -> bool {
        query_matches(query, &self.label, &self.section, &self.combinations)
    }
}

/// The grid of `group`'s `rows`: names at the left and keys at the right,
/// both wrapping when the window is narrow.
fn shortcut_grid(group: Group, rows: &[&ShortcutRow]) -> (gtk::Grid, Vec<ShortcutLine>) {
    let grid = gtk::Grid::builder().column_spacing(24).row_spacing(6).build();
    let mut lines = Vec::new();
    for (line, row) in (0..).zip(rows.iter()) {
        let name = gtk::Label::builder()
            .label(&row.label)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        let key_label = gtk::Label::builder()
            .label(row.keys.join(", "))
            .xalign(1.0)
            .justify(gtk::Justification::Right)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["dialog-hint"])
            .build();
        grid.attach(&name, 0, line, 1, 1);
        grid.attach(&key_label, 1, line, 1, 1);
        lines.push(ShortcutLine {
            name,
            keys: key_label,
            label: row.label.clone(),
            section: group.label().to_owned(),
            combinations: row.keys.clone(),
        });
    }
    (grid, lines)
}

/// The words of `text` as the search compares them: lower case, with a
/// "+" or "-" between two keys read as a space, a "+", "-" or "*" on its
/// own read as the key's name, brackets dropped, and each word in one
/// spelling: "ctrl" for "control", "esc" for "escape", "del" for
/// "delete", "num" for "keypad" and "numeric".
fn search_words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut spaced = String::new();
    for (index, &character) in chars.iter().enumerate() {
        let before = index.checked_sub(1).and_then(|before| chars.get(before));
        let after = chars.get(index + 1);
        let joins_keys = |character: Option<&char>| {
            character.is_some_and(|other| !other.is_whitespace() && !matches!(other, '+' | '-'))
        };
        match character {
            '+' | '-' if joins_keys(before) && after.is_some_and(|other| !other.is_whitespace()) => {
                spaced.push(' ');
            }
            '+' => spaced.push_str(" plus "),
            '-' => spaced.push_str(" minus "),
            '*' => spaced.push_str(" asterisk "),
            '(' | ')' => spaced.push(' '),
            other => spaced.push(other),
        }
    }
    spaced
        .split_whitespace()
        .map(|word| match word {
            "control" => "ctrl",
            "escape" => "esc",
            "delete" => "del",
            "keypad" | "numeric" => "num",
            other => other,
        })
        .map(str::to_owned)
        .collect()
}

/// Whether the search `query` finds the shortcut named `label` in the
/// section `section`, whose key combinations, as the window shows them,
/// are `combinations`: when the query's text is in its name, section or
/// keys (unless the query is only symbols, such as "+", which then stands
/// for its key alone), or when each word of the query is a key of one of the
/// combinations, in any order, or begins a word of its name or section.
/// A one-letter word must be a key, so "shift e" finds Ctrl+Shift+E and
/// not every Shift key whose name has a word starting with "e".
fn query_matches(query: &str, label: &str, section: &str, combinations: &[String]) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    // A query with no letter or digit, such as "+", is a key's symbol:
    // as text it would match the "+" between the keys of every Ctrl and
    // Alt shortcut, so only its key is looked for.
    let symbol_only = !query.chars().any(char::is_alphanumeric);
    let text = format!("{label} {section} {}", combinations.join(", ")).to_lowercase();
    if !symbol_only && text.contains(&query) {
        return true;
    }
    let words = search_words(&query);
    let named = search_words(&format!("{label} {section}"));
    let in_name = |word: &str| word.chars().count() > 1 && named.iter().any(|named| named.starts_with(word));
    let fits = |keys: &[String]| words.iter().all(|word| keys.contains(word) || in_name(word));
    if combinations.is_empty() {
        return fits(&[]);
    }
    combinations
        .iter()
        .any(|combination| fits(&search_words(combination)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

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

    /// The search's words: one spelling, the separators between keys
    /// dropped, a key that is "+", "-" or "*" named.
    ///
    /// parity: CMD-032
    #[test]
    fn the_search_reads_keys_however_typed() {
        assert_eq!(search_words("Ctrl+Shift+E"), ["ctrl", "shift", "e"]);
        assert_eq!(search_words("control-shift-e"), ["ctrl", "shift", "e"]);
        assert_eq!(search_words("  ctrl   shift e "), ["ctrl", "shift", "e"]);
        assert_eq!(search_words("Num +"), ["num", "plus"]);
        assert_eq!(search_words("Num -"), ["num", "minus"]);
        assert_eq!(search_words("Num *"), ["num", "asterisk"]);
        assert_eq!(search_words("Ctrl++"), ["ctrl", "plus"]);
        assert_eq!(
            search_words("Ctrl+Plus (numeric keypad)"),
            ["ctrl", "plus", "num", "num"]
        );
        assert_eq!(search_words("Escape"), ["esc"]);
        assert_eq!(search_words("Delete"), ["del"]);
        assert_eq!(search_words("Ctrl+,"), ["ctrl", ","]);
    }

    /// A query finds a shortcut by one of its key combinations in any
    /// order, by words of its name or section, or as plain text; a
    /// one-letter word must be a key.
    ///
    /// parity: CMD-032
    #[test]
    fn a_query_finds_a_shortcut_by_its_keys_or_words() {
        let keys = ["Shift+Ctrl+E".to_owned()];
        let reveal = |query: &str| query_matches(query, "Open the navigation pane", "Navigation", &keys);
        for query in [
            "ctrl shift e",
            "Ctrl+Shift+E",
            "shift e",
            "e shift ctrl",
            "control-shift-e",
            "",
            "nav pane",
        ] {
            assert!(reveal(query), "{query:?}");
        }
        for query in ["ctrl shift x", "alt e", "shift o"] {
            assert!(!reveal(query), "{query:?}");
        }
        let copy = ["Ctrl+C".to_owned(), "Ctrl+Insert".to_owned()];
        assert!(query_matches("control insert", "Copy", "Files", &copy));
        assert!(
            !query_matches("c insert", "Copy", "Files", &copy),
            "keys of two combinations"
        );
        let close = ["Esc".to_owned()];
        assert!(query_matches("escape", "Close", "Dialogs", &close));
    }

    /// A query that is only a key's symbol, such as "+", "-" or "*",
    /// finds the shortcuts of that key, not every shortcut whose keys are
    /// joined by it: "+" finds Num + and Ctrl++, not Ctrl+C.
    ///
    /// parity: CMD-032
    #[test]
    fn a_symbol_alone_finds_its_key_not_the_separators() {
        let copy = ["Ctrl+C".to_owned(), "Ctrl+Insert".to_owned()];
        let back = ["Alt+Left".to_owned(), "Back".to_owned()];
        let plus = ["Num +".to_owned()];
        let minus = ["Num -".to_owned()];
        let larger = ["Ctrl++".to_owned(), "Ctrl+=".to_owned()];
        for query in ["+", " + ", "-", "*"] {
            assert!(
                !query_matches(query, "Copy", "Files", &copy),
                "{query:?} skips Copy"
            );
            assert!(
                !query_matches(query, "Back", "Navigation", &back),
                "{query:?} skips Back"
            );
        }
        assert!(query_matches(
            "+",
            "Expand the selected folder",
            "Navigation",
            &plus
        ));
        assert!(query_matches("+", "Larger text", "Text size", &larger));
        assert!(query_matches(
            "-",
            "Collapse the selected folder",
            "Navigation",
            &minus
        ));
        assert!(!query_matches(
            "-",
            "Expand the selected folder",
            "Navigation",
            &plus
        ));
        let settings = ["Ctrl+,".to_owned()];
        assert!(query_matches(",", "Settings", "Tabs and windows", &settings));
        assert!(!query_matches(",", "Copy", "Files", &copy));
    }

    /// Typing "+" in the shortcuts window shows the Plus key's shortcuts
    /// and hides Copy and the other Ctrl and Alt shortcuts.
    ///
    /// parity: CMD-032
    #[gtk::test]
    fn the_plus_key_alone_is_searchable() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let dialog = test.window.shortcuts_dialog();
        let shown = shown_after(&dialog, "+");
        assert!(shown.contains(&"Expand the selected folder in the navigation pane".to_owned()));
        assert!(shown.contains(&"Larger text".to_owned()));
        for hidden in ["Copy", "Paste", "Back", "New tab"] {
            assert!(
                !shown.contains(&hidden.to_owned()),
                "{hidden} is not a Plus key shortcut"
            );
        }
        dialog.finish();
    }

    /// The labels the search leaves shown after typing `query`.
    fn shown_after(dialog: &Dialog, query: &str) -> Vec<String> {
        let search = descendants::<gtk::Entry>(dialog).remove(0);
        search.set_text(query);
        descendants::<gtk::Label>(dialog)
            .iter()
            .filter(|label| label.get_visible() && label.parent().is_some_and(|grid| grid.get_visible()))
            .filter(|label| !label.has_css_class("dialog-hint"))
            .map(|label| label.text().to_string())
            .collect()
    }

    /// The search finds a shortcut by its keys however they are typed:
    /// with spaces, "+" or "-" between them, in any case and any order of
    /// the modifiers, with "control", "escape", "delete" and "keypad" for
    /// "Ctrl", "Esc", "Del" and "Num"; and by the words of its name and of
    /// its section. Keys that no shortcut has find nothing.
    ///
    /// parity: CMD-032
    #[gtk::test]
    fn the_shortcuts_window_finds_a_shortcut_by_its_keys_however_typed() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let dialog = test.window.shortcuts_dialog();
        let reveal = "Open the navigation pane to the current folder, while it is shown".to_owned();

        for query in [
            "ctrl shift e",
            "ctrl+shift+e",
            "Ctrl+Shift+E",
            "shift e",
            "control-shift-e",
            "Shift Ctrl E",
            "e ctrl shift",
            "navigation pane current",
        ] {
            assert!(
                shown_after(&dialog, query).contains(&reveal),
                "{query:?} finds Ctrl+Shift+E"
            );
        }
        assert!(shown_after(&dialog, "keypad +")
            .contains(&"Expand the selected folder in the navigation pane".to_owned()));
        assert!(shown_after(&dialog, "control c").contains(&"Copy".to_owned()));
        assert!(shown_after(&dialog, "delete").contains(&"Delete".to_owned()));
        assert!(
            shown_after(&dialog, "tabs and windows").contains(&"New tab".to_owned()),
            "a section's name finds its shortcuts"
        );
        let nothing = shown_after(&dialog, "ctrl shift x");
        assert!(!nothing.contains(&reveal), "other keys do not find it");
        assert!(!nothing.contains(&"Copy".to_owned()));
        dialog.finish();
    }

    /// The shortcuts window, opened over `test`'s window and laid out.
    fn opened(test: &TestWindow) -> Dialog {
        let dialog = test.window.shortcuts_dialog();
        dialog.open();
        wait_until("the shortcuts window to be shown", || {
            dialog.is_mapped() && dialog.width() > 0
        });
        wait_for_frames(&dialog, 3);
        dialog
    }

    /// The scrolling body of the shortcuts window.
    fn body_scroller(dialog: &Dialog) -> gtk::ScrolledWindow {
        descendants::<gtk::ScrolledWindow>(dialog).remove(0)
    }

    /// The largest right edge of the shown key labels in `scroller`.
    fn right_edge_of_keys(scroller: &gtk::ScrolledWindow) -> f32 {
        descendants::<gtk::Label>(scroller)
            .iter()
            .filter(|label| label.has_css_class("dialog-hint") && label.is_drawable())
            .filter_map(|label| label.compute_bounds(scroller))
            .map(|bounds| bounds.x() + bounds.width())
            .fold(0.0, f32::max)
    }

    /// The scroll bar never covers the keys: they keep 12 pixels clear of
    /// the right edge, the width of an overlay scroll bar under the
    /// pointer, and sit left of an always-shown scroll bar.
    ///
    /// parity: CMD-032
    #[gtk::test]
    fn the_scroll_bar_never_covers_the_keys() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window.set_default_size(900, 480);
        wait_until("the window to be short", || {
            (test.window.height() - 480).abs() <= 8
        });
        let dialog = opened(&test);
        let scroller = body_scroller(&dialog);
        assert!(scroller.vscrollbar().is_visible(), "the list scrolls");

        #[expect(clippy::cast_precision_loss, reason = "widths are small")]
        let width = scroller.width() as f32;
        let right = right_edge_of_keys(&scroller);
        assert!(right > 0.0, "keys are shown");
        assert!(
            right <= width - 12.0,
            "keys end at {right} of {width}: under the overlay bar"
        );

        scroller.set_overlay_scrolling(false);
        wait_for_frames(&dialog, 3);
        let bar = scroller
            .vscrollbar()
            .compute_bounds(&scroller)
            .expect("the bar is laid out");
        let right = right_edge_of_keys(&scroller);
        assert!(
            right <= bar.x(),
            "keys end at {right}, the bar starts at {}",
            bar.x()
        );
        dialog.finish();
    }

    /// Puts the text size back to its default when dropped.
    struct TextSizeReset<'a>(&'a TestWindow);

    impl Drop for TextSizeReset<'_> {
        fn drop(&mut self) {
            self.0.activate("text-reset", None);
        }
    }

    /// Over the narrowest window (670 pixels, its minimum) with the largest
    /// text, the shortcuts window is no wider than the window: long names
    /// and keys wrap instead.
    ///
    /// parity: CMD-032
    #[gtk::test]
    fn the_shortcuts_window_fits_a_narrow_window() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        // The text size is shared by every test window: put it back after
        // this test, also when it fails.
        let _reset = TextSizeReset(&test);
        for _ in 0..10 {
            test.activate("text-larger", None);
        }
        test.window.set_default_size(670, 720);
        wait_until("the window to be narrow", || {
            (test.window.width() - 670).abs() <= 8
        });
        let dialog = opened(&test);
        assert!(
            dialog.width() <= test.window.width(),
            "{} pixels wide over a {}-pixel window",
            dialog.width(),
            test.window.width()
        );
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
            keys_of("Open the navigation pane to the current folder, while it is shown"),
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
