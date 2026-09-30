// SPDX-License-Identifier: AGPL-3.0-only
//! Keyboard input in the folder views: type-to-select, Escape, Enter and
//! the keyboard context menu.
//!
//! GTK has no public way to synthesise key events, so these tests emit
//! `key-pressed` on the views' capture-phase key controller, which is what
//! a real key press reaches first. Typed text arrives through the input
//! method, whose `commit` handler is [`BrowserWindow::type_text`].
//!
//! [`BrowserWindow::type_text`]: crate::window::BrowserWindow

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow, ThemeGuard};
use crate::window::menu_popover::MenuPopover;
use crate::window::quick_look::{PREVIEWER_INTERFACE, PREVIEWER_NAME, PREVIEWER_PATH};

/// Presses `key` in the details view, as far as the window's own key
/// handling goes. Returns true when the window handled the key itself.
fn press(test: &TestWindow, key: gdk::Key) -> bool {
    press_with(test, key, gdk::ModifierType::empty())
}

/// Presses `key` with `modifiers` held, as [`press`] does.
fn press_with(test: &TestWindow, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let view = test.window.folder_pane().details().column_view();
    let controller = view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the details view has a capture-phase key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &modifiers])
}

pub(super) fn hint(test: &TestWindow) -> String {
    let label = test.window.status_bar().typeahead_hint_label();
    label.text().to_string()
}

/// Whether the hint is drawn in the light palette's `hex` colour.
fn hint_is_drawn_in(test: &TestWindow, hex: &str) -> bool {
    let expected = gdk::RGBA::parse(hex).expect("a CSS colour");
    let drawn = test.window.status_bar().typeahead_hint_label().color();
    let channels = [
        (drawn.red(), expected.red()),
        (drawn.green(), expected.green()),
        (drawn.blue(), expected.blue()),
    ];
    channels.iter().all(|(a, b)| (a - b).abs() < 0.01)
}

/// parity: SEL-020, SEL-023, SEL-028
#[gtk::test]
fn typing_selects_the_next_matching_name_and_names_it_in_the_hint() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let _theme = ThemeGuard::keep();
    test.activate("theme", Some("light"));
    test.window.folder_pane().focus_view();
    test.window.type_text("n");
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert_eq!(hint(&test), "Jump to: n — Notes 2.txt");
    assert!(
        hint_is_drawn_in(&test, "#0067c0"),
        "a match is shown in the accent colour"
    );
    test.window.type_text("otes 1");
    assert_eq!(test.selected_names(), ["Notes 10.txt"]);
    assert!(
        test.window.folder_pane().view_has_focus(),
        "focus stays in the list"
    );
    let view = test.window.folder_pane().details().column_view();
    assert!(gtk::test_accessible_has_property(
        view,
        gtk::AccessibleProperty::Label
    ));
    wait_until("the hint to clear a second after the last key", || {
        hint(&test).is_empty()
    });
    assert_eq!(
        test.selected_names(),
        ["Notes 10.txt"],
        "the match stays selected"
    );
}

/// parity: SEL-034
#[gtk::test]
fn a_match_replaces_a_multi_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_all();
    test.window.type_text("r");
    assert_eq!(test.selected_names(), ["Résumé.txt"]);
}

/// parity: SEL-032
#[gtk::test]
fn a_match_below_the_visible_rows_is_scrolled_into_view() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();
    test.window.type_text("file 0250");
    assert_eq!(test.selected_names(), ["file 0250.txt"]);
    wait_until("the match to be scrolled to", || {
        test.window.folder_pane().scroll_position() > 0.0
    });
}

/// parity: SEL-030
#[gtk::test]
fn keys_typed_in_a_text_field_are_left_to_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(1);
    test.window.search_box().focus();
    assert!(!press(&test, gdk::Key::Escape), "the text field gets the key");
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// parity: SEL-025
#[gtk::test]
fn an_unmatched_prefix_keeps_the_selection_and_says_so() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let _theme = ThemeGuard::keep();
    test.activate("theme", Some("light"));
    test.window.type_text("d");
    test.window.type_text("zz");
    assert_eq!(test.selected_names(), ["Documents"]);
    assert_eq!(hint(&test), "No name starts with “dzz”");
    assert!(
        hint_is_drawn_in(&test, "#616161"),
        "a miss is muted (the light palette's ox_muted), not an error"
    );
}

/// parity: SEL-005, SEL-027
#[gtk::test]
fn escape_clears_the_typed_prefix_before_the_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.type_text("n");
    assert!(press(&test, gdk::Key::Escape), "Escape is handled by the window");
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt"],
        "the first Escape keeps the selection"
    );
    assert_eq!(hint(&test), "");
    press(&test, gdk::Key::Escape);
    assert!(
        test.selected_names().is_empty(),
        "the second Escape clears the selection"
    );
}

/// parity: NAV-004
#[gtk::test]
fn backspace_erases_a_typed_prefix_first_and_otherwise_goes_back() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the subfolder");
    test.window.navigate(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the folder again");
    test.window.type_text("no");

    assert!(press(&test, gdk::Key::BackSpace));
    assert_eq!(
        hint(&test),
        "Jump to: n — Notes 2.txt",
        "the prefix loses a letter"
    );
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    test.window.reset_typeahead();
    press(&test, gdk::Key::BackSpace);
    test.wait_for_listing("the folder before");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// parity: SEL-031
#[gtk::test]
fn leaving_the_view_starts_a_new_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();
    test.window.type_text("n");
    test.window.search_box().focus();
    assert_eq!(hint(&test), "", "the prefix ended with the focus");
    test.window.folder_pane().focus_view();
    test.window.type_text("r");
    assert_eq!(test.selected_names(), ["Résumé.txt"]);
    test.activate("view", Some("large"));
    assert_eq!(hint(&test), "", "changing the view ends the prefix");
}

/// parity: SEL-031
#[gtk::test]
fn navigation_keys_start_a_new_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.type_text("n");
    let handled = press(&test, gdk::Key::Down);
    assert!(!handled, "the view still moves the cursor");
    assert_eq!(hint(&test), "", "the prefix ended");
    test.window.type_text("r");
    assert_eq!(
        test.selected_names(),
        ["Résumé.txt"],
        "typing starts over instead of extending \"n\""
    );
}

/// parity: SEL-029
#[gtk::test]
fn modifier_keys_keep_the_typed_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.type_text("n");
    press(&test, gdk::Key::Shift_L);
    test.window.type_text("otes 1");
    assert_eq!(test.selected_names(), ["Notes 10.txt"]);
}

/// parity: SEL-029
#[gtk::test]
fn keys_with_ctrl_or_alt_never_start_a_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(0);
    for modifiers in [gdk::ModifierType::CONTROL_MASK, gdk::ModifierType::ALT_MASK] {
        let handled = press_with(&test, gdk::Key::r, modifiers);
        assert!(!handled, "{modifiers:?}+R goes on to the shortcuts");
        assert_eq!(
            test.window.folder_model().selected_positions(),
            [0],
            "{modifiers:?}"
        );
        assert!(hint(&test).is_empty(), "{modifiers:?}+R starts no prefix");
    }
}

/// parity: OPEN-002
#[gtk::test]
fn enter_opens_only_a_single_selected_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.folder_pane().details().column_view();
    let model = test.window.folder_model();
    model.select_only(1);
    model.selection().select_item(2, false);
    details.emit_by_name::<()>("activate", &[&2_u32]);
    wait_for(Duration::from_millis(200));
    assert!(
        test.context.recorded_launches().is_empty(),
        "several selected items open nothing"
    );
    model.select_only(1);
    details.emit_by_name::<()>("activate", &[&1_u32]);
    let notes = fixture.uri_of("Notes 2.txt");
    wait_until("the file to open", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), [notes]);
}

/// The triggers of the shortcuts `view` handles itself.
fn view_shortcuts(view: &impl IsA<gtk::Widget>) -> Vec<String> {
    let controllers = view.observe_controllers();
    let shortcut_controllers = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok());
    let mut triggers = Vec::new();
    for controller in shortcut_controllers {
        // A shortcut controller lists plain objects, all of them shortcuts.
        let shortcuts = controller
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|object| object.downcast::<gtk::Shortcut>().ok());
        let trigger_texts =
            shortcuts.filter_map(|shortcut| shortcut.trigger().map(|trigger| trigger.to_str()));
        triggers.extend(trigger_texts.map(|text| text.to_string()));
    }
    triggers
}

#[gtk::test]
fn the_menu_key_opens_the_context_menu_with_or_without_a_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.folder_pane().details().column_view();
    assert!(view_shortcuts(details).contains(&"Menu|<Shift>F10".to_owned()));
    let menus = descendants::<MenuPopover>(details);
    let [menu] = menus.as_slice() else {
        panic!("one context menu per view");
    };
    for selected in [Some(1), None] {
        match selected {
            Some(position) => test.window.folder_model().select_only(position),
            None => test.window.folder_model().select_none(),
        }
        test.activate("context-menu", None);
        assert!(menu.is_visible(), "the menu opens for the selection {selected:?}");
        menu.popdown();
    }
}

/// A stand-in for GNOME Sushi on the private session bus, recording the
/// calls it receives until it is dropped. It has Sushi's `Visible`
/// property and `SelectionEvent` signal.
struct FakePreviewer {
    bus: gio::DBusConnection,
    calls: Rc<RefCell<Vec<String>>>,
    visible: Rc<Cell<bool>>,
    owner: Option<gio::OwnerId>,
    registration: Option<gio::RegistrationId>,
}

impl FakePreviewer {
    fn start() -> Self {
        let xml = format!(
            "<node><interface name='{PREVIEWER_INTERFACE}'><method name='ShowFile'><arg type='s'/>\
             <arg type='s'/><arg type='b'/></method><method name='Close'/><signal name='SelectionEvent'>\
             <arg type='u'/></signal><property name='Visible' type='b' access='read'/></interface></node>"
        );
        let node = gio::DBusNodeInfo::for_xml(&xml).expect("valid introspection");
        let interface = node.lookup_interface(PREVIEWER_INTERFACE).expect("the interface");
        let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).expect("the private bus");
        let calls = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&calls);
        let visible = Rc::new(Cell::new(false));
        let shown = Rc::clone(&visible);
        let registration = bus
            .register_object(PREVIEWER_PATH, &interface)
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                let uri = parameters
                    .try_child_value(0)
                    .and_then(|uri| uri.str().map(str::to_owned));
                let call = uri.map_or_else(|| method.to_owned(), |uri| format!("{method} {uri}"));
                recorded.borrow_mut().push(call);
                invocation.return_value(None);
            })
            .property(move |_, _, _, _, _| shown.get().to_variant())
            .build()
            .expect("the object registers");
        let owner = gio::bus_own_name_on_connection(
            &bus,
            PREVIEWER_NAME,
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| {},
        );
        wait_until("the previewer's name", || name_has_owner(&bus));
        Self {
            bus,
            calls,
            visible,
            owner: Some(owner),
            registration: Some(registration),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    /// Sends `SelectionEvent`, as an arrow key pressed in Sushi's window
    /// does; `direction` is a `GtkDirectionType`.
    fn press_arrow(&self, direction: u32) {
        self.bus
            .emit_signal(
                None,
                PREVIEWER_PATH,
                PREVIEWER_INTERFACE,
                "SelectionEvent",
                Some(&(direction,).to_variant()),
            )
            .expect("the signal is sent");
    }

    /// Changes `Visible` and says so, as Sushi does when its window opens
    /// or closes.
    fn set_visible(&self, visible: bool) {
        self.visible.set(visible);
        let changed = glib::VariantDict::new(None);
        changed.insert_value("Visible", &visible.to_variant());
        let invalidated = Vec::<String>::new().to_variant();
        let arguments =
            glib::Variant::tuple_from_iter([PREVIEWER_INTERFACE.to_variant(), changed.end(), invalidated]);
        self.bus
            .emit_signal(
                None,
                PREVIEWER_PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(&arguments),
            )
            .expect("the signal is sent");
    }
}

impl Drop for FakePreviewer {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
        if let Some(registration) = self.registration.take() {
            // Dropping the test's bus connection unregisters it anyway.
            let _ = self.bus.unregister_object(registration);
        }
    }
}

/// Whether Sushi's name has an owner on `bus`.
fn name_has_owner(bus: &gio::DBusConnection) -> bool {
    let reply = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(PREVIEWER_NAME,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    );
    reply
        .ok()
        .and_then(|reply| reply.get::<(bool,)>())
        .is_some_and(|(owned,)| owned)
}

/// Space previews the selected file in GNOME's previewer, moving the
/// selection shows the next file, and Space closes it. The arrow keys in
/// the previewer move the selection, and once it closes itself the window
/// stops following the selection.
///
/// parity: PROP-012
#[gtk::test]
fn space_previews_the_selected_file_with_the_gnome_previewer() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let previewer = FakePreviewer::start();
    wait_until("the window sees the previewer", || {
        test.window.quick_look_is_available()
    });
    test.select_named("Notes 10.txt");

    assert!(press(&test, gdk::Key::space), "Space opens the preview");
    wait_until("ShowFile", || !previewer.calls().is_empty());
    test.select_named("Notes 2.txt");
    assert!(press(&test, gdk::Key::space), "Space closes the preview");
    wait_until("Close", || previewer.calls().len() == 3);

    let shown = |name: &str| format!("ShowFile {}", fixture.uri_of(name));
    assert_eq!(
        previewer.calls(),
        [shown("Notes 10.txt"), shown("Notes 2.txt"), "Close".to_owned()]
    );

    assert!(press(&test, gdk::Key::space), "Space opens it again");
    previewer.set_visible(true);
    let model = test.window.folder_model();
    let next = model.selected_positions()[0] + 1;
    let next_name = model.name_at(next).expect("a next item");
    previewer.press_arrow(3);
    wait_until("the next file shown", || previewer.calls().len() == 5);
    assert_eq!(model.selected_positions(), [next]);
    assert_eq!(previewer.calls()[4], shown(&next_name));

    previewer.set_visible(false);
    wait_until("the window lets go", || !test.window.quick_look_is_open());
    test.select_named("Notes 10.txt");
    assert!(press(&test, gdk::Key::space), "Space opens a new preview");
    wait_until("ShowFile", || previewer.calls().len() == 6);
    assert_eq!(previewer.calls()[5], shown("Notes 10.txt"));
}
