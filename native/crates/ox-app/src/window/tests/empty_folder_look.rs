// SPDX-License-Identifier: AGPL-3.0-only
//! How an empty folder looks, as in Windows Explorer: the file list stays,
//! with its column titles in Details, and a small grey "This folder is
//! empty." sits near its top, under the titles.

use gtk::prelude::*;

use super::geometry::bounds;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::PanePage;

/// Makes the empty folder "Empty" in `fixture`, opens a window on the
/// fixture and goes there.
fn window_on_empty_folder(fixture: &Fixture) -> TestWindow {
    std::fs::create_dir(fixture.path("Empty")).expect("the empty folder is made");
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Empty")));
    wait_until("the empty folder", || {
        !test.window.is_loading() && test.names().is_empty()
    });
    wait_for_frames(&test.window, 3);
    test
}

/// The small note that says a folder is empty, if one shows.
fn shown_note(test: &TestWindow) -> Option<gtk::Label> {
    descendants::<gtk::Label>(&test.window)
        .into_iter()
        .find(|label| label.has_css_class("empty-folder-note") && label.is_mapped())
}

/// The details view's row of column titles.
fn column_titles(test: &TestWindow) -> gtk::Widget {
    test.window
        .folder_pane()
        .details()
        .column_view()
        .first_child()
        .filter(|child| child.css_name() == "header")
        .expect("the details view has its titles")
}

/// parity: VIEW-048
#[gtk::test]
fn an_empty_folder_keeps_its_column_titles_and_says_so_under_them() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);

    assert_eq!(
        test.window.folder_pane().page(),
        Some(PanePage::Listing),
        "the list stays"
    );
    let titles = column_titles(&test);
    assert!(titles.is_mapped(), "the column titles show");
    let note = shown_note(&test).expect("the note shows");
    assert_eq!(note.text(), "This folder is empty.");
    let titles = bounds(&test, &titles);
    let note_place = bounds(&test, &note);
    let list = bounds(&test, test.window.folder_pane().details().column_view());
    let below_titles = note_place.y - (titles.y + titles.height);
    assert!(
        (0..=24).contains(&below_titles),
        "the note sits just under the titles: {below_titles} pixels below"
    );
    let note_middle = note_place.x + note_place.width / 2;
    let list_middle = list.x + list.width / 2;
    assert!(
        (note_middle - list_middle).abs() <= 2,
        "the note is centred over the list"
    );
    assert!(
        !descendants::<gtk::Label>(&test.window)
            .iter()
            .any(|label| label.has_css_class("empty-title") && label.is_mapped()),
        "no big title"
    );
}

/// The note goes as soon as the folder has an item, and comes back when
/// its last item goes.
///
/// parity: VIEW-048
#[gtk::test]
fn the_note_goes_with_the_first_item_and_comes_back_with_the_last() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);
    assert!(shown_note(&test).is_some());

    std::fs::write(fixture.path("Empty/First.txt"), "first").expect("the file is written");
    test.activate("refresh", None);
    wait_until("the item", || test.names() == ["First.txt"]);
    assert!(shown_note(&test).is_none(), "no note with an item");

    std::fs::remove_file(fixture.path("Empty/First.txt")).expect("the file goes");
    test.activate("refresh", None);
    wait_until("the note again", || {
        !test.window.is_loading() && test.names().is_empty() && shown_note(&test).is_some()
    });
    assert_eq!(test.window.folder_pane().page(), Some(PanePage::Listing));
}

/// In the icon views, which have no titles, the note sits near the top
/// of the pane.
///
/// parity: VIEW-048
#[gtk::test]
fn the_icon_views_say_so_near_their_top() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);

    test.activate("view", Some("large"));
    wait_for_frames(&test.window, 3);

    let note = shown_note(&test).expect("the note shows");
    let grid = bounds(&test, test.window.folder_pane().icon_view().grid());
    let note_place = bounds(&test, &note);
    assert!(
        (0..=24).contains(&(note_place.y - grid.y)),
        "the note sits near the top: {} pixels down",
        note_place.y - grid.y
    );
}

/// The note is only for an empty folder: a location that could not be
/// listed and a search without matches keep their page.
///
/// parity: VIEW-048, SRCH-013
#[gtk::test]
fn unavailable_locations_and_searches_keep_their_page() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    assert!(shown_note(&test).is_none());

    test.activate("go-to", Some(&fixture.uri()));
    wait_until("the folder", || !test.names().is_empty());
    test.window.search_box().entry().set_text("zzz-no-such-item");
    wait_until("the no-matches page", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    assert!(shown_note(&test).is_none());
}

/// Screen readers hear that the list is empty: the list is described by
/// the note while it shows, and keeps its own description throughout.
///
/// parity: VIEW-048, ACC-001
#[gtk::test]
fn screen_readers_hear_that_the_folder_is_empty() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);
    let list = test.window.folder_pane().view_widget();
    let described =
        |list: &gtk::Widget| gtk::test_accessible_has_relation(list, gtk::AccessibleRelation::DescribedBy);
    let own_description =
        |list: &gtk::Widget| gtk::test_accessible_has_property(list, gtk::AccessibleProperty::Description);

    assert!(described(&list), "the note describes the list");
    assert!(own_description(&list));

    std::fs::write(fixture.path("Empty/First.txt"), "first").expect("the file is written");
    test.activate("refresh", None);
    wait_until("the item", || test.names() == ["First.txt"]);
    assert!(!described(&list), "not once it has an item");
    assert!(own_description(&list), "its own description stays");
}

/// Whether `widget`, drawn in `test`'s window, shows the accent outline:
/// accent lines along at least half of its height and half of its width.
fn draws_accent_outline(test: &TestWindow, widget: &gtk::Widget) -> bool {
    const ACCENT: [u8; 3] = [0x00, 0x67, 0xc0];
    wait_for_frames(&test.window, 3);
    let window: gtk::Widget = test.window.clone().upcast();
    let place = widget.compute_bounds(&window).expect("the widget is laid out");
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(window.width()), f64::from(window.height()));
    let node = snapshot.to_node().expect("the window draws");
    let renderer = window
        .native()
        .and_then(|native| native.renderer())
        .expect("a drawn window");
    let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
    let rows = super::icons::pixel_rows(&texture);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window pixels"
    )]
    let (left, top, width, height) = (
        place.x() as usize,
        place.y() as usize,
        place.width() as usize,
        place.height() as usize,
    );
    let is_accent = |[blue, green, red, _]: [u8; 4]| {
        [red, green, blue]
            .iter()
            .zip(ACCENT)
            .all(|(shown, accent)| shown.abs_diff(accent) <= 3)
    };
    let mut in_column = vec![0_usize; width];
    let mut in_row = vec![0_usize; height];
    for (y, row) in rows[top..top + height].iter().enumerate() {
        for (x, pixel) in row[left..left + width].iter().enumerate() {
            if is_accent(*pixel) {
                in_column[x] += 1;
                in_row[y] += 1;
            }
        }
    }
    let vertical = in_column.iter().any(|count| *count > height / 2);
    let horizontal = in_row.iter().any(|count| *count > width / 2);
    vertical && horizontal
}

/// A drag over an empty folder draws the accent outline around its list,
/// under the column titles.
///
/// parity: DND-011
#[gtk::test]
fn an_empty_folder_draws_the_drop_outline() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);
    let list = test.window.folder_pane().view_widget();

    list.add_css_class("file-drop-active");
    let outlined = draws_accent_outline(&test, &list);
    list.remove_css_class("file-drop-active");

    assert!(outlined, "the outline shows");
    assert!(!draws_accent_outline(&test, &list), "and goes with the drag");
}

/// The outline shows in the icon views too.
///
/// parity: DND-011
#[gtk::test]
fn an_empty_folder_in_an_icon_view_draws_the_drop_outline() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);
    test.activate("view", Some("large"));
    let grid = test.window.folder_pane().view_widget();

    grid.add_css_class("file-drop-active");
    let outlined = draws_accent_outline(&test, &grid);
    grid.remove_css_class("file-drop-active");

    assert!(outlined, "the outline shows");
}

/// A folder with items shows the outline around its list as well, when a
/// drag is over its blank space.
///
/// parity: DND-011
#[gtk::test]
fn a_folder_with_items_draws_the_drop_outline_around_its_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let list = test.window.folder_pane().view_widget();

    list.add_css_class("file-drop-active");
    let outlined = draws_accent_outline(&test, &list);
    list.remove_css_class("file-drop-active");

    assert!(outlined, "the outline shows");
}

/// In split view each pane says for itself whether its folder is empty,
/// whichever pane is active.
///
/// parity: VIEW-048, VIEW-059
#[gtk::test]
fn each_pane_of_a_split_tab_says_whether_its_own_folder_is_empty() {
    use crate::window::session::PaneSide;
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    test.window.navigate_or_report(&fixture.uri_of("Documents"));
    test.wait_for_listing("the empty Documents in the right pane");
    let shows_note = |side| test.window.pane_on(side).shows_empty_note();

    assert!(shows_note(PaneSide::End), "Documents is empty");
    assert!(!shows_note(PaneSide::Start), "the folder with items is not");
    test.window.activate_pane(PaneSide::Start);
    crate::test_support::harness::settle();
    assert!(shows_note(PaneSide::End), "the inactive pane keeps its note");
    assert!(!shows_note(PaneSide::Start));
}

/// A longer note, as a translation may be, wraps inside a narrow pane
/// instead of running past it.
///
/// parity: VIEW-048
#[gtk::test]
fn a_long_note_wraps_inside_the_pane() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);
    let note = shown_note(&test).expect("the note shows");
    let one_line = bounds(&test, &note).height;
    note.set_label(&"Dieser Ordner ist leer. ".repeat(12));
    wait_for_frames(&test.window, 3);

    let list = bounds(&test, test.window.folder_pane().details().column_view());
    let wrapped = bounds(&test, &note);
    assert!(
        wrapped.x >= list.x && wrapped.x + wrapped.width <= list.x + list.width,
        "inside the pane"
    );
    assert!(wrapped.height > one_line, "on more than one line");
}

/// Grouping an empty folder shows no groups, and the note still shows.
///
/// parity: VIEW-048
#[gtk::test]
fn an_empty_folder_grouped_by_date_still_says_so() {
    let fixture = Fixture::standard();
    let test = window_on_empty_folder(&fixture);

    test.activate("group-by", Some("modified"));
    wait_for_frames(&test.window, 3);

    assert!(shown_note(&test).is_some());
}
