// SPDX-License-Identifier: AGPL-3.0-only
//! Where the keyboard goes around the folder list: typing to select works
//! after the user clicks a place, a command, a crumb or the Details pane, not
//! only while the list itself has the keyboard, and no page switch or address
//! edit leaves it on a control GTK picked (such as the Details pane's close
//! button).
//!
//! GTK has no public way to synthesise input, so these tests emit the
//! `pressed` and `key-pressed` signals of the controllers a real click or key
//! reaches: the sidebar's rows, the window's own bubble-phase key handling and
//! the views' capture-phase one.

use std::path::PathBuf;

use gtk::gdk;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::file_uri;
use ox_core::places::FolderLocations;

use crate::folder_view::grid::IconSize;

use crate::test_support::harness::{
    capture, descendants, settle, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::address_bar::AddressMode;
use crate::window::tab_order::Stop;

use super::narrow_windows::resize;

/// A home with the standard folders the sidebar lists, so a row of it opens a
/// folder the test filled: the test home of the harness is the machine's
/// temporary one, whose folders the test does not set up.
struct TestHome {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
}

impl TestHome {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room");
        let base = std::fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        std::fs::create_dir_all(home.join("Documents")).expect("a folder");
        std::fs::create_dir_all(&config).expect("a folder");
        std::fs::write(home.join("Documents").join("Notes 2.txt"), "x").expect("a file");
        Self {
            _root: root,
            home,
            config,
        }
    }

    fn locations(&self) -> FolderLocations {
        FolderLocations::new(self.home.clone(), &self.config)
    }

    /// The URI of `name` in the home.
    fn uri_of(&self, name: &str) -> String {
        file_uri(&self.home.join(name))
    }
}

/// The sidebar row whose name is `label`.
fn place_row(test: &TestWindow, label: &str) -> gtk::ListBoxRow {
    let rows = descendants::<gtk::ListBoxRow>(test.window.sidebar().list());
    rows.into_iter()
        .find(|row| {
            descendants::<gtk::Label>(row)
                .into_iter()
                .any(|text| text.text() == label)
        })
        .unwrap_or_else(|| panic!("the sidebar lists {label}"))
}

/// Presses `key` as GTK delivers it to the window's own bubble-phase key
/// handling, which runs after the focused widget had its turn. Returns true
/// when the window took the key.
fn press_after_focus(test: &TestWindow, key: gdk::Key) -> bool {
    let no_keycode = 0_u32;
    let none = gdk::ModifierType::empty();
    let controllers: Vec<gtk::EventControllerKey> = test
        .window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .filter(|controller| controller.propagation_phase() == gtk::PropagationPhase::Bubble)
        .collect();
    controllers.iter().any(|controller| {
        controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &none])
    })
}

/// The widget holding the keyboard, if any.
fn focused(test: &TestWindow) -> Option<gtk::Widget> {
    GtkWindowExt::focus(&test.window)
}

/// The Details pane's Properties button, a control of the pane that can
/// hold the keyboard.
fn details_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(test.window.details_pane())
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.properties"))
        .expect("the Details pane has a Properties button")
}

/// The Back button of the navigation row.
fn back_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(&*test.window.imp().navigation_buttons)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.back"))
        .expect("the navigation row shows Back")
}

/// The Refresh button of the navigation row, which is always enabled.
fn refresh_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(&*test.window.imp().navigation_buttons)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.refresh"))
        .expect("the navigation row shows Refresh")
}

/// parity: SIDE-002, SEL-020
#[gtk::test]
fn clicking_a_place_puts_the_keyboard_in_the_folder() {
    let home = TestHome::new();
    let folders = home.locations();
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), folders, |_| {});
    test.wait_for_listing("the home folder");
    let documents = place_row(&test, "Documents");

    documents.grab_focus();
    documents.activate();
    test.wait_for_listing("the place");
    settle();

    assert_eq!(
        test.window.current_uri(),
        Some(home.uri_of("Documents")),
        "the row opened its folder"
    );
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the keyboard is in the list, not on the row clicked"
    );
    test.window.type_text("n");
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// parity: SEL-020
#[gtk::test]
fn a_typed_address_leaves_the_keyboard_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let path = fixture.path("Documents").display().to_string();

    test.activate("location", None);
    let entry = test.window.address_bar().entry();
    entry.set_text(&path);
    entry.emit_by_name::<()>("activate", &[]);
    wait_until("the typed folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    test.wait_for_listing("Documents");
    settle();

    assert!(
        test.window.folder_pane().view_has_focus(),
        "the keyboard followed the address into the list"
    );
}

/// parity: SEL-020
#[gtk::test]
fn ending_address_editing_moves_the_keyboard_out_of_the_entry_first() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");

    test.activate("location", None);
    assert_eq!(
        focused(&test)
            .map(|focus| focus.type_().name().to_string())
            .as_deref(),
        Some("GtkText"),
        "the address entry has the keyboard"
    );

    test.window.finish_address();

    assert_eq!(
        test.window.address_bar().mode(),
        crate::window::address_bar::AddressMode::Crumbs
    );
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the entry hands the keyboard to the list, not to the next control"
    );
}

/// parity: SEL-020, SEL-030
#[gtk::test]
fn typing_on_a_command_or_the_details_pane_selects_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.open_details_pane();
    let refresh = refresh_button(&test);
    let properties = details_button(&test);

    for button in [refresh, properties] {
        // The pane shows Properties only with something selected.
        test.select_named("Documents");
        wait_until("the control shown", || button.is_drawable());
        assert!(button.grab_focus(), "the control takes the keyboard");
        assert!(!test.window.folder_pane().view_has_focus());

        assert!(
            press_after_focus(&test, gdk::Key::n),
            "the window takes the typed letter"
        );

        assert_eq!(
            test.selected_names(),
            ["Notes 2.txt"],
            "the letter selected the name it starts"
        );
        assert!(
            test.window.folder_pane().view_has_focus(),
            "the keyboard moved from the control into the list"
        );
    }
}

/// parity: SEL-020
#[gtk::test]
fn a_landing_page_never_keeps_the_keyboard_on_the_hidden_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");

    test.window.navigate("This PC").expect("a page");
    settle();

    assert!(!test.window.folder_pane().view_has_focus());
    let focus = focused(&test).expect("a landing page card takes the keyboard");
    assert!(
        focus.is_ancestor(test.window.folder_pane().landing()),
        "the keyboard is on the page shown, not on the list it replaced"
    );
}

/// parity: NAV-002
#[gtk::test]
fn going_back_puts_the_keyboard_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    let back = back_button(&test);
    back.grab_focus();

    back.emit_clicked();

    wait_until("the folder left", || {
        test.window.current_uri() == Some(fixture.uri())
    });
    test.wait_for_listing("the first folder");
    settle();
    assert!(
        test.window.folder_pane().view_has_focus(),
        "Back returns the keyboard to the list"
    );
}

/// parity: SEL-020
#[gtk::test]
fn switching_views_keeps_the_keyboard_in_the_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let notes = test.position_of("Notes 2.txt");
    test.window.folder_pane().focus_view();
    test.window.folder_model().select_only(notes);
    test.window.folder_pane().focus_item(notes);
    assert!(test.window.folder_pane().view_has_focus());

    test.activate("view", Some("large"));
    crate::test_support::harness::wait_for_frames(&test.window, 3);

    assert!(
        test.window.folder_pane().view_has_focus(),
        "the view that replaces the list takes the keyboard"
    );
    assert_eq!(test.selected_names(), ["Notes 2.txt"], "the selection stays");
}

/// parity: VIEW-027
#[gtk::test]
fn closing_the_details_pane_puts_the_keyboard_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.open_details_pane();
    test.select_named("Notes 2.txt");
    let button = details_button(&test);
    wait_until("the Properties button", || button.is_drawable());
    assert!(
        button.grab_focus(),
        "the pane's Properties button takes the keyboard"
    );

    test.activate("details-pane", None);

    wait_until("the pane to hide", || !test.window.details_pane().is_visible());
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the folder takes the keyboard the hidden pane had"
    );
}

/// parity: SEL-020
#[gtk::test]
fn switching_views_on_the_unavailable_page_keeps_try_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading()
            && test.window.folder_pane().page() == Some(crate::window::folder_pane::PanePage::Empty)
    });
    let retry = descendants::<gtk::Button>(&test.window.folder_pane().empty_area())
        .into_iter()
        .find(WidgetExt::is_visible)
        .expect("Try again shows");
    retry.grab_focus();

    test.window
        .show_view(crate::window::FolderView::Icons(IconSize::LARGE));

    assert!(
        retry.has_focus(),
        "switching the hidden list leaves the page's own control alone"
    );
}

/// A letter no row of an open menu takes stays with the menu: typing on the
/// window's chrome goes to the folder, typing in its menus does not.
///
/// parity: SEL-030
#[gtk::test]
fn typing_in_a_menu_leaves_the_folder_alone() {
    let fixture = Fixture::standard();
    let test = super::geometry::laid_out(&fixture.uri());
    test.wait_for_listing("the first folder");
    let view = super::command_bar::menu_of(&test, "View");
    view.popup();
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    test.window.folder_pane().model().select_none();
    assert!(view.row("Terminal").grab_focus(), "a menu row takes the keyboard");

    assert!(
        !press_after_focus(&test, gdk::Key::n),
        "the window leaves the letter to the menu"
    );
    assert!(view.is_visible(), "the menu stays open");
    assert!(
        test.selected_names().is_empty(),
        "nothing was typed into the folder"
    );
    view.popdown();
}

/// A letter typed on a tab stays with the tab strip.
///
/// parity: SEL-030
#[gtk::test]
fn typing_on_a_tab_leaves_the_folder_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.window.folder_pane().model().select_none();
    let tab = descendants::<gtk::Widget>(test.window.tab_strip())
        .into_iter()
        .find(|widget| widget.is_focusable() && widget.is_visible())
        .expect("a tab takes the keyboard");
    assert!(tab.grab_focus(), "the tab takes the keyboard");

    assert!(
        !press_after_focus(&test, gdk::Key::n),
        "the window leaves the letter to the tab strip"
    );
    assert!(
        test.selected_names().is_empty(),
        "nothing was typed into the folder"
    );
}

/// While "Working on it..." hides the previous folder's rows, a letter typed
/// on a command selects none of them.
///
/// parity: SEL-030
#[gtk::test]
fn typing_while_a_location_opens_selects_nothing_hidden() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.window.folder_pane().model().select_none();
    test.window.folder_pane().show_working_note(true);
    let refresh = refresh_button(&test);
    assert!(refresh.grab_focus(), "the command takes the keyboard");

    assert!(
        !press_after_focus(&test, gdk::Key::n),
        "the window leaves the letter alone"
    );
    assert!(test.selected_names().is_empty(), "no hidden row was selected");
}

/// What the window draws now, in device pixels, from its outer edge.
struct Picture {
    pixels: Vec<u8>,
    stride: usize,
    width: usize,
    height: usize,
    /// Device pixels per logical pixel.
    scale: f32,
    /// The window's outer edge in its own coordinates.
    origin: (f32, f32),
}

impl Picture {
    fn of(window: &gtk::Window) -> Self {
        let native = window.upcast_ref::<gtk::Native>();
        #[expect(clippy::cast_precision_loss, reason = "scale factors are small integers")]
        let scale = native.scale_factor() as f32;
        let paintable = gtk::WidgetPaintable::new(Some(native));
        let snapshot = gtk::Snapshot::new();
        snapshot.scale(scale, scale);
        paintable.snapshot(
            &snapshot,
            f64::from(paintable.intrinsic_width()),
            f64::from(paintable.intrinsic_height()),
        );
        let node = snapshot.to_node().expect("a shown window draws");
        let renderer = native.renderer().expect("a shown window has a renderer");
        let texture = renderer.render_texture(&node, None);
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let outer = window.compute_bounds(window).expect("a shown window has bounds");
        Self {
            pixels: bytes.to_vec(),
            stride,
            width: usize::try_from(texture.width()).expect("a width"),
            height: usize::try_from(texture.height()).expect("a height"),
            scale,
            origin: (outer.x(), outer.y()),
        }
    }

    /// How many device pixels of `widget`'s box differ between the two
    /// pictures.
    fn changed_in(&self, other: &Self, widget: &gtk::Widget, window: &gtk::Window) -> usize {
        let bounds = widget.compute_bounds(window).expect("a shown widget has bounds");
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "window measures are small and positive"
        )]
        let device = |logical: f32, origin: f32| ((logical - origin) * self.scale).max(0.0) as usize;
        let left = device(bounds.x(), self.origin.0);
        let top = device(bounds.y(), self.origin.1);
        let right = device(bounds.x() + bounds.width(), self.origin.0).min(self.width);
        let bottom = device(bounds.y() + bounds.height(), self.origin.1).min(self.height);
        (top..bottom)
            .flat_map(|y| (left..right).map(move |x| y * self.stride + x * 4))
            .filter(|at| self.pixels[*at..*at + 4] != other.pixels[*at..*at + 4])
            .count()
    }
}

/// How long a control's focus ring may take to settle after a key.
const RING_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

/// Device pixels a focus ring changes at the least: a 2 px ring round
/// the smallest stop, a 20 px chevron, changes far more.
const RING_PIXELS: usize = 40;

/// One press of Tab (Shift+Tab `backward`) on the private display's real
/// keyboard: the widget it focused, which must draw its focus.
fn tab_and_see_the_ring(test: &TestWindow, pointer: &mut Pointer, backward: bool) -> gtk::Widget {
    let window = test.window.upcast_ref::<gtk::Window>();
    let before = Picture::of(window);
    if backward {
        pointer.hold_key(gdk::Key::Shift_L);
    }
    pointer.tap_key(gdk::Key::Tab);
    if backward {
        pointer.let_go_key(gdk::Key::Shift_L);
    }
    wait_for(RING_SETTLE);
    wait_for_frames(window, 2);
    let focus = focused(test).expect("Tab leaves the keyboard on a control");
    assert!(window.gets_focus_visible(), "Tab shows where the keyboard is");
    let after = Picture::of(window);
    let changed = after.changed_in(&before, &focus, window);
    assert!(
        changed >= RING_PIXELS,
        "{} with {:?} draws its focus: {changed} pixels changed",
        focus.type_().name(),
        focus.css_classes()
    );
    focus
}

/// Where the stops are, by name, for a failure and the captures.
fn stop_name(stop: Option<Stop>) -> String {
    let name = stop.map_or_else(|| "none".to_owned(), |stop| format!("{stop:?}").to_lowercase());
    name.replace('(', "-").replace(')', "")
}

/// The user's steps on Windows' order: open OX-Test > Template test >
/// Inside A, click blank space in the address bar, press Esc, then press
/// Tab. Esc leaves the keyboard in the file list, on its current item, and
/// Tab goes on from there as in Windows File Explorer: the status bar's
/// view buttons, the tab in front and its Close button, New tab, the navigation buttons, then
/// the address bar (the chevron first, each crumb, the history chevron),
/// the search box, the command bar, the navigation pane as one stop, Map
/// network drive and back to the file list. Every stop draws its focus,
/// light and dark: no press of Tab leaves the keyboard where nothing
/// shows it. In a narrow window the « takes the chevron's place as the
/// address bar's first stop. The keys are real ones on the private
/// display, so GTK's own key handling decides what each one does.
///
/// parity: ACC-016, NAV-018, NAV-023
#[gtk::test]
fn tab_after_escaping_the_address_goes_round_in_explorers_order_with_every_stop_shown() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::empty();
    let inside = fixture.path("OX-Test").join("Template test").join("Inside A");
    std::fs::create_dir_all(&inside).expect("the test folders");
    std::fs::write(inside.join("Plan.txt"), "x").expect("a file");
    for theme in ["light", "dark"] {
        let test = TestWindow::open(&file_uri(&inside));
        test.activate("theme", Some(theme));
        test.wait_for_listing("Inside A");
        test.window.present();
        wait_for_frames(&test.window, 3);
        let mut pointer = Pointer::connect();
        walk_the_tab_order(&test, &mut pointer, theme);
    }
}

/// A point of `address` with no button under it, right of the crumbs: the
/// empty spot the user clicked.
fn blank_spot(address: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let address = address.upcast_ref::<gtk::Widget>();
    let middle = f64::from(address.height()) / 2.0;
    let on_a_button = |x: f64| {
        let picked = address.pick(x, middle, gtk::PickFlags::DEFAULT);
        std::iter::successors(picked, WidgetExt::parent)
            .take_while(|widget| widget != address)
            .any(|widget| widget.is::<gtk::Button>())
    };
    (1..address.width() / 2)
        .map(|step| f64::from(address.width() - 4 * step))
        .find(|x| !on_a_button(*x) && !on_a_button(*x - 8.0) && !on_a_button(*x + 8.0))
        .map(|x| (x, middle))
        .expect("the address bar has blank space")
}

fn walk_the_tab_order(test: &TestWindow, pointer: &mut Pointer, theme: &str) {
    let address = test.window.address_bar();
    let blank = blank_spot(address);
    let (x, y) = screen_point(pointer, address, blank, (400, 70));
    pointer.move_to(x, y);
    // The window settles under the pointer before the click.
    wait_for(RING_SETTLE);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    wait_until("the address to be edited", || {
        address.mode() == AddressMode::Entry
    });
    pointer.tap_key(gdk::Key::Escape);
    wait_until("the crumbs back", || address.mode() == AddressMode::Crumbs);
    // The pointer rests on the status bar's count, away from every stop.
    let count = (12.0, f64::from(test.window.status_bar().height()) / 2.0);
    let (x, y) = screen_point(pointer, test.window.status_bar(), count, (40, 780));
    pointer.move_to(x, y);
    wait_for(RING_SETTLE);
    let start = Stop::Pane(test.window.active_side());
    assert_eq!(
        test.window.focused_stop(),
        Some(start),
        "Esc leaves the keyboard in the file list"
    );
    assert!(test.window.folder_pane().view_has_focus());
    capture(&test.window, &format!("address-tab-fix-{theme}-00-escape.png"));

    let mut visited: Vec<(Stop, gtk::Widget)> = Vec::new();
    for press in 1..=60 {
        let focus = tab_and_see_the_ring(test, pointer, false);
        let stop = test.window.focused_stop();
        capture(
            &test.window,
            &format!("address-tab-fix-{theme}-{press:02}-{}.png", stop_name(stop)),
        );
        if stop == Some(start) {
            break;
        }
        visited.push((stop.unwrap_or_else(|| panic!("{focus:?} is in no stop")), focus));
    }
    let mut stops: Vec<Stop> = visited.iter().map(|(stop, _)| *stop).collect();
    let presses: Vec<(Stop, usize)> = {
        let mut counted = Vec::new();
        for stop in &stops {
            match counted.last_mut() {
                Some((last, count)) if last == stop => *count += 1,
                _ => counted.push((*stop, 1)),
            }
        }
        counted
    };
    stops.dedup();
    assert_eq!(
        stops,
        [
            Stop::Status,
            Stop::Tabs,
            Stop::NewTab,
            Stop::Navigation,
            Stop::Address,
            Stop::Search,
            Stop::Commands,
            Stop::Places,
            Stop::MapNetworkDrive,
        ],
        "Windows' order from the file list round to it"
    );
    assert!(
        presses.contains(&(Stop::Places, 1)),
        "the navigation pane is one stop: {presses:?}"
    );

    let first_in_tabs = visited.iter().find(|(stop, _)| *stop == Stop::Tabs);
    assert!(
        first_in_tabs.is_some_and(|(_, widget)| widget.has_css_class("tab")),
        "the tab in front is the strip's stop, before its Close button"
    );

    address_stops_follow_the_crumbs(test, &visited);
    the_shift_tab_and_the_overflow_stops(test, pointer, theme);
}

/// The address bar's stops among `visited`: the chevron, each crumb, then
/// the history chevron.
fn address_stops_follow_the_crumbs(test: &TestWindow, visited: &[(Stop, gtk::Widget)]) {
    let address = test.window.address_bar();
    let in_address: Vec<gtk::Widget> = visited
        .iter()
        .filter(|(stop, _)| *stop == Stop::Address)
        .map(|(_, widget)| widget.clone())
        .collect();
    let (chevron, _) = address.strip_buttons();
    let crumbs: Vec<gtk::Widget> = address.crumb_buttons().into_iter().map(Cast::upcast).collect();
    assert_eq!(
        in_address.first(),
        Some(chevron.upcast_ref()),
        "the chevron is the first stop"
    );
    assert_eq!(
        &in_address[1..=crumbs.len()],
        crumbs.as_slice(),
        "then every crumb"
    );
    assert!(
        in_address
            .last()
            .is_some_and(|last| last.has_css_class("address-chevron")),
        "then the history chevron"
    );
}

/// Shift+Tab from the chevron goes back to the navigation buttons; in a
/// narrow window the « is the address bar's first stop, and shows it.
fn the_shift_tab_and_the_overflow_stops(test: &TestWindow, pointer: &mut Pointer, theme: &str) {
    let address = test.window.address_bar();
    let (chevron, _) = address.strip_buttons();
    // Shift+Tab goes back: from the chevron to the navigation buttons.
    while test.window.focused_stop() != Some(Stop::Address) {
        tab_and_see_the_ring(test, pointer, false);
    }
    assert_eq!(focused(test).as_ref(), Some(chevron.upcast_ref()));
    tab_and_see_the_ring(test, pointer, true);
    assert_eq!(test.window.focused_stop(), Some(Stop::Navigation));

    // Narrow, the « is the first stop of the address bar, and shows it.
    resize(test, 760);
    let (_, overflow) = address.strip_buttons();
    wait_until("the first crumbs to collapse", || {
        overflow.is_mapped() && !address.hidden_crumbs().is_empty()
    });
    let focus = tab_and_see_the_ring(test, pointer, false);
    assert_eq!(
        focus,
        overflow.clone().upcast::<gtk::Widget>(),
        "the « is the first stop"
    );
    capture(
        &test.window,
        &format!("address-tab-fix-{theme}-narrow-overflow.png"),
    );
    let next = tab_and_see_the_ring(test, pointer, false);
    let first_shown = address.crumb_buttons()[address.hidden_crumbs().len()].clone();
    assert_eq!(
        next,
        first_shown.upcast::<gtk::Widget>(),
        "then the first crumb shown"
    );
}

/// After Esc in an empty folder the keyboard rests on the empty list,
/// which has no item to ring: "This folder is empty." takes the items'
/// ring instead, for the keyboard only.
///
/// parity: ACC-016, ACC-009
#[gtk::test]
fn the_keyboard_on_an_empty_list_rings_its_note() {
    let fixture = Fixture::empty();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the empty folder");
    let pane = test.window.folder_pane();
    let ringed = || {
        pane.parts_overlay()
            .is_some_and(|overlay| overlay.has_css_class("keyboard-on-empty-list"))
    };
    assert!(pane.shows_empty_note());

    test.window.set_focus_visible(false);
    pane.focus_view();
    settle();
    assert!(pane.view_has_focus());
    assert!(!ringed(), "a click shows no ring");

    test.window.set_focus_visible(true);
    settle();
    assert!(ringed(), "the keyboard rings the note");
    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        capture(&test.window, &format!("address-tab-fix-{theme}-empty-list.png"));
    }

    test.window
        .address_bar()
        .child_focus(gtk::DirectionType::TabForward);
    settle();
    assert!(!ringed(), "the ring goes with the keyboard");
}
