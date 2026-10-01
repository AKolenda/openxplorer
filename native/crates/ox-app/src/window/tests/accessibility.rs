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
use crate::folder_view::column_widths;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::resizer_control::ResizerControl;
use crate::test_support::harness::{
    application, descendants, skin, wait_for_frames, wait_until, Fixture, TestWindow,
};
use crate::text_size::TextSize;
use crate::window::dialog::{ButtonStyle, Dialog};

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
    for count in [gtk::AccessibleRelation::RowCount, gtk::AccessibleRelation::ColCount] {
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
    let titles = crate::folder_view::column_titles::title_buttons(&details);
    let roles: Vec<gtk::AccessibleRole> = titles.iter().map(AccessibleExt::accessible_role).collect();
    assert!(
        roles
            .iter()
            .all(|role| *role == gtk::AccessibleRole::ColumnHeader),
        "{roles:?}"
    );
    let resizers = descendants::<ResizerControl>(&details);
    assert_eq!(resizers.len(), titles.len(), "every column has its resizer");
    let size = resizers.last().expect("the Size column's resizer");
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
    dialog.add_button("Save", ButtonStyle::Primary);
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
    let type_title = &titles[3];
    assert!(titles.iter().all(WidgetExt::is_focusable));
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

/// Every motion in the skin is a CSS transition or animation, which GTK
/// skips while the desktop turns animations off; the app never turns them
/// back on.
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
        "only the snapshot hook and the tests draw frame by frame"
    );
}

/// The app's source files, outside the snapshot hook and the test
/// support, that move something frame by frame, which GTK's animation
/// setting would not stop.
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
            let moves = ["add_tick_callback", "TimedAnimation", "SpringAnimation"]
                .iter()
                .any(|call| text.contains(call));
            if moves && !path.ends_with("snapshot.rs") && !path.ends_with("accessibility.rs") {
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
    dialog.add_button("Replace", ButtonStyle::Primary);
    dialog.add_button("Skip duplicates", ButtonStyle::Standard);
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
