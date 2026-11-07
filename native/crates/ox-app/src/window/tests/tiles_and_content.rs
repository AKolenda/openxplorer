// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's Tiles and Content views in a real window (VIEW-071): what
//! their items say, how they are laid out, their keys, renaming in place,
//! the rubber band and each folder remembering them.

use gtk::prelude::*;

use super::file_ops_support::{name_editor, select_names};
use crate::folder_view::cells::{FileCell, ItemLines};
use crate::test_support::harness::{
    application, descendants, wait_for_frames, wait_until, Fixture, TestWindow,
};
use crate::window::folder_pane::FolderView;
use crate::window::rubber_band::BandMode;
use crate::window::session::Direction;
use crate::window::WindowAction;

/// Shows `view` in `test` and waits for its first item to be laid out.
fn show(test: &TestWindow, view: FolderView) {
    test.activate("view", Some(view.as_str()));
    let pane = test.window.folder_pane();
    assert_eq!(pane.view(), view);
    wait_until("the view to lay out its items", || {
        let shown = pane.view_widget();
        pane.owners()
            .widget_at(0)
            .is_some_and(|cell| cell.is_ancestor(&shown) && cell.height() > 0)
    });
}

/// The cell showing the item called `name` (as shown) in the visible view.
fn cell_named(test: &TestWindow, name: &str) -> FileCell {
    let view = test.window.folder_pane().view_widget();
    descendants::<FileCell>(&view)
        .into_iter()
        .find(|cell| cell.name() == name)
        .unwrap_or_else(|| panic!("a cell shows {name}"))
}

/// The bounds of the cell showing `position`, in the view's coordinates.
fn cell_bounds(test: &TestWindow, position: u32) -> gtk::graphene::Rect {
    let pane = test.window.folder_pane();
    let view = pane.view_widget();
    pane.owners()
        .file_cell_at(position, &view)
        .and_then(|cell| cell.compute_bounds(&view))
        .expect("the item is on screen")
}

/// View > Tiles (Ctrl+Shift+7) shows each item's type and, for a file, its
/// size under its name, written as the status bar writes it; View >
/// Content (Ctrl+Shift+8) shows the type under the name and the date
/// modified and size at the right. A folder has no size.
///
/// parity: VIEW-071
#[gtk::test]
fn tiles_and_content_show_each_items_type_and_size_beside_its_name() {
    let fixture = Fixture::standard();
    std::fs::write(fixture.path("Survey.bin"), vec![0; 1_300_000]).expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    let view = WindowAction::View.detailed_name();
    let keys = |target: &str| application().accels_for_action(&format!("{view}::{target}"));
    assert_eq!(keys("tiles"), ["<Shift><Control>7"]);
    assert_eq!(keys("content"), ["<Shift><Control>8"]);
    for view in [FolderView::Tiles, FolderView::Content] {
        show(&test, view);
        let notes = cell_named(&test, "Notes 2").shown_item_lines();
        let lines = notes.unwrap_or_else(|| panic!("{view:?} shows the item's lines"));
        assert_eq!(lines.kind, "Text Document", "{view:?}");
        assert_eq!(lines.size, "20 bytes", "{view:?}: the fixture's 20-byte file");
        assert!(!lines.modified.is_empty(), "{view:?}: a date modified");
        let survey = cell_named(&test, "Survey.bin").shown_item_lines();
        let survey = survey.unwrap_or_else(|| panic!("{view:?} shows the large file's lines"));
        assert_eq!(
            survey.size, "1.23 MB",
            "{view:?}: three digits, as the status bar"
        );
        let folder = cell_named(&test, "Documents").shown_item_lines();
        let folder = folder.unwrap_or_else(|| panic!("{view:?} shows the folder's lines"));
        assert_eq!(
            (folder.kind.as_str(), folder.size.as_str()),
            ("File folder", ""),
            "{view:?}: a folder has no size"
        );
    }
    show(&test, FolderView::Details);
    let lines: Option<ItemLines> = cell_named(&test, "Notes 2").shown_item_lines();
    assert_eq!(lines, None, "Details keeps them in its columns");
}

/// Tiles fill rows across the width, 256 pixels wide each; Content lists
/// one item per row across the whole width, each row under the last.
///
/// parity: VIEW-071
#[gtk::test]
fn tiles_fill_the_width_and_content_rows_span_it() {
    let fixture = Fixture::with_files(24);
    let test = TestWindow::open(&fixture.uri());
    show(&test, FolderView::Tiles);
    wait_for_frames(&test.window, 4);
    let grid = test.window.folder_pane().icon_view().grid().clone();
    wait_until("several columns of tiles", || grid.max_columns() >= 2);
    let (first, next) = (cell_bounds(&test, 0), cell_bounds(&test, 1));
    assert!(
        (first.y() - next.y()).abs() < 1.0,
        "the second tile is beside the first"
    );
    assert!(next.x() > first.x() + 200.0, "tiles are about 250 pixels wide");

    show(&test, FolderView::Content);
    wait_for_frames(&test.window, 4);
    assert_eq!(grid.max_columns(), 1);
    let (first, next) = (cell_bounds(&test, 0), cell_bounds(&test, 1));
    assert!((first.x() - next.x()).abs() < 1.0, "one column");
    assert!(next.y() >= first.y() + first.height(), "each row under the last");
    let width = test.window.folder_pane().view_widget().width();
    #[expect(clippy::cast_precision_loss, reason = "widget sizes are small")]
    let spans = first.width() > width as f32 * 0.8;
    assert!(spans, "a row spans the view: {} of {width}", first.width());
}

/// F2 in Tiles and Content edits the name on its own line, beside the
/// icon, and Enter renames the file.
///
/// parity: VIEW-071, OPS-010
#[gtk::test]
fn renaming_in_tiles_and_content_edits_the_name_line_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    for (view, from, to) in [
        (FolderView::Tiles, "Notes 2.txt", "Plans.txt"),
        (FolderView::Content, "Plans.txt", "Budget.txt"),
    ] {
        show(&test, view);
        select_names(&test, &[from]);
        test.activate("rename", None);
        let field = name_editor(&test);
        let line = field.parent().expect("the field is shown");
        assert!(
            line.has_css_class("item-lines"),
            "{view:?}: the field takes the name's line"
        );
        field.set_text(to);
        field.emit_activate();
        wait_until("the renamed item to be selected", || {
            test.selected_names() == [to]
        });
        assert!(fixture.path(to).is_file(), "{view:?}: renamed");
    }
}

/// A band drawn from the blank space under Content's rows selects the
/// rows it crosses, as in the other views.
///
/// parity: VIEW-071, SEL-012
#[gtk::test]
fn a_rubber_band_selects_content_rows() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    show(&test, FolderView::Content);
    wait_for_frames(&test.window, 4);
    let pane = test.window.folder_pane();
    let view = pane.view_widget();
    let last = pane.model().n_items() - 1;
    let below = cell_bounds(&test, last);
    let start = (40.0, f64::from(below.y() + below.height()) + 20.0);
    assert!(test.window.is_blank_space(&view, start.0, start.1));
    let second = cell_bounds(&test, 1);
    test.window.begin_band(&view, start, BandMode::Replace);
    test.window
        .move_band((60.0, f64::from(second.y() + second.height() / 2.0)));
    test.window.end_band();
    assert_eq!(
        test.window.folder_model().selected_positions(),
        (1..=last).collect::<Vec<u32>>()
    );
}

/// Each folder remembers Tiles and Content, as it remembers the other
/// views, and saves them under keys of their own.
///
/// parity: VIEW-071, VIEW-020
#[gtk::test]
fn each_folder_remembers_tiles_and_content() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let saved_mode = |uri: &str| test.context.settings_data().preferences.view_for(uri).mode;
    show(&test, FolderView::Tiles);
    wait_until("the folder's view to be saved", || {
        saved_mode(&fixture.uri()) == "tiles"
    });
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    assert_eq!(test.window.folder_pane().view(), FolderView::Details);
    show_empty(&test, FolderView::Content);
    wait_until("Documents' view to be saved", || {
        saved_mode(&fixture.uri_of("Documents")) == "content"
    });
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("the folder again");
    assert_eq!(test.window.folder_pane().view(), FolderView::Tiles);
    assert_eq!(test.action_state("view").as_deref(), Some("tiles"));
    test.window.go_history(Direction::Forward);
    test.wait_for_listing("Documents again");
    assert_eq!(test.window.folder_pane().view(), FolderView::Content);
}

/// Shows `view` in a folder with nothing to lay out.
fn show_empty(test: &TestWindow, view: FolderView) {
    test.activate("view", Some(view.as_str()));
    assert_eq!(test.window.folder_pane().view(), view);
}
