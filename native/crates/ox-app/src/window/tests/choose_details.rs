// SPDX-License-Identifier: AGPL-3.0-only
//! The column titles' menu ends with "More...", which opens Windows 11
//! File Explorer's "Choose Details" window: the columns as check boxes,
//! Move Up, Move Down, Show and Hide, the selected column's width, and OK
//! and Cancel (VIEW-033, VIEW-034).

use gtk::gdk;
use gtk::prelude::*;
use ox_core::settings::Settings;

use super::name_conflicts::texts;
use crate::folder_view::column_widths;
use crate::folder_view::details::DetailsView;
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::choose_details::{ChooseDetails, DetailsChoice};
use crate::window::menu_popover::{ItemAvailability, MenuPopover};

/// The columns of a new window's details view, after Name.
const DEFAULT_CHOSEN: [SortColumn; 3] = [SortColumn::Modified, SortColumn::Type, SortColumn::Size];

/// The columns of `choice`'s rows, in order.
fn columns(choice: &DetailsChoice) -> Vec<SortColumn> {
    choice.rows().iter().map(|row| row.column).collect()
}

/// The list starts with the shown columns in their order, Name first,
/// then the others in alphabetical order, as Windows lists them. As in
/// Windows, Name cannot be hidden but moves down like any other row, and
/// only the first row cannot move up. A Name moved down is listed where it
/// is.
///
/// parity: VIEW-033, VIEW-034
#[test]
fn the_choice_lists_the_shown_columns_first_then_the_rest_alphabetically() {
    let choice = DetailsChoice::new(&DEFAULT_CHOSEN, |_| 120);
    assert_eq!(
        columns(&choice),
        [
            SortColumn::Name,
            SortColumn::Modified,
            SortColumn::Type,
            SortColumn::Size,
            SortColumn::Created,
            SortColumn::Extension,
            SortColumn::Owner,
            SortColumn::Permissions,
        ]
    );
    let shown: Vec<bool> = choice.rows().iter().map(|row| row.shown).collect();
    assert_eq!(shown, [true, true, true, true, false, false, false, false]);

    let other = DetailsChoice::new(&[SortColumn::Size, SortColumn::Owner], |_| 120);
    assert_eq!(
        columns(&other),
        [
            SortColumn::Name,
            SortColumn::Size,
            SortColumn::Owner,
            SortColumn::Created,
            SortColumn::Modified,
            SortColumn::Extension,
            SortColumn::Permissions,
            SortColumn::Type,
        ]
    );

    let last = choice.rows().len() - 1;
    assert!(
        !choice.can_move_up(0) && choice.can_move_down(0),
        "Name moves down only"
    );
    assert!(!choice.can_hide(0) && !choice.can_show(0), "Name always shows");
    assert!(choice.can_move_up(1), "a row may go above Name");
    assert!(choice.can_move_down(1) && choice.can_hide(1) && !choice.can_show(1));
    assert!(choice.can_move_up(last) && !choice.can_move_down(last));
    assert!(choice.can_show(4) && !choice.can_hide(4));

    let moved = DetailsChoice::new(
        &[SortColumn::Modified, SortColumn::Name, SortColumn::Size],
        |_| 120,
    );
    assert_eq!(
        columns(&moved)[..3],
        [SortColumn::Modified, SortColumn::Name, SortColumn::Size]
    );
    assert!(moved.rows()[1].shown && !moved.can_hide(1));
}

/// Show, Hide and the moves change the columns chosen. Name stays shown;
/// moved down, it is chosen where it is, and back at the top it is left
/// out, as the columns were always saved.
///
/// parity: VIEW-033, VIEW-034
#[test]
fn showing_hiding_and_moving_change_the_columns_chosen() {
    let mut choice = DetailsChoice::new(&DEFAULT_CHOSEN, |_| 120);
    choice.set_shown(4, true);
    assert_eq!(choice.move_up(4), 3);
    assert_eq!(
        choice.chosen(),
        [
            SortColumn::Modified,
            SortColumn::Type,
            SortColumn::Created,
            SortColumn::Size
        ]
    );
    choice.set_shown(2, false);
    assert_eq!(
        choice.chosen(),
        [SortColumn::Modified, SortColumn::Created, SortColumn::Size]
    );
    assert_eq!(choice.move_down(1), 2);
    assert_eq!(choice.move_down(2), 3);
    assert_eq!(
        choice.chosen(),
        [SortColumn::Created, SortColumn::Modified, SortColumn::Size]
    );
    choice.set_shown(0, false);
    assert!(choice.rows()[0].shown, "Name cannot be hidden");
    assert_eq!(choice.move_up(0), 0, "the first row cannot move up");
    assert_eq!(choice.move_down(0), 1, "Name moves down");
    // Only the hidden Type is above it: Name still comes first.
    assert_eq!(
        choice.chosen(),
        [SortColumn::Created, SortColumn::Modified, SortColumn::Size]
    );
    assert_eq!(choice.move_down(1), 2);
    assert_eq!(
        choice.chosen(),
        [
            SortColumn::Created,
            SortColumn::Name,
            SortColumn::Modified,
            SortColumn::Size
        ]
    );
    assert_eq!(choice.move_up(2), 1);
    assert_eq!(choice.move_up(1), 0, "Name back at the top");
    assert_eq!(choice.move_down(0), 1);
    assert_eq!(
        choice.chosen(),
        [SortColumn::Created, SortColumn::Modified, SortColumn::Size]
    );
}

/// The width field takes digits only; a width is kept within the
/// column's limits, so 0 is its narrowest and a huge number its widest,
/// and an empty field changes nothing.
///
/// parity: VIEW-033
#[test]
fn widths_are_kept_within_the_columns_limits() {
    let mut choice = DetailsChoice::new(&DEFAULT_CHOSEN, |_| 120);
    let size = 3;
    let limits = column_widths::width_limits(SortColumn::Size);
    assert_eq!(choice.set_width_text(size, "200"), Some(200));
    assert_eq!(choice.set_width_text(size, "0"), Some(*limits.start()));
    assert_eq!(choice.set_width_text(size, "99999999999999"), Some(*limits.end()));
    assert_eq!(choice.set_width_text(size, ""), None);
    assert_eq!(choice.set_width_text(size, "12a"), None);
    assert_eq!(choice.rows()[size].width, *limits.end());
    assert_eq!(choice.changed_widths(), [(SortColumn::Size, *limits.end())]);
    assert_eq!(super::super::choose_details::digits_only("1a2 3-"), "123");
}

/// A window on a folder with a few fictional files.
fn window_with_files() -> (Fixture, TestWindow) {
    let fixture = Fixture::empty();
    for name in ["Garden plan.txt", "Recipe ideas.txt", "Trip notes.txt"] {
        fixture.write(name);
    }
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    (fixture, test)
}

/// Opens Choose Details from the titles' menu's "More..." and returns it.
fn open_from_more(test: &TestWindow) -> ChooseDetails {
    let details = test.window.folder_pane().details().clone();
    let header = details.header().expect("column titles");
    let menu = test.window.show_column_menu(&header, 20.0, 4.0);
    let items = menu.items();
    let more = items.last().expect("the menu's last item");
    assert_eq!(more.label, "More...");
    assert_ne!(more.availability, ItemAvailability::Disabled);
    menu.popdown();
    test.activate("choose-details", None);
    wait_until("Choose Details", || {
        ChooseDetails::shown_over(&test.window).is_some()
    });
    let shown = ChooseDetails::shown_over(&test.window).expect("Choose Details");
    wait_for_frames(&shown.dialog, 3);
    shown
}

/// The window is Windows 11's "Choose Details": its title and sentence,
/// "Details:" over the columns as check boxes (the shown ones first),
/// Move Up, Move Down, Show and Hide beside them, the selected column's
/// width under them and OK and Cancel at the bottom. It opens on Name,
/// drawn like the other rows, which can only move down, and it does not
/// resize.
///
/// parity: VIEW-033, VIEW-034
#[gtk::test]
fn more_opens_choose_details_as_windows_draws_it() {
    let (_fixture, test) = window_with_files();
    let shown = open_from_more(&test);

    assert_eq!(shown.dialog.title().as_deref(), Some("Choose Details"));
    assert!(!shown.dialog.is_resizable(), "a fixed-size window");
    let words = texts(&shown.dialog);
    for text in [
        "Choose Details",
        "Select the details you want to display for the items in this folder.",
        "Details:",
        "Width of selected column (in pixels):",
    ] {
        assert!(words.contains(&text.to_owned()), "{text:?} in {words:?}");
    }
    let labels = |button: &gtk::Button| button.label().map(String::from).unwrap_or_default();
    assert_eq!(
        [&shown.move_up, &shown.move_down, &shown.show, &shown.hide].map(labels),
        ["Move Up", "Move Down", "Show", "Hide"]
    );
    assert_eq!([&shown.ok, &shown.cancel].map(labels), ["OK", "Cancel"]);
    assert_eq!(
        shown.row_states(),
        [
            ("Name".to_owned(), true, true),
            ("Date modified".to_owned(), true, true),
            ("Type".to_owned(), true, true),
            ("Size".to_owned(), true, true),
            ("Date created".to_owned(), false, true),
            ("File extension".to_owned(), false, true),
            ("Owner".to_owned(), false, true),
            ("Permissions".to_owned(), false, true),
        ]
    );
    assert_eq!(shown.selected_index(), Some(0));
    let sensitive = |shown: &ChooseDetails| {
        [&shown.move_up, &shown.move_down, &shown.show, &shown.hide].map(WidgetExt::is_sensitive)
    };
    assert_eq!(sensitive(&shown), [false, true, false, false], "Name");
    let name_width: u32 = shown.width.text().parse().expect("Name's width in pixels");
    assert!(name_width >= 140, "Name's width on screen, {name_width}");

    let name_check = descendants::<gtk::CheckButton>(&shown.list)
        .into_iter()
        .next()
        .expect("Name's check box");
    name_check.set_active(false);
    assert!(name_check.is_active(), "a click leaves Name checked");
    assert!(shown.row_states()[0].1, "Name stays checked");
    shown.select(1);
    assert_eq!(sensitive(&shown), [true, true, false, true], "Date modified");
    shown.select(4);
    assert_eq!(sensitive(&shown), [true, true, true, false], "Date created");
    shown.select(7);
    assert_eq!(sensitive(&shown), [true, false, true, false], "Permissions");
    let size = test
        .window
        .folder_pane()
        .details()
        .column(SortColumn::Size)
        .expect("Size");
    shown.select(3);
    assert_eq!(
        shown.width.text().parse::<f64>().ok(),
        column_widths::saved_width(SortColumn::Size, size.fixed_width())
    );

    let size_before = shown.dialog.default_size();
    shown.cancel.emit_clicked();
    wait_until("Choose Details to close", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    let again = open_from_more(&test);
    assert_eq!(again.dialog.default_size(), size_before, "the same size again");
    again.cancel.emit_clicked();
}

/// OK applies the columns shown, their order and the width typed, and
/// saves them for the folder, as the titles' menu does; Cancel and
/// Escape change nothing, and reopening shows the columns as they are.
///
/// parity: VIEW-033, VIEW-034
#[gtk::test]
fn ok_applies_the_choice_and_cancel_discards_it() {
    let (fixture, test) = window_with_files();
    let details = test.window.folder_pane().details().clone();
    let created = details.column(SortColumn::Created).expect("Date created");
    let created_before = created.fixed_width();

    let change = |shown: &ChooseDetails| {
        shown.select(4);
        shown.show.emit_clicked();
        shown.move_up.emit_clicked();
        shown.width.set_text("222");
        shown.select(1);
        shown.hide.emit_clicked();
    };
    let shown = open_from_more(&test);
    change(&shown);
    assert_eq!(shown.selected_index(), Some(1));
    shown.cancel.emit_clicked();
    wait_until("Cancel to close it", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    assert_eq!(details.chosen_columns(), DEFAULT_CHOSEN);
    assert_eq!(created.fixed_width(), created_before);

    let shown = open_from_more(&test);
    change(&shown);
    shown.dialog.close();
    wait_until("closing to cancel it", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    assert_eq!(details.chosen_columns(), DEFAULT_CHOSEN);

    let shown = open_from_more(&test);
    assert_eq!(shown.row_states()[4], ("Date created".to_owned(), false, true));
    change(&shown);
    shown.ok.emit_clicked();
    wait_until("OK to close it", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    assert_eq!(
        details.chosen_columns(),
        [SortColumn::Type, SortColumn::Created, SortColumn::Size]
    );
    assert_eq!(
        created.fixed_width(),
        column_widths::fixed_width(SortColumn::Created, Some(222))
    );
    assert!(created.is_visible());
    let saved = || {
        Settings::open(test.settings_directory())
            .data()
            .preferences
            .view_for(&fixture.uri())
    };
    wait_until("the folder's columns to be saved", || {
        saved().details_columns.as_deref()
            == Some(&["type".to_owned(), "created".to_owned(), "size".to_owned()][..])
    });
    assert_eq!(saved().column_widths.and_then(|widths| widths.created), Some(222));

    let shown = open_from_more(&test);
    assert_eq!(
        shown
            .row_states()
            .into_iter()
            .map(|(label, checked, _)| (label, checked))
            .collect::<Vec<_>>()[..5],
        [
            ("Name".to_owned(), true),
            ("Type".to_owned(), true),
            ("Date created".to_owned(), true),
            ("Size".to_owned(), true),
            ("Date modified".to_owned(), false),
        ]
    );
    shown.select(2);
    assert_eq!(shown.width.text(), "222");
    shown.cancel.emit_clicked();
}

/// Name moved down with Move Down shows after the column now above it,
/// is saved there for the folder and is listed there when Choose Details
/// opens again, as in Windows.
///
/// parity: VIEW-033, VIEW-034
#[gtk::test]
fn name_moved_down_shows_after_the_column_above_it() {
    let (fixture, test) = window_with_files();
    let details = test.window.folder_pane().details().clone();
    let shown = open_from_more(&test);
    shown.select(0);
    shown.move_down.emit_clicked();
    assert_eq!(shown.selected_index(), Some(1));
    assert!(shown.move_up.is_sensitive() && !shown.hide.is_sensitive());
    shown.ok.emit_clicked();
    wait_until("OK to close it", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    assert_eq!(
        details.chosen_columns(),
        [
            SortColumn::Modified,
            SortColumn::Name,
            SortColumn::Type,
            SortColumn::Size
        ]
    );
    let visible: Vec<SortColumn> = details
        .column_order()
        .into_iter()
        .filter(|column| {
            details
                .column(*column)
                .is_some_and(|view_column| view_column.is_visible())
        })
        .collect();
    assert_eq!(
        visible,
        [
            SortColumn::Modified,
            SortColumn::Name,
            SortColumn::Type,
            SortColumn::Size
        ]
    );
    wait_until("the folder's columns to be saved", || {
        Settings::open(test.settings_directory())
            .data()
            .preferences
            .view_for(&fixture.uri())
            .details_columns
            .as_deref()
            == Some(
                &[
                    "modified".to_owned(),
                    "name".to_owned(),
                    "type".to_owned(),
                    "size".to_owned(),
                ][..],
            )
    });
    let again = open_from_more(&test);
    let labels: Vec<String> = again
        .row_states()
        .into_iter()
        .map(|(label, _, _)| label)
        .collect();
    assert_eq!(labels[..2], ["Date modified".to_owned(), "Name".to_owned()]);
    // Back to the top: the columns are saved as they always were.
    again.select(1);
    again.move_up.emit_clicked();
    again.ok.emit_clicked();
    wait_until("Name back in front", || {
        details.chosen_columns() == [SortColumn::Modified, SortColumn::Type, SortColumn::Size]
    });
}

/// With one view for every folder, OK saves the columns for all of them,
/// as the titles' menu does.
///
/// parity: VIEW-033
#[gtk::test]
fn ok_saves_the_shared_view_when_folders_share_one() {
    let (_fixture, test) = window_with_files();
    test.share_one_view();
    let shown = open_from_more(&test);
    shown.select(6);
    shown.show.emit_clicked();
    shown.ok.emit_clicked();
    wait_until("the shared columns to be saved", || {
        test.context
            .settings_data()
            .preferences
            .view_options
            .details_columns
            == ["modified", "type", "size", "owner"]
    });
}

/// The width field takes digits only, 0 gives the narrowest width and a
/// huge number the widest.
///
/// parity: VIEW-033
#[gtk::test]
fn the_width_field_takes_digits_and_keeps_the_limits() {
    let (_fixture, test) = window_with_files();
    let details = test.window.folder_pane().details().clone();
    let limits = column_widths::width_limits(SortColumn::Type);
    for (typed, expected) in [("0", *limits.start()), ("99999999", *limits.end())] {
        let shown = open_from_more(&test);
        shown.select(2);
        shown.width.set_text("");
        shown.width.insert_text("ab", &mut 0);
        assert_eq!(shown.width.text(), "", "letters are not taken");
        shown.width.set_text(typed);
        shown.ok.emit_clicked();
        wait_until("OK to close it", || {
            ChooseDetails::shown_over(&test.window).is_none()
        });
        assert_eq!(
            details
                .column(SortColumn::Type)
                .map(|column| column.fixed_width()),
            Some(column_widths::fixed_width(SortColumn::Type, Some(expected))),
            "{typed}"
        );
    }
}

/// The titles' menu open on `details`, if any.
fn open_titles_menu(details: &DetailsView) -> Option<MenuPopover> {
    std::iter::successors(details.first_child(), WidgetExt::next_sibling)
        .filter_map(|child| child.downcast::<MenuPopover>().ok())
        .find(WidgetExt::is_mapped)
}

/// With the real keyboard, once a right-click on a title has opened the
/// titles' menu: Up and Enter choose "More...", the arrows move through
/// the columns, Space checks one, Tab reaches Move Up, which Space
/// presses, Escape cancels, and Enter in the list is OK.
///
/// parity: VIEW-033, VIEW-034
#[gtk::test]
fn choose_details_works_with_the_keyboard_alone() {
    let mut pointer = Pointer::connect();
    let (_fixture, test) = window_with_files();
    let details = test.window.folder_pane().details().clone();
    let header = details.header().expect("column titles");
    let title = header.first_child().expect("the Name title");
    let on_title = screen_point(&mut pointer, &title, (20.0, 10.0), (720, 500));

    let open_by_keys = |pointer: &mut Pointer| {
        pointer.move_to(on_title.0, on_title.1);
        pointer.press(Button::Secondary);
        pointer.release(Button::Secondary);
        wait_until("the titles' menu", || open_titles_menu(&details).is_some());
        let menu = open_titles_menu(&details).expect("the titles' menu");
        wait_for_frames(&menu, 3);
        // Up from the first item wraps to the last, "More...".
        pointer.tap_key(gdk::Key::Up);
        wait_for_frames(&menu, 2);
        pointer.tap_key(gdk::Key::Return);
        wait_until("Choose Details", || {
            ChooseDetails::shown_over(&test.window).is_some()
        });
        let shown = ChooseDetails::shown_over(&test.window).expect("Choose Details");
        wait_for_frames(&shown.dialog, 4);
        // The keyboard goes to the window under the pointer.
        let inside = screen_point(pointer, &shown.dialog, (30.0, 30.0), on_title);
        pointer.move_to(inside.0, inside.1);
        wait_for_frames(&shown.dialog, 2);
        shown
    };
    let change_by_keys = |pointer: &mut Pointer, shown: &ChooseDetails| {
        for _ in 0..4 {
            pointer.tap_key(gdk::Key::Down);
        }
        wait_until("Date created selected", || shown.selected_index() == Some(4));
        pointer.tap_key(gdk::Key::space);
        wait_until("Date created checked", || shown.row_states()[4].1);
        pointer.tap_key(gdk::Key::Tab);
        wait_for_frames(&shown.dialog, 2);
        wait_until("Move Up focused", || shown.move_up.has_focus());
        pointer.tap_key(gdk::Key::space);
        wait_until("Date created moved up", || shown.selected_index() == Some(3));
    };

    let shown = open_by_keys(&mut pointer);
    assert_eq!(shown.selected_index(), Some(0));
    change_by_keys(&mut pointer, &shown);
    pointer.tap_key(gdk::Key::Escape);
    wait_until("Escape to close it", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    assert_eq!(details.chosen_columns(), DEFAULT_CHOSEN);

    let shown = open_by_keys(&mut pointer);
    change_by_keys(&mut pointer, &shown);
    // Back to the list, where Enter is OK.
    pointer.hold_key(gdk::Key::Shift_L);
    pointer.tap_key(gdk::Key::Tab);
    pointer.let_go_key(gdk::Key::Shift_L);
    wait_for_frames(&shown.dialog, 2);
    wait_until("the list focused", || shown.list.focus_child().is_some());
    pointer.tap_key(gdk::Key::Return);
    wait_until("Enter to close it", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    assert_eq!(
        details.chosen_columns(),
        [
            SortColumn::Modified,
            SortColumn::Type,
            SortColumn::Created,
            SortColumn::Size
        ]
    );
    pointer.move_to(720, 500);
}

/// A real right-click on the empty part of the titles' row opens the
/// menu too, with Size Column to Fit grayed, and a real click on
/// "More..." opens Choose Details.
///
/// parity: VIEW-033
#[gtk::test]
fn a_real_click_on_more_opens_choose_details() {
    let mut pointer = Pointer::connect();
    let (_fixture, test) = window_with_files();
    let details = test.window.folder_pane().details().clone();
    let header = details.header().expect("column titles");
    // Name fills the spare width; with Name alone, fitted, room is left.
    for column in ["modified", "type", "size"] {
        test.activate("details-column", Some(column));
    }
    test.activate("fit-all-columns", None);
    wait_for_frames(&test.window, 4);
    let column_view = details.column_view().clone();
    let titles_end = std::iter::successors(header.first_child(), WidgetExt::next_sibling)
        .filter(WidgetExt::is_visible)
        .filter_map(|title| title.compute_bounds(&column_view))
        .map(|bounds| bounds.x() + bounds.width())
        .fold(0.0_f32, f32::max);
    let past_titles = f64::from(titles_end) + 40.0;
    assert!(
        past_titles + 20.0 < f64::from(column_view.width()),
        "room after the titles"
    );
    assert_eq!(details.column_at(past_titles), None);
    let on_header = screen_point(&mut pointer, &column_view, (past_titles, 10.0), (720, 500));
    pointer.move_to(on_header.0, on_header.1);
    pointer.press(Button::Secondary);
    pointer.release(Button::Secondary);
    wait_until("the titles' menu", || open_titles_menu(&details).is_some());
    let menu = open_titles_menu(&details).expect("the titles' menu");
    assert_eq!(menu.items()[0].availability, ItemAvailability::Disabled);
    wait_for_frames(&test.window, 3);
    wait_for_frames(&menu, 3);
    let list = menu.row_list();
    let last = descendants::<gtk::ListBoxRow>(&list).len() - 1;
    let row = list
        .row_at_index(i32::try_from(last).expect("a few rows"))
        .expect("More...");
    let middle = (20.0, f64::from(row.height()) / 2.0);
    let on_row = screen_point(&mut pointer, &row, middle, on_header);
    pointer.move_to(on_row.0, on_row.1);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    wait_until("Choose Details", || {
        ChooseDetails::shown_over(&test.window).is_some()
    });
    let shown = ChooseDetails::shown_over(&test.window).expect("Choose Details");
    shown.cancel.emit_clicked();
    wait_until("it to close", || {
        ChooseDetails::shown_over(&test.window).is_none()
    });
    pointer.move_to(720, 500);
}

/// Opens the titles' menu with a real right-click at `x` in `widget`
/// and returns it. On this display without a window manager, the first
/// click after a menu closed is sometimes swallowed, so a click that
/// opens nothing is made once more.
fn right_click_menu(
    pointer: &mut Pointer,
    details: &DetailsView,
    widget: &gtk::Widget,
    x: f64,
) -> MenuPopover {
    let point = screen_point(pointer, widget, (x, 10.0), (720, 500));
    for _ in 0..2 {
        pointer.move_to(point.0, point.1);
        pointer.press(Button::Secondary);
        pointer.release(Button::Secondary);
        for _ in 0..40 {
            if open_titles_menu(details).is_some() {
                break;
            }
            crate::test_support::harness::wait_for(std::time::Duration::from_millis(50));
        }
        if open_titles_menu(details).is_some() {
            break;
        }
    }
    let menu = open_titles_menu(details).expect("the titles' menu");
    wait_for_frames(&menu, 3);
    menu
}

/// Closes the open titles' menu with a real Escape.
fn escape_menu(pointer: &mut Pointer, details: &DetailsView) {
    pointer.tap_key(gdk::Key::Escape);
    wait_until("the menu to close", || open_titles_menu(details).is_none());
}

/// With the real pointer: a right-click on each column title opens the
/// titles' menu fitting that column, and a click on each column's row
/// shows or hides it; Name's row is grayed and a click on it changes
/// nothing.
///
/// parity: VIEW-033
#[gtk::test]
fn real_clicks_on_every_title_and_every_column_row() {
    let mut pointer = Pointer::connect();
    let (_fixture, test) = window_with_files();
    let details = test.window.folder_pane().details().clone();
    let header = details.header().expect("column titles");
    for column in [
        SortColumn::Name,
        SortColumn::Modified,
        SortColumn::Type,
        SortColumn::Size,
    ] {
        let x = details.title_middle(column).expect("a shown title");
        let menu = right_click_menu(&mut pointer, &details, &header, x);
        assert_eq!(
            menu.items()[0].target,
            Some(column.as_str().to_variant()),
            "{column:?}"
        );
        escape_menu(&mut pointer, &details);
    }

    let mut expected = DEFAULT_CHOSEN.to_vec();
    let toggles = [
        SortColumn::Created,
        SortColumn::Extension,
        SortColumn::Owner,
        SortColumn::Permissions,
        SortColumn::Modified,
        SortColumn::Type,
        SortColumn::Size,
        SortColumn::Name,
    ];
    for column in toggles {
        let x = details.title_middle(SortColumn::Name).expect("the Name title");
        let menu = right_click_menu(&mut pointer, &details, &header, x);
        let index = menu
            .items()
            .iter()
            .position(|item| item.label == column.label())
            .expect("the column's row");
        let row = menu
            .row_list()
            .row_at_index(i32::try_from(index).expect("a few rows"))
            .expect("the row");
        let middle = (30.0, f64::from(row.height()) / 2.0);
        let on_row = screen_point(&mut pointer, &row, middle, (720, 500));
        pointer.move_to(on_row.0, on_row.1);
        pointer.press(Button::Primary);
        pointer.release(Button::Primary);
        if column == SortColumn::Name {
            assert!(!row.is_sensitive(), "Name is grayed");
            escape_menu(&mut pointer, &details);
        } else {
            wait_until("the menu to close", || open_titles_menu(&details).is_none());
            if expected.contains(&column) {
                expected.retain(|shown| *shown != column);
            } else {
                expected.push(column);
            }
        }
        wait_until(&format!("{column:?} toggled"), || {
            details.chosen_columns() == expected
        });
        let shown = details
            .column(column)
            .is_some_and(|view_column| view_column.is_visible());
        assert_eq!(
            shown,
            column == SortColumn::Name || expected.contains(&column),
            "{column:?}"
        );
    }
    pointer.move_to(720, 500);
}

/// Search results and the Recycle Bin list in Details too, and their
/// titles open the same menu.
///
/// parity: VIEW-033
#[gtk::test]
fn the_titles_menu_opens_in_search_results_and_the_recycle_bin() {
    super::file_ops_support::require_private_trash();
    let (fixture, test) = window_with_files();
    let check_menu = |what: &str| {
        let details = test.window.folder_pane().details().clone();
        let header = details.header().expect("column titles");
        let x = details.title_middle(SortColumn::Name).expect("the Name title");
        let menu = test.window.show_column_menu(&header, x, 4.0);
        let labels: Vec<String> = menu.items().into_iter().map(|item| item.label).collect();
        assert_eq!(
            labels.first().map(String::as_str),
            Some("Size Column to Fit"),
            "{what}"
        );
        assert_eq!(labels.last().map(String::as_str), Some("More..."), "{what}");
        menu.popdown();
    };
    test.search_for("Garden");
    wait_until("the search result", || test.names() == ["Garden plan.txt"]);
    check_menu("search results");

    gtk::gio::File::for_path(fixture.path("Trip notes.txt"))
        .trash(gtk::gio::Cancellable::NONE)
        .expect("the private Recycle Bin");
    test.window
        .navigate(ox_core::location::TRASH_URI)
        .expect("the Recycle Bin");
    wait_until("the trashed file", || {
        test.names().iter().any(|name| name == "Trip notes.txt")
    });
    wait_for_frames(&test.window, 3);
    check_menu("the Recycle Bin");
}

/// A way to close Choose Details, and its name.
type Closing = (&'static str, fn(&ChooseDetails));

/// Choose Details is freed once it closes, by OK, Cancel or Escape:
/// its controls hold no callback that keeps the window itself alive, so
/// opening it again and again does not keep every earlier one.
///
/// parity: VIEW-033
#[gtk::test]
fn choose_details_is_freed_once_it_closes() {
    let (_fixture, test) = window_with_files();
    let closings: [Closing; 3] = [
        ("OK", |shown| shown.ok.emit_clicked()),
        ("Cancel", |shown| shown.cancel.emit_clicked()),
        ("Escape", |shown| shown.dialog.close()),
    ];
    for (how, close) in closings {
        let gone = {
            let shown = open_from_more(&test);
            shown.select(4);
            shown.show.emit_clicked();
            shown.move_up.emit_clicked();
            shown.width.set_text("150");
            let gone = shown.dialog.downgrade();
            close(&shown);
            gone
        };
        wait_until(&format!("Choose Details to close with {how}"), || {
            ChooseDetails::shown_over(&test.window).is_none()
        });
        wait_until(&format!("Choose Details to be freed after {how}"), || {
            gone.upgrade().is_none()
        });
    }
}
