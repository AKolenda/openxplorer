// SPDX-License-Identifier: AGPL-3.0-only
//! Using the address bar: the crumbs' clicks, keys and wheel, editing
//! the address and submitting it, against `renderNavigation`,
//! `editAddress`, `finishAddress` and `submitAddress` in
//! `desktop/ui/app.js`.
//!
//! GTK has no public way to synthesise input events, so these tests emit
//! the signals of the controllers a real click, key or wheel reaches.

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::archives::{archive_browser, fixture_with_zip};
use crate::locations::Page;
use crate::test_support::harness::{application, descendants, wait_for, wait_until, Fixture, TestWindow};
use crate::window::address_bar::AddressMode;
use crate::window::menu_popover::MenuPopover;

/// The controllers of type `T` that `widget` has.
fn controllers<T: IsA<gtk::EventController> + IsA<glib::Object>>(widget: &impl IsA<gtk::Widget>) -> Vec<T> {
    widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<T>().ok())
        .collect()
}

/// The click gesture of `widget` for mouse button `button`.
pub(super) fn click_gesture(widget: &impl IsA<gtk::Widget>, button: u32) -> gtk::GestureClick {
    controllers::<gtk::GestureClick>(widget)
        .into_iter()
        .find(|gesture| gesture.button() == button)
        .unwrap_or_else(|| panic!("the widget listens to button {button}"))
}

/// Emits `key-pressed` with `key` on the first key controller of `widget`;
/// whether a handler took the key.
fn press_key(widget: &impl IsA<gtk::Widget>, key: gdk::Key) -> bool {
    let keys = controllers::<gtk::EventControllerKey>(widget);
    let controller = keys.first().expect("the widget has a key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>(
        "key-pressed",
        &[&key.into_glib(), &no_keycode, &gdk::ModifierType::empty()],
    )
}

/// The box that holds the crumb buttons.
fn crumb_box(test: &TestWindow) -> gtk::Widget {
    let crumbs = test.window.address_bar().crumb_buttons();
    let first = crumbs.first().expect("a location has crumbs");
    first.parent().expect("crumbs sit in their box")
}

/// A crumb's location.
fn crumb_target(crumb: &gtk::Button) -> String {
    let target = crumb
        .action_target_value()
        .and_then(|target| target.get::<String>());
    target.expect("every crumb opens a location")
}

/// Shows `uri` in the active tab's frame without listing it, as a tab on
/// a share the test cannot reach would show it.
fn show_unlisted(test: &TestWindow, uri: &str) {
    {
        let mut session = test.window.imp().session.borrow_mut();
        let tab = session.active_mut().expect("one tab");
        tab.history.push(uri);
    }
    test.window.render_navigation();
}

/// parity: NAV-017
#[gtk::test]
fn an_smb_path_has_server_share_and_folder_crumbs_with_the_uri_encoded() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    show_unlisted(&test, "smb://nas:1445/Team%20files/Q3%20%231");

    let crumbs = test.window.address_bar().crumb_buttons();
    let labels: Vec<String> = crumbs
        .iter()
        .filter_map(|crumb| crumb.label().map(Into::into))
        .collect();
    assert_eq!(labels[1..], ["Team files", "Q3 #1"]);
    assert!(crumb_target(&crumbs[0]).starts_with("smb://nas:1445/"));
    assert_eq!(crumb_target(&crumbs[2]), "smb://nas:1445/Team%20files/Q3%20%231");
    let dividers: Vec<String> = descendants::<gtk::Label>(&crumb_box(&test))
        .iter()
        .filter(|label| label.has_css_class("crumb-divider"))
        .map(|label| label.text().to_string())
        .collect();
    assert_eq!(dividers, ["\\", "\\"]);
}

/// parity: NAV-018
#[gtk::test]
fn left_and_right_stop_at_the_first_and_last_crumb() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let first = crumbs.first().expect("crumbs");
    let last = crumbs.last().expect("crumbs");
    let before_last = &crumbs[crumbs.len() - 2];

    last.grab_focus();
    assert!(press_key(&crumb_box(&test), gdk::Key::Right));
    assert!(last.has_focus(), "Right stops at the last crumb");
    press_key(&crumb_box(&test), gdk::Key::Left);
    assert!(before_last.has_focus());
    first.grab_focus();
    press_key(&crumb_box(&test), gdk::Key::Left);
    assert!(first.has_focus(), "Left stops at the first crumb");
}

/// parity: NAV-018
#[gtk::test]
fn activating_a_focused_crumb_opens_it_not_the_selected_file() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let parent = crumbs[crumbs.len() - 2].clone();
    test.window.navigate(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the folder");
    test.window
        .folder_model()
        .select_only(test.position_of("Notes 2.txt"));

    parent.grab_focus();
    parent.activate();
    test.wait_for_listing("the crumb's folder");

    assert_eq!(test.window.current_uri(), Some(crumb_target(&parent)));
    assert!(test.context.recorded_launches().is_empty(), "no file opens");
    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
}

/// parity: NAV-018
#[gtk::test]
fn a_middle_clicked_crumb_opens_in_a_background_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let parent = &crumbs[crumbs.len() - 2];

    click_gesture(parent, gdk::BUTTON_MIDDLE).emit_by_name::<()>("released", &[&1_i32, &1.0_f64, &1.0_f64]);

    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// parity: NAV-023
#[gtk::test]
fn the_wheel_scrolls_overflowing_crumbs_sideways() {
    let fixture = Fixture::standard();
    let mut deep = fixture.root().to_path_buf();
    for level in 0..14 {
        deep.push(format!("A rather long folder name at level {level}"));
    }
    std::fs::create_dir_all(&deep).expect("deep fixture folders");
    let test = TestWindow::open(&ox_core::location::file_uri(&deep));
    let adjustment = test.window.address_bar().crumb_adjustment();
    wait_until("the crumbs to overflow", || {
        adjustment.upper() > adjustment.page_size()
    });
    let scroller = descendants::<gtk::ScrolledWindow>(test.window.address_bar())
        .into_iter()
        .next()
        .expect("the crumbs scroll");
    adjustment.set_value(0.0);
    let wheel = controllers::<gtk::EventControllerScroll>(&scroller);
    let wheel = wheel.first().expect("the crumbs follow the wheel");

    let handled = wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &1.0_f64]);

    assert!(handled, "the page itself does not scroll");
    assert!(
        adjustment.value() > 0.0,
        "a plain wheel scrolls the crumbs sideways"
    );
}

/// parity: NAV-026
#[gtk::test]
fn ctrl_l_alt_d_the_chevron_and_blank_space_edit_the_whole_address() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    let keys = application().accels_for_action("win.location");
    assert_eq!(keys, ["<Control>l", "<Alt>d"]);
    let path = fixture.root().display().to_string();
    let tooltip = format!("{path} · Click blank space or press Ctrl+L to edit");
    assert_eq!(address.tooltip_text().as_deref(), Some(tooltip.as_str()));
    let chevron = descendants::<gtk::Button>(address)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.address-history"))
        .expect("the edit chevron");
    assert!(gtk::test_accessible_has_property(
        &chevron,
        gtk::AccessibleProperty::Label
    ));

    let scroller = descendants::<gtk::ScrolledWindow>(address)
        .into_iter()
        .next()
        .expect("the crumbs");
    click_gesture(&scroller, gdk::BUTTON_PRIMARY)
        .emit_by_name::<()>("released", &[&1_i32, &1.0_f64, &1.0_f64]);

    assert_eq!(address.mode(), AddressMode::Entry);
    let entry = address.entry();
    assert_eq!(entry.text().as_str(), path);
    let length = i32::try_from(path.chars().count()).expect("a short path");
    assert_eq!(entry.selection_bounds(), Some((0, length)), "fully selected");
}

/// parity: NAV-026
#[gtk::test]
fn the_settings_tab_has_no_address_to_edit() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("settings", None);
    wait_until("the Settings tab", || {
        test.window.current_uri().as_deref() == Some(Page::Settings.uri())
    });

    test.activate("location", None);

    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
}

/// parity: NAV-028
#[gtk::test]
fn a_second_ctrl_l_returns_to_the_crumbs_unless_something_was_typed() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    test.activate("location", None);

    test.activate("location", None);

    assert_eq!(address.mode(), AddressMode::Crumbs);
    test.activate("location", None);
    address.entry().set_text("/tm");
    test.activate("location", None);
    assert_eq!(
        address.mode(),
        AddressMode::Entry,
        "after typing, editing goes on"
    );
}

/// parity: NAV-043
#[gtk::test]
fn f4_lists_the_typed_addresses_most_recent_first_and_a_row_goes_there() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    assert_eq!(application().accels_for_action("win.address-history"), ["F4"]);
    for folder in ["Documents", ""] {
        test.activate("location", None);
        address.submit_text(&fixture.path(folder).display().to_string());
        test.wait_for_listing(folder);
    }

    test.activate("address-history", None);

    assert_eq!(address.mode(), AddressMode::Entry);
    assert!(address.suggestions_shown());
    let documents = fixture.path("Documents").display().to_string();
    let root = fixture.path("").display().to_string();
    assert_eq!(address.suggestion_rows(), [root.clone(), documents.clone()]);
    let list_keys = controllers::<gtk::EventControllerKey>(&address.entry())
        .into_iter()
        .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the list's keys");
    for _ in 0..2 {
        let no_keycode = 0_u32;
        let down = gdk::Key::Down.into_glib();
        list_keys.emit_by_name::<bool>("key-pressed", &[&down, &no_keycode, &gdk::ModifierType::empty()]);
    }
    assert_eq!(
        address.entry().text(),
        documents,
        "Down puts the row into the address"
    );
    address.entry().emit_by_name::<()>("activate", &[]);
    wait_until("the typed address", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    test.wait_for_listing("Documents");

    test.activate("address-history", None);
    assert_eq!(address.suggestion_rows(), [documents, root]);
    address.suggestion_row(1).activate();
    wait_until("the chosen address", || {
        test.window.current_uri() == Some(fixture.uri())
    });
}

/// parity: NAV-030
#[gtk::test]
fn typing_a_path_offers_the_matching_names() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    test.activate("location", None);

    let typed = format!("{}/No", fixture.root().display());
    address.entry().set_text(&typed);

    wait_until("the completions", || address.suggestions_shown());
    let base = fixture.root().display();
    assert_eq!(
        address.suggestion_rows(),
        [format!("{base}/Notes 2.txt"), format!("{base}/Notes 10.txt")]
    );
    address.hide_suggestions();
}

/// parity: NAV-020
#[gtk::test]
fn choosing_a_folder_from_a_crumb_menu_goes_there() {
    let fixture = Fixture::standard();
    for folder in ["Documents/Letters", "Documents/Bills"] {
        std::fs::create_dir(fixture.path(folder)).expect("fixture subfolder");
    }
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let address = test.window.address_bar();
    let target = (fixture.uri_of("Documents"), "", 0_u32).to_variant();

    WidgetExt::activate_action(&test.window, "win.crumb-subfolders", Some(&target)).expect("the action");

    let menu = || {
        descendants::<MenuPopover>(address)
            .into_iter()
            .find(WidgetExt::is_visible)
    };
    wait_until("the crumb menu", || menu().is_some());
    let menu = menu().expect("the menu");
    assert_eq!(menu.row_labels(), ["Bills", "Letters"]);
    menu.row("Letters").activate();
    wait_until("the chosen folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents/Letters"))
    });
    test.wait_for_listing("Letters");
    wait_for(std::time::Duration::from_millis(100));
    assert!(descendants::<MenuPopover>(address).is_empty(), "the menu is gone");
}

/// parity: NAV-022
#[gtk::test]
fn the_wheel_on_a_crumb_goes_to_the_next_folder_beside_it() {
    let fixture = Fixture::standard();
    for folder in ["Music", ".cache"] {
        std::fs::create_dir(fixture.path(folder)).expect("fixture subfolder");
    }
    let deep = (1..=12).fold(String::from("Documents"), |path, level| {
        format!("{path}/A rather long folder name at level {level}")
    });
    std::fs::create_dir_all(fixture.path(&deep)).expect("fixture subfolders");
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let address = test.window.address_bar();
    let scroll_last_crumb = |dy: f64| {
        let crumb = address.crumb_buttons().pop().expect("a crumb");
        let wheel = controllers::<gtk::EventControllerScroll>(&crumb)
            .pop()
            .expect("the crumb's wheel");
        wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &dy])
    };

    assert!(!scroll_last_crumb(0.1), "a fraction of a notch does nothing");
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert!(scroll_last_crumb(1.0));
    wait_until("the next folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Music"))
    });
    test.wait_for_listing("Music");
    assert!(scroll_last_crumb(1.0));
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri_of("Music")),
        "hidden .cache is skipped and Music is last"
    );

    test.window
        .navigate(&fixture.uri_of(&deep))
        .expect("valid folder");
    wait_until("the crumbs to overflow", || address.crumbs_overflow());
    assert!(!scroll_last_crumb(1.0), "overflowing crumbs scroll sideways");
}

/// parity: NAV-029
#[gtk::test]
fn the_address_can_stay_editable_across_folders_until_toggled_back() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();

    test.activate("editable-location", None);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("Documents");
    test.window.folder_pane().focus_view();

    assert_eq!(address.mode(), AddressMode::Entry);
    assert_eq!(
        address.entry().text(),
        fixture.path("Documents").display().to_string()
    );
    let state = test
        .window
        .lookup_action("editable-location")
        .and_then(|action| action.state());
    assert_eq!(state.and_then(|state| state.get::<bool>()), Some(true));
    test.activate("editable-location", None);
    assert_eq!(address.mode(), AddressMode::Crumbs);
}

/// parity: NAV-027
#[gtk::test]
fn escape_keeps_the_folder_and_the_selection_and_switching_tabs_ends_editing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    test.window
        .folder_model()
        .select_only(test.position_of("Notes 2.txt"));
    test.activate("location", None);
    address.entry().set_text("/tmp");

    assert!(press_key(&address.entry(), gdk::Key::Escape));

    assert_eq!(address.mode(), AddressMode::Crumbs);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    test.activate("location", None);
    test.activate("previous-tab", None);
    assert_eq!(address.mode(), AddressMode::Crumbs);
}

/// parity: NAV-033
#[gtk::test]
fn a_typed_relative_name_or_tilde_opens_in_the_same_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("location", None);

    test.window.submit_address("Documents");

    wait_until("the typed folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
    test.window.submit_address("~");
    let home = ox_core::location::file_uri(&glib::home_dir());
    wait_until("the home folder", || {
        test.window.current_uri() == Some(home.clone())
    });
    assert_eq!(test.window.tab_count(), 1);
}

/// parity: NAV-033, NAV-035
#[gtk::test]
fn a_refused_address_is_explained_and_the_field_stays_open() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("location", None);

    test.window.submit_address("C:\\Windows");

    assert_eq!(
        test.window.shown_message().as_str(),
        "Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share."
    );
    assert_eq!(test.window.address_bar().mode(), AddressMode::Entry);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// parity: NAV-036
#[gtk::test]
fn a_typed_web_address_opens_in_the_browser_and_other_schemes_are_refused() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("location", None);

    test.window.submit_address(" https://example.org/docs ");

    assert_eq!(test.context.recorded_launches(), ["https://example.org/docs"]);
    let notice = test.window.shown_message();
    assert_eq!(
        notice.as_str(),
        "Opening https://example.org/docs in your web browser."
    );
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    test.activate("location", None);
    test.window.submit_address("ftp://example.org/");
    assert_eq!(
        test.window.shown_message().as_str(),
        "Only local paths, smb:// locations and connected devices are supported in this build."
    );
    assert_eq!(test.context.recorded_launches().len(), 1);
}

/// parity: NAV-031
#[gtk::test]
fn the_address_menu_copies_the_address_and_pastes_one_to_go_to() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));

    test.activate("copy-address", None);
    let clipboard = test.window.clipboard();
    let copied = glib::MainContext::default().block_on(clipboard.read_text_future());
    let copied = copied.ok().flatten().map(String::from);
    assert_eq!(copied, Some(fixture.path("Documents").display().to_string()));
    clipboard.set_text(&fixture.root().display().to_string());
    test.activate("paste-address", None);
    wait_until("the pasted address", || {
        test.window.current_uri() == Some(fixture.uri())
    });
}

/// parity: NAV-032
#[gtk::test]
fn a_middle_click_on_blank_address_space_opens_the_selected_text() {
    let fixture = Fixture::standard();
    fixture.write("Documents/Letter.txt");
    let test = TestWindow::open(&fixture.uri());
    let address = test.window.address_bar();
    let primary = WidgetExt::display(address).primary_clipboard();
    let middle = click_gesture(address, gdk::BUTTON_MIDDLE);
    primary.set_text("Documents");

    middle.emit_by_name::<()>("released", &[&1_i32, &1.0_f64, &1.0_f64]);

    wait_until("the selected folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    test.wait_for_listing("Documents");
    test.window.navigate(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the fixture");
    primary.set_text(&fixture.path("Documents/Letter.txt").display().to_string());
    middle.emit_by_name::<()>("released", &[&1_i32, &1.0_f64, &1.0_f64]);
    wait_until("the file selected in its folder", || {
        test.window.folder_pane().model().selected_uris() == [fixture.uri_of("Documents/Letter.txt")]
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.activate("address-history", None);
    assert!(
        address.suggestion_rows().is_empty(),
        "pasted text is not typed history"
    );
}

/// parity: NAV-033
#[gtk::test]
fn a_typed_zip_opens_the_archive_browser() {
    let fixture = fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    test.activate("location", None);

    test.window
        .submit_address(fixture.path("Bundle.zip").to_str().expect("UTF-8 fixture"));

    archive_browser(&test);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}
