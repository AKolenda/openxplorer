// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar's menus: New, Sort, View, More options and the
//! appearance choices.
//!
//! Ports `openNewMenu`, the Sort and View menus of `setup()`, the More
//! options menu and `appearanceMenu` in `v2.0.0:desktop/ui/app.js`, in their
//! order. Each item runs a window or application action; the choices and
//! toggles show a check mark while their action's state matches.

use ox_core::i18n::gettext;
use ox_core::settings::Theme;

use crate::application::AppAction;
use crate::folder_view::grid::IconSize;
use crate::folder_view::sort_roles::SortRole;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::icons::Icon;
use crate::text_size::Step;
use crate::window::folder_pane::FolderView;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;

/// A menu line that runs `action`.
fn item(label: &str, glyph: Icon, action: WindowAction) -> MenuEntry {
    MenuItem::new(label, glyph, action).into()
}

/// The New menu (`openNewMenu`), which the folder background's "New…"
/// opens too, with the user's `templates` before "From template…"
/// (OPS-003) and Dolphin's link item (OPS-004).
pub(in crate::window) fn new_menu(templates: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuItem::new(&gettext("Folder"), Icon::FolderAdd, WindowAction::NewFolder)
            .with_shortcut("Ctrl+Shift+N")
            .into(),
        item(
            &gettext("Text document"),
            Icon::DocumentText,
            WindowAction::NewTextDocument,
        ),
        item(&gettext("File…"), Icon::DocumentAdd, WindowAction::NewFile),
        MenuEntry::Divider,
        item(
            &gettext("Markdown document"),
            Icon::Markdown,
            WindowAction::NewMarkdownDocument,
        ),
        item(&gettext("CSV file"), Icon::Table, WindowAction::NewCsvFile),
        item(&gettext("JSON file"), Icon::Braces, WindowAction::NewJsonFile),
        item(
            &gettext("HTML document"),
            Icon::Code,
            WindowAction::NewHtmlDocument,
        ),
        MenuEntry::Divider,
    ];
    if !templates.is_empty() {
        entries.extend(templates);
        entries.push(MenuEntry::Divider);
    }
    entries.extend([
        item(
            &gettext("From template…"),
            Icon::DocumentCopy,
            WindowAction::NewFromTemplate,
        ),
        MenuEntry::Divider,
        item(
            &gettext("Link to file or folder…"),
            Icon::Link,
            WindowAction::NewLink,
        ),
    ]);
    entries
}

/// The Sort menu's item for `column`.
fn column_item(column: SortColumn) -> MenuEntry {
    MenuItem::choice(
        column.label(),
        Icon::ArrowSort,
        WindowAction::Sort,
        column.as_str(),
    )
    .into()
}

/// The Sort menu's item for `direction`.
fn direction_item(label: &str, glyph: Icon, direction: SortDirection) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::Direction, direction.as_str()).into()
}

/// The Sort menu's item for a further key (VIEW-019).
fn role_item(role: SortRole) -> MenuEntry {
    MenuItem::choice(role.label(), Icon::ArrowSort, WindowAction::Sort, role.as_str()).into()
}

/// The Sort menu: the columns and Dolphin's further keys, then the
/// direction, then grouping and folders first. The direction has an item
/// each, where app.js had one item that flips it.
pub(in crate::window) fn sort_menu() -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = SortColumn::IN_SORT_MENU.into_iter().map(column_item).collect();
    entries.push(MenuEntry::Divider);
    entries.extend(SortRole::ALL.into_iter().map(role_item));
    entries.extend([
        MenuEntry::Divider,
        direction_item(&gettext("Ascending"), Icon::ArrowUp, SortDirection::Ascending),
        direction_item(&gettext("Descending"), Icon::ArrowDown, SortDirection::Descending),
        MenuEntry::Divider,
        MenuItem::toggle(
            &ox_core::i18n::gettext("Show in groups"),
            Icon::TextBulletList,
            WindowAction::Groups,
        )
        .into(),
        MenuItem::toggle(
            &ox_core::i18n::gettext("Folders first"),
            Icon::Folder,
            WindowAction::FoldersFirst,
        )
        .into(),
    ]);
    entries
}

/// The View menu's item for `view`, showing its Explorer shortcut.
fn view_item(label: &str, glyph: Icon, view: FolderView) -> MenuEntry {
    let item = MenuItem::choice(label, glyph, WindowAction::View, view.as_str());
    match view.shortcut() {
        Some((_, shortcut)) => item.with_shortcut(shortcut).into(),
        None => item.into(),
    }
}

/// The View menu's item for a text-size `step`, showing its `shortcut`.
fn text_size_item(label: &str, glyph: Icon, step: Step, shortcut: &'static str) -> MenuEntry {
    MenuItem::new(label, glyph, WindowAction::TextSize(step))
        .with_shortcut(shortcut)
        .into()
}

/// The View menu: the views (Details, List and Explorer's four icon
/// sizes), the hidden-files, details-pane and navigation-pane toggles,
/// Dolphin's display style dialog, then the text size.
pub(in crate::window) fn view_menu() -> Vec<MenuEntry> {
    let details = view_item(&gettext("Details"), Icon::TextBulletList, FolderView::Details);
    let compact = view_item(&gettext("List"), Icon::Table, FolderView::Compact);
    let mut entries = vec![details, compact];
    let icon_sizes = IconSize::NAMED.into_iter().filter_map(|size| {
        let label = size.label()?;
        Some(view_item(label, Icon::Grid, FolderView::Icons(size)))
    });
    entries.extend(icon_sizes);
    entries.extend([
        MenuEntry::Divider,
        MenuItem::toggle(&gettext("Show hidden files"), Icon::Eye, WindowAction::Hidden)
            .with_shortcut("Ctrl+H")
            .into(),
        // With the panes and options Windows 11 lists under View > Show.
        MenuItem::toggle(
            &gettext("Compact view"),
            Icon::TextBulletList,
            WindowAction::CompactView,
        )
        .into(),
        MenuItem::toggle(
            &gettext("Details pane"),
            Icon::PanelRight,
            WindowAction::DetailsPane,
        )
        .with_shortcut("Alt+Shift+P")
        .into(),
        MenuItem::toggle(&gettext("Navigation pane"), Icon::Folder, WindowAction::Sidebar)
            .with_shortcut("F9")
            .into(),
        MenuItem::toggle(
            &ox_core::i18n::gettext("Split view"),
            Icon::PanelRight,
            WindowAction::SplitView,
        )
        .with_shortcut("F3")
        .into(),
        MenuItem::toggle(
            &gettext("Folder tree"),
            Icon::Organization,
            WindowAction::FolderTree,
        )
        .with_shortcut("F7")
        .into(),
        // Dolphin's Terminal panel embeds Konsole; VTE for GTK 4 is not
        // available everywhere the app ships, so this opens the desktop's
        // terminal in the folder shown instead (OPEN-022).
        MenuItem::new(
            &gettext("Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenTerminal,
        )
        .with_shortcut("Ctrl+Shift+F4")
        .into(),
        MenuEntry::Divider,
        item(
            ox_core::i18n::gettext_static("Adjust view display style…"),
            Icon::Settings,
            WindowAction::ViewProperties,
        ),
        MenuEntry::Divider,
        text_size_item(&gettext("Larger text"), Icon::Add, Step::Increase, "Ctrl++"),
        text_size_item(&gettext("Smaller text"), Icon::Subtract, Step::Decrease, "Ctrl+−"),
        text_size_item(
            &gettext("Reset text size"),
            Icon::ArrowReset,
            Step::Reset,
            "Ctrl+0",
        ),
    ]);
    entries
}

/// The appearance menu's item for `theme`.
fn theme_item(label: &str, glyph: Icon, theme: Theme) -> MenuEntry {
    MenuItem::choice(label, glyph, WindowAction::Theme, theme.as_str()).into()
}

/// The three appearance choices (`appearanceMenu`).
pub(super) fn appearance_items() -> [MenuEntry; 3] {
    [
        theme_item(&gettext("Light appearance"), Icon::WeatherSunny, Theme::Light),
        theme_item(&gettext("Dark appearance"), Icon::WeatherMoon, Theme::Dark),
        theme_item(&gettext("Use system appearance"), Icon::Desktop, Theme::System),
    ]
}

/// The More options menu, plus the selection commands the native context
/// menu used to hold.
pub(super) fn more_menu() -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuItem::new(&gettext("New window"), Icon::WindowNew, AppAction::NewWindow)
            .with_shortcut("Ctrl+N")
            .into(),
        item(&gettext("Settings"), Icon::Settings, WindowAction::Settings),
        item(
            &gettext("Default file explorer…"),
            Icon::Folder,
            WindowAction::DefaultFileExplorer,
        ),
        MenuItem::toggle(
            &gettext("Cache this folder for search"),
            Icon::Search,
            WindowAction::CacheFolder,
        )
        .into(),
        item(
            &gettext("Map network location"),
            Icon::Organization,
            WindowAction::MapNetworkLocation,
        ),
        item(&gettext("Pin current folder"), Icon::Pin, WindowAction::PinFolder),
        MenuEntry::Divider,
    ];
    entries.extend(appearance_items());
    entries.push(MenuItem::toggle(&gettext("Show hidden files"), Icon::Eye, WindowAction::Hidden).into());
    entries.extend([
        MenuEntry::Divider,
        MenuItem::new(&gettext("Select all"), Icon::SelectAllOn, WindowAction::SelectAll)
            .with_shortcut("Ctrl+A")
            .into(),
        item(
            &gettext("Select none"),
            Icon::SelectAllOff,
            WindowAction::SelectNone,
        ),
        item(
            &gettext("Invert selection"),
            Icon::ArrowSwap,
            WindowAction::InvertSelection,
        ),
        item(
            &gettext("Select items matching…"),
            Icon::Search,
            WindowAction::SelectMatching,
        ),
        MenuEntry::Divider,
        // app.js asked for a `code` glyph it did not have and drew a
        // document; the native app has the code glyph.
        MenuItem::new(
            &ox_core::i18n::gettext("Keyboard shortcuts"),
            Icon::Table,
            WindowAction::KeyboardShortcuts,
        )
        .with_shortcut("Ctrl+?")
        .into(),
        MenuItem::new(
            &ox_core::i18n::gettext("Help"),
            Icon::DocumentText,
            WindowAction::Help,
        )
        .with_shortcut("F1")
        .into(),
        item(&gettext("License & source"), Icon::Code, WindowAction::License),
        item(&gettext("About this build"), Icon::Info, WindowAction::About),
    ]);
    entries
}
