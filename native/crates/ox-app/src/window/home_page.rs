// SPDX-License-Identifier: AGPL-3.0-only
//! Home laid out as Windows 11 File Explorer's Home (HOME-012 to
//! HOME-016), while Settings > General > Start up > "Open Quick access
//! on Home" is on. It is off by default: Home then opens the home
//! folder, as the Python app's did (NAV-038).
//!
//! Only what Microsoft documents is drawn:
//!
//! - Quick access: the navigation pane's pinned folders, the same list
//!   ("available by default as pinned folders in Quick access in both
//!   File Explorer Home and the left navigation pane",
//!   support.microsoft.com, File Explorer in Windows), as tiles of the
//!   Tiles view, as This PC draws its drives. A tile's menu is the pin's,
//!   with Unpin from Quick access; a middle-click opens the folder in a
//!   background tab (Windows Insider blog, 2026-08-17).
//! - A switcher between Recent and Favorites ("File Explorer Home with
//!   new tabs for Recent, Favorites, and Shared", Windows Insider blog,
//!   build 22635.3930). Shared lists files shared through a Microsoft
//!   account, so it is left out.
//! - Recent: the desktop's recently used files, as `GVfs` lists them at
//!   `recent:///`, the same source as the navigation pane's Recent files;
//!   a row opens the real file (`EntryMeta::recent_target`) and offers
//!   Open file location (Windows Insider blog, Release Preview
//!   26100.7015).
//! - Favorites: files the user added (pinned files became Favorites,
//!   Windows Insider blog, builds 22557 and 22593), saved in the
//!   settings file as `favorites`.
//! - Sections fold and stay folded ("Home now preserves whether sections
//!   are expanded or collapsed", Windows Insider blog, 2026-08-17), saved
//!   as `homeFolded`.
//!
//! Nothing here sends anything anywhere: Recent shows only what `GVfs`
//! already lists, and Favorites is a list in the settings file.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry};
use ox_core::location::{same_location, LocationContext, RECENT_URI};
use ox_core::places::Place;
use ox_core::settings::{favorite_location, Preferences, SettingsError};

use crate::icons::{self, Art, ArtImage, Icon, Storage};
use crate::places::Places;
use crate::settings_store::Change;

use super::context_menu::{MENU_KEY, SHIFT_F10_KEY};
use super::gestures;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover, MenuStyle};
use super::place_menus::classic_or_compact;
use super::result_location::LocationTarget;
use super::sidebar::menu::pin_menu;
use super::widget_tree::{children, remove_children, toggle_class};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The class the page carries while it shows Home.
const HOME_CLASS: &str = "home-page";
/// The class the page carries once it has been drawn, after which its
/// own classes, not the saved choice, say what is folded.
const DRAWN_CLASS: &str = "home-drawn";
/// The class the page carries while Favorites is the list shown.
const FAVORITES_SHOWN_CLASS: &str = "home-favorites-shown";
/// Pixels of a chevron.
const CHEVRON: i32 = 16;
/// Pixels of a row's icon in the Recent and Favorites lists, as
/// Windows 11's Home draws them.
const ROW_ICON: i32 = 20;
/// Pixels of a Quick access tile's folder, as Windows 11's Home draws it.
const TILE_ICON: i32 = 48;
/// Pixels of the glyph a standard folder shows on its folder.
const TILE_EMBLEM: i32 = 20;
/// A Quick access tile's room inside its padding: Windows 11's Home puts
/// tiles about 284 pixels apart across and 60 down.
const TILE_CONTENT: (i32, i32) = (284, 60);
/// Pixels of the pin glyph under a pinned folder's location.
const PIN_GLYPH: i32 = 12;
/// Most tiles a row of Quick access holds; the pane's width decides.
const MOST_TILES_PER_ROW: u32 = 32;
/// The most recent files the list shows.
const MOST_RECENT: usize = 50;
/// What the Recent list reads of each row: an item's own attributes and
/// when it was used.
const RECENT_ATTRIBUTES: &str = concat!("recent::modified,", "standard::name");

/// A section of Home that folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    /// The pinned folders.
    QuickAccess,
    /// The Recent and Favorites lists.
    Files,
}

impl Section {
    const ALL: [Section; 2] = [Section::QuickAccess, Section::Files];

    /// The key `homeFolded` saves it under.
    const fn key(self) -> &'static str {
        match self {
            Section::QuickAccess => "quickAccess",
            Section::Files => "files",
        }
    }

    /// The class of its fold button.
    const fn fold_class(self) -> &'static str {
        match self {
            Section::QuickAccess => "home-quick-fold",
            Section::Files => "home-files-fold",
        }
    }

    /// The class of what it folds.
    const fn body_class(self) -> &'static str {
        match self {
            Section::QuickAccess => "home-quick-body",
            Section::Files => "home-files-body",
        }
    }

    /// The class the page carries while it is folded.
    const fn folded_class(self) -> &'static str {
        match self {
            Section::QuickAccess => "home-quick-folded",
            Section::Files => "home-files-folded",
        }
    }
}

/// The lists of the switcher, in the order Microsoft names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Recent,
    Favorites,
}

impl Tab {
    const ALL: [Tab; 2] = [Tab::Recent, Tab::Favorites];

    fn title(self) -> &'static str {
        match self {
            Tab::Recent => ox_core::i18n::gettext_static("Recent"),
            Tab::Favorites => ox_core::i18n::gettext_static("Favorites"),
        }
    }

    /// The glyph before the title: a clock for Recent, a star for
    /// Favorites, as on Windows 11's pills.
    const fn icon(self) -> Icon {
        match self {
            Tab::Recent => Icon::History,
            Tab::Favorites => Icon::Star,
        }
    }

    const fn class(self) -> &'static str {
        match self {
            Tab::Recent => "home-tab-recent",
            Tab::Favorites => "home-tab-favorites",
        }
    }

    const fn list_class(self) -> &'static str {
        match self {
            Tab::Recent => "home-recent",
            Tab::Favorites => "home-favorites",
        }
    }

    /// What the list says while it is empty: the app's own words.
    fn empty_text(self) -> &'static str {
        match self {
            Tab::Recent => ox_core::i18n::gettext_static("Files you open recently show here."),
            Tab::Favorites => ox_core::i18n::gettext_static("Files you add to Favorites show here."),
        }
    }
}

/// One tile or row of Home and what it does.
struct Item {
    child: gtk::FlowBoxChild,
    /// Double-click and Enter: an action and its target.
    open: (WindowAction, String),
    /// The right-click menu, in the compact style.
    menu: Vec<MenuEntry>,
    /// What Alt+Enter shows the Properties of.
    properties: String,
}

impl Item {
    fn open(&self) {
        let (action, target) = &self.open;
        action.activate_from(&self.child, Some(&target.to_variant()));
    }

    /// Opens the menu at `x`, `y` in the item, hung from the folder pane,
    /// which outlives a redraw of Home.
    fn popup_menu(&self, x: f64, y: f64, classic: bool) {
        let window = self.child.root().and_downcast::<BrowserWindow>();
        let style = if classic {
            MenuStyle::Classic
        } else {
            window.map_or(MenuStyle::Compact, |window| window.preferred_menu_style())
        };
        let owner = self
            .child
            .ancestor(super::folder_pane::FolderPane::static_type())
            .unwrap_or_else(|| self.child.clone().upcast());
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let at = gtk::graphene::Point::new(x as f32, y as f32);
        let point = self.child.compute_point(&owner, &at).unwrap_or(at);
        let popover = MenuPopover::new(classic_or_compact(copy_entries(&self.menu), style));
        popover.set_style_and_strip(style, Vec::new());
        popover.set_offset(0, 0);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_parent(&owner);
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let pointing_to = gtk::gdk::Rectangle::new(point.x() as i32, point.y() as i32, 1, 1);
        popover.set_pointing_to(Some(&pointing_to));
        popover.connect_closed(|popover| {
            let closed = popover.clone();
            glib::idle_add_local_once(move || closed.unparent());
        });
        popover.popup();
    }
}

/// A copy of `entries`, which a menu takes for its own.
fn copy_entries(entries: &[MenuEntry]) -> Vec<MenuEntry> {
    entries.to_vec()
}

/// An item of a file's menu running `action` on `uri`.
fn file_item(label: &str, glyph: Icon, action: WindowAction, uri: &str) -> MenuEntry {
    MenuItem::with_text_target(&ox_core::i18n::gettext(label), glyph, action, uri).into()
}

/// The menu of a Recent or Favorites row for the file at `uri`.
fn file_menu(uri: &str, is_favorite: bool) -> Vec<MenuEntry> {
    let favorite = if is_favorite {
        file_item(
            "Remove from Favorites",
            Icon::StarFilled,
            WindowAction::RemoveFromFavorites,
            uri,
        )
    } else {
        file_item("Add to Favorites", Icon::Star, WindowAction::AddToFavorites, uri)
    };
    vec![
        file_item("Open", Icon::Open, WindowAction::OpenItemAt, uri),
        file_item(
            "Open file location",
            Icon::Folder,
            WindowAction::OpenFileLocationOf,
            uri,
        ),
        MenuEntry::Divider,
        favorite,
        MenuEntry::Divider,
        file_item("Copy as path", Icon::Copy, WindowAction::CopyPathOf, uri),
        file_item("Properties", Icon::Info, WindowAction::PropertiesOf, uri),
    ]
}

fn label(text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes([class])
        .build()
}

/// Where a Quick access folder is, for its tile's second line: "Home"
/// for a folder in the home folder, else the name of the folder it is
/// in. Windows shows its cloud status there, which has no meaning here.
fn folder_location(uri: &str, locations: &LocationContext) -> String {
    match ox_core::location::parent_location(uri) {
        Some(parent) => locations.title_for(&parent),
        None => String::new(),
    }
}

/// A Quick access tile as Windows 11's Home draws it: a 48-pixel folder
/// (a standard folder's glyph on it), then three lines: the name, where
/// the folder is, and the pin glyph.
fn folder_tile(place: &Place, window: Option<&BrowserWindow>, locations: &LocationContext) -> Item {
    let art = if place.is_shared {
        ArtImage::new(
            Art::for_quick_access(place.known_folder, Storage::Network),
            TILE_ICON,
        )
    } else {
        ArtImage::new(Art::Folder, TILE_ICON)
    };
    let icon = gtk::Overlay::builder()
        .child(&art)
        .valign(gtk::Align::Center)
        .build();
    if let Some(folder) = place.known_folder.filter(|_| !place.is_shared) {
        let emblem = ArtImage::new(Art::for_known_folder(folder), TILE_EMBLEM);
        emblem.set_halign(gtk::Align::Center);
        emblem.set_valign(gtk::Align::End);
        emblem.set_margin_bottom(6);
        emblem.add_css_class("home-emblem");
        icon.add_overlay(&emblem);
    }
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 1);
    lines.set_valign(gtk::Align::Center);
    lines.set_hexpand(true);
    lines.append(&label(&place.label, "tile-name"));
    lines.append(&label(
        &folder_location(&place.uri, locations),
        "home-tile-location",
    ));
    let pin = icons::image(Icon::Pin, PIN_GLYPH);
    pin.set_halign(gtk::Align::Start);
    pin.add_css_class("home-pin");
    pin.update_property(&[gtk::accessible::Property::Label(ox_core::i18n::gettext_static(
        "Pinned",
    ))]);
    lines.append(&pin);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    content.set_margin_start(8);
    content.append(&icon);
    content.append(&lines);
    content.set_size_request(TILE_CONTENT.0 - 8, TILE_CONTENT.1);
    let child = gtk::FlowBoxChild::builder()
        .child(&content)
        .name(&place.uri)
        .build();
    child.update_property(&[gtk::accessible::Property::Label(&place.label)]);
    gestures::open_folder_on_middle_click(&child, &place.uri);
    let caching = window.and_then(|window| window.caching_of(&place.uri));
    Item {
        child,
        open: (WindowAction::GoTo, place.uri.clone()),
        menu: pin_menu(&place.uri, caching, place.known_folder.is_none()),
        properties: place.uri.clone(),
    }
}

/// A file's File location, as Windows 11's Home lists it: the folder it
/// is in, from the home folder down with "/" between folders, "Home" for
/// the home folder itself, and the whole path of a folder elsewhere.
pub(super) fn file_location(uri: &str, home: &std::path::Path) -> String {
    let file = gio::File::for_uri(uri);
    let Some(parent) = file.parent() else {
        return String::new();
    };
    if let Some(path) = parent.path() {
        if path == home {
            return ox_core::i18n::gettext("Home");
        }
        if let Ok(inside) = path.strip_prefix(home) {
            return inside.to_string_lossy().into_owned();
        }
        return path.to_string_lossy().into_owned();
    }
    parent.parse_name().to_string()
}

/// The widths of the list's columns, as Windows 11's Home starts them:
/// Name 484 pixels from its text to Date accessed, Date accessed 298 to
/// File location, File location 300. They are not stretched to the
/// window's edge; a narrow window narrows Name first.
const NAME_WIDTH: i32 = 484;
const DATE_WIDTH: i32 = 298;
const LOCATION_WIDTH: i32 = 300;
/// Home's spacing, measured on Windows 11's Home: its chevrons about 30
/// pixels in from the pane's edge, the first tile's name about 37
/// pixels under the heading's middle, 45 pixels from the last tiles to
/// the pills, 51 from the pills' middle to the column headers' and 50
/// from there to the first row's.
const SECTION_INDENT: i32 = 14;
/// Quick access's tiles stand further in than the heading: their folders
/// about 92 pixels from the pane's edge, as in Windows 11.
const TILES_INDENT: i32 = 76;
const LIST_INDENT: i32 = 48;
const TILES_BELOW_HEADING: i32 = 6;
const PILLS_BELOW_TILES: i32 = 45;
const HEADER_BELOW_PILLS: i32 = 19;
const ROWS_BELOW_HEADER: i32 = 14;
/// The narrowest Name gets in a narrow window, and the narrowest Date
/// accessed and File location.
const NARROWEST_NAME: i32 = 140;
const NARROWEST_DETAIL: i32 = 90;
/// The page's own padding across (`.landing .page.home-page`).
const PAGE_PADDING: i32 = 20;

/// Runs `apply` with the width of the pane Home is scrolled in (what is
/// on screen, which a wide page does not widen) once `widget` is shown,
/// and again whenever that width changes, as the pane's horizontal
/// adjustment says. Nothing runs between changes, so an idle window
/// draws nothing.
fn follow_visible_width(widget: &impl IsA<gtk::Widget>, apply: Rc<dyn Fn(i32)>) {
    let watched: Rc<std::cell::RefCell<Option<(gtk::Adjustment, glib::SignalHandlerId)>>> = Rc::default();
    let on_map = watched.clone();
    widget.connect_map(move |widget| {
        let Some(scroller) = widget
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            return;
        };
        let adjustment = scroller.hadjustment();
        // Changes asked for while the pane is laid out wait until it is.
        let later = apply.clone();
        let id = adjustment.connect_page_size_notify(move |adjustment| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a pane is a few thousand pixels wide"
            )]
            let width = adjustment.page_size().round() as i32;
            let later = later.clone();
            glib::idle_add_local_once(move || later(width));
        });
        if let Some((earlier, earlier_id)) = on_map.replace(Some((adjustment, id))) {
            earlier.disconnect(earlier_id);
        }
        let first = apply.clone();
        let scroller = scroller.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(width) = scroller
                .upgrade()
                .map(|scroller| scroller.width())
                .filter(|width| *width > 0)
            {
                first(width);
            }
        });
    });
    widget.connect_unmap(move |_| {
        if let Some((adjustment, id)) = watched.take() {
            adjustment.disconnect(id);
        }
    });
}

/// Name, Date accessed and File location's widths in `room` pixels:
/// Windows' 484, 298 and 300 while they fit; then Name narrows, down to
/// 140; then the other two share what is left, down to 90 each.
fn column_widths(room: i32) -> (i32, i32, i32) {
    let details = DATE_WIDTH + LOCATION_WIDTH;
    if room >= NAME_WIDTH + details {
        return (NAME_WIDTH, DATE_WIDTH, LOCATION_WIDTH);
    }
    if room - details >= NARROWEST_NAME {
        return (room - details, DATE_WIDTH, LOCATION_WIDTH);
    }
    let left = (room - NARROWEST_NAME).max(2 * NARROWEST_DETAIL);
    let date = (left * DATE_WIDTH / details).max(NARROWEST_DETAIL);
    (NARROWEST_NAME, date, (left - date).max(NARROWEST_DETAIL))
}
/// Pixels from one row of the list to the next, as Windows 11's Home
/// spaces them, and in Compact view.
const ROW_PITCH: i32 = 36;
const COMPACT_ROW_PITCH: i32 = 26;
/// Pixels from a row's left edge to its icon, and from the icon to the
/// name: Windows puts names about 90 pixels in from the pane's edge.
const ROW_INDENT: i32 = 34;
const ICON_GAP: i32 = 14;

/// The columns of the Recent and Favorites list: every row's cells and
/// the header's share their widths, which the header sets.
#[derive(Clone)]
struct Columns {
    name: gtk::SizeGroup,
    date: gtk::SizeGroup,
    location: gtk::SizeGroup,
    home: std::path::PathBuf,
    pitch: i32,
}

impl Columns {
    /// Columns for rows `pitch` pixels apart.
    fn new(home: std::path::PathBuf, compact: bool) -> Self {
        Self {
            name: gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal),
            date: gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal),
            location: gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal),
            home,
            pitch: if compact { COMPACT_ROW_PITCH } else { ROW_PITCH },
        }
    }

    /// A row's or the header's cells, put in their column groups.
    fn cells(&self, name: &impl IsA<gtk::Widget>, date: &gtk::Label, location: &gtk::Label) {
        name.set_hexpand(false);
        for cell in [date, location] {
            cell.set_hexpand(false);
        }
        self.name.add_widget(name);
        self.date.add_widget(date);
        self.location.add_widget(location);
    }

    /// The header: Name, Date accessed, File location, at Windows'
    /// widths, Name narrowed while the list is too narrow for them.
    fn header(&self) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("home-list-header");
        row.set_halign(gtk::Align::Start);
        row.set_size_request(-1, ROW_PITCH);
        row.set_margin_top(HEADER_BELOW_PILLS);
        row.set_margin_bottom(ROWS_BELOW_HEADER);
        // Name stands over the rows' icons, as in Windows 11; the rows'
        // indent is a spacer, as a size group counts a cell's margins.
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_size_request(ROW_INDENT, -1);
        row.append(&spacer);
        let name = label(ox_core::i18n::gettext_static("Name"), "home-column");
        name.set_size_request(NAME_WIDTH, -1);
        let date = label(ox_core::i18n::gettext_static("Date accessed"), "home-column");
        date.set_size_request(DATE_WIDTH, -1);
        let location = label(ox_core::i18n::gettext_static("File location"), "home-column");
        location.set_size_request(LOCATION_WIDTH, -1);
        self.cells(&name, &date, &location);
        row.append(&name);
        row.append(&date);
        row.append(&location);
        let cells = [name.downgrade(), date.downgrade(), location.downgrade()];
        follow_visible_width(
            &row,
            Rc::new(move |room| {
                let room = room - LIST_INDENT - ROW_INDENT - PAGE_PADDING;
                let (name_width, date_width, location_width) = column_widths(room);
                for (cell, width) in cells.iter().zip([name_width, date_width, location_width]) {
                    if let Some(cell) = cell.upgrade() {
                        if cell.width_request() != width {
                            cell.set_size_request(width, -1);
                        }
                    }
                }
            }),
        );
        row
    }
}

/// A row of the Recent or Favorites list for the file named `name` at
/// `uri`, last used at `accessed`: its icon and name, Date accessed and
/// File location, as Windows 11's Home lists them.
fn file_row(name: &str, uri: &str, accessed: Option<u64>, favorites: &[String], columns: &Columns) -> Item {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("home-row");
    row.set_halign(gtk::Align::Start);
    row.set_size_request(-1, columns.pitch);
    let icon = ArtImage::new(Art::for_file_name(name), ROW_ICON);
    icon.set_margin_end(ICON_GAP);
    icon.set_valign(gtk::Align::Center);
    let name_text = label(name, "home-row-name");
    // The icon and the name share the Name column, as in Windows 11.
    let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    name_cell.append(&icon);
    name_cell.append(&name_text);
    let date = label(
        &ox_core::format::date_short_time_text(accessed),
        "home-row-detail",
    );
    let location = label(&file_location(uri, &columns.home), "home-row-detail");
    columns.cells(&name_cell, &date, &location);
    name_text.set_valign(gtk::Align::Center);
    for cell in [&date, &location] {
        cell.set_valign(gtk::Align::Center);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_size_request(ROW_INDENT, -1);
    row.append(&spacer);
    row.append(&name_cell);
    row.append(&date);
    row.append(&location);
    let child = gtk::FlowBoxChild::builder().child(&row).name(uri).build();
    child.update_property(&[gtk::accessible::Property::Label(name)]);
    let is_favorite = favorites.iter().any(|favorite| same_location(favorite, uri));
    Item {
        child,
        open: (WindowAction::OpenItemAt, uri.to_owned()),
        menu: file_menu(uri, is_favorite),
        properties: uri.to_owned(),
    }
}

/// The name a favorite shows: its file's name, read from its location
/// alone, so drawing Home reads no drive or share.
fn favorite_name(uri: &str) -> String {
    let file = gio::File::for_uri(uri);
    file.basename().map_or_else(
        || file.parse_name().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A chevron, pointing right while folded and down while open.
fn chevron(folded: bool) -> Icon {
    if folded {
        Icon::ChevronRight16
    } else {
        Icon::ChevronDown16
    }
}

/// The fold button of `section`, drawn in `page`.
fn fold_button(page: &gtk::Box, section: Section) -> Option<gtk::Button> {
    descendants_with_class(page, section.fold_class())
        .into_iter()
        .next()
        .and_downcast()
}

/// What `section` folds, drawn in `page`.
fn section_body(page: &gtk::Box, section: Section) -> Option<gtk::Widget> {
    children(page).find(|child| child.has_css_class(section.body_class()))
}

/// The widgets under `root` carrying `class`, in order.
fn descendants_with_class(root: &impl IsA<gtk::Widget>, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack: Vec<gtk::Widget> = children(root.upcast_ref()).collect();
    stack.reverse();
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) {
            found.push(widget.clone());
        }
        let mut below: Vec<gtk::Widget> = children(&widget).collect();
        below.reverse();
        stack.extend(below);
    }
    found
}

/// Folds or unfolds `section` of `page`, and saves the choice when `save`
/// says so.
fn set_folded(page: &gtk::Box, section: Section, folded: bool, save: bool) {
    toggle_class(page, section.folded_class(), folded);
    if let Some(body) = section_body(page, section) {
        body.set_visible(!folded);
    }
    if let Some(button) = fold_button(page, section) {
        if let Some(image) = descendants_of_type::<gtk::Image>(&button).into_iter().next() {
            icons::set_icon(&image, chevron(folded), CHEVRON);
        }
        button.update_state(&[gtk::accessible::State::Expanded(Some(!folded))]);
    }
    if save {
        if let Some(window) = page.root().and_downcast::<BrowserWindow>() {
            window.save_home_folded(section, folded);
        }
    }
}

/// The widgets of type `T` under `root`, in order.
fn descendants_of_type<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut found = Vec::new();
    let mut stack: Vec<gtk::Widget> = vec![root.clone().upcast()];
    while let Some(widget) = stack.pop() {
        if let Ok(typed) = widget.clone().downcast::<T>() {
            found.push(typed);
        }
        let mut below: Vec<gtk::Widget> = children(&widget).collect();
        below.reverse();
        stack.extend(below);
    }
    found
}

/// Whether `section` of `page` is folded.
fn is_section_folded(page: &gtk::Box, section: Section) -> bool {
    page.has_css_class(section.folded_class())
}

/// Gives `fold` its keys: Left folds, Right unfolds, as a group header
/// does in Explorer; Up and Down move between the sections.
fn connect_fold_keys(fold: &gtk::Button, page: &gtk::Box, section: Section) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        page,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, modifiers| {
            if !modifiers.is_empty() {
                return glib::Propagation::Proceed;
            }
            let folded = is_section_folded(&page, section);
            show_focus(&page);
            match key {
                gtk::gdk::Key::Left if !folded => set_folded(&page, section, true, true),
                gtk::gdk::Key::Right if folded => set_folded(&page, section, false, true),
                gtk::gdk::Key::Left | gtk::gdk::Key::Right => {}
                gtk::gdk::Key::Down => focus_below(&page, section),
                gtk::gdk::Key::Up => focus_above(&page, section),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    ));
    fold.add_controller(keys);
}

/// Shows where the keyboard is, as GTK does once a key moves it: these
/// keys are handled here, before the window sees them.
fn show_focus(page: &gtk::Box) {
    if let Some(window) = page.root().and_downcast::<gtk::Window>() {
        window.set_focus_visible(true);
    }
}

/// The item box `section` shows now, if it shows one.
fn shown_items(page: &gtk::Box, section: Section) -> Option<gtk::FlowBox> {
    if is_section_folded(page, section) {
        return None;
    }
    let body = section_body(page, section)?;
    descendants_of_type::<gtk::FlowBox>(&body)
        .into_iter()
        .find(|flow| flow.is_drawable() || flow.parent().is_some_and(|list| list.is_visible()))
}

/// The shown tab of the switcher.
fn shown_tab(page: &gtk::Box) -> Tab {
    if page.has_css_class(FAVORITES_SHOWN_CLASS) {
        Tab::Favorites
    } else {
        Tab::Recent
    }
}

/// The button of `tab`.
fn tab_button(page: &gtk::Box, tab: Tab) -> Option<gtk::ToggleButton> {
    descendants_with_class(page, tab.class())
        .into_iter()
        .next()
        .and_downcast()
}

/// Gives `child` the keyboard and selects it, as the arrows do.
fn focus_item(child: &gtk::FlowBoxChild) {
    child.child_focus(gtk::DirectionType::TabForward);
}

/// The first or last item of `flow`.
fn end_item(flow: &gtk::FlowBox, last: bool) -> Option<gtk::FlowBoxChild> {
    let count = children(flow).count();
    let index = if last { count.checked_sub(1)? } else { 0 };
    flow.child_at_index(i32::try_from(index).ok()?)
}

/// Down from `section`'s heading: its first item, or the next heading.
fn focus_below(page: &gtk::Box, section: Section) {
    if let Some(first) = shown_items(page, section).and_then(|flow| end_item(&flow, false)) {
        focus_item(&first);
    } else if section == Section::QuickAccess {
        focus_switcher(page);
    }
}

/// Up from `section`'s heading: the last item of the section above, or
/// its heading.
fn focus_above(page: &gtk::Box, section: Section) {
    if section == Section::QuickAccess {
        return;
    }
    match shown_items(page, Section::QuickAccess).and_then(|flow| end_item(&flow, true)) {
        Some(last) => focus_item(&last),
        None => {
            if let Some(fold) = fold_button(page, Section::QuickAccess) {
                fold.grab_focus();
            }
        }
    }
}

/// Gives the keyboard to the switcher's shown tab.
fn focus_switcher(page: &gtk::Box) {
    if let Some(button) = tab_button(page, shown_tab(page)) {
        button.grab_focus();
    }
}

/// Shows `tab`'s list.
fn show_tab(page: &gtk::Box, tab: Tab) {
    toggle_class(page, FAVORITES_SHOWN_CLASS, tab == Tab::Favorites);
    for each in Tab::ALL {
        if let Some(list) = descendants_with_class(page, each.list_class()).into_iter().next() {
            list.set_visible(each == tab);
        }
        if let Some(button) = tab_button(page, each) {
            if button.is_active() != (each == tab) {
                button.set_active(each == tab);
            }
        }
    }
    recount(page);
}

/// Left and Right move between the tabs and show the one reached, as a
/// tab list does; Up and Down leave the switcher.
fn connect_tab_keys(button: &gtk::ToggleButton, page: &gtk::Box, tab: Tab) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        page,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, modifiers| {
            if !modifiers.is_empty() {
                return glib::Propagation::Proceed;
            }
            show_focus(&page);
            let other = match tab {
                Tab::Recent => Tab::Favorites,
                Tab::Favorites => Tab::Recent,
            };
            match key {
                gtk::gdk::Key::Left if tab == Tab::Favorites => {
                    show_tab(&page, other);
                    focus_switcher(&page);
                }
                gtk::gdk::Key::Right if tab == Tab::Recent => {
                    show_tab(&page, other);
                    focus_switcher(&page);
                }
                gtk::gdk::Key::Left | gtk::gdk::Key::Right => {}
                gtk::gdk::Key::Down => focus_below(&page, Section::Files),
                gtk::gdk::Key::Up => focus_above(&page, Section::Files),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    ));
    button.add_controller(keys);
}

/// Up from the first row of `flow` reaches its section's heading, and
/// Down from Quick access's last row the switcher, as in Explorer.
fn connect_edge_keys(flow: &gtk::FlowBox, page: &gtk::Box, section: Section) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        flow,
        #[weak]
        page,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, modifiers| {
            let up = match key {
                gtk::gdk::Key::Up => true,
                gtk::gdk::Key::Down => false,
                _ => return glib::Propagation::Proceed,
            };
            if !modifiers.is_empty() {
                return glib::Propagation::Proceed;
            }
            let (Some(focused), Some(end)) = (flow.focus_child(), end_item(&flow, !up)) else {
                return glib::Propagation::Proceed;
            };
            let row_of = |item: &gtk::Widget| item.compute_bounds(&flow).map(|bounds| bounds.y());
            if row_of(&focused) != row_of(end.upcast_ref()) {
                return glib::Propagation::Proceed;
            }
            match (up, section) {
                (true, Section::QuickAccess) => {
                    if let Some(fold) = fold_button(&page, section) {
                        fold.grab_focus();
                    }
                }
                (true, Section::Files) | (false, Section::QuickAccess) => focus_switcher(&page),
                (false, Section::Files) => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    ));
    flow.add_controller(keys);
}

/// The item `child` is, among `items`.
fn item_of<'a>(items: &'a [Item], child: &gtk::FlowBoxChild) -> Option<&'a Item> {
    items.iter().find(|item| item.child == *child)
}

/// The selected item of `flow`, among `items`.
fn selected_item<'a>(flow: &gtk::FlowBox, items: &'a [Item]) -> Option<&'a Item> {
    let child = flow.selected_children().into_iter().next()?;
    item_of(items, &child)
}

/// The Menu key and Shift+F10 open the selected item's menu under it;
/// Alt+Enter opens its Properties.
fn connect_item_keys(flow: &gtk::FlowBox, items: &Rc<Vec<Item>>) {
    let shortcuts = gtk::ShortcutController::new();
    for (key, shift_f10) in [(MENU_KEY, false), (SHIFT_F10_KEY, true)] {
        let menu = gtk::CallbackAction::new(glib::clone!(
            #[strong]
            items,
            move |flow, _| {
                let Some(item) = flow
                    .downcast_ref::<gtk::FlowBox>()
                    .and_then(|flow| selected_item(flow, &items))
                else {
                    return glib::Propagation::Proceed;
                };
                // Shift+F10 opens the file list's menu, the classic one
                // only with the Settings choice "Shift+F10 opens the
                // classic menu", as on This PC.
                let classic = shift_f10
                    && flow
                        .root()
                        .and_downcast::<BrowserWindow>()
                        .is_some_and(|window| window.shift_f10_menu_style() == MenuStyle::Classic);
                let width = f64::from(item.child.width());
                let height = f64::from(item.child.height());
                item.popup_menu(width / 2.0, height / 2.0, classic);
                glib::Propagation::Stop
            }
        ));
        let trigger = gtk::ShortcutTrigger::parse_string(key);
        shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(menu)));
    }
    let properties = gtk::CallbackAction::new(glib::clone!(
        #[strong]
        items,
        move |flow, _| {
            let Some(item) = flow
                .downcast_ref::<gtk::FlowBox>()
                .and_then(|flow| selected_item(flow, &items))
            else {
                return glib::Propagation::Proceed;
            };
            WindowAction::PropertiesOf.activate_from(&item.child, Some(&item.properties.to_variant()));
            glib::Propagation::Stop
        }
    ));
    for key in ["<Alt>Return", "<Alt>KP_Enter"] {
        let trigger = gtk::ShortcutTrigger::parse_string(key);
        shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(properties.clone())));
    }
    flow.add_controller(shortcuts);
}

/// Pixels from one Quick access tile to the next across, as Windows 11's
/// Home places them: the tile's room and its padding.
const TILE_PITCH: i32 = TILE_CONTENT.0;
/// The tile box's own padding across (`flowbox.drive-tiles`).
const TILE_BOX_PADDING: i32 = 8;

/// Keeps Quick access's tiles a tile's width apart, as Windows 11 does,
/// rather than spread over the whole width: the box holds as many tiles
/// per row as fit and stays at the start of the row.
fn keep_tiles_apart(flow: &gtk::FlowBox) {
    flow.set_halign(gtk::Align::Start);
    let weak = flow.downgrade();
    follow_visible_width(
        flow,
        Rc::new(move |room| {
            let Some(flow) = weak.upgrade() else {
                return;
            };
            let fit = (room - TILES_INDENT - PAGE_PADDING - TILE_BOX_PADDING) / TILE_PITCH;
            let per_row = u32::try_from(fit.max(1)).unwrap_or(1).min(MOST_TILES_PER_ROW);
            if flow.max_children_per_line() != per_row {
                flow.set_max_children_per_line(per_row);
            }
        }),
    );
}

/// A box of `items`: tiles for Quick access, one row each for the lists.
/// Double-click and Enter open an item; a right-click (which selects it),
/// the Menu key or Shift+F10 opens its menu, and Alt+Enter its
/// Properties.
fn item_box(page: &gtk::Box, items: Vec<Item>, section: Section, rows: bool) -> gtk::FlowBox {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(false)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(if rows { 1 } else { MOST_TILES_PER_ROW })
        .column_spacing(0)
        .row_spacing(0)
        .valign(gtk::Align::Start)
        .css_classes(if rows {
            ["drive-tiles", "home-rows"]
        } else {
            ["drive-tiles", "home-tiles"]
        })
        .build();
    for item in &items {
        flow.append(&item.child);
    }
    if !rows {
        keep_tiles_apart(&flow);
    }
    let items = Rc::new(items);
    flow.connect_child_activated(glib::clone!(
        #[strong]
        items,
        move |_, child| {
            if let Some(item) = item_of(&items, child) {
                item.open();
            }
        }
    ));
    for item in items.iter() {
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gtk::gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(glib::clone!(
            #[weak]
            flow,
            #[weak]
            items,
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(child) = gesture.widget().and_downcast::<gtk::FlowBoxChild>() else {
                    return;
                };
                flow.select_child(&child);
                focus_item(&child);
                if let Some(item) = item_of(&items, &child) {
                    let shift = gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::SHIFT_MASK);
                    item.popup_menu(x, y, shift);
                }
            }
        ));
        item.child.add_controller(right_click);
    }
    connect_item_keys(&flow, &items);
    connect_edge_keys(&flow, page, section);
    // One selection on the page at a time, as in one item view: selecting
    // in one section clears the other.
    flow.connect_selected_children_changed(glib::clone!(
        #[weak]
        page,
        move |flow| {
            if flow.selected_children().is_empty() {
                return;
            }
            for other in descendants_of_type::<gtk::FlowBox>(&page) {
                if other != *flow {
                    other.unselect_all();
                }
            }
        }
    ));
    flow
}

/// What an empty list or section says.
fn empty_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["home-empty", "quiet"])
        .build();
    label.update_property(&[gtk::accessible::Property::Label(text)]);
    label
}

/// Fills `list` with `items`, or with what it says while empty.
fn fill_list(page: &gtk::Box, list: &gtk::Box, items: Vec<Item>, tab: Tab, columns: &Columns) {
    remove_children(list);
    if items.is_empty() {
        list.append(&empty_label(tab.empty_text()));
    } else {
        list.append(&columns.header());
        list.append(&item_box(page, items, Section::Files, true));
    }
    recount(page);
}

/// The heading of Quick access: a chevron, the title and a line to the
/// right edge, as This PC's group headers.
fn quick_access_heading(page: &gtk::Box, folded: bool) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&icons::image(chevron(folded), CHEVRON));
    row.append(
        &gtk::Label::builder()
            .label(ox_core::i18n::gettext_static("Quick access"))
            .xalign(0.0)
            .css_classes(["group-title"])
            .build(),
    );
    let button = gtk::Button::builder()
        .child(&row)
        .css_classes(["group-header", Section::QuickAccess.fold_class()])
        .build();
    connect_fold(&button, page, Section::QuickAccess, folded);
    button
}

/// Clicking `button` folds or unfolds `section`; it carries its state for
/// screen readers.
fn connect_fold(button: &gtk::Button, page: &gtk::Box, section: Section, folded: bool) {
    button.update_state(&[gtk::accessible::State::Expanded(Some(!folded))]);
    button.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| {
            let folded = is_section_folded(&page, section);
            set_folded(&page, section, !folded, true);
        }
    ));
    connect_fold_keys(button, page, section);
}

/// The switcher: a chevron that folds the lists, then a button for each
/// list, the shown one checked, and a line to the right edge.
fn switcher(page: &gtk::Box, folded: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("home-switcher");
    let fold = gtk::Button::builder()
        .child(&icons::image(chevron(folded), CHEVRON))
        .css_classes(["group-header", Section::Files.fold_class()])
        .tooltip_text(ox_core::i18n::gettext_static("Recent and Favorites"))
        .build();
    fold.update_property(&[gtk::accessible::Property::Label(ox_core::i18n::gettext_static(
        "Recent and Favorites",
    ))]);
    connect_fold(&fold, page, Section::Files, folded);
    row.append(&fold);
    let mut first: Option<gtk::ToggleButton> = None;
    for tab in Tab::ALL {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&icons::image(tab.icon(), 16));
        content.append(&gtk::Label::new(Some(tab.title())));
        let button = gtk::ToggleButton::builder()
            .child(&content)
            .css_classes(["home-tab", tab.class()])
            .build();
        button.update_property(&[gtk::accessible::Property::Label(tab.title())]);
        if let Some(first) = &first {
            button.set_group(Some(first));
        }
        button.connect_toggled(glib::clone!(
            #[weak]
            page,
            move |button| {
                if button.is_active() && shown_tab(&page) != tab {
                    show_tab(&page, tab);
                }
            }
        ));
        connect_tab_keys(&button, page, tab);
        row.append(&button);
        first.get_or_insert(button);
    }
    row
}

/// Where the keyboard was on Home before a redraw, to give it back.
enum KeptFocus {
    Fold(Section),
    Tab(Tab),
    Item(String),
}

impl KeptFocus {
    fn read(page: &gtk::Box) -> Option<Self> {
        let focused = page.root()?.focus()?;
        if !focused.is_ancestor(page) {
            return None;
        }
        if let Some(item) = focused.ancestor(gtk::FlowBoxChild::static_type()) {
            return Some(KeptFocus::Item(item.widget_name().to_string()));
        }
        let widgets: Vec<gtk::Widget> = std::iter::successors(Some(focused), WidgetExt::parent).collect();
        if let Some(section) = Section::ALL.into_iter().find(|section| {
            widgets
                .iter()
                .any(|widget| widget.has_css_class(section.fold_class()))
        }) {
            return Some(KeptFocus::Fold(section));
        }
        Tab::ALL
            .into_iter()
            .find(|tab| widgets.iter().any(|widget| widget.has_css_class(tab.class())))
            .map(KeptFocus::Tab)
    }

    fn restore(&self, page: &gtk::Box) {
        match self {
            KeptFocus::Fold(section) => {
                if let Some(fold) = fold_button(page, *section) {
                    fold.grab_focus();
                }
            }
            KeptFocus::Tab(tab) => {
                if let Some(button) = tab_button(page, *tab) {
                    button.grab_focus();
                }
            }
            KeptFocus::Item(name) => {
                let child = descendants_of_type::<gtk::FlowBoxChild>(page)
                    .into_iter()
                    .find(|child| child.widget_name() == name.as_str() && child.is_visible());
                if let Some(child) = child {
                    focus_item(&child);
                } else {
                    focus_switcher(page);
                }
            }
        }
    }
}

/// Draws Home into `page`, replacing what it showed: Quick access from
/// `places`, Favorites from `preferences`, and Recent once `GVfs` has
/// listed it.
pub(super) fn render(
    page: &gtk::Box,
    places: &Places,
    preferences: &Preferences,
    locations: &LocationContext,
) {
    let kept = KeptFocus::read(page);
    remove_children(page);
    page.add_css_class(HOME_CLASS);
    let drawn = page.has_css_class(DRAWN_CLASS);
    let folded = |section: Section| {
        if drawn {
            is_section_folded(page, section)
        } else {
            preferences.home_folded.iter().any(|key| key == section.key())
        }
    };
    let quick_folded = folded(Section::QuickAccess);
    let files_folded = folded(Section::Files);
    let window = page.root().and_downcast::<BrowserWindow>();

    let heading = quick_access_heading(page, quick_folded);
    heading.set_margin_start(SECTION_INDENT);
    page.append(&heading);
    let quick_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    quick_body.set_margin_start(TILES_INDENT);
    quick_body.set_margin_top(TILES_BELOW_HEADING);
    quick_body.add_css_class(Section::QuickAccess.body_class());
    let tiles: Vec<Item> = places
        .quick_access
        .iter()
        .map(|place| folder_tile(place, window.as_ref(), locations))
        .collect();
    if tiles.is_empty() {
        quick_body.append(&empty_label(ox_core::i18n::gettext_static(
            "Folders you pin to Quick access show here.",
        )));
    } else {
        quick_body.append(&item_box(page, tiles, Section::QuickAccess, false));
    }
    page.append(&quick_body);

    let pills = switcher(page, files_folded);
    pills.set_margin_start(SECTION_INDENT);
    pills.set_margin_top(PILLS_BELOW_TILES);
    page.append(&pills);
    let files_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    files_body.set_margin_start(LIST_INDENT);
    files_body.add_css_class(Section::Files.body_class());
    let recent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    recent.add_css_class(Tab::Recent.list_class());
    let favorites = gtk::Box::new(gtk::Orientation::Vertical, 0);
    favorites.add_css_class(Tab::Favorites.list_class());
    files_body.append(&recent);
    files_body.append(&favorites);
    page.append(&files_body);
    list_favorites(page, &favorites, preferences.favorites.clone());
    list_recent(page, &recent, preferences.favorites.clone());
    watch_recent_files(page);

    for section in Section::ALL {
        set_folded(page, section, folded(section), false);
    }
    show_tab(page, shown_tab(page));
    page.add_css_class(DRAWN_CLASS);
    if let Some(kept) = kept {
        kept.restore(page);
    }
    recount(page);
}

/// How many items Home shows, as Windows 11's status bar counts them on
/// Home: the Quick access tiles and the rows of the list shown.
pub(super) fn item_count(page: &gtk::Box) -> Option<u32> {
    if !page.has_css_class(HOME_CLASS) {
        return None;
    }
    let tiles = descendants_with_class(page, "home-tiles")
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .count();
    let rows = descendants_with_class(page, shown_tab(page).list_class())
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .count();
    u32::try_from(tiles + rows).ok()
}

/// Has the window count Home's items again in its status bar.
fn recount(page: &gtk::Box) {
    if let Some(window) = page.root().and_downcast::<BrowserWindow>() {
        window.update_status();
    }
}

/// Stops showing Home in `page`, which shows another landing page.
pub(super) fn leave(page: &gtk::Box) {
    page.remove_css_class(HOME_CLASS);
}

/// Whether Recent keeps a file last used at `used` (seconds since the
/// epoch), it being `now`, under the desktop's privacy `policy`: none
/// while it remembers no recent files, else those used within its
/// maximum age, as the navigation pane's Recent locations keeps them.
fn kept_by_policy(used: Option<u64>, now: u64, policy: crate::app_context::RecentPolicy) -> bool {
    if !policy.remember {
        return false;
    }
    match policy.max_age_days {
        None => true,
        Some(days) => used.is_some_and(|used| now.saturating_sub(used) <= days * 86_400),
    }
}

/// The recently used files `GVfs` lists, newest first, each with the file
/// it stands for; folders and rows without a file are left out.
async fn recent_files() -> Vec<(Entry, String, Option<u64>)> {
    // The desktop's privacy settings, as the navigation pane's Recent
    // locations follows them: no file while it remembers none, and only
    // those used within its maximum age.
    let policy = crate::app_context::desktop_recent_policy();
    if !policy.remember {
        return Vec::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let folder = gio::File::for_uri(RECENT_URI);
    let attributes = format!("{},{RECENT_ATTRIBUTES}", entry::ATTRIBUTES);
    let Ok(enumerator) = folder
        .enumerate_children_future(
            &attributes,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
    else {
        return Vec::new();
    };
    let mut found: Vec<(i64, Entry, String)> = Vec::new();
    loop {
        let Ok(infos) = enumerator.next_files_future(64, glib::Priority::DEFAULT).await else {
            break;
        };
        if infos.is_empty() {
            break;
        }
        for info in infos {
            let file = enumerator.child(&info);
            let entry = entry::entry_from_info(&file, &info);
            if entry.is_dir {
                continue;
            }
            let Some(target) = entry.meta.recent_target.clone() else {
                continue;
            };
            let used = info.attribute_int64("recent::modified");
            if !kept_by_policy(u64::try_from(used).ok().filter(|used| *used > 0), now, policy) {
                continue;
            }
            found.push((used, entry, target));
        }
    }
    found.sort_by_key(|(used, _, _)| std::cmp::Reverse(*used));
    found.truncate(MOST_RECENT);
    found
        .into_iter()
        .map(|(used, entry, target)| (entry, target, u64::try_from(used).ok().filter(|used| *used > 0)))
        .collect()
}

/// The class the page carries once it follows the desktop's recently
/// used list.
const WATCHED_CLASS: &str = "home-recent-watched";
/// How long Recent waits after the desktop's list changes before it asks
/// `GVfs` again, which reads the same file and may not have yet.
const RECENT_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Lists Recent again whenever the desktop's recently used list changes,
/// as a file opened elsewhere joins it, while `page` shows Home.
fn watch_recent_files(page: &gtk::Box) {
    if page.has_css_class(WATCHED_CLASS) {
        return;
    }
    page.add_css_class(WATCHED_CLASS);
    let weak = page.downgrade();
    let manager = gtk::RecentManager::default();
    let handler = std::rc::Rc::new(std::cell::Cell::new(None::<glib::SignalHandlerId>));
    let id = manager.connect_changed(glib::clone!(
        #[strong]
        handler,
        move |manager| {
            let Some(page) = weak.upgrade() else {
                if let Some(id) = handler.take() {
                    manager.disconnect(id);
                }
                return;
            };
            let page = page.downgrade();
            glib::timeout_add_local_once(RECENT_SETTLE, move || {
                let Some(page) = page.upgrade() else {
                    return;
                };
                // A pane that went on to a folder keeps its hidden Home
                // until it shows a page again, which draws Home anew.
                if !page.has_css_class(HOME_CLASS) || !page.is_mapped() {
                    return;
                }
                let Some(list) = descendants_with_class(&page, Tab::Recent.list_class())
                    .into_iter()
                    .next()
                    .and_downcast::<gtk::Box>()
                else {
                    return;
                };
                let favorites = page
                    .root()
                    .and_downcast::<BrowserWindow>()
                    .map(|window| window.context().settings_data().preferences.favorites)
                    .unwrap_or_default();
                list_recent(&page, &list, favorites);
            });
        }
    ));
    handler.set(Some(id));
}

/// Whether the window `page` is in shows Compact view.
fn is_compact(page: &gtk::Box) -> bool {
    page.root()
        .is_some_and(|root| root.has_css_class("compact-density"))
}

/// When each of `favorites` was last opened (`time::access`), read off
/// the main thread; `None` for a file that cannot be read now, or one on
/// a share or device, which drawing Home never reads (HOME-014).
async fn favorite_times(favorites: &[String]) -> Vec<Option<u64>> {
    let mut times = Vec::with_capacity(favorites.len());
    for uri in favorites {
        let file = gio::File::for_uri(uri);
        if !file.is_native() {
            times.push(None);
            continue;
        }
        let info = file
            .query_info_future(
                "time::access",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await;
        let accessed = info.ok().and_then(|info| {
            info.has_attribute("time::access")
                .then(|| info.attribute_uint64("time::access"))
        });
        times.push(accessed);
    }
    times
}

/// Lists `favorites` into `list` once their dates are read, if `page`
/// still shows this drawing of Home then.
fn list_favorites(page: &gtk::Box, list: &gtk::Box, favorites: Vec<String>) {
    let page = page.downgrade();
    let list = list.downgrade();
    glib::spawn_future_local(async move {
        let times = favorite_times(&favorites).await;
        let (Some(page), Some(list)) = (page.upgrade(), list.upgrade()) else {
            return;
        };
        if list.parent().is_none() || !page.has_css_class(HOME_CLASS) {
            return;
        }
        let kept = KeptFocus::read(&page);
        let columns = Columns::new(glib::home_dir(), is_compact(&page));
        let rows = favorites
            .iter()
            .zip(times)
            .map(|(uri, accessed)| file_row(&favorite_name(uri), uri, accessed, &favorites, &columns))
            .collect();
        fill_list(&page, &list, rows, Tab::Favorites, &columns);
        list.set_visible(shown_tab(&page) == Tab::Favorites);
        if let Some(kept) = kept {
            kept.restore(&page);
        }
    });
}

/// Lists the recent files into `list` once `GVfs` has read them, if `page`
/// still shows this drawing of Home then.
fn list_recent(page: &gtk::Box, list: &gtk::Box, favorites: Vec<String>) {
    let page = page.downgrade();
    let list = list.downgrade();
    glib::spawn_future_local(async move {
        let files = recent_files().await;
        let (Some(page), Some(list)) = (page.upgrade(), list.upgrade()) else {
            return;
        };
        if list.parent().is_none() || !page.has_css_class(HOME_CLASS) {
            return;
        }
        let kept = KeptFocus::read(&page);
        let columns = Columns::new(glib::home_dir(), is_compact(&page));
        let rows = files
            .iter()
            .map(|(entry, target, used)| file_row(&entry.name, target, *used, &favorites, &columns))
            .collect();
        fill_list(&page, &list, rows, Tab::Recent, &columns);
        list.set_visible(shown_tab(&page) == Tab::Recent);
        if let Some(kept) = kept {
            kept.restore(&page);
        }
    });
}

impl BrowserWindow {
    pub(super) fn install_home_page_actions(&self) {
        use super::actions::text_action;
        self.add_action_entries([
            text_action(WindowAction::AddToFavorites, |window, uri| {
                window.set_favorite(uri, true);
            }),
            text_action(WindowAction::RemoveFromFavorites, |window, uri| {
                window.set_favorite(uri, false);
            }),
            text_action(WindowAction::OpenItemAt, BrowserWindow::open_item_at),
            text_action(WindowAction::OpenFileLocationOf, |window, uri| {
                window.open_item_location(uri.to_owned(), LocationTarget::ThisTab);
            }),
        ]);
    }

    /// Adds the file at `uri` to Favorites, or removes it; every window
    /// draws Home again once it is saved.
    fn set_favorite(&self, uri: &str, favorite: bool) {
        let uri = uri.to_owned();
        let change: Change = Box::new(move |settings| settings.set_favorite(&uri, favorite).map(|_| ()));
        self.context().change_settings(
            change,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result: Result<(), SettingsError>| {
                    if let Err(error) = result {
                        window.show_message(&ox_core::i18n::format_message(
                            "Could not save Favorites: {error}",
                            &[("error", &error.to_string())],
                        ));
                    }
                }
            ),
        );
    }

    /// Whether the file at `uri` is among the Favorites, which hold it
    /// as [`favorite_location`] writes it (a remote one without its user
    /// name).
    pub(super) fn is_favorite(&self, uri: &str) -> bool {
        let Some(location) = favorite_location(uri) else {
            return false;
        };
        let favorites = self.context().settings_data().preferences.favorites;
        favorites
            .iter()
            .any(|favorite| same_location(favorite, &location))
    }

    /// Saves that `section` of Home is folded or not, without drawing
    /// anything again: this window shows it already.
    fn save_home_folded(&self, section: Section, folded: bool) {
        // Changes only this section's key of the file as it is when the
        // change runs, so two folds in a row both stay.
        let key = section.key();
        let change: Change = Box::new(move |settings| settings.set_home_folded(key, folded).map(|_| ()));
        self.context().remember_change(change);
    }

    /// Opens the item at `uri` as a double-click does: a folder in the
    /// tab, a file in its default application. A file that is gone is
    /// said so in a dialog, as the file list says it.
    fn open_item_at(&self, uri: &str) {
        // Only the location is known here: the double-click's steps read
        // the item, mounting its share first, and stand it for that file,
        // as a Recent files row stands for the file it names.
        let file = gio::File::for_uri(uri);
        let mut entry = entry::entry_from_info(&file, &gio::FileInfo::new());
        entry.meta.recent_target = Some(uri.to_owned());
        self.activate_listed_entry(entry);
    }
}

/// Gives the keyboard to Home's first heading, as a page opens with
/// nothing selected; whether `page` shows Home.
pub(super) fn focus_first(page: &gtk::Box) -> bool {
    if !page.has_css_class(HOME_CLASS) {
        return false;
    }
    if let Some(fold) = fold_button(page, Section::QuickAccess) {
        fold.grab_focus();
    }
    true
}

/// Puts the keyboard on what `page`, showing Home, calls `name`: a tab of
/// the switcher, which it shows, Quick access's heading, or a tile or row,
/// which it selects (the snapshot hook's `select=`). Whether it found one.
pub(super) fn select_named(page: &gtk::Box, name: &str) -> bool {
    if !page.has_css_class(HOME_CLASS) || !page.is_drawable() {
        return false;
    }
    if let Some(tab) = Tab::ALL.into_iter().find(|tab| tab.title() == name) {
        show_tab(page, tab);
        focus_switcher(page);
        return true;
    }
    if name == ox_core::i18n::gettext_static("Quick access") {
        return fold_button(page, Section::QuickAccess).is_some_and(|fold| fold.grab_focus());
    }
    let shown = |child: &gtk::FlowBoxChild| {
        child
            .ancestor(gtk::Box::static_type())
            .is_some_and(|list| list.is_drawable())
            && descendants_of_type::<gtk::Label>(child)
                .first()
                .is_some_and(|label| label.text() == name)
    };
    let Some(child) = descendants_of_type::<gtk::FlowBoxChild>(page)
        .into_iter()
        .find(shown)
    else {
        return false;
    };
    if let Some(flow) = child.parent().and_downcast::<gtk::FlowBox>() {
        flow.select_child(&child);
    }
    focus_item(&child);
    true
}

/// The section titles and switcher tabs `page` shows, for tests.
#[cfg(test)]
pub(crate) fn section_titles(page: &gtk::Box) -> Vec<String> {
    let mut titles: Vec<String> = descendants_with_class(page, Section::QuickAccess.fold_class())
        .iter()
        .flat_map(descendants_of_type::<gtk::Label>)
        .map(|label| label.text().to_string())
        .collect();
    titles.extend(Tab::ALL.into_iter().filter_map(|tab| {
        let button = tab_button(page, tab)?;
        descendants_of_type::<gtk::Label>(&button)
            .first()
            .map(|label| label.text().to_string())
    }));
    titles
}

/// The switcher's shown tab, for tests.
#[cfg(test)]
pub(crate) fn shown_tab_title(page: &gtk::Box) -> Option<String> {
    page.has_css_class(HOME_CLASS)
        .then(|| shown_tab(page).title().to_owned())
}

/// Shows the tab titled `title`, by a click on it, for tests.
#[cfg(test)]
pub(crate) fn choose_tab(page: &gtk::Box, title: &str) {
    let tab = Tab::ALL
        .into_iter()
        .find(|tab| tab.title() == title)
        .expect("a tab of that title");
    tab_button(page, tab).expect("the tab's button").set_active(true);
}

/// The names of the Quick access tiles, for tests.
#[cfg(test)]
pub(crate) fn tile_names(page: &gtk::Box) -> Vec<String> {
    descendants_with_class(page, "home-tiles")
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .filter_map(|child| item_name(&child))
        .collect()
}

/// The names of the rows of the list shown, for tests.
#[cfg(test)]
pub(crate) fn list_names(page: &gtk::Box) -> Vec<String> {
    let tab = shown_tab(page);
    descendants_with_class(page, tab.list_class())
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .filter_map(|child| item_name(&child))
        .collect()
}

/// What the list shown says while it is empty, for tests.
#[cfg(test)]
pub(crate) fn empty_text(page: &gtk::Box) -> Option<String> {
    let tab = shown_tab(page);
    descendants_with_class(page, tab.list_class())
        .iter()
        .flat_map(|list| descendants_with_class(list, "home-empty"))
        .find_map(|label| label.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
}

/// The name an item shows, for tests.
#[cfg(test)]
fn item_name(child: &gtk::FlowBoxChild) -> Option<String> {
    descendants_of_type::<gtk::Label>(child)
        .first()
        .map(|label| label.text().to_string())
}

/// The item named `name`, tile or row, for tests.
#[cfg(test)]
fn item_named(page: &gtk::Box, name: &str) -> gtk::FlowBoxChild {
    descendants_of_type::<gtk::FlowBoxChild>(page)
        .into_iter()
        .filter(|child| {
            child.is_visible()
                && child
                    .parent()
                    .and_then(|flow| flow.parent())
                    .is_some_and(|list| list.is_visible())
        })
        .find(|child| item_name(child).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("Home shows an item named {name}"))
}

/// The menu of the item named `name`, opened by `accelerator`, for
/// tests.
#[cfg(test)]
pub(crate) fn item_menu(page: &gtk::Box, name: &str, accelerator: &str) -> MenuPopover {
    let child = item_named(page, name);
    let flow = child
        .parent()
        .and_downcast::<gtk::FlowBox>()
        .expect("an item's box");
    flow.select_child(&child);
    let pane = child
        .ancestor(super::folder_pane::FolderPane::static_type())
        .expect("the folder pane");
    let menus = |pane: &gtk::Widget| -> Vec<MenuPopover> {
        children(pane)
            .filter_map(|widget| widget.downcast::<MenuPopover>().ok())
            .collect()
    };
    let before = menus(&pane).len();
    let (key, modifiers) = gtk::accelerator_parse(accelerator).expect("a valid accelerator");
    let ran = flow
        .observe_controllers()
        .into_iter()
        .filter_map(|controller| controller.ok().and_downcast::<gtk::ShortcutController>())
        .flat_map(|shortcuts| {
            (0..shortcuts.n_items())
                .filter_map(move |index| shortcuts.item(index).and_downcast::<gtk::Shortcut>())
        })
        .find(|shortcut| {
            shortcut.trigger().is_some_and(|trigger| {
                trigger
                    .downcast_ref::<gtk::KeyvalTrigger>()
                    .is_some_and(|keys| keys.keyval() == key && keys.modifiers() == modifiers)
            })
        })
        .and_then(|shortcut| shortcut.action())
        .is_some_and(|action| action.activate(gtk::ShortcutActionFlags::empty(), &flow, None));
    assert!(ran, "{accelerator} opens the item's menu");
    let opened = menus(&pane);
    assert!(opened.len() > before, "a menu opened");
    opened.last().expect("the menu").clone()
}

/// The labels of the menu of the item named `name`, opened by the Menu
/// key, for tests.
#[cfg(test)]
pub(crate) fn menu_labels(page: &gtk::Box, name: &str) -> Vec<String> {
    let menu = item_menu(page, name, MENU_KEY);
    let labels = menu.row_labels();
    menu.popdown();
    labels
}

/// Opens the item named `name` as a double-click does, for tests.
#[cfg(test)]
pub(crate) fn open_item(page: &gtk::Box, name: &str) {
    let child = item_named(page, name);
    child.activate();
}

/// The fold buttons, Quick access first, for tests.
#[cfg(test)]
pub(crate) fn fold_buttons(page: &gtk::Box) -> Vec<gtk::Button> {
    Section::ALL
        .into_iter()
        .filter_map(|section| fold_button(page, section))
        .collect()
}

/// Whether the section at `index` (0 Quick access, 1 the lists) is
/// folded and hidden, for tests.
#[cfg(test)]
pub(crate) fn is_folded(page: &gtk::Box, index: usize) -> bool {
    let section = Section::ALL[index];
    let hidden = section_body(page, section).is_some_and(|body| !body.is_visible());
    is_section_folded(page, section) && hidden
}

/// The tile named `name` in Quick access, for tests.
#[cfg(test)]
fn tile_named(page: &gtk::Box, name: &str) -> gtk::FlowBoxChild {
    descendants_with_class(page, "home-tiles")
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .find(|child| item_name(child).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("Quick access has a tile named {name}"))
}

/// The lines of text the tile named `name` shows, for tests.
#[cfg(test)]
pub(crate) fn tile_lines(page: &gtk::Box, name: &str) -> Vec<String> {
    descendants_of_type::<gtk::Label>(&tile_named(page, name))
        .iter()
        .map(|label| label.text().to_string())
        .collect()
}

/// Whether the tile named `name` shows the pin glyph, for tests.
#[cfg(test)]
pub(crate) fn tile_has_pin(page: &gtk::Box, name: &str) -> bool {
    !descendants_with_class(&tile_named(page, name), "home-pin").is_empty()
}

/// The pixel size of the folder of the tile named `name`, for tests.
#[cfg(test)]
pub(crate) fn tile_icon_size(page: &gtk::Box, name: &str) -> i32 {
    let tile = tile_named(page, name);
    let overlay = descendants_of_type::<gtk::Overlay>(&tile)
        .into_iter()
        .next()
        .expect("the tile's folder");
    overlay
        .child()
        .expect("the folder's art")
        .measure(gtk::Orientation::Horizontal, -1)
        .1
}

/// Whether the tab titled `title` shows a glyph, for tests.
#[cfg(test)]
pub(crate) fn tab_has_icon(page: &gtk::Box, title: &str) -> bool {
    Tab::ALL
        .into_iter()
        .find(|tab| tab.title() == title)
        .and_then(|tab| tab_button(page, tab))
        .is_some_and(|button| !descendants_of_type::<gtk::Image>(&button).is_empty())
}

/// The column headers of the list shown, for tests.
#[cfg(test)]
pub(crate) fn list_headers(page: &gtk::Box) -> Vec<String> {
    let tab = shown_tab(page);
    descendants_with_class(page, tab.list_class())
        .iter()
        .flat_map(|list| descendants_with_class(list, "home-column"))
        .filter_map(|label| label.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

/// The texts of the row named `name` in the list shown, for tests.
#[cfg(test)]
pub(crate) fn row_cells(page: &gtk::Box, name: &str) -> Vec<String> {
    let tab = shown_tab(page);
    let row = descendants_with_class(page, tab.list_class())
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .find(|child| item_name(child).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the list has a row named {name}"));
    descendants_of_type::<gtk::Label>(&row)
        .iter()
        .map(|label| label.text().to_string())
        .collect()
}

/// The name, date and location cells of the row named `name` in the
/// list shown, for tests.
#[cfg(test)]
pub(crate) fn row_cell_widgets(page: &gtk::Box, name: &str) -> Vec<gtk::Widget> {
    let tab = shown_tab(page);
    let row = descendants_with_class(page, tab.list_class())
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .find(|child| item_name(child).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the list has a row named {name}"));
    descendants_of_type::<gtk::Label>(&row)
        .into_iter()
        .map(gtk::Label::upcast)
        .collect()
}

/// The icon of the row named `name` in the list shown, for tests.
#[cfg(test)]
pub(crate) fn row_icon(page: &gtk::Box, name: &str) -> gtk::Widget {
    let tab = shown_tab(page);
    let row = descendants_with_class(page, tab.list_class())
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .find(|child| item_name(child).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the list has a row named {name}"));
    descendants_of_type::<ArtImage>(&row)
        .into_iter()
        .next()
        .expect("a row's icon")
        .upcast()
}

/// The folder of the tile named `name`, for tests.
#[cfg(test)]
pub(crate) fn tile_folder(page: &gtk::Box, name: &str) -> gtk::Widget {
    descendants_of_type::<gtk::Overlay>(&tile_named(page, name))
        .into_iter()
        .next()
        .expect("the tile's folder")
        .upcast()
}

/// How many tiles a row of Quick access holds now, for tests.
#[cfg(test)]
pub(crate) fn tiles_per_row(page: &gtk::Box) -> u32 {
    descendants_with_class(page, "home-tiles")
        .into_iter()
        .next()
        .and_downcast::<gtk::FlowBox>()
        .map_or(0, |flow| flow.max_children_per_line())
}

/// The widgets the spacing test measures, for tests.
#[cfg(test)]
pub(crate) struct Metrics {
    pub(crate) rows: Vec<gtk::Widget>,
    pub(crate) header: gtk::Widget,
    pub(crate) pills: gtk::Widget,
    pub(crate) columns: Vec<gtk::Widget>,
    pub(crate) tiles: Vec<gtk::Widget>,
    pub(crate) first_tile_name: gtk::Widget,
    pub(crate) heading: gtk::Widget,
    pub(crate) chevron: gtk::Widget,
}

/// The widgets of `page` the spacing test measures, for tests.
#[cfg(test)]
pub(crate) fn metrics(page: &gtk::Box) -> Metrics {
    let list = descendants_with_class(page, shown_tab(page).list_class())
        .into_iter()
        .next()
        .expect("the list shown");
    let header = descendants_with_class(&list, "home-list-header")
        .into_iter()
        .next()
        .expect("the column headers");
    let tiles: Vec<gtk::Widget> = descendants_with_class(page, "home-tiles")
        .iter()
        .flat_map(descendants_of_type::<gtk::FlowBoxChild>)
        .map(gtk::FlowBoxChild::upcast)
        .collect();
    let first_tile_name = descendants_with_class(&tiles[0], "tile-name")
        .into_iter()
        .next()
        .expect("a tile's name");
    let heading = fold_button(page, Section::QuickAccess).expect("Quick access's heading");
    let chevron = descendants_of_type::<gtk::Image>(&heading)
        .into_iter()
        .next()
        .expect("the heading's chevron");
    Metrics {
        rows: descendants_of_type::<gtk::FlowBoxChild>(&list)
            .into_iter()
            .map(gtk::FlowBoxChild::upcast)
            .collect(),
        columns: descendants_with_class(&header, "home-column"),
        header,
        pills: tab_button(page, Tab::Recent).expect("the Recent pill").upcast(),
        tiles,
        first_tile_name,
        heading: heading.upcast(),
        chevron: chevron.upcast(),
    }
}

#[cfg(test)]
mod layout_tests {
    use super::file_location;

    /// A file's location is its folder, from the home folder down, with
    /// "/" between folders; a file right in the home folder is in "Home";
    /// elsewhere the folder's whole path.
    #[test]
    fn the_file_location_is_the_folder_from_home_down() {
        let home = std::path::Path::new("/home/demo");
        assert_eq!(
            file_location("file:///home/demo/Downloads/Setup%20guide.pdf", home),
            "Downloads"
        );
        assert_eq!(
            file_location("file:///home/demo/Downloads/sub/a.txt", home),
            "Downloads/sub"
        );
        assert_eq!(file_location("file:///home/demo/a.txt", home), "Home");
        assert_eq!(file_location("file:///srv/shared/a.txt", home), "/srv/shared");
    }

    /// Recent keeps no file while the desktop remembers none, and only
    /// those used within its maximum age.
    #[test]
    fn recent_follows_the_desktops_privacy_policy() {
        use super::kept_by_policy;
        use crate::app_context::RecentPolicy;
        const DAY: u64 = 86_400;
        let now = 100 * DAY;
        let off = RecentPolicy {
            remember: false,
            max_age_days: None,
        };
        let week = RecentPolicy {
            remember: true,
            max_age_days: Some(7),
        };
        let always = RecentPolicy {
            remember: true,
            max_age_days: None,
        };
        assert!(!kept_by_policy(Some(now), now, off));
        assert!(kept_by_policy(Some(now - 7 * DAY), now, week));
        assert!(!kept_by_policy(Some(now - 8 * DAY), now, week));
        assert!(
            !kept_by_policy(None, now, week),
            "an undated file is not known to be recent"
        );
        assert!(kept_by_policy(Some(1), now, always));
    }

    /// The columns keep Windows' widths while they fit, then Name
    /// narrows, then the other two, never below their narrowest.
    #[test]
    fn the_columns_narrow_name_first() {
        use super::column_widths;
        assert_eq!(column_widths(1200), (484, 298, 300));
        assert_eq!(column_widths(800), (202, 298, 300));
        let (name, date, location) = column_widths(400);
        assert_eq!(name, 140);
        assert!(
            date >= 90 && location >= 90 && date + location <= 260,
            "{date} {location}"
        );
        assert_eq!(column_widths(100), (140, 90, 90));
    }
}
