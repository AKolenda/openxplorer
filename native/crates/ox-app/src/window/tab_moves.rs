// SPDX-License-Identifier: AGPL-3.0-only
//! Moving tabs: along the strip, into another window, or into a new window
//! of their own (TAB-029 to TAB-039).
//!
//! Ports `prepareTabTransfer`, `detachTab`, `moveTabMenu`,
//! `moveTabToWindow`, `reorderTab`, `receiveTransferredTab` and
//! `applyPendingTabRestore` of `desktop/ui/app.js`, and the handoff of
//! `desktop/tab_transfers.py`. Every window lives in this process, so a
//! tab moves as a [`MovedTab`], the state `prepareTabTransfer` sent: its
//! history, selection, scroll position, view, sort and Settings page. The
//! destination restores the history, view and sort at once, and the
//! selection and scroll position once its folder is listed. The drag
//! gesture that moves tabs is in [`drag`].
//!
//! Safety rule "the original is kept until the destination has the tab"
//! (`tab_transfers.py`): the source removes its tab only after the
//! destination added it, and every refusal keeps the original with the
//! Python app's message. A tab cannot leave a window that runs or plans a
//! file operation or shows a dialog (TAB-031), and such a window takes no
//! tab (TAB-037).

mod drag;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::sorting::SortOrder;
use crate::history::History;
use crate::icons::Icon;
use crate::locations::Page;
use crate::settings_page::SettingsView;

use super::actions::tab_action;
use super::folder_pane::FolderView;
use super::menu_popover::{MenuEntry, MenuItem};
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

pub(super) use drag::OutgoingTabDrag;

/// The title of a window without one, in the list of windows.
const UNTITLED_WINDOW: &str = "OpenXplorer window";

/// Why a tab did not move; the original stays where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum TabMoveRefusal {
    /// The source window runs a file operation or shows a dialog.
    #[error("Close this tab’s dialog and finish file operations before moving it.")]
    SourceBusy,
    /// The destination runs a file operation or shows a dialog, or closed.
    #[error("The destination was busy or closed. The original tab was kept.")]
    DestinationBusy,
    /// The tab closed, or the destination is the window it is in.
    #[error("Choose a different, ready OpenXplorer window.")]
    NoDestination,
    /// The drag ended without a place that takes the tab.
    #[error("Tab move cancelled. To detach, release below the tab strip or use Move tab to new window.")]
    Cancelled,
}

/// What a tab takes along when it moves to another window
/// (`prepareTabTransfer`).
#[derive(Debug, Clone)]
pub(super) struct MovedTab {
    /// Where it has been and where it is.
    history: History,
    /// The selected items' URIs.
    selected: Vec<String>,
    /// The vertical scroll position.
    scroll: f64,
    /// Details or icons. The view is the window's, as in the Python app,
    /// so the destination shows the moved tab's view.
    view: FolderView,
    /// The details' sort order, also the window's.
    sort: SortOrder,
    /// The page Settings shows, for the Settings tab.
    settings_view: Option<SettingsView>,
}

/// The menu that moves tab `id` into one of `windows`, which lists each
/// window's id, title and whether it can take a tab, or into a new window
/// (`moveTabMenu`).
fn move_tab_menu(id: TabId, windows: &[OtherWindow]) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = windows.iter().map(|window| window.menu_item(id).into()).collect();
    if entries.is_empty() {
        let none = MenuItem::new(
            "No other OpenXplorer windows",
            Icon::Desktop,
            WindowAction::MoveTabIntoWindow,
        );
        entries.push(none.disabled_when(true).into());
    }
    entries.push(MenuEntry::Divider);
    let new_window = MenuItem::with_target(
        "Move tab to new window",
        Icon::Share,
        WindowAction::MoveTabToNewWindow,
        id.to_variant(),
    );
    entries.push(new_window.into());
    entries
}

/// Another window, as the move-tab menu lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OtherWindow {
    /// The application's id of the window.
    id: u32,
    /// Its title.
    title: String,
    /// It can take a tab now.
    is_ready: bool,
}

impl OtherWindow {
    /// The item that moves tab `tab` into this window.
    fn menu_item(&self, tab: TabId) -> MenuItem {
        let target = (tab.to_raw(), self.id).to_variant();
        let item = MenuItem::with_target(
            &self.title,
            Icon::Desktop,
            WindowAction::MoveTabIntoWindow,
            target,
        );
        item.disabled_when(!self.is_ready)
    }
}

impl BrowserWindow {
    /// True while the window writes files or shows a dialog: its tabs
    /// stay, and it takes no tab (TAB-031, TAB-037).
    fn is_busy_for_tab_moves(&self) -> bool {
        self.is_writing_files() || self.shows_dialog()
    }

    /// True while a dialog of this window is open.
    fn shows_dialog(&self) -> bool {
        let this = self.upcast_ref::<gtk::Window>();
        let toplevels = gtk::Window::list_toplevels();
        toplevels
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Window>())
            .any(|window| window.is_visible() && window.transient_for().as_ref() == Some(this))
    }

    /// What tab `id` takes along when it moves, with the selection and
    /// scroll position of the tab in front as they are now.
    fn moved_tab(&self, id: TabId) -> Option<MovedTab> {
        if self.imp().session.borrow().is_active(id) {
            self.save_tab_view();
        }
        let session = self.imp().session.borrow();
        let tab = session.tab(id)?;
        let is_settings = Page::from_uri(tab.uri()) == Some(Page::Settings);
        Some(MovedTab {
            history: tab.history.clone(),
            selected: tab.selected.clone(),
            scroll: tab.scroll,
            view: self.folder_pane().view(),
            sort: self.folder_pane().details().sort_order(),
            settings_view: is_settings.then(|| self.settings_page().view()),
        })
    }

    /// Adds `tab`, moved from another window, before tab `before` or at
    /// the end, and shows it (`receiveTransferredTab`). A Settings tab
    /// shows this window's Settings at its page instead of a second one.
    ///
    /// # Errors
    ///
    /// [`TabMoveRefusal::DestinationBusy`] while this window runs a file
    /// operation or shows a dialog; nothing changes then.
    fn receive_tab(&self, tab: MovedTab, before: Option<TabId>) -> Result<(), TabMoveRefusal> {
        if self.is_busy_for_tab_moves() {
            return Err(TabMoveRefusal::DestinationBusy);
        }
        if let Some(view) = tab.settings_view {
            self.open_settings(Some(view));
            return Ok(());
        }
        self.save_tab_view();
        let uri = tab.history.current().to_owned();
        let id = self.imp().session.borrow_mut().insert_moved(tab.history, before);
        if let Some(received) = self.imp().session.borrow_mut().tab_mut(id) {
            received.selected = tab.selected;
            received.scroll_after_listing = Some(tab.scroll);
        }
        self.context().remember_network(&uri);
        self.show_view(tab.view);
        self.folder_pane().details().sort_by(tab.sort);
        self.show_tab(id);
        Ok(())
    }

    /// Hands tab `id` to `destination`, before its tab `before` or at the
    /// end, and raises it. The caller removes the tab here afterwards.
    ///
    /// # Errors
    ///
    /// Why the tab stays: this window or the destination is busy, the tab
    /// closed, or the destination is this window.
    fn hand_over_tab(
        &self,
        id: TabId,
        destination: &BrowserWindow,
        before: Option<TabId>,
    ) -> Result<(), TabMoveRefusal> {
        if destination == self {
            return Err(TabMoveRefusal::NoDestination);
        }
        if self.is_busy_for_tab_moves() {
            return Err(TabMoveRefusal::SourceBusy);
        }
        let tab = self.moved_tab(id).ok_or(TabMoveRefusal::NoDestination)?;
        destination.receive_tab(tab, before)?;
        destination.present();
        Ok(())
    }

    /// Opens a new window with tab `id` (`detachTab`), not presented yet.
    ///
    /// # Errors
    ///
    /// Why the tab stays: this window is busy, or the tab closed.
    fn detach_tab(&self, id: TabId) -> Result<BrowserWindow, TabMoveRefusal> {
        if self.is_busy_for_tab_moves() {
            return Err(TabMoveRefusal::SourceBusy);
        }
        let tab = self.moved_tab(id).ok_or(TabMoveRefusal::NoDestination)?;
        let app = self.application().ok_or(TabMoveRefusal::NoDestination)?;
        let window = BrowserWindow::new(&app, self.context());
        if let Err(refusal) = window.receive_tab(tab, None) {
            window.close();
            return Err(refusal);
        }
        Ok(window)
    }

    /// "Move tab to new window": opens a window with tab `id` and removes
    /// it here, closing this window when it was its only tab (TAB-029).
    pub(super) fn move_tab_to_new_window(&self, id: TabId) {
        match self.detach_tab(id) {
            Ok(window) => {
                window.present();
                self.release_moved_tab(id);
            }
            Err(refusal) => self.show_message(&refusal.to_string()),
        }
    }

    /// Moves tab `id` into the window with the application id `window_id`
    /// (`moveTabToWindow`), then removes it here (TAB-030).
    pub(super) fn move_tab_into_window(&self, id: TabId, window_id: u32) {
        let destination = self
            .application()
            .and_then(|app| app.window_by_id(window_id))
            .and_downcast::<BrowserWindow>();
        let Some(destination) = destination else {
            self.show_message(&TabMoveRefusal::DestinationBusy.to_string());
            return;
        };
        match self.hand_over_tab(id, &destination, None) {
            Ok(()) => self.release_moved_tab(id),
            Err(refusal) => self.show_message(&refusal.to_string()),
        }
    }

    /// Moves tab `id` before tab `before`, or to the end (`reorderTab`).
    fn reorder_tab(&self, id: TabId, before: Option<TabId>) {
        self.imp().session.borrow_mut().move_before(id, before);
        self.render_tabs();
    }

    /// "Move tab to window…": lists the other windows under tab `id`
    /// (`moveTabMenu`), or says why the tab cannot move.
    pub(super) fn show_move_tab_menu(&self, id: TabId) {
        if self.is_busy_for_tab_moves() {
            self.show_message(&TabMoveRefusal::SourceBusy.to_string());
            return;
        }
        let entries = move_tab_menu(id, &self.other_windows());
        self.tab_strip().show_menu_under_tab(id, entries);
    }

    /// The other open windows, in the application's order.
    fn other_windows(&self) -> Vec<OtherWindow> {
        let windows = self.application().map(|app| app.windows()).unwrap_or_default();
        windows
            .into_iter()
            .filter_map(|window| window.downcast::<BrowserWindow>().ok())
            .filter(|window| window != self)
            .map(|window| OtherWindow {
                id: window.id(),
                title: window
                    .title()
                    .map_or_else(|| UNTITLED_WINDOW.to_owned(), |title| title.to_string()),
                is_ready: !window.is_busy_for_tab_moves(),
            })
            .collect()
    }

    /// Adds the tab-moving actions: the tab menu's two items and the
    /// window list's items.
    pub(super) fn install_tab_move_actions(&self) {
        let into_window = gio::ActionEntry::builder(WindowAction::MoveTabIntoWindow.name())
            .parameter_type(Some(&<(u64, u32)>::static_variant_type()))
            .activate(|window: &BrowserWindow, _, target| {
                let Some((tab, window_id)) = target.and_then(glib::Variant::get::<(u64, u32)>) else {
                    return;
                };
                window.move_tab_into_window(TabId::from_raw(tab), window_id);
            })
            .build();
        self.add_action_entries([
            tab_action(
                WindowAction::MoveTabToNewWindow,
                BrowserWindow::move_tab_to_new_window,
            ),
            tab_action(WindowAction::MoveTabToWindow, BrowserWindow::show_move_tab_menu),
            into_window,
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{
        capture_popover, settle, wait_until, Fixture, OpenedWindows, TestWindow,
    };
    use crate::window::dialog::Dialog;
    use crate::window::menu_popover::ItemAvailability;

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    }

    fn tab() -> TabId {
        TabId::from_raw(7)
    }

    /// parity: TAB-030
    #[test]
    fn the_move_menu_lists_the_other_windows_and_a_new_window() {
        let windows = [
            OtherWindow {
                id: 2,
                title: "Documents".to_owned(),
                is_ready: true,
            },
            OtherWindow {
                id: 3,
                title: "Downloads".to_owned(),
                is_ready: false,
            },
        ];

        let menu = move_tab_menu(tab(), &windows);

        assert_eq!(
            labels(&menu),
            ["Documents", "Downloads", "-", "Move tab to new window"]
        );
        let MenuEntry::Item(busy) = &menu[1] else {
            panic!("the second entry is a window");
        };
        assert_eq!(busy.target, Some((7_u64, 3_u32).to_variant()));
        assert_eq!(
            busy.availability,
            ItemAvailability::Disabled,
            "a busy window takes no tab"
        );
    }

    /// parity: TAB-030
    #[test]
    fn without_other_windows_the_move_menu_says_so() {
        let menu = move_tab_menu(tab(), &[]);

        assert_eq!(
            labels(&menu),
            ["No other OpenXplorer windows", "-", "Move tab to new window"]
        );
        let MenuEntry::Item(none) = &menu[0] else {
            panic!("the first entry is an item");
        };
        assert_eq!(none.availability, ItemAvailability::Disabled);
    }

    fn tab_ids(window: &BrowserWindow) -> Vec<TabId> {
        let session = window.imp().session.borrow();
        session.tabs().iter().map(|tab| tab.id).collect()
    }

    fn active_tab(window: &BrowserWindow) -> TabId {
        window
            .imp()
            .session
            .borrow()
            .active_id()
            .expect("a window has a tab")
    }

    fn run_tab_action(window: &BrowserWindow, action: WindowAction, target: &glib::Variant) {
        WidgetExt::activate_action(window, &action.detailed_name(), Some(target))
            .expect("the window has the action");
    }

    /// A window with a tab on `first` and, in front, a tab that went from
    /// `first` to the folder of `many` with a scrolled, partly selected
    /// view.
    fn window_with_a_travelled_tab(first: &Fixture, many: &Fixture) -> TestWindow {
        let test = TestWindow::open(&first.uri());
        test.window.add_tab(&first.uri()).expect("a folder");
        test.wait_for_listing("the second tab");
        test.window.navigate(&many.uri()).expect("a folder");
        test.wait_for_listing("the scrolled folder");
        test.window
            .folder_model()
            .select_only(test.position_of("file 0042.txt"));
        test.window.folder_pane().restore_scroll_position(600.0);
        wait_until("the scroll", || {
            test.window.folder_pane().scroll_position() > 599.0
        });
        test
    }

    /// parity: TAB-029, TAB-038, TAB-039
    #[gtk::test]
    fn a_tab_moved_to_a_new_window_keeps_its_folder_history_selection_and_scroll() {
        let first = Fixture::standard();
        let many = Fixture::with_files(200);
        let test = window_with_a_travelled_tab(&first, &many);
        let moved = active_tab(&test.window);

        run_tab_action(
            &test.window,
            WindowAction::MoveTabToNewWindow,
            &moved.to_variant(),
        );
        let opened = OpenedWindows::only(&[&test.window]);
        let window = opened.window();
        wait_until("the moved tab to be listed", || window.is_listed());

        assert_eq!(window.current_uri(), Some(many.uri()));
        let history = window
            .imp()
            .session
            .borrow()
            .active()
            .map(|tab| tab.history.clone());
        let mut expected = crate::history::History::new(&first.uri());
        expected.push(&many.uri());
        assert_eq!(history, Some(expected));
        wait_until("the selection", || {
            window.folder_model().selected_items().len() == 1
        });
        assert_eq!(
            window.folder_model().selected_items()[0].entry().name,
            "file 0042.txt"
        );
        wait_until("the scroll position", || {
            window.folder_pane().scroll_position() > 599.0
        });
        assert_eq!(
            tab_ids(&test.window).len(),
            1,
            "the original went once the new window had it"
        );
        assert!(!tab_ids(&test.window).contains(&moved));
    }

    /// parity: TAB-029
    #[gtk::test]
    fn moving_the_only_tab_to_a_new_window_closes_the_old_one() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let source = test.window.downgrade();
        let only = active_tab(&test.window);

        run_tab_action(&test.window, WindowAction::MoveTabToNewWindow, &only.to_variant());
        let opened = OpenedWindows::only(&[&test.window]);
        settle();

        assert_eq!(opened.window().current_uri(), Some(fixture.uri()));
        assert!(
            source.upgrade().is_none_or(|window| !window.is_visible()),
            "the old window closed"
        );
    }

    /// parity: TAB-030
    #[gtk::test]
    fn a_tab_moved_into_another_window_goes_there_and_leaves_here() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let other = test.open_beside(&fixture.uri());
        let moved = active_tab(&test.window);

        let target = (moved.to_raw(), other.window.id()).to_variant();
        run_tab_action(&test.window, WindowAction::MoveTabIntoWindow, &target);

        assert_eq!(other.window.tab_count(), 2);
        assert_eq!(other.window.current_uri(), Some(fixture.uri_of("Documents")));
        assert_eq!(test.window.tab_count(), 1);
        assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    }

    /// parity: TAB-030
    #[gtk::test]
    fn move_tab_to_window_lists_the_other_windows() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let other = test.open_beside(&fixture.uri_of("Documents"));
        let title = "Documents — OpenXplorer";
        wait_until("the other window's title", || {
            other.window.title().is_some_and(|shown| shown == title)
        });
        let tab = active_tab(&test.window);

        run_tab_action(&test.window, WindowAction::MoveTabToWindow, &tab.to_variant());
        let menu = test.window.tab_strip().menu();
        wait_until("the window list", || menu.is_mapped());

        capture_popover(&test.window, menu.upcast_ref(), "native-menu-move-tab.png");
        assert_eq!(menu.row_labels(), [title, "-", "Move tab to new window"]);
        menu.popdown();
    }

    /// parity: TAB-031, TAB-037
    #[gtk::test]
    fn a_busy_window_keeps_its_tabs_and_takes_none() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let other = test.open_beside(&fixture.uri());
        let tab = active_tab(&test.window);
        let target = (tab.to_raw(), other.window.id()).to_variant();

        let operation = test.window.begin_operation("Preparing copy…");
        run_tab_action(&test.window, WindowAction::MoveTabIntoWindow, &target);
        let busy_source = test.window.shown_message();
        test.window.end_operation();
        let other_operation = other.window.begin_operation("Preparing copy…");
        run_tab_action(&test.window, WindowAction::MoveTabIntoWindow, &target);
        let busy_destination = test.window.shown_message();
        other.window.end_operation();

        assert!(operation.is_some() && other_operation.is_some());
        assert_eq!(
            busy_source,
            "Close this tab’s dialog and finish file operations before moving it."
        );
        assert_eq!(
            busy_destination,
            "The destination was busy or closed. The original tab was kept."
        );
        assert_eq!(test.window.tab_count(), 2);
        assert_eq!(other.window.tab_count(), 1);
    }

    /// parity: TAB-031, TAB-037
    #[gtk::test]
    fn a_window_with_an_open_dialog_keeps_its_tabs_and_takes_none() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let other = test.open_beside(&fixture.uri());
        let tab = active_tab(&test.window);
        let target = (tab.to_raw(), other.window.id()).to_variant();

        let dialog = Dialog::new(&test.window, "Rename", "Type a new name.");
        dialog.open();
        run_tab_action(&test.window, WindowAction::MoveTabIntoWindow, &target);
        let with_own_dialog = test.window.shown_message();
        dialog.finish();
        let other_dialog = Dialog::new(&other.window, "Rename", "Type a new name.");
        other_dialog.open();
        run_tab_action(&test.window, WindowAction::MoveTabIntoWindow, &target);
        let with_destination_dialog = test.window.shown_message();
        other_dialog.finish();

        assert_eq!(with_own_dialog, TabMoveRefusal::SourceBusy.to_string());
        assert_eq!(
            with_destination_dialog,
            TabMoveRefusal::DestinationBusy.to_string()
        );
        assert_eq!(test.window.tab_count(), 2);
        assert_eq!(other.window.tab_count(), 1);
    }
}
