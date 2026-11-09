// SPDX-License-Identifier: AGPL-3.0-only
//! Home laid out as Windows 11 File Explorer's Home (HOME-012 to
//! HOME-016), while "Show Quick access, Recent and Favorites on Home" is
//! on. It is off by default: Home then opens the home folder, as the
//! Python app's did (NAV-038).
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
use ox_core::location::{same_location, RECENT_URI};
use ox_core::places::Place;
use ox_core::settings::{Preferences, PreferencesUpdate, SettingsError};

use crate::folder_view::icon_size::{tiles_cell, CellSize, TILES_ICON};
use crate::icons::{self, Art, ArtImage, Icon, Storage};
use crate::places::Places;
use crate::settings_store::Change;
use crate::text_size::TextSize;

use super::context_menu::{CLASSIC_MENU_KEY, MENU_KEY};
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
/// Pixels of a row's icon in the Recent and Favorites lists.
const ROW_ICON: i32 = 16;
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

/// A Quick access tile: the folder's art left of its name, at the Tiles
/// view's size, as This PC's drive tiles.
fn folder_tile(place: &Place, window: Option<&BrowserWindow>, cell: CellSize) -> Item {
    let storage = if place.is_shared {
        Storage::Network
    } else {
        Storage::Local
    };
    let art = ArtImage::new(Art::for_quick_access(place.known_folder, storage), TILES_ICON);
    let (child, _) = super::landing::tile_frame(&art, &place.label, &place.uri, cell);
    gestures::open_folder_on_middle_click(&child, &place.uri);
    let caching = window.and_then(|window| window.caching_of(&place.uri));
    Item {
        child,
        open: (WindowAction::GoTo, place.uri.clone()),
        menu: pin_menu(&place.uri, caching, place.known_folder.is_none()),
        properties: place.uri.clone(),
    }
}

/// A row of the Recent or Favorites list for the file named `name` at
/// `uri`.
fn file_row(name: &str, uri: &str, favorites: &[String]) -> Item {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&ArtImage::new(Art::for_file_name(name), ROW_ICON));
    row.append(&label(name, "home-row-name"));
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
    for (key, classic) in [(MENU_KEY, false), (CLASSIC_MENU_KEY, true)] {
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
fn fill_list(page: &gtk::Box, list: &gtk::Box, items: Vec<Item>, tab: Tab) {
    remove_children(list);
    if items.is_empty() {
        list.append(&empty_label(tab.empty_text()));
    } else {
        list.append(&item_box(page, items, Section::Files, true));
    }
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
    row.append(
        &gtk::Separator::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
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
        let button = gtk::ToggleButton::builder()
            .label(tab.title())
            .css_classes(["home-tab", tab.class()])
            .build();
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
    row.append(
        &gtk::Separator::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build(),
    );
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
pub(super) fn render(page: &gtk::Box, places: &Places, preferences: &Preferences, text_size: TextSize) {
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

    page.append(&quick_access_heading(page, quick_folded));
    let quick_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    quick_body.add_css_class(Section::QuickAccess.body_class());
    let cell = tiles_cell(text_size);
    let tiles: Vec<Item> = places
        .quick_access
        .iter()
        .map(|place| folder_tile(place, window.as_ref(), cell))
        .collect();
    if tiles.is_empty() {
        quick_body.append(&empty_label(ox_core::i18n::gettext_static(
            "Folders you pin to Quick access show here.",
        )));
    } else {
        quick_body.append(&item_box(page, tiles, Section::QuickAccess, false));
    }
    page.append(&quick_body);

    page.append(&switcher(page, files_folded));
    let files_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    files_body.add_css_class(Section::Files.body_class());
    let recent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    recent.add_css_class(Tab::Recent.list_class());
    let favorites = gtk::Box::new(gtk::Orientation::Vertical, 0);
    favorites.add_css_class(Tab::Favorites.list_class());
    files_body.append(&recent);
    files_body.append(&favorites);
    page.append(&files_body);
    let favorite_rows = preferences
        .favorites
        .iter()
        .map(|uri| file_row(&favorite_name(uri), uri, &preferences.favorites))
        .collect();
    fill_list(page, &favorites, favorite_rows, Tab::Favorites);
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
}

/// Stops showing Home in `page`, which shows another landing page.
pub(super) fn leave(page: &gtk::Box) {
    page.remove_css_class(HOME_CLASS);
}

/// The recently used files `GVfs` lists, newest first, each with the file
/// it stands for; folders and rows without a file are left out.
async fn recent_files() -> Vec<(Entry, String)> {
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
            found.push((used, entry, target));
        }
    }
    found.sort_by_key(|(used, _, _)| std::cmp::Reverse(*used));
    found.truncate(MOST_RECENT);
    found
        .into_iter()
        .map(|(_, entry, target)| (entry, target))
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
                if !page.has_css_class(HOME_CLASS) {
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
        let rows = files
            .iter()
            .map(|(entry, target)| file_row(&entry.name, target, &favorites))
            .collect();
        fill_list(&page, &list, rows, Tab::Recent);
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

    /// Whether the file at `uri` is among the Favorites.
    pub(super) fn is_favorite(&self, uri: &str) -> bool {
        let favorites = self.context().settings_data().preferences.favorites;
        favorites.iter().any(|favorite| same_location(favorite, uri))
    }

    /// Saves that `section` of Home is folded or not, without drawing
    /// anything again: this window shows it already.
    fn save_home_folded(&self, section: Section, folded: bool) {
        let mut keys = self.context().settings_data().preferences.home_folded;
        keys.retain(|key| key != section.key());
        if folded {
            keys.push(section.key().to_owned());
        }
        self.context().remember_preferences(PreferencesUpdate {
            home_folded: Some(keys),
            ..PreferencesUpdate::default()
        });
    }

    /// Opens the item at `uri` as a double-click does: a folder in the
    /// tab, a file in its default application. A file that is gone is
    /// said so in a dialog, as the file list says it.
    fn open_item_at(&self, uri: &str) {
        let uri = uri.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match super::activation::query_entry(&uri).await {
                    Ok(entry) if entry.is_dir => window.navigate_or_report(&entry.uri),
                    Ok(entry) => window.open_file(&entry),
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        ));
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
        tab_button(page, tab)
            .and_then(|button| button.label())
            .map(|text| text.to_string())
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

/// The labels of the menu of the item named `name`, opened by the Menu
/// key, for tests.
#[cfg(test)]
pub(crate) fn menu_labels(page: &gtk::Box, name: &str) -> Vec<String> {
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
    let (key, modifiers) = gtk::accelerator_parse(MENU_KEY).expect("a valid accelerator");
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
    assert!(ran, "the Menu key opens the item's menu");
    let opened = menus(&pane);
    assert!(opened.len() > before, "a menu opened");
    let menu = opened.last().expect("the menu").clone();
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
