// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' context menus: right-click, the Menu key and
//! Shift+F10 (CMD-008 to CMD-015).
//!
//! Ports the rows' `contextmenu` handlers, `entryMenu`, `backgroundMenu`
//! and the `ContextMenu`/Shift+F10 keys of `onKey` in `v2.0.0:desktop/ui/app.js`.
//! A right-click on an unselected item selects only it first and opens
//! that item's menu; on blank space it clears the selection and opens the
//! folder's menu. The Settings choice "Right-click menu" picks the
//! classic or the compact style for a right-click; the keyboard always
//! opens the classic menu (CMD-013), pointing at the first selected item
//! as in Windows 11 and Dolphin (CMD-014). The compact menu's "Show more
//! options" and the folder menu's "New…" open their menus at the same
//! point. What the menus list is in [`entries`].

mod entries;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};
use ox_core::entry::Entry;
use ox_core::integration::{is_disk_image, DiskTool};
use ox_core::location::{is_smb_location, is_smb_server, RECENT_LOCATIONS_URI};
use ox_core::ops::JournalDirection;
use ox_core::settings::ContextMenu as MenuStyleChoice;

use crate::integration::{self, ApplicationChoice, Tool};
use crate::locations::Page;

use super::actions::{plain_action, text_action};
use super::command_bar::{group_by_menu, new_menu, sort_menu, view_menu};
use super::disk_tools::is_installed;
use super::menu_popover::{MenuEntry, MenuPopover, MenuStyle};
use super::widget_tree::children;
use super::window_action::WindowAction;
use super::BrowserWindow;

use entries::{
    background_menu, classic_look, compact_background_menu, item_menu, recycle_bin_background_menu,
    recycle_bin_item_menu, zip_background_menu, zip_item_menu, BackgroundSubmenus, Comparison, ContextMenu,
    ItemFacts, ItemLocation, ItemShape,
};
pub(super) use entries::{SendToDrive, SendToTargets};

/// How far into a row, and from the view's corner without one, a menu
/// opened from the keyboard points.
const KEYBOARD_MENU_INSET: i32 = 40;

/// The most applications the item menu offers beside Open with… (OPEN-013).
const MENU_APPLICATIONS: usize = 3;

/// True for an archive the app extracts itself, by its name or its type
/// (`isZipEntry`): a ZIP, or a TAR plain or compressed (ARC-024).
fn is_zip(name: &str, content_type: Option<&str>) -> bool {
    ox_core::archive::is_supported_archive(name, content_type)
}

/// What `entry` is, as its menu cares.
fn item_shape(entry: &Entry) -> ItemShape {
    if entry.is_dir {
        ItemShape::Folder
    } else if is_zip(&entry.name, entry.content_type.as_deref()) {
        ItemShape::ZipArchive
    } else {
        ItemShape::File
    }
}

/// Where the item at `uri` is, as its menu cares.
fn item_location(uri: &str) -> ItemLocation {
    if is_smb_server(uri) {
        ItemLocation::SmbServer
    } else if is_smb_location(uri) {
        ItemLocation::SmbShare
    } else {
        ItemLocation::Local
    }
}

impl BrowserWindow {
    /// Gives `view` its context menu and the gestures and keys that open it.
    pub(super) fn attach_context_menu(&self, view: &gtk::Widget) {
        let popover = MenuPopover::new(Vec::new());
        // Context menus open at the pointer, without the offset of a
        // button's drop-down.
        popover.set_offset(0, 0);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_parent(view);
        // Escape, a click outside and a choice all give the keyboard back
        // to the file pane; a chosen command runs after this, so a
        // rename field or a dialog still takes it.
        popover.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |_| {
                // The folder may have filled or emptied while the menu was
                // open, hiding `view`: the keyboard goes to the page that
                // shows now.
                if view.is_drawable() {
                    view.grab_focus();
                } else if let Some(side) = window.side_holding(&view) {
                    window.pane_on(side).focus_view();
                }
            }
        ));
        view.connect_destroy(glib::clone!(
            #[weak]
            popover,
            move |_| popover.unparent()
        ));
        view.add_controller(self.context_menu_shortcuts());
        // As in Windows Explorer, the press selects and the release opens
        // the menu, so a right-button drag can start from the press
        // (DND-027): a drag cancels the click before its release.
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |_, _, x, y| window.select_for_context_menu(&view, x, y)
        ));
        right_click.connect_released(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
                let point = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                let style = menu_style_for(window.preferred_menu_style(), gesture.current_event_state());
                window.open_context_menu(&view, &point, style);
            }
        ));
        view.add_controller(right_click);
    }

    /// The style the Settings choice "Right-click menu" asks for.
    fn preferred_menu_style(&self) -> MenuStyle {
        match self.context().settings_data().preferences.context_menu {
            MenuStyleChoice::Win10 => MenuStyle::Classic,
            MenuStyleChoice::Win11 => MenuStyle::Compact,
        }
    }

    /// A right-click on an unselected item selects only it; on blank space
    /// it clears the selection.
    fn select_for_context_menu(&self, view: &gtk::Widget, x: f64, y: f64) {
        let model = self.folder_pane().model();
        match self.folder_pane().owners().position_at(view, x, y) {
            Some(position) if !model.selection().is_selected(position) => model.select_only(position),
            Some(_) => {}
            None => model.select_none(),
        }
    }

    /// Opens the menu of the selection, or of the folder without one, in
    /// `style`, pointing at `point` in `view`.
    fn open_context_menu(&self, view: &gtk::Widget, point: &gdk::Rectangle, style: MenuStyle) {
        let Some(popover) = context_menu_of(view) else {
            return;
        };
        let mut menu = self.menu_for_selection(style);
        let mut extras = Vec::new();
        // Service actions run programs on files: the Recycle Bin's items
        // and those inside a ZIP (ARC-026) are not files they can open.
        let folder = self.command_facts().folder;
        if !folder.is_recycle_bin && !folder.is_zip {
            self.append_service_actions(&mut extras, style == MenuStyle::Classic);
        }
        if self.administrator_target().is_some() {
            extras.push(
                super::menu_popover::MenuItem::new(
                    ox_core::i18n::gettext_static("Open as administrator…"),
                    crate::icons::Icon::ShieldLock,
                    WindowAction::OpenAsAdministrator,
                )
                .into(),
            );
        }
        add_extras(&mut menu.entries, extras);
        if style == MenuStyle::Classic {
            menu.entries = classic_look(menu.entries);
        }
        popover.set_entries(menu.entries);
        popover.set_style_and_strip(style, menu.strip);
        popover.set_pointing_to(Some(point));
        popover.popup();
    }

    /// What the menu lists for the selection now.
    fn menu_for_selection(&self, style: MenuStyle) -> ContextMenu {
        let items = self.folder_pane().model().selected_items();
        let facts = self.command_facts();
        if facts.folder.is_zip {
            let entries = if items.is_empty() {
                zip_background_menu()
            } else {
                zip_item_menu()
            };
            return ContextMenu {
                entries,
                strip: Vec::new(),
            };
        }
        let in_recycle_bin = facts.folder.is_recycle_bin;
        let entries = match (items.first(), in_recycle_bin) {
            (None, true) => recycle_bin_background_menu(),
            (None, false) if style == MenuStyle::Compact => {
                self.refresh_template_menu();
                compact_background_menu(
                    &self.journal_label(JournalDirection::Undo),
                    BackgroundSubmenus {
                        view: view_menu(),
                        sort: sort_menu(),
                        group: group_by_menu(),
                        new: new_menu(self.template_menu_entries()),
                    },
                )
            }
            (None, false) => {
                self.refresh_template_menu();
                background_menu(
                    &self.journal_label(JournalDirection::Undo),
                    &self.journal_label(JournalDirection::Redo),
                    &self.folder_applications(),
                    BackgroundSubmenus {
                        view: view_menu(),
                        sort: sort_menu(),
                        group: group_by_menu(),
                        new: new_menu(self.template_menu_entries()),
                    },
                )
            }
            (Some(_), true) => recycle_bin_item_menu(items.len() == 1),
            (Some(first), false) => {
                let mut facts = self.item_facts(first.entry(), items.len() == 1);
                let two_files = items.len() == 2 && items.iter().all(|item| !item.entry().is_dir);
                if two_files && Tool::Diff.installed().is_some() {
                    facts.comparison = Comparison::TwoFiles;
                }
                return item_menu(&facts, style);
            }
        };
        ContextMenu {
            entries,
            strip: Vec::new(),
        }
    }

    /// The applications the folder menu offers for the folder shown.
    fn folder_applications(&self) -> Vec<ApplicationChoice> {
        match self.current_uri() {
            Some(uri) if Page::from_uri(&uri).is_none() => {
                integration::menu_applications(&uri, None, true, MENU_APPLICATIONS)
            }
            _ => Vec::new(),
        }
    }

    /// The facts the menu of `entry` reads; `is_single` when it is the
    /// only item selected.
    fn item_facts(&self, entry: &Entry, is_single: bool) -> ItemFacts {
        let caching = if entry.is_dir {
            self.caching_of(entry.navigation_uri())
        } else {
            None
        };
        ItemFacts {
            navigation_uri: entry.navigation_uri().to_owned(),
            shape: item_shape(entry),
            location: item_location(&entry.uri),
            is_read_only: self.imp().locations.borrow().is_snapshot_location(&entry.uri),
            is_single,
            has_file_location: self.located_file(entry).is_some(),
            is_symlink: entry.is_symlink,
            comparison: Comparison::Unavailable,
            editors: self.context().desktop_integration().known_editor_shortcuts(),
            applications: if is_single {
                let content_type = entry.content_type.as_deref();
                integration::menu_applications(&entry.uri, content_type, entry.is_dir, MENU_APPLICATIONS)
            } else {
                Vec::new()
            },
            caching,
            delete_label: self.delete_label(),
            disk_tool: disk_tool_of(entry),
            send_to: self.send_to_targets(),
        }
    }

    /// Opens the classic menu from Shift+F10, pointing at the first
    /// selected item, or near the top of the view without one.
    pub(super) fn open_context_menu_from_keyboard(&self) {
        self.open_keyboard_menu(MenuStyle::Classic);
    }

    /// Opens the menu the Menu key opens, the right-click menu's style,
    /// as Windows 11 Explorer's Menu key opens its compact menu.
    pub(super) fn open_context_menu_from_menu_key(&self) {
        self.open_keyboard_menu(self.preferred_menu_style());
    }

    /// Opens a menu in `style` from the keyboard, pointing at the first
    /// selected item, or near the top of the view without one.
    fn open_keyboard_menu(&self, style: MenuStyle) {
        let view = self.folder_pane().input_widget();
        let selected = self.folder_pane().model().first_selected();
        if let Some(position) = selected {
            self.folder_pane().reveal(position);
        }
        let row = selected.and_then(|position| self.folder_pane().owners().widget_at(position));
        let bounds = row.and_then(|row| row.compute_bounds(&view));
        self.open_context_menu(&view, &keyboard_menu_anchor(bounds), style);
    }

    /// "Show more options": the classic menu where the compact one was.
    pub(super) fn show_more_options(&self) {
        let view = self.folder_pane().input_widget();
        let Some(point) = context_menu_of(&view).and_then(|popover| popover.pointing_to().1.into()) else {
            return;
        };
        self.open_context_menu(&view, &point, MenuStyle::Classic);
    }

    /// The folder menu's "New…", "Sort by" and "View": `entries`, the
    /// command bar's menu, where the folder menu was.
    fn show_menu_in_place(&self, entries: Vec<MenuEntry>) {
        let view = self.folder_pane().input_widget();
        let Some(popover) = context_menu_of(&view) else {
            return;
        };
        popover.set_entries(entries);
        popover.set_style_and_strip(MenuStyle::Classic, Vec::new());
        popover.popup();
    }

    /// Adds the actions the context menus run themselves: "Show more
    /// options", "New…", "Sort by", "View", "Unpin from Quick access" and "Open windows…".
    pub(super) fn install_context_menu_actions(&self) {
        self.install_sidebar_hiding();
        self.add_action_entries([
            plain_action(WindowAction::ShowMoreOptions, BrowserWindow::show_more_options),
            plain_action(WindowAction::ShowNewMenu, |window| {
                window.refresh_template_menu();
                window.show_menu_in_place(new_menu(window.template_menu_entries()));
            }),
            plain_action(WindowAction::ShowSortMenu, |window| {
                window.show_menu_in_place(sort_menu());
            }),
            plain_action(WindowAction::ShowViewMenu, |window| {
                window.show_menu_in_place(view_menu());
            }),
            text_action(WindowAction::Unpin, BrowserWindow::unpin),
            plain_action(WindowAction::AddPlace, |window| {
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    window,
                    async move { window.add_place().await }
                ));
            }),
            text_action(WindowAction::EditPin, |window, uri| {
                let uri = uri.to_owned();
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    window,
                    async move { window.edit_pin(uri).await }
                ));
            }),
            plain_action(WindowAction::OpenWindows, BrowserWindow::show_open_windows),
            plain_action(
                WindowAction::ClearRecentLocations,
                BrowserWindow::clear_recent_locations,
            ),
        ]);
    }

    /// "Clear recent locations": forgets the visited folders, and lists
    /// Recent locations again where it is shown (SIDE-026).
    fn clear_recent_locations(&self) {
        crate::folder_view::recent_locations::clear_recent_locations();
        if self.current_uri().as_deref() == Some(RECENT_LOCATIONS_URI) {
            WindowAction::Refresh.activate_from(self, None);
        }
        self.show_message(&ox_core::i18n::gettext("Recent locations cleared."));
    }

    /// The context menu of the view shown, for tests.
    #[cfg(test)]
    pub(super) fn context_menu(&self) -> MenuPopover {
        context_menu_of(&self.folder_pane().input_widget()).expect("every view has a context menu")
    }

    /// Opens the submenu labelled `label` of the open context menu, for
    /// the snapshot hook's `submenu=` step; false when there is none.
    pub(crate) fn open_context_submenu(&self, label: &str) -> bool {
        context_menu_of(&self.folder_pane().input_widget())
            .is_some_and(|menu| menu.is_visible() && menu.open_submenu_labelled(label))
    }

    /// Opens the context menu as a right-click at the item at `position`,
    /// or on blank space below the items without one, for tests: the
    /// selection changes as [`Self::select_for_context_menu`] decides.
    #[cfg(test)]
    pub(super) fn right_click(&self, position: Option<u32>) {
        let view = self.folder_pane().input_widget();
        let (x, y) = match position {
            Some(position) => {
                let point_on_item = || self.point_on_item(&view, position);
                crate::test_support::harness::wait_until("the right-clicked item to be laid out", || {
                    point_on_item().is_some()
                });
                point_on_item().expect("the right-clicked item is laid out")
            }
            None => (4.0, f64::from(view.height()) - 4.0),
        };
        self.select_for_context_menu(&view, x, y);
        let point = gdk::Rectangle::new(KEYBOARD_MENU_INSET, KEYBOARD_MENU_INSET, 1, 1);
        self.open_context_menu(&view, &point, self.preferred_menu_style());
    }

    /// The view's right-button gesture and a point on the item at
    /// `position`, to press and release it there, for tests.
    #[cfg(test)]
    pub(super) fn right_button_on_item(&self, position: u32) -> (gtk::GestureClick, f64, f64) {
        let view = self.folder_pane().input_widget();
        crate::test_support::harness::wait_until("the item to be laid out", || {
            self.point_on_item(&view, position).is_some()
        });
        let (x, y) = self.point_on_item(&view, position).expect("the item is laid out");
        let gesture = view
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
            .find(|gesture| gesture.button() == gdk::BUTTON_SECONDARY)
            .expect("the view takes right-clicks");
        (gesture, x, y)
    }

    /// A point in `view` on the item at `position`, once it is laid out
    /// there, for tests.
    #[cfg(test)]
    fn point_on_item(&self, view: &gtk::Widget, position: u32) -> Option<(f64, f64)> {
        let owners = self.folder_pane().owners();
        let bounds = owners.file_cell_at(position, view)?.compute_bounds(view)?;
        let (x, y) = (
            f64::from(bounds.x()) + 4.0,
            f64::from(bounds.y() + bounds.height() / 2.0),
        );
        (owners.position_at(view, x, y) == Some(position)).then_some((x, y))
    }
}

/// Where a menu opened from the keyboard points: into the selected row,
/// [`KEYBOARD_MENU_INSET`] from its start, or near the top of the view
/// without one.
fn keyboard_menu_anchor(row_bounds: Option<graphene::Rect>) -> gdk::Rectangle {
    let Some(bounds) = row_bounds else {
        return gdk::Rectangle::new(KEYBOARD_MENU_INSET, KEYBOARD_MENU_INSET, 1, 1);
    };
    let x = whole_pixels(bounds.x()) + KEYBOARD_MENU_INSET;
    let y = whole_pixels(bounds.y());
    let height = whole_pixels(bounds.height());
    gdk::Rectangle::new(x, y, 1, height)
}

/// A widget coordinate cut to whole pixels.
#[expect(clippy::cast_possible_truncation, reason = "widget bounds are small")]
fn whole_pixels(coordinate: f32) -> i32 {
    coordinate as i32
}

/// The context menu popover attached to `view`.
fn context_menu_of(view: &gtk::Widget) -> Option<MenuPopover> {
    children(view).find_map(|child| child.downcast::<MenuPopover>().ok())
}

/// The key that opens the right-click menu's style from the keyboard.
pub(super) const MENU_KEY: &str = "Menu";

/// The key that opens the classic menu, `win.context-menu`, as "Show more
/// options" does.
pub(super) const CLASSIC_MENU_KEY: &str = "<Shift>F10";

impl BrowserWindow {
    /// The Menu key opens the right-click menu's style and Shift+F10 the
    /// classic menu, as in Windows 11 Explorer. They are view shortcuts,
    /// not application accelerators, so the address and search entries
    /// keep their own text menus on those keys.
    fn context_menu_shortcuts(&self) -> gtk::ShortcutController {
        let shortcuts = gtk::ShortcutController::new();
        let menu_key = gtk::CallbackAction::new(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _| {
                window.open_context_menu_from_menu_key();
                glib::Propagation::Stop
            }
        ));
        let menu_key_trigger = gtk::ShortcutTrigger::parse_string(MENU_KEY);
        shortcuts.add_shortcut(gtk::Shortcut::new(menu_key_trigger, Some(menu_key)));
        let classic = gtk::ShortcutTrigger::parse_string(CLASSIC_MENU_KEY);
        let action = gtk::NamedAction::new(&WindowAction::ContextMenu.detailed_name());
        shortcuts.add_shortcut(gtk::Shortcut::new(classic, Some(action)));
        shortcuts
    }
}

/// The installed disk tool the menu of `entry` offers: Mount disk image
/// for a local disk image, Analyse disk usage for a local folder.
fn disk_tool_of(entry: &Entry) -> Option<DiskTool> {
    if !entry.navigation_uri().starts_with("file:") {
        return None;
    }
    let tool = if entry.is_dir {
        DiskTool::AnalyseUsage
    } else if is_disk_image(&entry.name) {
        DiskTool::MountImage
    } else {
        return None;
    };
    is_installed(tool).then_some(tool)
}

/// The style of a right-click menu: `preferred`, or the classic menu when
/// Shift is held, as Shift+right-click opens it in Windows 11 Explorer.
fn menu_style_for(preferred: MenuStyle, modifiers: gdk::ModifierType) -> MenuStyle {
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        MenuStyle::Classic
    } else {
        preferred
    }
}

/// Adds `extras`, the service actions and "Open as administrator…", to the
/// end of a menu, or in Windows 11's compact menu above its last item,
/// "Show more options", in a group of their own, as Explorer puts the
/// commands other programs add there.
fn add_extras(entries: &mut Vec<MenuEntry>, extras: Vec<MenuEntry>) {
    if extras.is_empty() {
        return;
    }
    let more = entries.iter().rposition(|entry| {
        matches!(
            entry,
            MenuEntry::Item(item)
                if item.action == super::menu_popover::MenuAction::Window(WindowAction::ShowMoreOptions)
        )
    });
    match more {
        Some(at) => {
            let group = std::iter::once(MenuEntry::Divider).chain(extras);
            // Before the divider that sets "Show more options" apart.
            let before = if at > 0 && matches!(entries[at - 1], MenuEntry::Divider) {
                at - 1
            } else {
                at
            };
            entries.splice(before..before, group);
        }
        None => entries.extend(extras),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    }

    fn entry(label: &str, action: WindowAction) -> MenuEntry {
        super::super::menu_popover::MenuItem::new(label, crate::icons::Icon::Info, action).into()
    }

    /// The service actions and "Open as administrator…" go above "Show
    /// more options" in a group of their own, and at the end of a menu
    /// without it.
    ///
    /// parity: CMD-010
    #[test]
    fn extras_sit_above_show_more_options_or_at_the_end() {
        let extras = || vec![entry("Open as administrator…", WindowAction::OpenAsAdministrator)];
        let mut compact = vec![
            entry("Properties", WindowAction::Properties),
            MenuEntry::Divider,
            entry("Show more options", WindowAction::ShowMoreOptions),
        ];
        add_extras(&mut compact, extras());
        assert_eq!(
            labels(&compact),
            [
                "Properties",
                "-",
                "Open as administrator…",
                "-",
                "Show more options"
            ]
        );
        let mut classic = vec![entry("Properties", WindowAction::Properties)];
        add_extras(&mut classic, extras());
        assert_eq!(labels(&classic), ["Properties", "Open as administrator…"]);
        let mut unchanged = classic.clone();
        add_extras(&mut unchanged, Vec::new());
        assert_eq!(unchanged, classic, "no extras, no divider");
    }

    /// Shift+right-click opens the classic menu whatever the choice, as in
    /// Windows 11 Explorer.
    ///
    /// parity: CMD-010
    #[test]
    fn shift_right_click_opens_the_classic_menu() {
        let shift = gdk::ModifierType::SHIFT_MASK;
        assert_eq!(menu_style_for(MenuStyle::Compact, shift), MenuStyle::Classic);
        assert_eq!(
            menu_style_for(MenuStyle::Compact, gdk::ModifierType::empty()),
            MenuStyle::Compact
        );
        assert_eq!(
            menu_style_for(MenuStyle::Classic, gdk::ModifierType::empty()),
            MenuStyle::Classic
        );
    }

    #[test]
    fn zip_archives_are_known_by_name_or_type() {
        assert!(is_zip("Photos.ZIP", None));
        assert!(is_zip("download", Some("application/x-zip-compressed")));
        assert!(!is_zip("notes.txt", Some("text/plain")));
        assert!(is_zip("backup.tar.gz", None));
    }
}
