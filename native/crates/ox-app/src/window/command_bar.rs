// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar under the navigation row.
//!
//! Ports `section.commandbar` in `desktop/ui/index.html` and its menus in
//! `setup()` and `openNewMenu` of `desktop/ui/app.js`, in the same order:
//! New ▾ │ Cut, Copy, Paste, Rename, Copy path, Move to Trash │ Sort ▾,
//! View ▾, More options, then at the right the appearance toggle, Settings
//! and the Details toggle. Every control runs a window or application
//! action; commands whose workflow is not ported are disabled actions
//! ([`super::unported`]), so they show greyed out with a tooltip.

use gtk::prelude::*;

use crate::folder_view::grid::IconSize;
use crate::folder_view::sorting::SortColumn;
use crate::icons::{self, Glyph};
use crate::theme::Appearance;

use super::breakpoints::WindowWidth;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::unported;

/// Whether a command stays in a compact window (the 680-pixel rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InCompactWindow {
    /// Shown at every width.
    Kept,
    /// Hidden at 680 pixels or less (`.commandbar #cut{display:none}`).
    Hidden,
}

/// An icon-only command (`button.command` in index.html).
struct IconCommand {
    glyph: Glyph,
    action: &'static str,
    /// The accessible name (`aria-label`).
    name: &'static str,
    /// The tooltip (`title`).
    tooltip: &'static str,
    compact: InCompactWindow,
}

/// Cut to Move to Trash, as index.html lists them.
const EDIT_COMMANDS: [IconCommand; 6] = [
    IconCommand {
        glyph: Glyph::Cut,
        action: "win.cut",
        name: "Cut",
        tooltip: "Cut (Ctrl+X)",
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Glyph::Copy,
        action: "win.copy",
        name: "Copy",
        tooltip: "Copy (Ctrl+C)",
        compact: InCompactWindow::Kept,
    },
    IconCommand {
        glyph: Glyph::Paste,
        action: "win.paste",
        name: "Paste",
        tooltip: "Paste files (Ctrl+V)",
        compact: InCompactWindow::Kept,
    },
    IconCommand {
        glyph: Glyph::Rename,
        action: "win.rename",
        name: "Rename",
        tooltip: "Rename (F2)",
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Glyph::Share,
        action: "win.copy-path",
        name: "Copy path",
        tooltip: "Copy path (does not change sharing permissions)",
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Glyph::Trash,
        action: "win.trash",
        name: "Move to Trash",
        tooltip: "Move to Trash (Delete)",
        compact: InCompactWindow::Kept,
    },
];

/// The command bar's widgets that the window updates.
#[derive(Debug)]
pub(super) struct CommandBar {
    /// The bar.
    pub root: gtk::Box,
    theme: gtk::MenuButton,
    theme_glyph: gtk::Image,
    theme_label: gtk::Label,
    /// Cut, Rename, Copy path and Details, which a compact window hides.
    hidden_when_compact: Vec<gtk::Widget>,
}

impl CommandBar {
    /// The command bar, showing the light appearance until told otherwise.
    pub fn new() -> Self {
        // The gaps are CSS `border-spacing`, which narrow windows shrink.
        let root = gtk::Box::builder().css_classes(["commandbar"]).build();
        root.update_property(&[gtk::accessible::Property::Label("File commands")]);
        let mut hidden_when_compact = Vec::new();
        root.append(&file_commands(&mut hidden_when_compact));
        let theme_glyph = icons::glyph(Appearance::Light.glyph(), 17);
        let theme_label = gtk::Label::new(Some(Appearance::Light.label()));
        let theme = theme_button(&theme_glyph, &theme_label);
        root.append(&theme);
        let settings = icon_button(&IconCommand {
            glyph: Glyph::Settings,
            action: "win.settings",
            name: "Settings",
            tooltip: "Settings (Ctrl+,)",
            compact: InCompactWindow::Kept,
        });
        settings.add_css_class("settings-button");
        root.append(&settings);
        let details = details_toggle();
        hidden_when_compact.push(details.clone().upcast());
        root.append(&details);
        Self {
            root,
            theme,
            theme_glyph,
            theme_label,
            hidden_when_compact,
        }
    }

    /// Shows the drawn appearance on the theme button: a sun and "Light"
    /// or a moon and "Dark", with `tooltip` saying what was chosen
    /// (`applyTheme` in app.js).
    pub fn show_appearance(&self, appearance: Appearance, tooltip: &str) {
        icons::set_glyph(&self.theme_glyph, appearance.glyph(), 17);
        self.theme_label.set_text(appearance.label());
        self.theme.set_tooltip_text(Some(tooltip));
    }

    /// Hides what the web layout hides in a window of `band`'s width: the
    /// appearance label from 1050 pixels, and Cut, Rename, Copy path and
    /// Details from 680.
    pub fn fit_to_width(&self, band: WindowWidth) {
        self.theme_label.set_visible(band.shows_appearance_label());
        for control in &self.hidden_when_compact {
            control.set_visible(!band.is_compact());
        }
    }

    /// The theme button's tooltip, for tests.
    #[cfg(test)]
    pub fn appearance_tooltip(&self) -> Option<String> {
        self.theme.tooltip_text().map(|text| text.to_string())
    }
}

/// New ▾ │ Cut … Move to Trash │ Sort ▾, View ▾ and More options, adding
/// the commands a compact window hides to `hidden_when_compact`. They
/// scroll without a scroll bar, so the window can be narrower than all the
/// commands (the web bar clips them); the appearance, Settings and Details
/// buttons stay at the right.
fn file_commands(hidden_when_compact: &mut Vec<gtk::Widget>) -> gtk::ScrolledWindow {
    let group = gtk::Box::builder().css_classes(["command-group"]).build();
    group.append(&text_menu_button("New", Glyph::Plus, new_menu()));
    group.append(&separator());
    for command in &EDIT_COMMANDS {
        let button = icon_button(command);
        if command.compact == InCompactWindow::Hidden {
            hidden_when_compact.push(button.clone().upcast());
        }
        group.append(&button);
    }
    group.append(&separator());
    group.append(&text_menu_button("Sort", Glyph::Sort, sort_menu()));
    group.append(&text_menu_button("View", Glyph::Grid, view_menu()));
    group.append(&more_button());
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true)
        .hexpand(true)
        .child(&group)
        .build()
}

fn separator() -> gtk::Separator {
    gtk::Separator::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .build()
}

fn icon_button(command: &IconCommand) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::glyph(command.glyph, 18))
        .tooltip_text(unported::tooltip(command.action, command.tooltip))
        .action_name(command.action)
        .valign(gtk::Align::Center)
        .css_classes(["command"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(command.name)]);
    button
}

/// A glyph, a label and the chevron that marks a menu (`setButton` with
/// `arrow`).
fn text_menu_content(label: &str, glyph: Glyph) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    content.append(&icons::glyph(glyph, 17));
    content.append(&gtk::Label::new(Some(label)));
    let chevron = icons::glyph(Glyph::Down, 17);
    chevron.add_css_class("chevron");
    content.append(&chevron);
    content
}

fn text_menu_button(label: &str, glyph: Glyph, entries: Vec<MenuEntry>) -> gtk::MenuButton {
    gtk::MenuButton::builder()
        .child(&text_menu_content(label, glyph))
        .popover(&MenuPopover::new(entries))
        .valign(gtk::Align::Center)
        .css_classes([
            "command",
            "text-command",
            &format!("{}-command", label.to_lowercase()),
        ])
        .build()
}

/// The New menu (`openNewMenu`).
fn new_menu() -> Vec<MenuEntry> {
    let item = |label: &str, glyph: Glyph, action: &str| MenuEntry::Item(MenuItem::new(label, glyph, action));
    vec![
        MenuItem::new("Folder", Glyph::FolderLine, "win.new-folder")
            .with_shortcut("Ctrl+Shift+N")
            .into(),
        item("Text document", Glyph::Documents, "win.new-text-document"),
        item("File…", Glyph::Documents, "win.new-file"),
        MenuEntry::Divider,
        item("Markdown document", Glyph::Documents, "win.new-markdown-document"),
        item("CSV file", Glyph::List, "win.new-csv-file"),
        item("JSON file", Glyph::Documents, "win.new-json-file"),
        item("HTML document", Glyph::Documents, "win.new-html-document"),
        MenuEntry::Divider,
        item("From template…", Glyph::Copy, "win.new-from-template"),
    ]
}

/// The Sort menu: the columns, then the direction. The direction has an
/// item each, where app.js had one item that flips it.
fn sort_menu() -> Vec<MenuEntry> {
    let columns = SortColumn::ALL
        .into_iter()
        .map(|column| MenuItem::choice(column.label(), Glyph::Sort, "win.sort", column.key()).into());
    let mut entries: Vec<MenuEntry> = columns.collect();
    entries.push(MenuEntry::Divider);
    entries.push(MenuItem::choice("Ascending", Glyph::Up, "win.direction", "ascending").into());
    entries.push(MenuItem::choice("Descending", Glyph::Down, "win.direction", "descending").into());
    entries
}

/// The View menu: the views (every icon size the native app has), the
/// hidden-files and details-pane toggles, then the text size.
fn view_menu() -> Vec<MenuEntry> {
    let mut entries = vec![MenuItem::choice("Details", Glyph::List, "win.view", "details").into()];
    let icon_sizes = IconSize::ALL
        .into_iter()
        .map(|size| MenuItem::choice(size.label(), Glyph::Grid, "win.view", size.key()).into());
    entries.extend(icon_sizes);
    entries.extend([
        MenuEntry::Divider,
        MenuItem::toggle("Show hidden files", Glyph::Eye, "win.hidden").into(),
        MenuItem::toggle("Details pane", Glyph::Details, "win.details-pane").into(),
        MenuEntry::Divider,
        MenuItem::new("Larger text", Glyph::Plus, "win.text-larger")
            .with_shortcut("Ctrl++")
            .into(),
        MenuItem::new("Smaller text", Glyph::Minus, "win.text-smaller")
            .with_shortcut("Ctrl+−")
            .into(),
        MenuItem::new("Reset text size", Glyph::Refresh, "win.text-reset")
            .with_shortcut("Ctrl+0")
            .into(),
    ]);
    entries
}

/// The three appearance choices (`appearanceMenu`).
fn appearance_items() -> [MenuEntry; 3] {
    [
        MenuItem::choice("Light appearance", Glyph::Sun, "win.theme", "light").into(),
        MenuItem::choice("Dark appearance", Glyph::Moon, "win.theme", "dark").into(),
        MenuItem::choice("Use system appearance", Glyph::Desktop, "win.theme", "system").into(),
    ]
}

/// The More options menu, plus the selection commands the native context
/// menu used to hold.
fn more_menu() -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuItem::new("New window", Glyph::Plus, "app.new-window")
            .with_shortcut("Ctrl+N")
            .into(),
        MenuItem::new("Settings", Glyph::Settings, "win.settings").into(),
        MenuItem::new(
            "Default file explorer…",
            Glyph::FolderLine,
            "win.default-file-explorer",
        )
        .into(),
        MenuItem::new("Cache this folder for search", Glyph::Search, "win.cache-folder").into(),
        MenuItem::new("Map network location", Glyph::Network, "win.map-network-location").into(),
        MenuItem::new("Pin current folder", Glyph::Pin, "win.pin-folder").into(),
        MenuEntry::Divider,
    ];
    entries.extend(appearance_items());
    entries.push(MenuItem::toggle("Show hidden files", Glyph::Eye, "win.hidden").into());
    entries.extend([
        MenuEntry::Divider,
        MenuItem::new("Select all", Glyph::List, "win.select-all")
            .with_shortcut("Ctrl+A")
            .into(),
        MenuItem::new("Select none", Glyph::Cancel, "win.select-none").into(),
        MenuItem::new("Invert selection", Glyph::Refresh, "win.invert-selection").into(),
        MenuEntry::Divider,
        MenuItem::new("License & source", Glyph::Documents, "win.license").into(),
        MenuItem::new("About this build", Glyph::Info, "win.about").into(),
    ]);
    entries
}

fn more_button() -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .child(&icons::glyph(Glyph::More, 18))
        .tooltip_text("More options")
        .popover(&MenuPopover::new(more_menu()))
        .valign(gtk::Align::Center)
        .css_classes(["command", "more-command"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("More options")]);
    button
}

/// The appearance toggle (`#theme-toggle`) showing `glyph` and `label`;
/// [`CommandBar::show_appearance`] sets them and the tooltip.
fn theme_button(glyph: &gtk::Image, label: &gtk::Label) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    content.append(glyph);
    content.append(label);
    let button = gtk::MenuButton::builder()
        .child(&content)
        .popover(&MenuPopover::new(appearance_items().to_vec()))
        .valign(gtk::Align::Center)
        .css_classes(["command", "text-command", "theme-toggle"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Appearance")]);
    button
}

/// The Details button shows whether the pane is open: a toggle bound to
/// the boolean `win.details-pane` action.
fn details_toggle() -> gtk::ToggleButton {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    content.append(&icons::glyph(Glyph::Details, 17));
    content.append(&gtk::Label::new(Some("Details")));
    gtk::ToggleButton::builder()
        .child(&content)
        .action_name("win.details-pane")
        .valign(gtk::Align::Center)
        .css_classes(["command", "text-command", "details-toggle"])
        .build()
}
