// SPDX-License-Identifier: AGPL-3.0-only
//! Windows File Explorer's "Size all columns to fit": Ctrl+plus on the
//! number pad, and the column titles' menu, fit every shown details column
//! to its widest content and title (VIEW-072). Ctrl+plus on the main
//! keyboard keeps changing the text size.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::press_shortcut;
use crate::folder_view::column_widths;
use crate::folder_view::details::DetailsView;
use crate::folder_view::icon_size::IconSize;
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{application, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::folder_pane::FolderView;
use crate::window::menu_popover::{ItemAvailability, MenuPopover};
use crate::window::window_action::WindowAction;

/// A fictional name long enough to need a wider Name column.
const LONG_NAME: &str = "Quarterly planning notes for the example garden project, second draft.txt";

/// A folder with a short and a long name.
fn fixture() -> Fixture {
    let fixture = Fixture::empty();
    for name in ["a.txt", LONG_NAME] {
        fixture.write(name);
    }
    fixture
}

/// Each column the view shows, with its fixed width, in column order.
fn shown_widths(details: &DetailsView) -> Vec<(SortColumn, i32)> {
    SortColumn::ALL
        .into_iter()
        .filter_map(|column| Some((column, details.column(column)?)))
        .filter(|(_, view_column)| view_column.is_visible())
        .map(|(column, view_column)| (column, view_column.fixed_width()))
        .collect()
}

/// The width `column` takes when fitted: its widest text, or its title
/// when that is wider, within its limits.
fn fitted(details: &DetailsView, column: SortColumn) -> i32 {
    let mut widest = details.widest_text(column);
    if column == SortColumn::Name {
        widest += details.name_check_room();
    }
    let text = column_widths::fixed_width(column, Some(column_widths::fitted_width(column, widest)));
    text.max(column_widths::clamped_fixed_width(
        column,
        details.title_width(column),
    ))
}

/// Checks that every shown column of `details` is fitted.
fn assert_all_fitted(details: &DetailsView) {
    for (column, width) in shown_widths(details) {
        assert_eq!(width, fitted(details, column), "{column:?}");
        assert!(
            width >= details.title_width(column),
            "{column:?} shows its whole title"
        );
    }
}

/// Ctrl+plus on the number pad runs "Size all columns to fit", and only
/// that; Ctrl+plus and Ctrl+= keep making the text larger.
///
/// parity: VIEW-072, VIEW-043
#[gtk::test]
fn ctrl_keypad_plus_is_size_all_columns_to_fit_and_ctrl_plus_keeps_the_text_size() {
    let app = application();
    assert_eq!(
        app.accels_for_action(&WindowAction::FitAllColumns.detailed_name()),
        ["<Control>KP_Add"]
    );
    let runs = |accelerator: &str| -> Vec<String> {
        let actions = app.actions_for_accel(accelerator);
        actions.iter().map(ToString::to_string).collect()
    };
    assert_eq!(runs("<Primary>KP_Add"), ["win.fit-all-columns"]);
    assert_eq!(runs("<Primary>plus"), ["win.text-larger"]);
    assert_eq!(runs("<Primary>equal"), ["win.text-larger"]);
}

/// In Details, Ctrl+plus on the number pad fits every shown column to its
/// widest name or value, never narrower than its title: the long name
/// shows whole, Name stops filling the width, the text size stays, and
/// the widths are reported to be saved.
///
/// parity: VIEW-072
#[gtk::test]
fn ctrl_keypad_plus_fits_every_shown_details_column() {
    let fixture = fixture();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    assert_eq!(pane.view(), FolderView::Details);
    let details = pane.details().clone();
    wait_for_frames(&test.window, 3);
    let reported = Rc::new(Cell::new(false));
    let sink = Rc::clone(&reported);
    details.connect_columns_resized(move |_| sink.set(true));
    let text_size = test.window.skin().text_size();
    let name = details.column(SortColumn::Name).expect("a Name column");
    assert!(name.expands(), "Name fills the width before the fit");

    press_shortcut(&test, gdk::Key::KP_Add, gdk::ModifierType::CONTROL_MASK);

    assert_eq!(test.window.skin().text_size(), text_size, "the text size stays");
    assert!(!name.expands(), "a fitted Name has a width of its own");
    assert_all_fitted(&details);
    let short_only = column_widths::fitted_width(SortColumn::Name, 40.0);
    assert!(
        name.fixed_width() > i32::try_from(short_only).unwrap_or_default() + 200,
        "the long name widened Name to {}",
        name.fixed_width()
    );
    wait_until("the fitted widths to be reported", || reported.get());
}

/// Grouped, the group headings do not count, and the items still fit;
/// with the Details pane open the fit is the same.
///
/// parity: VIEW-072
#[gtk::test]
fn grouped_details_with_the_details_pane_open_fit_as_well() {
    let fixture = fixture();
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    test.open_details_pane();
    wait_for_frames(&test.window, 3);
    let details = test.window.folder_pane().details().clone();

    press_shortcut(&test, gdk::Key::KP_Add, gdk::ModifierType::CONTROL_MASK);

    assert_all_fitted(&details);
}

/// In the other views Ctrl+plus on the number pad changes nothing: not
/// the hidden details columns, not the text size, not the view.
///
/// parity: VIEW-072
#[gtk::test]
fn ctrl_keypad_plus_does_nothing_in_the_other_views() {
    let fixture = fixture();
    let test = TestWindow::open(&fixture.uri());
    let views = [
        FolderView::Icons(IconSize::LARGE),
        FolderView::Compact,
        FolderView::Tiles,
        FolderView::Content,
    ];
    for view in views {
        test.window.show_view(view);
        wait_for_frames(&test.window, 2);
        let details = test.window.folder_pane().details().clone();
        let before = shown_widths(&details);
        let text_size = test.window.skin().text_size();

        press_shortcut(&test, gdk::Key::KP_Add, gdk::ModifierType::CONTROL_MASK);

        assert_eq!(test.window.folder_pane().view(), view);
        assert_eq!(shown_widths(&details), before, "{view:?}");
        assert_eq!(test.window.skin().text_size(), text_size, "{view:?}");
    }
}

/// The column titles' menu starts as Windows 11's: "Size column to fit"
/// for the title it was opened on, "Size all columns to fit", then the
/// columns. Outside the titles there is no column to fit. Each item fits
/// its columns.
///
/// parity: VIEW-072, VIEW-033
#[gtk::test]
fn the_titles_menu_sizes_one_column_or_all_to_fit() {
    let fixture = fixture();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let details = test.window.folder_pane().details().clone();
    let header = details.header().expect("column titles");
    let size_title = std::iter::successors(header.first_child(), WidgetExt::next_sibling)
        .find(|title| {
            title.is_visible() && details.column_at(title_middle(title, &header)) == Some(SortColumn::Size)
        })
        .expect("a Size title");
    let x = title_middle(&size_title, &header);

    let menu = test.window.show_column_menu(&header, x, 4.0);
    let items = menu.items();
    let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels[..2], ["Size column to fit", "Size all columns to fit"]);
    assert_eq!(items[0].target, Some("size".to_variant()));
    assert_ne!(items[0].availability, ItemAvailability::Disabled);
    menu.popdown();

    let past_titles = f64::from(header.width()) + 50.0;
    let outside = test.window.show_column_menu(&header, past_titles, 4.0);
    assert_eq!(outside.items()[0].availability, ItemAvailability::Disabled);
    outside.popdown();

    let name = details.column(SortColumn::Name).expect("a Name column");
    test.activate("fit-column", Some("size"));
    assert_eq!(
        details
            .column(SortColumn::Size)
            .map(|column| column.fixed_width()),
        Some(fitted(&details, SortColumn::Size))
    );
    assert!(name.expands(), "Size column to fit leaves Name as it was");
    test.activate("fit-all-columns", None);
    assert_all_fitted(&details);
}

/// The middle of `title`, a position in `header`.
fn title_middle(title: &gtk::Widget, header: &gtk::Widget) -> f64 {
    let bounds = title.compute_bounds(header).expect("a laid-out title");
    f64::from(bounds.x() + bounds.width() / 2.0)
}

/// Where the pointer first looks for the window.
const PROBE: (i32, i32) = (8, 8);

/// The titles' menu open on `header`, if any.
fn open_titles_menu(header: &gtk::Widget) -> Option<MenuPopover> {
    std::iter::successors(header.first_child(), WidgetExt::next_sibling)
        .filter_map(|child| child.downcast::<MenuPopover>().ok())
        .find(WidgetExt::is_mapped)
}

/// A real right-click on a column title opens the titles' menu, and a real
/// click on "Size all columns to fit" fits the columns. The menu never
/// opened from a real right-click before: its gesture waited for the
/// press after the titles had it.
///
/// parity: VIEW-072, VIEW-033
#[gtk::test]
fn a_real_right_click_on_a_title_opens_the_menu_that_sizes_all_columns() {
    let fixture = fixture();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let details = test.window.folder_pane().details().clone();
    let header = details.header().expect("column titles");
    let size_title = std::iter::successors(header.first_child(), WidgetExt::next_sibling)
        .find(|title| {
            title.is_visible() && details.column_at(title_middle(title, &header)) == Some(SortColumn::Size)
        })
        .expect("a Size title");
    let on_title = screen_point(&mut pointer, &size_title, (20.0, 10.0), PROBE);

    pointer.move_to(on_title.0, on_title.1);
    pointer.press(Button::Secondary);
    pointer.release(Button::Secondary);
    wait_until("the titles' menu", || open_titles_menu(&header).is_some());
    let menu = open_titles_menu(&header).expect("the titles' menu");
    assert_eq!(menu.items()[0].target, Some("size".to_variant()));
    let row = menu.row_list().row_at_index(1).expect("Size all columns to fit");
    let middle = (20.0, f64::from(row.height()) / 2.0);
    let on_row = screen_point(&mut pointer, &row, middle, on_title);
    pointer.move_to(on_row.0, on_row.1);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);

    wait_until("the menu to close", || open_titles_menu(&header).is_none());
    assert_all_fitted(&details);
}

/// The real keys: Ctrl held with plus on the number pad fits the columns,
/// and Ctrl with plus on the main keyboard makes the text larger without
/// changing a column.
///
/// parity: VIEW-072, VIEW-043
#[gtk::test]
fn the_real_ctrl_keypad_plus_fits_and_the_main_ctrl_plus_resizes_text() {
    let fixture = fixture();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let details = test.window.folder_pane().details().clone();
    test.window.folder_pane().focus_view();
    let text_size = test.window.skin().text_size();

    pointer.hold_key(gdk::Key::Control_L);
    pointer.tap_key(gdk::Key::KP_Add);
    pointer.let_go_key(gdk::Key::Control_L);
    wait_until("the columns to fit", || {
        details
            .column(SortColumn::Name)
            .is_some_and(|name| !name.expands())
    });
    assert_all_fitted(&details);
    assert_eq!(test.window.skin().text_size(), text_size);

    let fitted_widths = shown_widths(&details);
    pointer.hold_key(gdk::Key::Control_L);
    pointer.tap_key(gdk::Key::equal);
    pointer.let_go_key(gdk::Key::Control_L);
    wait_until("the text to grow", || test.window.skin().text_size() != text_size);
    assert_eq!(shown_widths(&details), fitted_widths, "Ctrl+= fits nothing");
    test.activate("text-reset", None);
}
