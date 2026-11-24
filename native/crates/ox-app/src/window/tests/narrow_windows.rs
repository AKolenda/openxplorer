// SPDX-License-Identifier: AGPL-3.0-only
//! Narrow windows, against the `@media(max-width)` rules of
//! `v2.0.0:desktop/ui/style.css`: the details pane gives way at 960 pixels
//! without changing the preference, tabs narrow, and a compact window
//! drops the search box, some commands and two columns.

use gtk::prelude::*;

use super::geometry::{bounds, button_for, laid_out, laid_out_with_details_pane};
use crate::folder_view::sorting::SortColumn;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// Resizes the window to `width` pixels and waits until it is laid out.
pub(super) fn resize(test: &TestWindow, width: i32) {
    test.window.set_default_size(width, 720);
    wait_until("the new width", || (test.window.width() - width).abs() <= 8);
    wait_for_frames(&test.window, 3);
}

/// parity: LOOK-022
#[gtk::test]
fn the_details_pane_gives_way_below_961_pixels_and_comes_back() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    let pane = test.window.details_pane();
    assert!(pane.is_visible());
    resize(&test, 940);
    assert!(!pane.is_visible(), "no room for the pane");
    let switched_on = test
        .window
        .action_state("details-pane")
        .and_then(|state| state.get::<bool>());
    assert_eq!(switched_on, Some(true), "still switched on");
    let tab = test.window.tab_strip().tab_list().first_child().expect("a tab");
    assert_eq!(bounds(&test, &tab).width, 180, "narrower tabs");
    resize(&test, 1320);
    assert!(pane.is_visible(), "the pane comes back");
}

/// parity: LOOK-022
#[gtk::test]
fn a_compact_window_drops_the_search_box_some_commands_and_two_columns() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    resize(&test, 672);
    assert!(!test.window.search_box().is_visible());
    for action in ["win.cut", "win.rename", "win.share"] {
        assert!(!button_for(&test, action).is_visible(), "{action} is hidden");
    }
    assert!(button_for(&test, "win.copy").is_visible(), "Copy stays");
    let details = test.window.folder_pane().details();
    for column in [SortColumn::Modified, SortColumn::Type] {
        let shown = details.column(column).is_some_and(|column| column.is_visible());
        assert!(!shown, "{column:?} is hidden");
    }
    resize(&test, 1320);
    assert!(test.window.search_box().is_visible());
    assert!(button_for(&test, "win.cut").is_visible());
}

/// How a folder pane's Details view lays its columns out: the width its
/// columns take, the width shown, and Name's width, title included.
struct ColumnRoom {
    columns: f64,
    shown: f64,
    name: i32,
}

/// The column room of `pane`'s Details view.
fn column_room(test: &TestWindow, pane: &crate::window::folder_pane::FolderPane) -> ColumnRoom {
    let details = pane.details();
    let view = details.column_view();
    let scroller = view
        .parent()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("the view scrolls");
    let header = view.first_child().expect("the header");
    let name_title = header.first_child().expect("the Name title");
    ColumnRoom {
        columns: scroller.hadjustment().upper(),
        shown: scroller.hadjustment().page_size(),
        name: bounds(test, &name_title).width,
    }
}

/// Name, the column that takes the room left, gives way first when the
/// list is narrower than its columns: down to 140 pixels (plus the
/// 14-pixel end), and only then does the list scroll sideways. With the
/// details pane open in a 1000-pixel window, the list's columns all fit:
/// before, Name kept 260 pixels and Type and Size were cut off at the
/// pane's edge, reachable only by a scroll bar that shows only under the
/// pointer.
///
/// parity: LOOK-022
#[gtk::test]
fn the_columns_fit_beside_the_details_pane_and_name_gives_way_first() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    resize(&test, 1000);
    let pane = test.window.details_pane();
    assert!(pane.is_visible(), "the pane is open");
    let room = column_room(&test, test.window.folder_pane());
    assert!(
        room.columns <= room.shown + 1.0,
        "every column shows beside the pane: {} of {} pixels",
        room.shown,
        room.columns
    );
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    assert!(
        list.x + list.width <= bounds(&test, pane).x,
        "nothing of the list lies under the pane"
    );
    let name = test
        .window
        .folder_pane()
        .details()
        .column(SortColumn::Name)
        .expect("a Name column");
    assert!(name.expands(), "Name still takes any room left");
    resize(&test, 1320);
    let room = column_room(&test, test.window.folder_pane());
    assert!(room.name > 260 + 14, "a wide window gives Name its room back");
}

/// In a narrower window the details pane gives way to the list: between
/// 961 and 1190 pixels it narrows with the window, from 235 pixels down to
/// 180, instead of keeping 235.
///
/// parity: LOOK-022
#[gtk::test]
fn the_details_pane_narrows_with_the_window() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    let pane = test.window.details_pane();
    resize(&test, 1180);
    let at_1180 = bounds(&test, pane).width;
    resize(&test, 1000);
    let at_1000 = bounds(&test, pane).width;
    assert!(
        at_1000 < at_1180,
        "narrower in a narrower window: {at_1000} and {at_1180}"
    );
    assert!(
        (180..=190).contains(&at_1000),
        "about 180 pixels near 961: {at_1000}"
    );
    assert!((230..=235).contains(&at_1180), "235 pixels near 1190: {at_1180}");
}

/// A 900-pixel window, alone and split in two: in each pane the columns
/// either all show, or Name has given way to its narrowest and the list
/// scrolls sideways to the rest. Before, Name kept 260 pixels in a split
/// pane while Type and Size were cut off.
///
/// parity: LOOK-022, VIEW-059
#[gtk::test]
fn in_a_900_pixel_window_name_gives_way_before_the_list_scrolls() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    resize(&test, 900);
    let check = |what: &str, pane: &crate::window::folder_pane::FolderPane| {
        let room = column_room(&test, pane);
        let fits = room.columns <= room.shown + 1.0;
        assert!(
            fits || room.name == 140 + 14,
            "{what}: the columns fit, or Name is at its narrowest before the list scrolls: Name {} pixels, {} of {} shown",
            room.name,
            room.shown,
            room.columns
        );
        if !fits {
            assert!(
                room.columns > room.shown,
                "{what}: the rest is reached by scrolling"
            );
        }
    };
    check("one pane", test.window.folder_pane());
    let alone = column_room(&test, test.window.folder_pane());
    assert!(
        alone.columns <= alone.shown + 1.0,
        "a 900-pixel window shows every column"
    );
    test.activate("split-view", None);
    wait_for_frames(&test.window, 3);
    for (index, pane) in test.window.folder_panes().iter().enumerate() {
        check(&format!("split pane {index}"), pane);
    }
}

/// A 1366-pixel window, a laptop's width at 1x, with the details pane
/// open and the tab split in two: in each pane every column shows, or Name
/// has given way to its narrowest and the list scrolls sideways to the
/// rest, with a horizontal scroll bar that stays shown. Before, Type was cut off and Size hidden, reachable only by a
/// scroll bar that showed only under the pointer.
///
/// parity: LOOK-022, VIEW-059
#[gtk::test]
fn a_split_tab_beside_the_details_pane_at_1366_pixels_reaches_every_column() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    resize(&test, 1366);
    test.activate("split-view", None);
    wait_for_frames(&test.window, 3);
    assert!(test.window.details_pane().is_visible(), "the pane is open");
    let panes = test.window.folder_panes();
    assert_eq!(panes.len(), 2, "split in two");
    for (index, pane) in panes.iter().enumerate() {
        let room = column_room(&test, pane);
        let fits = room.columns <= room.shown + 1.0;
        let scrolls = room.name == 140 + 14 && room.columns > room.shown;
        assert!(
            fits || scrolls,
            "pane {index}: Name {} pixels, {} of {} pixels shown",
            room.name,
            room.shown,
            room.columns
        );
        let details = pane.details();
        if !fits {
            wait_for_frames(&test.window, 2);
            let shown_bar = descendants::<gtk::Scrollbar>(details).into_iter().any(|bar| {
                bar.orientation() == gtk::Orientation::Horizontal
                    && bar.is_mapped()
                    && !bar.has_css_class("overlay-indicator")
                    && bar.height() > 0
            });
            assert!(
                shown_bar,
                "pane {index}: a scroll bar shows that the list scrolls sideways, not only under the pointer"
            );
        }
        for column in [
            SortColumn::Name,
            SortColumn::Modified,
            SortColumn::Type,
            SortColumn::Size,
        ] {
            assert!(
                details.column(column).is_some_and(|column| column.is_visible()),
                "pane {index}: {column:?} is shown"
            );
        }
    }
}

/// The widest the window's least width may be at any text size: its
/// least size (`window_size.rs`), so a window at that size, or a laptop
/// screen, is never pushed wider.
const LEAST_WINDOW_WIDTH: i32 = 670;

/// Text at 200 %, as Ctrl+plus five times from 100 % makes it.
fn largest_text(test: &TestWindow) {
    larger_text(test, 5);
}

/// Text `steps` levels larger, as Ctrl+plus that many times from 100 %
/// makes it: 3 for 150 %.
fn larger_text(test: &TestWindow, steps: usize) {
    for _ in 0..steps {
        test.activate("text-larger", None);
    }
    wait_for_frames(&test.window, 3);
}

/// The leaves under `widget` that ask for more than `limit` pixels of
/// width on their own, for a failure message.
fn widest_leaves(widget: &gtk::Widget, limit: i32) -> Vec<String> {
    descendants(widget)
        .into_iter()
        .filter(|leaf: &gtk::Widget| {
            leaf.is_visible() && leaf.measure(gtk::Orientation::Horizontal, -1).0 > limit
        })
        .filter(|leaf: &gtk::Widget| {
            descendants(leaf).into_iter().skip(1).all(|child: gtk::Widget| {
                !child.is_visible() || child.measure(gtk::Orientation::Horizontal, -1).0 <= limit
            })
        })
        .map(|leaf: gtk::Widget| {
            let text = leaf
                .downcast_ref::<gtk::Label>()
                .map(|label| label.text().to_string());
            format!("{} {:?} {:?}", leaf.type_(), leaf.css_classes(), text)
        })
        .collect()
}

/// Opening Settings at the largest text size in a window at its least
/// width leaves the window as it is: every row wraps or narrows within
/// it rather than pushing the window wider, off a small screen.
///
/// parity: LOOK-022
#[gtk::test]
fn settings_at_the_largest_text_size_fit_the_least_window_width() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    resize(&test, 672);
    largest_text(&test);
    test.activate("settings", None);
    wait_for_frames(&test.window, 6);
    let least = test.window.measure(gtk::Orientation::Horizontal, -1).0;
    let leaves = widest_leaves(test.window.upcast_ref(), LEAST_WINDOW_WIDTH / 2);
    test.activate("text-reset", None);
    assert!(
        least <= LEAST_WINDOW_WIDTH,
        "the window needs {least} pixels with Settings open: {leaves:#?}"
    );
}

/// The navigation pane gives way to the files as the window narrows, as
/// Windows' does, and takes its width back as the window widens; nothing
/// is saved. At 200 % text its default width is 420 pixels, which in a
/// 672-pixel window would leave the files 250.
///
/// parity: LOOK-022
#[gtk::test]
fn the_navigation_pane_narrows_with_the_window_and_widens_back() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    largest_text(&test);
    let workspace = test.window.workspace().clone();
    wait_until("the wider default", || workspace.position() == 420);
    resize(&test, 672);
    wait_until("the pane to give way", || {
        let files = workspace.width() - workspace.position();
        files >= 300
    });
    resize(&test, 1320);
    wait_until("the pane to widen back", || workspace.position() == 420);
    let saved = test.context.settings_data().preferences.sidebar_width;
    test.activate("text-reset", None);
    assert_eq!(saved, None, "the window's own changes are not saved");
}

/// The "…" button and the labels of the items its menu lists.
fn see_more(test: &TestWindow) -> (gtk::MenuButton, Vec<String>) {
    let more = descendants::<gtk::MenuButton>(test.window.upcast_ref::<gtk::Widget>())
        .into_iter()
        .find(|button| button.has_css_class("more-command"))
        .expect("the See more button");
    let labels = more
        .popover()
        .and_downcast::<crate::window::menu_popover::MenuPopover>()
        .expect("its menu")
        .items()
        .into_iter()
        .map(|item| item.label)
        .collect();
    (more, labels)
}

/// Whether `command` shows whole in the command bar, left of Settings.
fn shows_whole_in_the_bar(test: &TestWindow, command: &impl IsA<gtk::Widget>) -> bool {
    let bar = descendants::<gtk::Widget>(test.window.upcast_ref::<gtk::Widget>())
        .into_iter()
        .find(|widget| widget.has_css_class("commandbar"))
        .expect("the command bar");
    let settings = descendants::<gtk::Widget>(&bar)
        .into_iter()
        .find(|widget| widget.has_css_class("settings-button"))
        .expect("Settings");
    let (Some(shown), Some(settings)) = (command.compute_bounds(&bar), settings.compute_bounds(&bar)) else {
        return false;
    };
    command.is_mapped() && shown.x() >= 0.0 && shown.x() + shown.width() <= settings.x() + 0.5
}

/// As in Windows 11, the commands that do not fit move into "…", from
/// the right, instead of being cut off with "…" itself: at 200 % text in
/// a 700-pixel window View and Sort are listed at the top of "…", which
/// stays in the bar; in a wide window they are back in the bar.
///
/// parity: LOOK-022
#[gtk::test]
fn commands_that_do_not_fit_move_into_see_more() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    largest_text(&test);
    resize(&test, 700);
    wait_for_frames(&test.window, 4);
    let (more, labels) = see_more(&test);
    let more_shows = shows_whole_in_the_bar(&test, &more);
    // Tab from the last command shown goes to "…", never to one that
    // moved into it.
    let left_out = test.window.command_bar().commands_left_out();
    let sort = descendants::<gtk::MenuButton>(test.window.upcast_ref::<gtk::Widget>())
        .into_iter()
        .find(|button| button.has_css_class("sort-command"))
        .expect("Sort");
    sort.grab_focus();
    test.window.emit_move_focus(gtk::DirectionType::TabForward);
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a focus");
    let tab_reaches_left_out = left_out
        .iter()
        .any(|command| focus.is_ancestor(command) || &focus == command);
    let tab_reaches_more = focus.is_ancestor(&more) || focus == more.clone().upcast::<gtk::Widget>();
    let wide = || {
        resize(&test, 1320);
        wait_for_frames(&test.window, 4);
        see_more(&test).1
    };
    let view_in_menu = labels.iter().any(|label| label == "View");
    let labels_wide = wide();
    test.activate("text-reset", None);
    assert!(more_shows, "See more shows whole beside the commands");
    assert!(
        !tab_reaches_left_out,
        "Tab skips the commands in See more, reaching {focus:?}"
    );
    assert!(tab_reaches_more, "Tab goes from Sort to See more, not {focus:?}");
    assert!(view_in_menu, "View moved into See more: {labels:?}");
    assert!(
        !labels_wide.iter().any(|label| label == "View"),
        "View is back in the bar: {labels_wide:?}"
    );
}
