// SPDX-License-Identifier: AGPL-3.0-only
//! Every right-click menu of the app, opened with the real pointer on
//! check.py's private display in both styles, row by row: each row and
//! strip button, submenus included, runs an action the window or the
//! application has, with a target of the type that action takes. A row
//! whose action is missing or takes another target does nothing when
//! chosen, as a menu runs its action without asking (`activate_action`'s
//! result is ignored), so this is where such a row shows.
//!
//! The menus are those of one file, several files, a folder, a mixed
//! selection, an MP3, an MP4, a JPEG, an ODT, a ZIP, a 7z archive, a
//! file with an odd name and a 5 GB disk image; the background of a
//! folder; Home, its Quick access tiles and its Recent and Favorites
//! rows; Gallery; This PC and a drive; Network; the Recycle Bin and an
//! item in it; an item inside a ZIP and its background; search results;
//! every navigation pane row; the address bar; the column titles; and a
//! tab. The log lists each menu's rows, for comparing with Windows 11.

use std::fs;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{GALLERY_URI, HOME_URI, NETWORK_URI, PC_URI, TRASH_URI};
use ox_core::settings::ContextMenu;

use super::command_bar_switch::Session;
use super::context_menus::{choose_menu_style, position_of};
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture};
use crate::test_support::pointer::Button;
use crate::window::menu_popover::{MenuAction, MenuEntry, MenuItem, MenuPopover};
use crate::window::BrowserWindow;

/// What the sweep found wrong, for one failure that lists them all.
#[derive(Debug, Default)]
struct Problems(Vec<String>);

impl Problems {
    fn note(&mut self, problem: String) {
        eprintln!("FAIL: {problem}");
        self.0.push(problem);
    }

    fn assert_none(&self) {
        assert!(self.0.is_empty(), "{} problems: {:#?}", self.0.len(), self.0);
    }
}

/// Checks that `item`, in the menu called `menu`, runs an action `window`
/// or the application has, with a target of the type it takes; and its
/// submenu's rows the same.
fn check_item(window: &BrowserWindow, menu: &str, item: &MenuItem, problems: &mut Problems) {
    let (group, name): (gio::ActionMap, &str) = match item.action {
        MenuAction::Window(action) => (window.clone().upcast(), action.name()),
        MenuAction::Application(action) => (
            window
                .application()
                .expect("the window has its application")
                .upcast(),
            action.name(),
        ),
    };
    let label = &item.label;
    match group.lookup_action(name) {
        None => problems.note(format!("{menu} › {label}: no action {name}")),
        Some(action) => {
            let takes = action.parameter_type();
            let given = item.target.as_ref().map(glib::Variant::type_);
            // A row with a submenu opens it, and a grayed row runs
            // nothing: their own action does not run.
            let runs = item.submenu.is_empty()
                && item.availability != crate::window::menu_popover::ItemAvailability::Disabled;
            if runs && takes.as_deref() != given {
                problems.note(format!(
                    "{menu} › {label}: {name} takes {takes:?}, the row gives {given:?}"
                ));
            }
        }
    }
    for entry in &item.submenu {
        if let MenuEntry::Item(inner) = entry {
            check_item(window, &format!("{menu} › {label}"), inner, problems);
        }
    }
}

/// The menu of the window that is open now, once it has drawn.
pub(super) fn open_menu(window: &BrowserWindow) -> Option<MenuPopover> {
    let shown = || {
        descendants::<MenuPopover>(window)
            .into_iter()
            .find(WidgetExt::is_mapped)
    };
    for _ in 0..200 {
        if let Some(menu) = shown() {
            wait_for_frames(&menu, 2);
            return Some(menu);
        }
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(10));
    }
    None
}

/// The rows of `menu` as one line for the log: dividers as `|`, the
/// strip's buttons in brackets, grayed rows in parentheses.
pub(super) fn summary(menu: &MenuPopover) -> String {
    let mut parts = Vec::new();
    let strip = menu.strip_labels();
    if !strip.is_empty() {
        parts.push(format!("[{}]", strip.join(", ")));
    }
    let disabled: Vec<String> = menu
        .rows()
        .into_iter()
        .filter(|row| !row.is_sensitive())
        .filter_map(|row| crate::window::menu_popover::row_label(&row))
        .collect();
    for label in menu.row_labels() {
        if label == "-" {
            parts.push("|".to_owned());
        } else if disabled.contains(&label) {
            parts.push(format!("({label})"));
        } else {
            parts.push(label);
        }
    }
    parts.join(", ")
}

/// Right-clicks the screen point `at` with the real pointer and checks
/// the menu that opens, called `name`; then closes it.
fn sweep_at(session: &mut Session, name: &str, at: (i32, i32), problems: &mut Problems) {
    let window = session.test.window.clone();
    let mut menu = None;
    for attempt in 0..2 {
        glide_to(session, at);
        session.pointer.press(Button::Secondary);
        session.pointer.release(Button::Secondary);
        menu = open_menu(&window);
        if menu.is_some() {
            break;
        }
        eprintln!("RETRY {name}: right-click {attempt} at {at:?} opened no menu");
    }
    let Some(menu) = menu else {
        problems.note(format!("{name}: a right-click opened no menu"));
        return;
    };
    sweep_menu(&window, name, &menu, problems);
    close(session, &menu);
}

thread_local! {
    /// Where the sweep left the real pointer, so it glides on from there.
    static POINTER_AT: std::cell::Cell<Option<(i32, i32)>> = const { std::cell::Cell::new(None) };
}

/// Glides the real pointer to `at` from where the sweep left it, as a
/// hand moves it: jumped there in one step right after a menu's popup
/// left the screen under it, X11 gives GTK the next press nowhere it can
/// deliver it (a test artifact, see
/// `the_right_click_after_choosing_from_a_menu_opens_the_next_menu`).
fn glide_to(session: &mut Session, at: (i32, i32)) {
    match POINTER_AT.get() {
        Some(from) if from != at => session.pointer.glide(from, at, 8),
        _ => session.pointer.move_to(at.0, at.1),
    }
    POINTER_AT.set(Some(at));
}

/// Checks the rows and strip of the open `menu`, called `name`.
fn sweep_menu(window: &BrowserWindow, name: &str, menu: &MenuPopover, problems: &mut Problems) {
    eprintln!("MENU {name}: {}", summary(menu));
    for item in menu.items().iter().chain(menu.strip_items().iter()) {
        check_item(window, name, item, problems);
    }
}

/// Closes `menu` with a real Escape and waits until it has.
pub(super) fn close(session: &mut Session, menu: &MenuPopover) {
    session.pointer.tap_key(gtk::gdk::Key::Escape);
    if menu.is_mapped() {
        wait_for_frames(menu, 2);
    }
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());
    // GTK lets go of the menu's pointer grab after it has closed.
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(250));
}

/// The screen point on the item called `name` of the file list.
pub(super) fn on_item(session: &Session, name: &str) -> (i32, i32) {
    let position = position_of(&session.test, name);
    let (_, x, y) = session.test.window.right_button_on_item(position);
    session.on_screen(&session.test.window.folder_pane().input_widget(), (x, y))
}

/// The screen point on blank space at the bottom of the file list.
pub(super) fn on_blank(session: &Session) -> (i32, i32) {
    let view = session.test.window.folder_pane().input_widget();
    session.on_screen(&view, (6.0, f64::from(view.height()) - 6.0))
}

/// Selects the items called `names`, so a right-click on one of them
/// keeps them all.
fn select(session: &Session, names: &[&str]) {
    let model = session.test.window.folder_model();
    let positions: Vec<u32> = names
        .iter()
        .map(|name| position_of(&session.test, name))
        .collect();
    model.select_positions(&positions);
}

/// Both styles, compact first.
const STYLES: [ContextMenu; 2] = [ContextMenu::Win11, ContextMenu::Win10];

/// The file list's menus on every kind of item and selection, and on
/// blank space, in both styles.
///
/// parity: CMD-008, CMD-009, CMD-010, CMD-011
#[gtk::test]
fn every_file_list_menu_runs_actions_the_window_has() {
    let fixture = Fixture::real_world();
    let mut session = Session::open_on(fixture, None);
    let mut problems = Problems::default();
    let files = [
        "Quarterly report.pdf",
        "Holiday song.mp3",
        "Beach clip.mp4",
        "Sunset (2).jpg",
        "Meeting notes.odt",
        "Photos.zip",
        "Archive.7z",
        "Résumé [final] 🙂.txt",
        "-dash first.txt",
        "Backup disk.img",
        "Documents",
    ];
    for style in STYLES {
        choose_menu_style(&session.test, style);
        for name in files {
            let at = on_item(&session, name);
            sweep_at(&mut session, &format!("{style:?} {name}"), at, &mut problems);
        }
        for (what, names) in [
            (
                "several files",
                &["Holiday song.mp3", "Beach clip.mp4", "Quarterly report.pdf"][..],
            ),
            ("two files", &["Holiday song.mp3", "Beach clip.mp4"][..]),
            ("a folder and a file", &["Documents", "Quarterly report.pdf"][..]),
        ] {
            select(&session, names);
            let at = on_item(&session, names[0]);
            sweep_at(&mut session, &format!("{style:?} {what}"), at, &mut problems);
        }
        session.test.window.folder_model().select_none();
        let at = on_blank(&session);
        sweep_at(
            &mut session,
            &format!("{style:?} folder background"),
            at,
            &mut problems,
        );
    }
    problems.assert_none();
}

/// The menus of Home, Gallery, This PC, Network and the Recycle Bin, on
/// their items and blank space, in both styles.
///
/// parity: HOME-013, HOME-014, HOME-022, OPS-040, CMD-010
#[gtk::test]
fn every_page_menu_runs_actions_the_window_has() {
    let fixture = Fixture::real_world();
    let report = fixture.uri_of("Quarterly report.pdf");
    let mut session = Session::open_on(fixture, None);
    let mut problems = Problems::default();
    super::home_page::open_quick_access_on_home(&session.test, true);
    session.test.activate("add-to-favorites", Some(&report));
    // Something in the Recycle Bin.
    let position = position_of(&session.test, "Meeting notes.odt");
    session.test.window.folder_model().select_only(position);
    session.test.activate("trash", None);
    wait_until("the file to go", || {
        !session.test.names().contains(&"Meeting notes.odt".to_owned())
    });
    for style in STYLES {
        choose_menu_style(&session.test, style);
        for (page, uri) in [
            ("Home", HOME_URI),
            ("Gallery", GALLERY_URI),
            ("This PC", PC_URI),
            ("Network", NETWORK_URI),
            ("Recycle Bin", TRASH_URI),
        ] {
            session.test.activate("go-to", Some(uri));
            wait_until(&format!("{page} to show"), || {
                session.test.window.current_uri().as_deref() == Some(uri)
            });
            wait_for_frames(&session.test.window, 6);
            for (index, item) in page_items(&session).into_iter().enumerate().take(6) {
                let at = session.middle_of(&item);
                sweep_at(
                    &mut session,
                    &format!("{style:?} {page} item {index}"),
                    at,
                    &mut problems,
                );
            }
            let pane = session.test.window.folder_pane().clone();
            let at = session.on_screen(
                &pane,
                (f64::from(pane.width()) - 12.0, f64::from(pane.height()) - 12.0),
            );
            sweep_at(
                &mut session,
                &format!("{style:?} {page} background"),
                at,
                &mut problems,
            );
        }
    }
    problems.assert_none();
}

/// The items a page shows: Home's and This PC's tiles and rows, the
/// listed files elsewhere.
fn page_items(session: &Session) -> Vec<gtk::Widget> {
    let pane = session.test.window.folder_pane();
    let tiles = descendants::<gtk::FlowBoxChild>(pane)
        .into_iter()
        .filter(WidgetExt::is_drawable)
        .map(Cast::upcast);
    let rows = descendants::<gtk::ListBoxRow>(pane)
        .into_iter()
        .filter(WidgetExt::is_drawable)
        .filter(|row| row.ancestor(MenuPopover::static_type()).is_none())
        .map(Cast::upcast);
    let mut items: Vec<gtk::Widget> = tiles.chain(rows).collect();
    if items.is_empty() {
        let owners = session.test.window.folder_pane().owners();
        let model = session.test.window.folder_model();
        items.extend((0..model.n_items().min(3)).filter_map(|position| owners.widget_at(position)));
    }
    items
}

/// The menus inside a ZIP and of search results.
///
/// parity: ARC-026, SRCH-015
#[gtk::test]
fn the_zip_and_search_result_menus_run_actions_the_window_has() {
    let fixture = super::archives::fixture_with_zip();
    fs::write(fixture.path("Documents/deep notes.txt"), b"x").expect("fixture file");
    fs::write(fixture.path("Documents/deep song.mp3"), b"ID3").expect("fixture file");
    let root = fixture.uri();
    let mut session = Session::open_on(fixture, None);
    let mut problems = Problems::default();
    let choose = ox_core::settings::PreferencesUpdate {
        zip_opening: Some(ox_core::settings::ZipOpening::Folder),
        ..ox_core::settings::PreferencesUpdate::default()
    };
    session
        .test
        .context
        .update_preferences(choose, |result| result.expect("saved"));
    for style in STYLES {
        choose_menu_style(&session.test, style);
        session.test.activate("go-to", Some(&root));
        session.test.wait_for_listing("the folder");
        session.test.select_named("Bundle.zip");
        session.test.activate("open", None);
        wait_until("readme.txt", || {
            session.test.names().contains(&"readme.txt".to_owned())
        });
        let at = on_item(&session, "readme.txt");
        sweep_at(
            &mut session,
            &format!("{style:?} item inside a ZIP"),
            at,
            &mut problems,
        );
        session.test.window.folder_model().select_none();
        let at = on_blank(&session);
        sweep_at(
            &mut session,
            &format!("{style:?} inside a ZIP"),
            at,
            &mut problems,
        );
    }
    session.test.activate("go-to", Some(&root));
    session.test.wait_for_listing("the folder");
    session.test.start_search_cache();
    session.test.index_folder(&root);
    session.test.search_for("deep");
    for style in STYLES {
        choose_menu_style(&session.test, style);
        wait_until("the results", || session.test.names().len() == 2);
        session.test.window.folder_model().select_none();
        let at = on_item(&session, "deep notes.txt");
        sweep_at(
            &mut session,
            &format!("{style:?} search result"),
            at,
            &mut problems,
        );
        select(&session, &["deep notes.txt", "deep song.mp3"]);
        let at = on_item(&session, "deep notes.txt");
        sweep_at(
            &mut session,
            &format!("{style:?} two search results"),
            at,
            &mut problems,
        );
        session.test.window.folder_model().select_none();
        let at = on_blank(&session);
        sweep_at(
            &mut session,
            &format!("{style:?} search results background"),
            at,
            &mut problems,
        );
    }
    problems.assert_none();
}

/// The menus of every navigation pane row, the address bar, the column
/// titles and a tab.
///
/// parity: SIDE-014, NAV-031, VIEW-010, TAB-012
#[gtk::test]
fn the_pane_bar_and_tab_menus_run_actions_the_window_has() {
    let fixture = Fixture::real_world();
    let documents = fixture.uri_of("Documents");
    let mut session = Session::open_on(fixture, None);
    let mut problems = Problems::default();
    session.test.activate("pin-location", Some(&documents));
    let sidebar = session.test.window.sidebar().clone();
    wait_until("the pin", || sidebar.labels().contains(&"Documents".to_owned()));
    wait_for_frames(&session.test.window, 3);
    for style in STYLES {
        choose_menu_style(&session.test, style);
        let labels = sidebar.labels();
        for (index, label) in labels.iter().enumerate() {
            let Some(row) = i32::try_from(index)
                .ok()
                .and_then(|index| sidebar.list().row_at_index(index))
                .filter(WidgetExt::is_drawable)
            else {
                continue;
            };
            let at = session.middle_of(&row);
            sweep_at(
                &mut session,
                &format!("{style:?} nav pane {label}"),
                at,
                &mut problems,
            );
        }
        let address = session.test.window.address_bar().clone();
        let at = session.middle_of(&address);
        sweep_at(&mut session, &format!("{style:?} address bar"), at, &mut problems);
        let header = session
            .test
            .window
            .folder_pane()
            .details()
            .header()
            .expect("column titles");
        let at = session.on_screen(&header, (20.0, 4.0));
        sweep_at(
            &mut session,
            &format!("{style:?} column titles"),
            at,
            &mut problems,
        );
        assert!(session.test.window.open_tab_menu(1), "a tab");
        let window = session.test.window.clone();
        match open_menu(&window) {
            Some(menu) => {
                sweep_menu(&window, &format!("{style:?} tab"), &menu, &mut problems);
                close(&mut session, &menu);
            }
            None => problems.note(format!("{style:?} tab: no menu")),
        }
    }
    problems.assert_none();
}

/// A right-click after the last menu closed, by Escape or by a click
/// elsewhere, opens the next menu.
///
/// parity: CMD-008
#[gtk::test]
fn a_right_click_after_a_menu_closed_opens_the_next_menu() {
    let mut session = Session::open_on(Fixture::real_world(), None);
    choose_menu_style(&session.test, ContextMenu::Win11);
    let window = session.test.window.clone();
    let names = [
        "Quarterly report.pdf",
        "Holiday song.mp3",
        "Beach clip.mp4",
        "Sunset (2).jpg",
    ];
    for (round, name) in names.iter().enumerate() {
        let at = on_item(&session, name);
        session.pointer.move_to(at.0, at.1);
        session.pointer.press(Button::Secondary);
        session.pointer.release(Button::Secondary);
        let menu = open_menu(&window);
        eprintln!(
            "round {round} {name}: {}",
            menu.as_ref().map_or("no menu".to_owned(), summary)
        );
        assert!(
            menu.is_some(),
            "round {round}: the right-click on {name} opens its menu"
        );
        let menu = menu.expect("the menu");
        if round % 2 == 0 {
            session.pointer.tap_key(gtk::gdk::Key::Escape);
        } else {
            let blank = on_blank(&session);
            session.pointer.move_to(blank.0, blank.1);
            session.pointer.press(Button::Primary);
            session.pointer.release(Button::Primary);
        }
        wait_until("the menu to close", || !menu.is_mapped());
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    }
}

/// Ctrl+clicking files and right-clicking one of them opens the menu of
/// all of them at the first right-click.
///
/// parity: CMD-008, SEL-003
#[gtk::test]
fn the_first_right_click_on_a_ctrl_clicked_selection_opens_its_menu() {
    let mut session = Session::open_on(Fixture::real_world(), None);
    choose_menu_style(&session.test, ContextMenu::Win11);
    let window = session.test.window.clone();
    for round in 0..3 {
        let names = ["Holiday song.mp3", "Beach clip.mp4", "Quarterly report.pdf"];
        let first = on_item(&session, names[0]);
        session.click_at(first);
        for name in &names[1..] {
            let at = on_item(&session, name);
            session.pointer.move_to(at.0, at.1);
            session.pointer.hold_key(gtk::gdk::Key::Control_L);
            session.pointer.press(Button::Primary);
            session.pointer.release(Button::Primary);
            session.pointer.let_go_key(gtk::gdk::Key::Control_L);
        }
        wait_until("three selected", || session.test.selected_names().len() == 3);
        let at = on_item(&session, names[1]);
        session.pointer.move_to(at.0, at.1);
        session.pointer.press(Button::Secondary);
        session.pointer.release(Button::Secondary);
        let menu = open_menu(&window);
        assert!(
            menu.is_some(),
            "round {round}: the first right-click opens the menu"
        );
        assert_eq!(session.test.selected_names().len(), 3, "the selection stays");
        let menu = menu.expect("the menu");
        close(&mut session, &menu);
    }
}

/// The screen point of the middle of the navigation pane's row `label`.
pub(super) fn on_sidebar_row(session: &Session, label: &str) -> (i32, i32) {
    let sidebar = session.test.window.sidebar();
    let index = sidebar
        .labels()
        .iter()
        .position(|shown| shown == label)
        .unwrap_or_else(|| panic!("the pane shows {label}"));
    let row = i32::try_from(index)
        .ok()
        .and_then(|index| sidebar.list().row_at_index(index))
        .expect("a row");
    session.middle_of(&row)
}

/// The locations of the window's tabs, in order.
pub(super) fn tab_locations(session: &Session) -> Vec<String> {
    session
        .test
        .window
        .imp()
        .session
        .borrow()
        .tabs()
        .iter()
        .map(|tab| tab.uri().to_owned())
        .collect()
}

/// Home's and Gallery's rows in the navigation pane open in a new tab or
/// window from their menu, as This PC's, Network's and Windows 11's do.
///
/// parity: SIDE-010, TAB-027, TAB-028
#[gtk::test]
fn home_and_gallery_open_in_a_new_tab_or_window_from_the_navigation_pane() {
    use crate::test_support::harness::OpenedWindows;
    let mut session = Session::open_on(Fixture::real_world(), None);
    let window = session.test.window.clone();
    let mut last = session.middle_of(&session.test.window.address_bar().clone());
    for style in STYLES {
        choose_menu_style(&session.test, style);
        // Home opens the home folder unless the Home page is chosen.
        let home = glib::filename_to_uri(glib::home_dir(), None).expect("the home folder's URI");
        for (label, uri) in [("Home", home.as_str()), ("Gallery", GALLERY_URI)] {
            let tabs = tab_locations(&session).len();
            let at = on_sidebar_row(&session, label);
            session.pointer.glide(last, at, 8);
            session.pointer.press(Button::Secondary);
            session.pointer.release(Button::Secondary);
            let menu = open_menu(&window);
            let menu = menu.unwrap_or_else(|| panic!("{label}'s menu"));
            let rows = menu.row_labels();
            assert!(
                rows.contains(&"Open in new tab".to_owned())
                    && rows.contains(&"Open in new window".to_owned()),
                "{style:?} {label}: {rows:?}"
            );
            let row = menu.row("Open in new tab");
            last = session.middle_of(&row);
            session.click_at(last);
            wait_until(&format!("{label} in a new tab"), || {
                tab_locations(&session).len() == tabs + 1
            });
            assert!(
                tab_locations(&session).iter().any(|tab| tab == uri),
                "{label}: {:?}",
                tab_locations(&session)
            );
        }
    }
    let at = on_sidebar_row(&session, "Gallery");
    session.pointer.glide(last, at, 8);
    session.pointer.press(Button::Secondary);
    session.pointer.release(Button::Secondary);
    let menu = open_menu(&window).expect("Gallery's menu");
    let at = session.middle_of(&menu.row("Open in new window"));
    session.click_at(at);
    let opened = OpenedWindows::only(&[&session.test.window]);
    wait_until("Gallery in a new window", || {
        opened.window().current_uri().as_deref() == Some(GALLERY_URI)
    });
}

/// After a row of a menu is chosen with a click, the next right-click
/// opens the next menu at once, in the file list and in the navigation
/// pane, whichever menu was chosen from. The pointer glides there, as a
/// hand moves it: jumped there in one step, with no motion after the
/// menu's popup left the screen under it, X11 gives GTK the next press
/// nowhere it can deliver it.
///
/// parity: CMD-008, SIDE-014
#[gtk::test]
fn the_right_click_after_choosing_from_a_menu_opens_the_next_menu() {
    let mut session = Session::open_on(Fixture::real_world(), None);
    choose_menu_style(&session.test, ContextMenu::Win11);
    let window = session.test.window.clone();
    let mut misses = Vec::new();
    // (where the menu opens, the row chosen, where the next right-click is)
    let rounds: [(&str, &str, &str); 6] = [
        (
            "file:Quarterly report.pdf",
            "Copy as path",
            "file:Holiday song.mp3",
        ),
        ("file:Holiday song.mp3", "Copy as path", "pane:Downloads"),
        ("pane:Downloads", "Copy as path", "pane:Music"),
        ("pane:Music", "Copy as path", "file:Beach clip.mp4"),
        ("pane:Music", "Open in new tab", "pane:Pictures"),
        ("pane:Pictures", "Copy as path", "pane:Videos"),
    ];
    for (round, (from, row, next)) in rounds.into_iter().enumerate() {
        let point = |session: &Session, place: &str| match place.split_once(':') {
            Some(("file", name)) => on_item(session, name),
            Some(("pane", label)) => on_sidebar_row(session, label),
            _ => unreachable!(),
        };
        let at = point(&session, from);
        session.pointer.move_to(at.0, at.1);
        session.pointer.press(Button::Secondary);
        session.pointer.release(Button::Secondary);
        let menu = open_menu(&window).unwrap_or_else(|| panic!("round {round}: {from}'s menu"));
        let clicked = session.middle_of(&menu.row(row));
        session.click_at(clicked);
        wait_until("the menu to close", || !menu.is_mapped());
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
        let at = point(&session, next);
        session.pointer.glide(clicked, at, 8);
        session.pointer.press(Button::Secondary);
        session.pointer.release(Button::Secondary);
        if let Some(menu) = open_menu(&window) {
            close(&mut session, &menu);
        } else {
            eprintln!("MISS round {round}: {from} › {row}, then {next}");
            misses.push(format!("{from} › {row}, then {next}"));
            // The second right-click, to go on.
            session.pointer.press(Button::Secondary);
            session.pointer.release(Button::Secondary);
            if let Some(menu) = open_menu(&window) {
                close(&mut session, &menu);
            }
        }
    }
    assert!(
        misses.is_empty(),
        "the first right-click opened nothing: {misses:#?}"
    );
}
