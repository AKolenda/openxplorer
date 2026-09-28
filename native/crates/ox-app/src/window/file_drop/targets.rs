// SPDX-License-Identifier: AGPL-3.0-only
//! The parts of the window that take dropped files, where a drop at each
//! point goes, and the highlight that shows it (DND-011, DND-014, DND-016,
//! TAB-018).
//!
//! Ports `publishFileDragLayout` and `showFileDropHint` of
//! `desktop/ui/app.js`. The web app published rectangles for the native
//! side to hit-test; here each part answers for a point itself, in its
//! own coordinates at its real size, so scaling and clipping need no
//! arithmetic. A folder view takes drops on a writable folder, on a
//! program, or on blank space for the folder shown; the sidebar on a
//! place's folder, and in Quick access to pin; the breadcrumbs and the
//! tabs on their folders. Holding a drag over a tab for 800 ms shows it,
//! as Windows Explorer does. Nothing takes drops while a file operation
//! runs; no highlight means no drop.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::DropDestination;
use crate::window::file_drag::{is_draggable_location, DraggedItems};
use crate::window::session::TabId;
use crate::window::sidebar::SidebarDropSpot;
use crate::window::BrowserWindow;

/// How long a drag must stay over a tab before the tab is shown.
const TAB_HOVER_DELAY: Duration = Duration::from_millis(800);

/// The CSS class of the folder view while a drop would go into the
/// folder it shows (`#file-scroll.file-drop-active`).
const VIEW_DROP_CLASS: &str = "file-drop-active";

/// A part of the window that takes dropped files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropZone {
    /// A folder view.
    FolderView,
    /// The sidebar.
    Sidebar,
    /// The breadcrumbs.
    Breadcrumbs,
    /// The tabs.
    Tabs,
}

/// Where a drop at one point of a zone goes, and what shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DropSpot {
    /// In a folder view: a folder or program under the pointer, at
    /// position `row`, or the folder shown when `row` is `None`.
    FolderView {
        destination: DropDestination,
        row: Option<u32>,
    },
    /// A sidebar row, or a place in Quick access.
    Sidebar(SidebarDropSpot),
    /// A crumb's folder.
    Crumb(String),
    /// A tab's folder.
    Tab { id: TabId, folder: String },
}

impl DropSpot {
    /// Where the drop goes.
    fn destination(&self) -> DropDestination {
        match self {
            DropSpot::FolderView { destination, .. } => destination.clone(),
            DropSpot::Sidebar(SidebarDropSpot::Folder { uri, .. }) => DropDestination::Folder(uri.clone()),
            DropSpot::Sidebar(SidebarDropSpot::Pin { before, .. }) => DropDestination::QuickAccess {
                before: before.clone(),
            },
            DropSpot::Crumb(folder) | DropSpot::Tab { folder, .. } => DropDestination::Folder(folder.clone()),
        }
    }
}

/// The formats a file drop target takes: this process's own dragged
/// items, and every format GTK reads a file list from.
fn file_drop_formats() -> gdk::ContentFormats {
    gdk::ContentFormatsBuilder::new()
        .add_type(DraggedItems::static_type())
        .add_type(gdk::FileList::static_type())
        .build()
        .union_deserialize_mime_types()
}

/// Every action a drop may run.
fn every_action() -> gdk::DragAction {
    gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK | gdk::DragAction::ASK
}

impl BrowserWindow {
    /// Lets `widget`, the window's `zone`, take dropped files.
    pub(in crate::window) fn attach_file_drop_zone(&self, widget: &impl IsA<gtk::Widget>, zone: DropZone) {
        let target = gtk::DropTargetAsync::new(Some(file_drop_formats()), every_action());
        let hover = glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |target: &gtk::DropTargetAsync, drop: &gdk::Drop, x: f64, y: f64| {
                window.hover_drop(zone, target, drop, x, y)
            }
        );
        target.connect_drag_enter(hover.clone());
        target.connect_drag_motion(hover);
        target.connect_drag_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.leave_drop_zone(zone)
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |target, drop, x, y| window.take_drop(zone, target, drop, x, y)
        ));
        widget.add_controller(target);
    }

    /// A drag moves over `zone`: highlights where a drop would go and
    /// returns the action it would run, or none.
    fn hover_drop(
        &self,
        zone: DropZone,
        target: &gtk::DropTargetAsync,
        drop: &gdk::Drop,
        x: f64,
        y: f64,
    ) -> gdk::DragAction {
        let spot = target
            .widget()
            .and_then(|widget| self.drop_spot(zone, &widget, x, y));
        self.show_drop_spot(zone, spot.as_ref());
        let action = self.drop_action(drop);
        match (spot, action) {
            (Some(_), Some(action)) => action.as_drag_action(),
            _ => gdk::DragAction::empty(),
        }
    }

    /// The drag left `zone`, or dropped there: its highlight goes, and
    /// what was learnt about programs under it is forgotten, as they may
    /// change before the next drag.
    fn leave_drop_zone(&self, zone: DropZone) {
        self.show_drop_spot(zone, None);
        if zone == DropZone::FolderView {
            self.forget_program_checks();
        }
    }

    /// Takes `drop` at (`x`, `y`) of `zone`: finds where it goes, then
    /// reads its items and sends them there once the handler returned.
    fn take_drop(
        &self,
        zone: DropZone,
        target: &gtk::DropTargetAsync,
        drop: &gdk::Drop,
        x: f64,
        y: f64,
    ) -> bool {
        let Some(widget) = target.widget() else {
            return false;
        };
        let spot = self.drop_spot(zone, &widget, x, y);
        self.leave_drop_zone(zone);
        let (Some(spot), Some(action)) = (spot, self.drop_action(drop)) else {
            return false;
        };
        if let Err(refusal) = self.check_idle() {
            self.show_message(&refusal.to_string());
            return false;
        }
        self.remember_drop_point(&widget, x, y);
        let destination = spot.destination();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            drop,
            async move {
                window.receive_drop(drop, destination, action).await;
            }
        ));
        true
    }

    /// Keeps where a drop happened, in the folder pane's coordinates, for
    /// the drop menu.
    fn remember_drop_point(&self, widget: &gtk::Widget, x: f64, y: f64) {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        let in_pane = widget.compute_point(self.folder_pane(), &point).unwrap_or(point);
        self.imp()
            .drop_point
            .set((f64::from(in_pane.x()), f64::from(in_pane.y())));
    }

    /// Where a drop at (`x`, `y`) of `widget`, the window's `zone`, goes;
    /// `None` where nothing takes it, and anywhere while a file operation
    /// runs.
    fn drop_spot(&self, zone: DropZone, widget: &gtk::Widget, x: f64, y: f64) -> Option<DropSpot> {
        if self.check_idle().is_err() {
            return None;
        }
        match zone {
            DropZone::FolderView => {
                let position = self.folder_pane().owners().position_at(widget, x, y);
                self.folder_view_spot(position)
            }
            DropZone::Sidebar => self.sidebar_spot(y),
            DropZone::Breadcrumbs => {
                let folder = self.address_bar().crumb_location_at(x, y)?;
                self.takes_drops(&folder).then_some(DropSpot::Crumb(folder))
            }
            DropZone::Tabs => {
                let tab = self.tab_strip().tab_at(x, y)?;
                self.takes_drops(&tab.uri).then_some(DropSpot::Tab {
                    id: tab.id,
                    folder: tab.uri,
                })
            }
        }
    }

    /// Where a drop on the folder view at `position`, or on blank space,
    /// goes: into a writable folder, to a program, or into the folder
    /// shown.
    fn folder_view_spot(&self, position: Option<u32>) -> Option<DropSpot> {
        let under_pointer = position.and_then(|position| self.item_destination(position));
        if let Some(destination) = under_pointer {
            return Some(DropSpot::FolderView {
                destination,
                row: position,
            });
        }
        let shown = self.shown_folder_for_drops()?;
        Some(DropSpot::FolderView {
            destination: DropDestination::Folder(shown),
            row: None,
        })
    }

    /// Where a drop on the item at `position` goes: into it when it is a
    /// folder that takes drops, to it when it is a program; `None` for any
    /// other item, whose drop goes into the folder shown.
    fn item_destination(&self, position: u32) -> Option<DropDestination> {
        let item = self.folder_pane().model().item(position)?;
        let entry = item.entry();
        if entry.is_dir && !entry.is_virtual {
            let folder = entry.navigation_uri().to_owned();
            return self
                .takes_drops(&folder)
                .then_some(DropDestination::Folder(folder));
        }
        self.program_under_drag(entry).map(DropDestination::Program)
    }

    /// Where a drop on the folder view at `position` goes, for tests that
    /// drop without a pointer.
    #[cfg(test)]
    pub(super) fn folder_view_destination(&self, position: Option<u32>) -> Option<DropDestination> {
        self.folder_view_spot(position).map(|spot| spot.destination())
    }

    /// The folder shown, when a drop on blank space may go into it: a
    /// writable folder that is listed without error and not searched.
    fn shown_folder_for_drops(&self) -> Option<String> {
        let shown = self.current_uri()?;
        let is_listed_cleanly = {
            let session = self.imp().session.borrow();
            session
                .active()
                .is_some_and(|tab| tab.error.is_none() && !tab.listing_state.is_listing())
        };
        let takes_drops =
            is_listed_cleanly && !self.folder_pane().model().is_searching() && self.takes_drops(&shown);
        takes_drops.then_some(shown)
    }

    /// Where a drop on the sidebar at `y` goes: a writable place's folder,
    /// or Quick access.
    fn sidebar_spot(&self, y: f64) -> Option<DropSpot> {
        let spot = self.sidebar().drop_spot_at(y)?;
        if let SidebarDropSpot::Folder { uri, .. } = &spot {
            if !self.takes_drops(uri) {
                return None;
            }
        }
        Some(DropSpot::Sidebar(spot))
    }

    /// True for a folder dropped items may go into: a writable local or
    /// SMB folder, not a page, a server or a previous version.
    fn takes_drops(&self, folder: &str) -> bool {
        is_draggable_location(folder) && self.imp().locations.borrow().is_writable_location(folder)
    }

    /// Highlights `spot` in `zone`, or nothing there.
    fn show_drop_spot(&self, zone: DropZone, spot: Option<&DropSpot>) {
        match zone {
            DropZone::FolderView => self.show_folder_view_spot(spot),
            DropZone::Sidebar => {
                let sidebar_spot = match spot {
                    Some(DropSpot::Sidebar(spot)) => Some(spot),
                    _ => None,
                };
                self.sidebar().show_drop_spot(sidebar_spot);
            }
            DropZone::Breadcrumbs => {
                let crumb = match spot {
                    Some(DropSpot::Crumb(folder)) => Some(folder.as_str()),
                    _ => None,
                };
                self.address_bar().highlight_crumb(crumb);
            }
            DropZone::Tabs => {
                let tab = match spot {
                    Some(DropSpot::Tab { id, .. }) => Some(*id),
                    _ => None,
                };
                self.tab_strip().highlight_drop_tab(tab);
                self.show_tab_after_hover(tab);
            }
        }
    }

    /// Highlights the folder view's row, or the whole view, of `spot`,
    /// and says which program a drop there opens.
    fn show_folder_view_spot(&self, spot: Option<&DropSpot>) {
        let pane = self.folder_pane();
        let (row, whole_view, program) = match spot {
            Some(DropSpot::FolderView { destination, row }) => {
                let program = match destination {
                    DropDestination::Program(program) => Some(program.name.clone()),
                    DropDestination::Folder(_) | DropDestination::QuickAccess { .. } => None,
                };
                (*row, row.is_none(), program)
            }
            _ => (None, false, None),
        };
        pane.owners().show_drop_target(row);
        let view = pane.view_widget();
        if whole_view {
            view.add_css_class(VIEW_DROP_CLASS);
        } else {
            view.remove_css_class(VIEW_DROP_CLASS);
        }
        let hint = program.map(|name| format!("Open with {name}"));
        pane.show_drag_hint(hint.as_deref());
    }

    /// Shows tab `id` once a drag has stayed over it for
    /// [`TAB_HOVER_DELAY`]; a drag that leaves it, or `None`, stops that.
    fn show_tab_after_hover(&self, id: Option<TabId>) {
        let waiting = self
            .imp()
            .tab_hover
            .borrow()
            .as_ref()
            .map(|(waiting, _)| *waiting);
        if waiting == id {
            return;
        }
        if let Some((_, timer)) = self.imp().tab_hover.take() {
            timer.remove();
        }
        let Some(id) = id else {
            return;
        };
        let timer = glib::timeout_add_local_once(
            TAB_HOVER_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().tab_hover.replace(None);
                    window.switch_tab(id);
                }
            ),
        );
        self.imp().tab_hover.replace(Some((id, timer)));
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use super::*;
    use crate::test_support::harness::{
        capture, capture_popover, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
    };
    use crate::window::file_drop::DropAction;

    /// Where a drop on the item called `name`, or on blank space for
    /// `None`, goes in `test`'s folder view.
    fn destination_of(test: &TestWindow, name: Option<&str>) -> Option<DropDestination> {
        let position = name.map(|name| test.position_of(name));
        test.window
            .folder_view_spot(position)
            .map(|spot| spot.destination())
    }

    /// The middle of `widget` in `ancestor`'s coordinates.
    fn middle_of(widget: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> (f64, f64) {
        let bounds = widget
            .compute_bounds(ancestor)
            .expect("a shown widget has bounds");
        let x = bounds.x() + bounds.width() / 2.0;
        let y = bounds.y() + bounds.height() / 2.0;
        (f64::from(x), f64::from(y))
    }

    /// A copy of `cp` called "copier" in `fixture`: a program that shows
    /// which arguments it got by what it creates.
    fn install_copier(fixture: &Fixture) {
        let copier = fixture.path("copier");
        std::fs::copy("/usr/bin/cp", &copier).expect("the test system has cp");
        std::fs::set_permissions(&copier, std::fs::Permissions::from_mode(0o755))
            .expect("the fixture is ours");
    }

    /// parity: DND-011
    #[gtk::test]
    fn a_drop_goes_into_the_folder_under_the_pointer_or_the_folder_shown() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let shown = Some(DropDestination::Folder(fixture.uri()));

        let on_folder = destination_of(&test, Some("Documents"));
        let on_file = destination_of(&test, Some("Notes 2.txt"));
        let on_blank = destination_of(&test, None);
        test.window.search_box().entry().set_text("Notes");
        wait_until("the search to filter", || {
            test.window.folder_model().is_searching()
        });
        let while_searching = destination_of(&test, None);

        assert_eq!(
            on_folder,
            Some(DropDestination::Folder(fixture.uri_of("Documents")))
        );
        assert_eq!(on_file, shown, "a plain file takes no drop; its folder does");
        assert_eq!(on_blank, shown);
        assert_eq!(while_searching, None, "search results take no drop");
    }

    /// parity: DND-011
    #[gtk::test]
    fn only_the_folder_under_a_drag_is_highlighted() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let documents = test.position_of("Documents");
        let owners = || test.window.folder_pane().owners();
        let view = test.window.folder_pane().view_widget();

        let on_folder = test.window.folder_view_spot(Some(documents));
        test.window
            .show_drop_spot(DropZone::FolderView, on_folder.as_ref());
        let row_while_on_folder = owners().is_shown_drop_target(documents);
        let view_while_on_folder = view.has_css_class(VIEW_DROP_CLASS);
        let on_blank = test.window.folder_view_spot(None);
        test.window
            .show_drop_spot(DropZone::FolderView, on_blank.as_ref());
        let row_while_on_blank = owners().is_shown_drop_target(documents);
        let view_while_on_blank = view.has_css_class(VIEW_DROP_CLASS);
        test.window.leave_drop_zone(DropZone::FolderView);

        assert_eq!(row_while_on_folder, Some(true));
        assert!(!view_while_on_folder);
        assert_eq!(row_while_on_blank, Some(false));
        assert!(view_while_on_blank, "blank space means the folder shown");
        assert_eq!(owners().is_shown_drop_target(documents), Some(false));
        assert!(
            !view.has_css_class(VIEW_DROP_CLASS),
            "no highlight once the drag left"
        );
    }

    /// parity: DND-009, DND-011, DND-014
    #[gtk::test]
    fn sidebar_places_take_drops_and_quick_access_pins_where_the_line_shows() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let sidebar = test.window.sidebar();
        let home = sidebar.middle_of("Home");
        let documents = sidebar.middle_of("Documents");
        let this_pc = sidebar.middle_of("This PC");

        let on_home = test.window.sidebar_spot(home);
        let above_documents = test.window.sidebar_spot(documents - 5.0);
        test.window
            .show_drop_spot(DropZone::Sidebar, above_documents.as_ref());
        let line = sidebar.drop_highlight_of("Documents");
        test.window.leave_drop_zone(DropZone::Sidebar);

        let home_uri = test.window.imp().locations.borrow().home_uri();
        let on_home = on_home.map(|spot| spot.destination());
        assert_eq!(on_home, Some(DropDestination::Folder(home_uri)));
        let pin_spot = above_documents.map(|spot| spot.destination());
        assert!(
            matches!(&pin_spot, Some(DropDestination::QuickAccess { before: Some(before) }) if before.ends_with("/Documents")),
            "{pin_spot:?}"
        );
        assert_eq!(line, Some("drop-before"));
        assert_eq!(sidebar.drop_highlight_of("Documents"), None);
        assert_eq!(test.window.sidebar_spot(this_pc), None, "a page takes no drop");
    }

    /// parity: DND-014
    #[gtk::test]
    fn folders_dropped_on_quick_access_are_pinned_and_files_are_not() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let destination = Some(DropDestination::QuickAccess { before: None });

        let folder_taken = test
            .window
            .complete_drop(&[fixture.uri()], destination.clone(), DropAction::Copy);
        wait_until("the pin", || {
            test.window
                .sidebar()
                .labels()
                .contains(&"Example projects".to_owned())
        });
        let pinned_message = test.window.shown_message();
        let file_taken =
            test.window
                .complete_drop(&[fixture.uri_of("Notes 2.txt")], destination, DropAction::Copy);
        wait_until("the refusal", || {
            test.window.shown_message().starts_with("Could not pin")
        });

        assert!(folder_taken && file_taken, "both are checked off the main thread");
        assert_eq!(pinned_message, "Pinned to Quick access. No files were moved.");
        assert!(!test.window.sidebar().labels().contains(&"Notes 2.txt".to_owned()));
    }

    /// parity: DND-011, DND-016
    #[gtk::test]
    fn a_crumb_takes_drops_for_its_folder() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri_of("Documents"));
        let address_bar = test.window.address_bar();
        let crumbs = address_bar.crumb_buttons();
        let parent = crumbs.iter().rev().nth(1).expect("the folder has a parent crumb");
        let (x, y) = middle_of(parent, address_bar);

        let spot = test
            .window
            .drop_spot(DropZone::Breadcrumbs, address_bar.upcast_ref(), x, y);
        test.window.show_drop_spot(DropZone::Breadcrumbs, spot.as_ref());
        let highlighted = parent.has_css_class("file-drop-active");
        test.window.leave_drop_zone(DropZone::Breadcrumbs);

        assert_eq!(
            spot.map(|spot| spot.destination()),
            Some(DropDestination::Folder(fixture.uri()))
        );
        assert!(highlighted);
        assert!(!parent.has_css_class("file-drop-active"));
    }

    /// parity: TAB-018, DND-016
    #[gtk::test]
    fn a_drop_on_a_tab_goes_into_its_folder_and_hovering_shows_the_tab() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        test.activate("previous-tab", None);
        // Showing a tab draws the strip's tabs anew.
        wait_for_frames(&test.window, 3);
        let strip = test.window.tab_strip();
        let second_tab = strip.tab_list().last_child().expect("two tabs");
        let (x, y) = middle_of(&second_tab, strip);

        let spot = test.window.drop_spot(DropZone::Tabs, strip.upcast_ref(), x, y);
        test.window.show_drop_spot(DropZone::Tabs, spot.as_ref());
        let is_highlighted = second_tab.has_css_class("file-drop-active");
        let before_the_delay = test.window.current_uri();
        wait_until("the hovered tab to show", || {
            test.window.current_uri() == Some(fixture.uri_of("Documents"))
        });
        test.window.leave_drop_zone(DropZone::Tabs);

        assert_eq!(
            spot.map(|spot| spot.destination()),
            Some(DropDestination::Folder(fixture.uri_of("Documents")))
        );
        assert!(is_highlighted);
        assert_eq!(
            before_the_delay,
            Some(fixture.uri()),
            "a tab shows only after the hover delay"
        );
    }

    /// parity: DND-020, DND-026
    #[gtk::test]
    fn items_dropped_on_a_program_are_given_to_it_as_arguments() {
        let fixture = Fixture::standard();
        install_copier(&fixture);
        let test = TestWindow::open(&fixture.uri());
        test.window.refresh();
        wait_until("the program to be listed", || {
            test.names().contains(&"copier".to_owned())
        });
        let copier = test.position_of("copier");

        let first_look = test.window.item_destination(copier);
        wait_until("GIO's answer", || test.window.item_destination(copier).is_some());
        let Some(DropDestination::Program(program)) = test.window.item_destination(copier) else {
            panic!("the copier is a program");
        };
        let spot = test.window.folder_view_spot(Some(copier));
        test.window.show_drop_spot(DropZone::FolderView, spot.as_ref());
        let hint = test.window.folder_pane().drag_hint();
        test.window.leave_drop_zone(DropZone::FolderView);
        let copy_name = "Notes 2 (dropped).txt";
        let dropped = vec![fixture.uri_of("Notes 2.txt"), fixture.uri_of(copy_name)];
        test.window.open_with_program(program, dropped);

        assert_eq!(first_look, None, "unknown until GIO answers");
        assert_eq!(hint.as_deref(), Some("Open with copier"));
        wait_until("the program to run", || fixture.path(copy_name).is_file());
        assert_eq!(test.window.folder_pane().drag_hint(), None);
    }

    /// parity: DND-026
    #[gtk::test]
    fn a_file_that_is_not_executable_is_no_program() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let notes = test.position_of("Notes 2.txt");

        test.window.item_destination(notes);
        wait_for(Duration::from_millis(200));

        assert_eq!(test.window.item_destination(notes), None);
    }

    /// With `OX_NATIVE_CAPTURE_DIR` set, saves the drop highlights (a
    /// folder row, the Quick access line and a crumb), the program hint
    /// and the drop menu in both themes; without it, proves they show.
    #[gtk::test]
    fn the_drop_highlights_the_program_hint_and_the_drop_menu_are_captured() {
        let _theme = ThemeGuard::keep();
        let source = Fixture::standard();
        let fixture = Fixture::standard();
        install_copier(&fixture);
        let test = TestWindow::open(&fixture.uri());
        let copier = test.position_of("copier");
        let sidebar = test.window.sidebar();
        for theme in ["light", "dark"] {
            test.activate("theme", Some(theme));
            // A new theme draws the sidebar's rows anew; a drag's next
            // motion would mark the new ones.
            wait_for_frames(&test.window, 3);
            let on_folder = test.window.folder_view_spot(Some(test.position_of("Documents")));
            test.window
                .show_drop_spot(DropZone::FolderView, on_folder.as_ref());
            let pin_line = test.window.sidebar_spot(sidebar.middle_of("Documents") - 5.0);
            test.window.show_drop_spot(DropZone::Sidebar, pin_line.as_ref());
            capture(&test.window, &format!("native-drop-targets-{theme}.png"));
            test.window.leave_drop_zone(DropZone::Sidebar);
            // Leaving the view forgot GIO's answers; a new drag asks again.
            wait_until("GIO's answer", || test.window.item_destination(copier).is_some());
            let on_program = test.window.folder_view_spot(Some(copier));
            test.window
                .show_drop_spot(DropZone::FolderView, on_program.as_ref());
            capture(&test.window, &format!("native-drop-program-{theme}.png"));
            test.window.leave_drop_zone(DropZone::FolderView);
            test.window
                .remember_drop_point(test.window.folder_pane().upcast_ref(), 300.0, 200.0);
            test.window
                .drop_files(&[source.uri_of("Notes 2.txt")], None, DropAction::Ask);
            let menu = test.window.drop_menu();
            wait_until("the drop menu", || menu.is_mapped());
            capture_popover(
                &test.window,
                menu.upcast_ref(),
                &format!("native-drop-menu-{theme}.png"),
            );
            menu.popdown();
            wait_until("the menu to close", || !menu.is_mapped());
        }
    }
}
