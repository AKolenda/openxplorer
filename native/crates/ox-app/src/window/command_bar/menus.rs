// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar's menus: New, Sort, View, More options and the
//! appearance choices.
//!
//! Ports `openNewMenu`, the Sort and View menus of `setup()`, the More
//! options menu and `appearanceMenu` in `desktop/ui/app.js`, in their
//! order. Each item runs a window or application action; the choices and
//! toggles show a check mark while their action's state matches.

use crate::application::AppAction;
use crate::folder_view::grid::IconSize;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::icons::Glyph;
use crate::text_size::Step;
use crate::theme::ThemePreference;
use crate::window::folder_pane::FolderView;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;

/// A menu line that runs `action`.
fn item(label: &str, glyph: Glyph, action: WindowAction) -> MenuEntry {
    MenuItem::new(label, glyph, action).into()
}

/// The New menu (`openNewMenu`).
pub(super) fn new_menu() -> Vec<MenuEntry> {
    vec![
        MenuItem::new("Folder", Glyph::FolderLine, WindowAction::NewFolder)
            .with_shortcut("Ctrl+Shift+N")
            .into(),
        item("Text document", Glyph::Documents, WindowAction::NewTextDocument),
        item("File…", Glyph::Documents, WindowAction::NewFile),
        MenuEntry::Divider,
        item(
            "Markdown document",
            Glyph::Documents,
            WindowAction::NewMarkdownDocument,
        ),
        item("CSV file", Glyph::List, WindowAction::NewCsvFile),
        item("JSON file", Glyph::Documents, WindowAction::NewJsonFile),
        item("HTML document", Glyph::Documents, WindowAction::NewHtmlDocument),
        MenuEntry::Divider,
        item("From template…", Glyph::Copy, WindowAction::NewFromTemplate),
    ]
}

/// The Sort menu's item for `column`.
fn column_item(column: SortColumn) -> MenuEntry {
    MenuItem::choice(column.label(), Glyph::Sort, WindowAction::Sort, column.key()).into()
}

/// The Sort menu's item for `direction`.
fn direction_item(label: &str, glyph: Glyph, direction: SortDirection) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::Direction, direction.key()).into()
}

/// The Sort menu: the columns, then the direction. The direction has an
/// item each, where app.js had one item that flips it.
pub(super) fn sort_menu() -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = SortColumn::ALL.into_iter().map(column_item).collect();
    entries.extend([
        MenuEntry::Divider,
        direction_item("Ascending", Glyph::Up, SortDirection::Ascending),
        direction_item("Descending", Glyph::Down, SortDirection::Descending),
    ]);
    entries
}

/// The View menu's item for `view`.
fn view_item(label: &str, glyph: Glyph, view: FolderView) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::View, view.key()).into()
}

/// The View menu's item for a text-size `step`, showing its `shortcut`.
fn text_size_item(label: &str, glyph: Glyph, step: Step, shortcut: &'static str) -> MenuEntry {
    MenuItem::new(label, glyph, WindowAction::TextSize(step))
        .with_shortcut(shortcut)
        .into()
}

/// The View menu: the views (every icon size the native app has), the
/// hidden-files and details-pane toggles, then the text size.
pub(super) fn view_menu() -> Vec<MenuEntry> {
    let mut entries = vec![view_item("Details", Glyph::List, FolderView::Details)];
    let icon_sizes = IconSize::ALL
        .into_iter()
        .map(|size| view_item(size.label(), Glyph::Grid, FolderView::Icons(size)));
    entries.extend(icon_sizes);
    entries.extend([
        MenuEntry::Divider,
        MenuItem::toggle("Show hidden files", Glyph::Eye, WindowAction::Hidden).into(),
        MenuItem::toggle("Details pane", Glyph::Details, WindowAction::DetailsPane).into(),
        MenuEntry::Divider,
        text_size_item("Larger text", Glyph::Plus, Step::Increase, "Ctrl++"),
        text_size_item("Smaller text", Glyph::Minus, Step::Decrease, "Ctrl+−"),
        text_size_item("Reset text size", Glyph::Refresh, Step::Reset, "Ctrl+0"),
    ]);
    entries
}

/// The appearance menu's item for `preference`.
fn theme_item(label: &str, glyph: Glyph, preference: ThemePreference) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::Theme, preference.key()).into()
}

/// The three appearance choices (`appearanceMenu`).
pub(super) fn appearance_items() -> [MenuEntry; 3] {
    [
        theme_item("Light appearance", Glyph::Sun, ThemePreference::Light),
        theme_item("Dark appearance", Glyph::Moon, ThemePreference::Dark),
        theme_item("Use system appearance", Glyph::Desktop, ThemePreference::System),
    ]
}

/// The More options menu, plus the selection commands the native context
/// menu used to hold.
pub(super) fn more_menu() -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuItem::new("New window", Glyph::Plus, AppAction::NewWindow)
            .with_shortcut("Ctrl+N")
            .into(),
        item("Settings", Glyph::Settings, WindowAction::Settings),
        item(
            "Default file explorer…",
            Glyph::FolderLine,
            WindowAction::DefaultFileExplorer,
        ),
        item(
            "Cache this folder for search",
            Glyph::Search,
            WindowAction::CacheFolder,
        ),
        item(
            "Map network location",
            Glyph::Network,
            WindowAction::MapNetworkLocation,
        ),
        item("Pin current folder", Glyph::Pin, WindowAction::PinFolder),
        MenuEntry::Divider,
    ];
    entries.extend(appearance_items());
    entries.push(MenuItem::toggle("Show hidden files", Glyph::Eye, WindowAction::Hidden).into());
    entries.extend([
        MenuEntry::Divider,
        MenuItem::new("Select all", Glyph::List, WindowAction::SelectAll)
            .with_shortcut("Ctrl+A")
            .into(),
        item("Select none", Glyph::Cancel, WindowAction::SelectNone),
        item("Invert selection", Glyph::Refresh, WindowAction::InvertSelection),
        MenuEntry::Divider,
        item("License & source", Glyph::Documents, WindowAction::License),
        item("About this build", Glyph::Info, WindowAction::About),
    ]);
    entries
}
