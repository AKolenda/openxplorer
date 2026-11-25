// SPDX-License-Identifier: AGPL-3.0-only
//! Accessibility: names and roles for screen readers, one tab stop for the
//! file list, keyboard control of tabs and column titles, dialogs that
//! keep and return focus, reduced motion and large text.
//!
//! GTK has no public way to synthesise key events, so these tests emit
//! `key-pressed` on a widget's key controller, which is what a real key
//! press reaches.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::support::app_menu;
use crate::dialog::Dialog;
use crate::folder_view::column_widths;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::resizer_control::ResizerControl;
use crate::test_support::harness::{
    application, descendants, skin, wait_for, wait_for_frames, wait_until, Fixture, TestWindow,
};
use crate::text_size::TextSize;
use crate::theme::desktop_text::{DesktopText, DesktopTextWatch};
use crate::window::focus_regions::Region;
use crate::window::window_action::WindowAction;
use crate::window::ButtonStyle;

/// Emits `key` with `modifiers` on the key controller of `widget`;
/// returns true when the widget handled it.
fn press_on(widget: &impl IsA<gtk::Widget>, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let controller = widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("the widget has a key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &modifiers])
}

/// Whether `widget` shows no text of its own, only a picture.
fn shows_no_text(widget: &gtk::Widget) -> bool {
    let labels = descendants::<gtk::Label>(widget);
    labels.iter().all(|label| label.text().is_empty())
}

/// parity: ACC-001, ACC-002
#[gtk::test]
fn the_file_list_its_items_and_every_icon_button_are_named() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.folder_pane().details().column_view().clone();
    let grid = test.window.folder_pane().icon_view().grid().clone();
    for view in [details.upcast_ref::<gtk::Widget>(), grid.upcast_ref()] {
        assert!(gtk::test_accessible_has_property(
            view,
            gtk::AccessibleProperty::Label
        ));
        assert!(gtk::test_accessible_has_property(
            view,
            gtk::AccessibleProperty::Description
        ));
    }
    for count in [
        gtk::AccessibleRelation::RowCount,
        gtk::AccessibleRelation::ColCount,
    ] {
        assert!(gtk::test_accessible_has_relation(&details, count), "{count:?}");
    }
    let rows: Vec<gtk::Widget> = descendants::<gtk::Widget>(&details)
        .into_iter()
        .filter(|widget| widget.accessible_role() == gtk::AccessibleRole::Row)
        .filter(|row| row.parent().is_some_and(|parent| parent.css_name() == "listview"))
        .collect();
    assert!(!rows.is_empty(), "the rows are drawn");
    for row in &rows {
        assert!(gtk::test_accessible_has_property(
            row,
            gtk::AccessibleProperty::Label
        ));
    }

    let hint = test.window.status_bar().typeahead_hint_label();
    assert_eq!(
        hint.accessible_role(),
        gtk::AccessibleRole::Status,
        "a live region"
    );
    // Buttons that sort: GTK's column-header role reaches no screen
    // reader as one (tests/screen_reader.rs).
    let titles = crate::folder_view::column_titles::title_buttons(&details);
    let roles: Vec<gtk::AccessibleRole> = titles.iter().map(AccessibleExt::accessible_role).collect();
    assert!(
        roles.iter().all(|role| *role == gtk::AccessibleRole::Button),
        "{roles:?}"
    );
    let resizers = descendants::<ResizerControl>(&details);
    assert_eq!(resizers.len(), titles.len(), "every column has its resizer");
    let size_position = SortColumn::ALL
        .iter()
        .position(|column| *column == SortColumn::Size);
    let size = &resizers[size_position.expect("a Size column")];
    assert_eq!(size.accessible_role(), gtk::AccessibleRole::Separator);
    assert!(!size.is_focusable(), "the title takes the keys");
    assert!(size.request_value(130.0), "a screen reader can set the width");
    let size_column = test.window.folder_pane().details().column(SortColumn::Size);
    let fixed = size_column.expect("a Size column").fixed_width();
    assert_eq!(column_widths::saved_width(SortColumn::Size, fixed), Some(130.0));

    let unnamed: Vec<String> = descendants::<gtk::Widget>(&test.window)
        .into_iter()
        .filter(|widget| widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>())
        .filter(|button| button.is_visible() && shows_no_text(button))
        .filter(|button| !gtk::test_accessible_has_property(button, gtk::AccessibleProperty::Label))
        .map(|button| {
            let parent = button
                .parent()
                .map(|parent| format!("{} {:?}", parent.type_(), parent.css_classes()));
            format!("{} {:?} in {parent:?}", button.type_(), button.css_classes())
        })
        .collect();
    assert!(unnamed.is_empty(), "icon buttons without a name: {unnamed:?}");
}

/// The file list is one tab stop whose focused row is the item itself,
/// drawn apart from the selection.
///
/// parity: ACC-003
#[gtk::test]
fn the_file_list_is_one_tab_stop_that_focuses_the_item_itself() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.folder_pane().details().column_view().clone();
    let grid = test.window.folder_pane().icon_view().grid().clone();
    assert_eq!(details.tab_behavior(), gtk::ListTabBehavior::Item);
    assert_eq!(grid.tab_behavior(), gtk::ListTabBehavior::Item);

    test.window.folder_pane().focus_view();
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("the list has focus");
    assert!(focus.is_ancestor(&details));
    assert_eq!(focus.accessible_role(), gtk::AccessibleRole::Row);
    assert!(gtk::test_accessible_has_property(
        &focus,
        gtk::AccessibleProperty::Label
    ));
}

/// parity: ACC-015
#[gtk::test]
fn f6_moves_focus_round_the_regions_and_back_to_the_same_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.open_details_pane();
    test.window.folder_pane().focus_view();
    let item = test.window.folder_pane().focused_position();
    let selected = test.window.folder_pane().model().selected_items().len();

    let mut visited = Vec::new();
    for _ in 0..8 {
        visited.push(
            test.window
                .focus_next_region(false)
                .expect("a region takes focus"),
        );
        assert_eq!(test.window.focused_region(), visited.last().copied());
    }
    assert_eq!(
        visited,
        [
            Region::Details,
            Region::StatusBar,
            Region::Tabs,
            Region::Address,
            Region::Search,
            Region::Commands,
            Region::Sidebar,
            Region::Files
        ]
    );
    assert_eq!(
        test.window.folder_pane().focused_position(),
        item,
        "the same item"
    );
    assert_eq!(test.window.focus_next_region(true), Some(Region::Sidebar));
    assert_eq!(test.window.folder_pane().model().selected_items().len(), selected);
}

/// Saves "F6 moves like Windows" `on` and waits for the window to read it.
fn choose_f6_like_windows(test: &TestWindow, on: bool) {
    let update = ox_core::settings::PreferencesUpdate {
        f6_like_windows: Some(on),
        ..ox_core::settings::PreferencesUpdate::default()
    };
    ox_core::settings::Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.f6_like_windows == on
    });
}

/// A window with the Details pane shown and the keyboard on its file
/// list, with the real pointer resting on its edge (the keyboard follows
/// the pointer on the private display), split in two when `split`.
fn window_for_real_f6(fixture: &Fixture, split: bool) -> (TestWindow, crate::test_support::pointer::Pointer) {
    use crate::test_support::pointer::{screen_point, Pointer};
    // Before the window opens, which a new pointer would put away.
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.open_details_pane();
    if split {
        test.activate("split-view", None);
    }
    test.window.present();
    wait_for_frames(&test.window, 3);
    let (x, y) = screen_point(&mut pointer, &test.window, (4.0, 4.0), (8, 8));
    pointer.move_to(x, y);
    wait_until("the window to have the keyboard", || test.window.is_active());
    test.window
        .pane_on(crate::window::session::PaneSide::Start)
        .focus_view();
    wait_until("the keyboard on the file list", || {
        test.window.focused_region() == Some(Region::Files)
    });
    (test, pointer)
}

/// The regions `presses` real F6 presses visit, one after the other
/// (Shift+F6 when `backward`).
fn regions_after_real_f6(
    test: &TestWindow,
    pointer: &mut crate::test_support::pointer::Pointer,
    presses: usize,
    backward: bool,
) -> Vec<Region> {
    let mut visited = Vec::new();
    for _ in 0..presses {
        let before = test.window.focused_region();
        if backward {
            pointer.hold_key(gdk::Key::Shift_L);
        }
        pointer.tap_key(gdk::Key::F6);
        if backward {
            pointer.let_go_key(gdk::Key::Shift_L);
        }
        wait_until("F6 to move the keyboard", || {
            test.window.focused_region().is_some() && test.window.focused_region() != before
        });
        visited.push(test.window.focused_region().expect("a region has the keyboard"));
    }
    visited
}

/// As it comes ("F6 moves like Windows" off), real F6 presses keep the
/// app's own cycle (ACC-015): the address bar, search box and command bar
/// are F6 stops between the tabs and the navigation pane.
///
/// parity: ACC-015
#[gtk::test]
fn real_f6_visits_the_address_bar_search_and_command_bar_as_it_comes() {
    let fixture = Fixture::standard();
    let (test, mut pointer) = window_for_real_f6(&fixture, false);
    assert!(!test.context.settings_data().preferences.f6_like_windows);
    assert_eq!(
        regions_after_real_f6(&test, &mut pointer, 8, false),
        [
            Region::Details,
            Region::StatusBar,
            Region::Tabs,
            Region::Address,
            Region::Search,
            Region::Commands,
            Region::Sidebar,
            Region::Files
        ]
    );
}

/// With "F6 moves like Windows" on, real F6 presses visit only the
/// regions Microsoft lists for Windows 11's F6 ("Tabs", "Navigation
/// pane", "Content pane", "Details pane (if shown)", "Status bar"; "Use a
/// screen reader to explore and navigate File Explorer in Windows"), each
/// pane of a split tab, and Shift+F6 goes back the same way. Tab from the
/// tabs still reaches the address bar, search box and command bar, and
/// F6 from the address bar goes on to the navigation pane.
///
/// parity: ACC-015
#[gtk::test]
fn real_f6_like_windows_leaves_the_address_bar_search_and_command_bar_to_tab() {
    let fixture = Fixture::standard();
    let (test, mut pointer) = window_for_real_f6(&fixture, true);
    choose_f6_like_windows(&test, true);
    assert_eq!(
        regions_after_real_f6(&test, &mut pointer, 6, false),
        [
            Region::EndPane,
            Region::Details,
            Region::StatusBar,
            Region::Tabs,
            Region::Sidebar,
            Region::Files
        ]
    );
    assert_eq!(
        regions_after_real_f6(&test, &mut pointer, 5, true),
        [
            Region::Sidebar,
            Region::Tabs,
            Region::StatusBar,
            Region::Details,
            Region::EndPane
        ]
    );

    // Tab from the tab in front goes on, past the tab's buttons, to the
    // address bar, the search box and the command bar.
    regions_after_real_f6(&test, &mut pointer, 3, false);
    assert_eq!(test.window.focused_region(), Some(Region::Tabs));
    let mut reached = Vec::new();
    for _ in 0..30 {
        pointer.tap_key(gdk::Key::Tab);
        wait_for(std::time::Duration::from_millis(50));
        let region = test.window.focused_region();
        if let Some(region) = region.filter(|region| reached.last() != Some(region)) {
            reached.push(region);
        }
        if region == Some(Region::Commands) {
            break;
        }
    }
    assert_eq!(
        reached
            .iter()
            .filter(|region| matches!(region, Region::Address | Region::Search | Region::Commands))
            .copied()
            .collect::<Vec<_>>(),
        [Region::Address, Region::Search, Region::Commands],
        "Tab reaches them in order: {reached:?}"
    );

    // From the address bar, F6 goes on to the navigation pane.
    pointer.hold_key(gdk::Key::Alt_L);
    pointer.tap_key(gdk::Key::d);
    pointer.let_go_key(gdk::Key::Alt_L);
    wait_until("Alt+D to edit the address", || {
        test.window.focused_region() == Some(Region::Address)
    });
    assert_eq!(
        regions_after_real_f6(&test, &mut pointer, 1, false),
        [Region::Sidebar]
    );

    // Turned off again, F6 takes the address bar back into its cycle.
    choose_f6_like_windows(&test, false);
    assert_eq!(
        regions_after_real_f6(&test, &mut pointer, 1, true),
        [Region::Commands]
    );
}

/// F6 into the tab row puts the keyboard on the tab in front, not on the
/// first tab's close button, so the Menu key and Shift+F10 open the menu
/// of the tab shown. Windows 11 includes the tab row in F6's cycle and
/// moves along it with the arrow keys (blogs.windows.com, Windows 11
/// Insider Preview Build 22621.290).
///
/// parity: ACC-015, TAB-012
#[gtk::test]
fn f6_into_the_tab_row_focuses_the_tab_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    wait_for_frames(&test.window, 3);
    test.window.folder_pane().focus_view();

    let mut region = None;
    for _ in 0..8 {
        region = test.window.focus_next_region(false);
        if region == Some(Region::Tabs) {
            break;
        }
    }
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a tab has focus");
    let front = test.window.tab_strip().tab_list().last_child().expect("two tabs");

    assert_eq!(region, Some(Region::Tabs));
    assert_eq!(focus, front, "the tab in front, not a close button");
}

/// parity: ACC-004
#[gtk::test]
fn a_dialog_focuses_its_field_closes_menus_and_enter_presses_its_button() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("pin-folder", None);
    wait_until("the pin", || {
        test.window
            .sidebar()
            .labels()
            .contains(&"Example projects".to_owned())
    });
    test.window.type_text("n");
    let menu = test.window.sidebar().right_click_row("Example projects");
    assert!(menu.is_visible());

    let dialog = Dialog::new(&test.window, "Rename", "");
    let field = dialog.add_text_field("Name", "Notes 2.txt");
    dialog.add_cancel_button();
    dialog.add_button("Save", ButtonStyle::Accent);
    dialog.open();
    let saved = Rc::new(Cell::new(None));
    let answer = Rc::clone(&saved);
    let waiting = dialog.clone();
    glib::spawn_future_local(async move {
        answer.set(Some(waiting.next_response().await.is_some()));
    });

    assert!(!menu.is_visible(), "opening a dialog closes the menus");
    let hint = test.window.status_bar().typeahead_hint_label();
    assert!(hint.text().is_empty(), "and ends type-to-select");
    let focus = gtk::prelude::GtkWindowExt::focus(&dialog);
    assert!(
        focus.is_some_and(|focus| focus.is_ancestor(&field)),
        "the first field has focus"
    );
    assert_eq!(field.selection_bounds(), Some((0, 11)), "with its text selected");
    // Enter reaches the field's text, which activates the default button.
    let text = field
        .delegate()
        .and_downcast::<gtk::Text>()
        .expect("an entry edits a text");
    text.emit_activate();
    wait_until("Enter to press Save", || saved.get().is_some());
    assert_eq!(saved.get(), Some(true));

    // While Save runs its buttons are off, and closing cancels the
    // operation, so a stalled share cannot hold the dialog open.
    let running = ox_core::transfer::Cancellation::new();
    dialog.set_busy(Some(&running));
    let save = descendants::<gtk::Button>(&dialog)
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Save"))
        .expect("a Save button");
    assert!(!save.is_sensitive(), "Save is off while it runs");
    dialog.close();
    assert!(running.is_cancelled(), "closing cancels the running operation");
    assert!(!dialog.is_visible());
}

/// parity: ACC-005
#[gtk::test]
fn closing_a_dialog_gives_focus_back_to_the_control_that_had_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.search_box().entry().grab_focus();
    let opener = gtk::prelude::GtkWindowExt::focus(&test.window);
    assert!(opener.is_some());

    let dialog = Dialog::new(&test.window, "Delete", "Move 1 item to the Trash?");
    dialog.add_cancel_button();
    dialog.add_button("Move to Trash", ButtonStyle::Danger);
    dialog.open();
    let focus = gtk::prelude::GtkWindowExt::focus(&dialog);
    assert!(
        focus.is_some_and(|focus| focus.is::<gtk::Button>()),
        "a button has focus"
    );
    dialog.close();

    assert!(!dialog.is_visible(), "Escape or closing cancels");
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&test.window), opener);
}

/// Enter or Space on a focused control is that control's: no window or
/// application shortcut takes them, so only the file list opens files.
///
/// parity: ACC-007
#[gtk::test]
fn no_shortcut_takes_plain_enter_or_space_from_the_focused_control() {
    let fixture = Fixture::standard();
    let _test = TestWindow::open(&fixture.uri());
    for accelerator in ["Return", "KP_Enter", "space"] {
        let actions = application().actions_for_accel(accelerator);
        assert!(actions.is_empty(), "{accelerator} runs {actions:?}");
    }
}

/// Only the tab in front is a tab stop; the arrows, Home and End show
/// another tab, which keeps focus; each close button names its tab.
///
/// parity: ACC-008
#[gtk::test]
fn the_arrow_keys_move_between_tabs_and_only_the_front_tab_is_a_tab_stop() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("open-tab", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the second tab");
    let tabs = || -> Vec<gtk::Box> {
        let list = test.window.tab_strip().tab_list();
        descendants::<gtk::Box>(&list)
            .into_iter()
            .filter(|tab| tab.accessible_role() == gtk::AccessibleRole::Tab)
            .collect()
    };
    let front = |tabs: &[gtk::Box]| tabs.iter().position(|tab| tab.has_css_class("active"));
    let shown = tabs();
    assert_eq!(front(&shown), Some(1));
    let focusable: Vec<bool> = shown.iter().map(WidgetExt::is_focusable).collect();
    assert_eq!(focusable, [false, true]);
    let close_names = descendants::<gtk::Button>(&shown[1])
        .into_iter()
        .filter(|button| button.has_css_class("tab-close"))
        .count();
    assert_eq!(close_names, 1);

    shown[1].grab_focus();
    assert!(press_on(&shown[1], gdk::Key::Left, gdk::ModifierType::empty()));
    test.wait_for_listing("the first tab");

    let shown = tabs();
    assert_eq!(front(&shown), Some(0));
    assert!(shown[0].has_focus(), "focus moves with the tab shown");
    assert!(press_on(&shown[0], gdk::Key::End, gdk::ModifierType::empty()));
    assert_eq!(front(&tabs()), Some(1));
}

/// parity: ACC-006
#[gtk::test]
fn column_titles_sort_and_resize_from_the_keyboard() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.folder_pane().details();
    let titles = crate::folder_view::column_titles::title_buttons(details.column_view());
    let type_index = details
        .column_order()
        .iter()
        .position(|column| *column == SortColumn::Type);
    let type_title = &titles[type_index.expect("a Type title")];
    assert!(titles.iter().all(WidgetExt::is_focusable));
    assert!(
        gtk::test_accessible_has_property(type_title, gtk::AccessibleProperty::Label),
        "named after its column, not its resizer"
    );
    let type_column = details.column(SortColumn::Type).expect("a Type column");
    let before = type_column.fixed_width();

    assert!(press_on(type_title, gdk::Key::Right, gdk::ModifierType::empty()));
    assert_eq!(type_column.fixed_width(), before + 10);
    assert!(press_on(
        type_title,
        gdk::Key::Right,
        gdk::ModifierType::SHIFT_MASK
    ));
    assert_eq!(type_column.fixed_width(), before + 50);
    // Home fits the column to the listed types, all shorter than 135.
    assert!(press_on(type_title, gdk::Key::Home, gdk::ModifierType::empty()));
    let fitted = type_column.fixed_width();
    assert!((80..before).contains(&fitted), "fitted to the items: {fitted}");
    assert!(press_on(type_title, gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(details.sort_order().column, SortColumn::Type);
    assert!(press_on(type_title, gdk::Key::space, gdk::ModifierType::empty()));
    assert_eq!(details.sort_order().direction, SortDirection::Descending);
}

/// What a screen reader hears matches what the window shows: the Details
/// button is named, the search box is named "Search <folder>" as it reads,
/// a tab is described by its location (not its close button's "Close
/// tab"), the row of column titles is named once, the title that shows
/// "Date modified" sorts by Date modified, and the rows are named as the
/// list shows them, without a hidden extension.
///
/// parity: ACC-001, ACC-002
#[gtk::test]
fn screen_readers_hear_what_the_window_shows() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details_button = descendants::<gtk::ToggleButton>(&test.window)
        .into_iter()
        .find(|button| button.has_css_class("details-toggle"))
        .expect("the Details button");
    assert!(
        gtk::test_accessible_has_property(&details_button, gtk::AccessibleProperty::Label),
        "the Details button has a name"
    );
    let placeholder = test
        .window
        .search_box()
        .entry()
        .placeholder_text()
        .unwrap_or_default();
    assert!(placeholder.starts_with("Search "), "{placeholder}");
    assert_eq!(test.window.search_box().accessible_name(), placeholder);

    let tabs: Vec<gtk::Widget> = descendants::<gtk::Widget>(&test.window)
        .into_iter()
        .filter(|widget| widget.accessible_role() == gtk::AccessibleRole::Tab)
        .collect();
    assert!(!tabs.is_empty(), "a tab is drawn");
    for tab in &tabs {
        assert!(
            gtk::test_accessible_has_property(tab, gtk::AccessibleProperty::Description),
            "a tab is described by its location"
        );
    }

    let details = test.window.folder_pane().details();
    let header = details.header().expect("the column titles");
    assert!(gtk::test_accessible_has_property(
        &header,
        gtk::AccessibleProperty::Label
    ));
    let titles = crate::folder_view::column_titles::title_buttons(details.column_view());
    let modified = titles
        .iter()
        .find(|title| {
            title.is_visible()
                && descendants::<gtk::Label>(*title)
                    .iter()
                    .any(|label| label.text() == "Date modified")
        })
        .expect("a title that shows Date modified");
    assert!(press_on(modified, gdk::Key::Return, gdk::ModifierType::empty()));
    assert_eq!(details.sort_order().column, SortColumn::Modified);

    let owners = test.window.folder_pane().owners();
    wait_until("the rows are named", || {
        let names = owners.detail_row_names();
        names.iter().any(|name| name == "Résumé") && !names.iter().any(|name| name == "Résumé.txt")
    });
}

/// Every motion in the skin is a CSS transition or animation, which GTK
/// skips while the desktop turns animations off, and the only motion drawn
/// frame by frame, tabs sliding aside for a dragged tab, reads the same
/// setting and puts the tabs in place at once; the app never turns
/// animations back on.
///
/// parity: ACC-010
#[gtk::test]
fn animations_stay_off_when_the_desktop_turns_them_off() {
    let settings = gtk::Settings::default().expect("a display");
    let before = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(false);
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("new-tab", None);
    test.wait_for_listing("the new tab");
    let still_off = !settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(before);
    assert!(still_off);
    assert_eq!(
        frame_driven_motion(),
        Vec::<String>::new(),
        "only the snapshot hook, the tests and motion that follows the desktop's animation setting draw frame by frame"
    );
}

/// The app's source files, outside the snapshot hook and the test
/// support, that move something frame by frame without reading GTK's
/// animation setting (`gtk-enable-animations`), which would then not stop
/// them.
fn frame_driven_motion() -> Vec<String> {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut folders = vec![source];
    let mut found = Vec::new();
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).expect("the source folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !path.ends_with("test_support") {
                    folders.push(path);
                }
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            // A file's own test module may draw frame by frame too.
            let text = text.split("#[cfg(test)]\nmod tests {").next().unwrap_or_default();
            let moves = ["add_tick_callback", "TimedAnimation", "SpringAnimation"]
                .iter()
                .any(|call| text.contains(call));
            let follows_the_setting = text.contains("is_gtk_enable_animations");
            if moves
                && !follows_the_setting
                && !path.ends_with("snapshot.rs")
                && !path.ends_with("accessibility.rs")
            {
                found.push(path.display().to_string());
            }
        }
    }
    found
}

/// At 200% text the rows grow with their text and a dialog still fits an
/// 800 x 600 window.
///
/// parity: ACC-012
#[gtk::test]
fn large_text_grows_rows_and_dialogs_still_fit_800_by_600() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let skin = skin();
    let before = skin.text_size();
    let row = test
        .window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("the Home row");
    let dialog = Dialog::new(
        &test.window,
        "Replace or skip files",
        "2 items are already in Documents.",
    );
    dialog.add_text_field("Name", "Notes 2.txt");
    dialog.add_check_button("Do this for all conflicts", false);
    dialog.add_button("Replace", ButtonStyle::Accent);
    dialog.add_button("Skip duplicates", ButtonStyle::Bordered);
    dialog.add_cancel_button();
    let height_at = |percent: u32| {
        skin.set_text_size(TextSize::from_percent(percent));
        wait_for_frames(&test.window, 3);
        row.measure(gtk::Orientation::Vertical, -1).0
    };
    let normal_row = height_at(100);
    let large_row = height_at(200);
    let dialog_width = dialog.measure(gtk::Orientation::Horizontal, -1).0;
    let dialog_height = dialog.measure(gtk::Orientation::Vertical, 800).0;
    skin.set_text_size(before);
    dialog.finish();

    assert!(large_row > normal_row, "{large_row} > {normal_row}");
    assert!(dialog_width <= 800, "{dialog_width} pixels wide");
    assert!(dialog_height <= 600, "{dialog_height} pixels high");
}

/// At 200% text, the columns nobody resized and the navigation pane at
/// its default width grow with the text, so dates, types and folder names
/// are not cut short; back at 100% they return to their default widths.
/// A width the user dragged grows with the text too, as Windows grows it.
///
/// parity: ACC-012, VIEW-028
#[gtk::test]
fn large_text_widens_default_columns_and_the_navigation_pane() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.set_default_size(1800, 900);
    wait_for_frames(&test.window, 3);
    let skin = skin();
    let before = skin.text_size();
    let details = test.window.folder_pane().details();
    let width_of = |column: SortColumn| details.column(column).expect("a column").fixed_width();
    let widths = || {
        (
            width_of(SortColumn::Modified),
            width_of(SortColumn::Type),
            test.window.workspace().position(),
        )
    };
    assert_eq!(widths(), (176, 135, 210), "the 100% defaults");

    skin.set_text_size(TextSize::from_percent(200));
    wait_for_frames(&test.window, 3);
    let large = widths();
    // Name, which fills the rest, keeps room for names: the list scrolls
    // sideways instead.
    let header = details.header().expect("the column titles");
    let name_title = header.first_child().expect("Name's title");
    let name_room = name_title.width_request();
    let header_width = header.width();
    skin.set_text_size(TextSize::from_percent(100));
    wait_for_frames(&test.window, 3);
    let back = widths();
    details
        .column(SortColumn::Type)
        .expect("Type")
        .set_fixed_width(150);
    skin.set_text_size(TextSize::from_percent(200));
    wait_for_frames(&test.window, 3);
    let dragged = width_of(SortColumn::Type);
    skin.set_text_size(before);

    assert_eq!(large, (282, 270, 420), "twice the text, wider columns and pane");
    assert_eq!(name_room, 280, "Name keeps room for names");
    assert!(
        header_width >= 280 + 282 + 270,
        "the list scrolls sideways: {header_width}"
    );
    assert_eq!(back, (176, 135, 210));
    assert_eq!(
        dragged, 300,
        "a width the user chose grows with the text, as in Windows"
    );
}

/// A folder opened at 200% text starts with the wide defaults; Ctrl+0
/// then brings every column nobody resized, Size included, back to its
/// 100% default, not only those that were already open when the text
/// grew.
///
/// parity: ACC-012, VIEW-028
#[gtk::test]
fn columns_of_a_folder_opened_at_large_text_shrink_after_ctrl_0() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.set_default_size(1800, 900);
    wait_for_frames(&test.window, 3);
    let skin = skin();
    let before = skin.text_size();
    let details = test.window.folder_pane().details();
    let width_of = |column: SortColumn| details.column(column).expect("a column").fixed_width();
    let widths = || [SortColumn::Modified, SortColumn::Type, SortColumn::Size].map(width_of);
    let normal = widths();

    skin.set_text_size(TextSize::from_percent(200));
    wait_for_frames(&test.window, 3);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("Documents");
    test.wait_for_listing("Documents");
    wait_for_frames(&test.window, 3);
    let large = widths();
    test.activate(WindowAction::TextSize(crate::text_size::Step::Reset).name(), None);
    wait_for_frames(&test.window, 3);
    let back = widths();
    skin.set_text_size(before);

    assert!(
        large[0] > normal[0] && large[1] > normal[1],
        "{large:?} after {normal:?}"
    );
    assert!(
        large[2] > normal[2],
        "Size holds its larger text: {large:?} after {normal:?}"
    );
    assert_eq!(back, normal, "back at the 100% defaults");
}

/// A window that opens with 200% text already chosen starts with the
/// navigation pane and the columns nobody resized at their widths for
/// that text, not at the 100% defaults until the text size next changes.
///
/// parity: ACC-012, VIEW-028
#[gtk::test]
fn a_window_opened_at_large_text_starts_with_wide_defaults() {
    let skin = crate::theme::Skin::detached();
    skin.set_text_size(TextSize::from_percent(200));
    let fixture = Fixture::standard();
    let test = TestWindow::open_with_skin(&fixture.uri(), &skin);
    wait_for_frames(&test.window, 3);
    let details = test.window.folder_pane().details();
    let width_of = |column: SortColumn| details.column(column).expect("a column").fixed_width();

    assert_eq!(test.window.workspace().position(), 420, "the navigation pane");
    assert_eq!(width_of(SortColumn::Modified), 282);
    assert_eq!(width_of(SortColumn::Type), 270);
}

/// In a window too narrow for the navigation pane's 200% default, the pane
/// stops where the folder pane keeps its room, and still returns to 210
/// pixels at 100% text: it was at its default, cut short.
///
/// parity: ACC-012
#[gtk::test]
fn a_navigation_pane_cut_short_at_large_text_returns_to_210() {
    let skin = crate::theme::Skin::detached();
    let fixture = Fixture::standard();
    let test = TestWindow::open_with_skin(&fixture.uri(), &skin);
    test.window.set_default_size(640, 700);
    wait_for_frames(&test.window, 3);

    skin.set_text_size(TextSize::from_percent(200));
    wait_for_frames(&test.window, 3);
    let large = test.window.workspace().position();
    skin.set_text_size(TextSize::from_percent(100));
    wait_for_frames(&test.window, 3);

    assert!((211..420).contains(&large), "cut short at 200%: {large}");
    assert_eq!(test.window.workspace().position(), 210);
}

/// The desktop's text scaling (GTK's font resolution) grows the rows on
/// top of the app's own text size. The test follows the display's
/// settings as the application does at startup.
///
/// parity: ACC-013
#[gtk::test]
fn the_desktop_text_scaling_grows_rows_on_top_of_the_app_size() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let settings = gtk::Settings::default().expect("the test display has settings");
    let before = settings.gtk_xft_dpi();
    let watch = DesktopTextWatch::new(&settings, |desktop| skin().set_desktop_text(desktop));
    let row = test
        .window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("the Home row");
    let height_at = |xft_dpi: i32| {
        settings.set_gtk_xft_dpi(xft_dpi);
        wait_for_frames(&test.window, 3);
        row.measure(gtk::Orientation::Vertical, -1).0
    };
    let unscaled = height_at(96 * 1024);
    let large_text = height_at(144 * 1024);
    let (chosen, drawn) = (skin().text_size(), skin().drawn_text_size());
    settings.set_gtk_xft_dpi(before);
    drop(watch);
    skin().set_desktop_text(DesktopText::default());

    assert!(
        drawn.percent() > chosen.percent(),
        "{drawn:?} is drawn for {chosen:?}"
    );
    assert!(large_text > unscaled, "{large_text} > {unscaled}");
}

/// A command bar menu is a list of rows Enter activates.
///
/// parity: ACC-006
#[gtk::test]
fn menus_open_their_items_from_the_keyboard() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sort = super::support::menu_button_with_class(&test, "sort-command");
    let menu = app_menu(&sort);
    let rows = descendants::<gtk::ListBoxRow>(&menu);
    assert!(!rows.is_empty());
    assert!(rows.iter().all(gtk::ListBoxRow::is_activatable));
}

/// Alt pressed and released alone moves the keyboard to the command bar's
/// first button with its focus ring showing, as Windows' "Alt moves
/// focus to the app bar" (Microsoft's "Use a screen reader to explore and
/// navigate File Explorer"); Escape gives it back to the item it left.
/// Alt with another key is that shortcut alone: Alt+D still edits the
/// address and leaves the command bar alone. Real keys, on the private
/// display.
///
/// parity: ACC-015
#[gtk::test]
fn alt_alone_moves_the_keyboard_to_the_command_bar_and_escape_brings_it_back() {
    use crate::test_support::pointer::{screen_point, Pointer};
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.present();
    test.window.folder_pane().focus_view();
    let item = test.window.folder_pane().focused_position();
    let mut pointer = Pointer::connect();
    // Without a window manager the keyboard follows the pointer.
    let (x, y) = screen_point(&mut pointer, &test.window, (300.0, 300.0), (300, 300));
    pointer.move_to(x, y);
    wait_until("the window to have the keyboard", || test.window.is_active());

    pointer.tap_key(gdk::Key::Alt_L);
    wait_until("the command bar to take the keyboard", || {
        test.window.focused_region() == Some(Region::Commands)
    });
    let focused = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a focused control");
    let first = descendants::<gtk::Widget>(test.window.command_bar())
        .into_iter()
        .find(|widget| widget.is_focusable() && widget.is_mapped() && widget.is_sensitive())
        .expect("the command bar has a control");
    assert!(
        focused == first || focused.is_ancestor(&first),
        "the first control, {first:?}, has the keyboard, not {focused:?}"
    );
    assert!(
        gtk::prelude::GtkWindowExt::gets_focus_visible(&test.window),
        "the focus ring shows"
    );

    pointer.tap_key(gdk::Key::Escape);
    wait_until("the file list to take the keyboard back", || {
        test.window.focused_region() == Some(Region::Files)
    });
    assert_eq!(
        test.window.folder_pane().focused_position(),
        item,
        "the same item"
    );

    pointer.hold_key(gdk::Key::Alt_L);
    pointer.tap_key(gdk::Key::d);
    pointer.let_go_key(gdk::Key::Alt_L);
    wait_until("Alt+D to edit the address", || {
        test.window.focused_region() == Some(Region::Address)
    });
    wait_for(std::time::Duration::from_millis(200));
    assert_eq!(
        test.window.focused_region(),
        Some(Region::Address),
        "Alt with another key does not move the keyboard to the command bar"
    );

    // Once the keyboard has left the command bar Alt moved it to, Escape
    // there no longer jumps back to the address bar Alt took it from.
    pointer.tap_key(gdk::Key::Alt_L);
    wait_until("Alt to move the keyboard to the command bar again", || {
        test.window.focused_region() == Some(Region::Commands)
    });
    pointer.tap_key(gdk::Key::F6);
    wait_until("F6 to leave the command bar", || {
        test.window.focused_region() != Some(Region::Commands)
    });
    pointer.hold_key(gdk::Key::Shift_L);
    pointer.tap_key(gdk::Key::F6);
    pointer.let_go_key(gdk::Key::Shift_L);
    wait_until("Shift+F6 to come back to the command bar", || {
        test.window.focused_region() == Some(Region::Commands)
    });
    pointer.tap_key(gdk::Key::Escape);
    wait_for(std::time::Duration::from_millis(200));
    assert_ne!(
        test.window.focused_region(),
        Some(Region::Address),
        "Escape gives the keyboard back only straight after Alt"
    );
    // Back to the screen's middle, as the other real-pointer tests end.
    pointer.move_to(720, 500);
}

/// The file list is named "Items View", as Windows names Explorer's list,
/// not with the typing hint, which only describes it.
#[gtk::test]
fn the_file_list_is_named_items_view_and_described_by_the_hint() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the fixture folder");
    let view = test.window.folder_pane().view_widget();

    assert_eq!(crate::folder_view::cells::ITEMS_VIEW_NAME, "Items View");
    assert_eq!(
        crate::folder_view::cells::ITEMS_VIEW_HINT,
        "Folder contents — type a filename prefix to select"
    );
    for property in [
        gtk::AccessibleProperty::Label,
        gtk::AccessibleProperty::Description,
    ] {
        assert!(gtk::test_accessible_has_property(&view, property), "{property:?}");
    }
}
