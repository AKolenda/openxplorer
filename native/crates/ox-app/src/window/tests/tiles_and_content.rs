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
/// Content (Ctrl+Shift+8) shows a file's type under its name and the date
/// modified and size at the right. A folder has no size, and in Content
/// no type.
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
        // Windows 11's Content puts nothing under a name without authors
        // or tags; its Tiles show the type (measured on its captures).
        let kind = if view == FolderView::Tiles {
            "Text Document"
        } else {
            ""
        };
        assert_eq!(lines.kind, kind, "{view:?}");
        assert_eq!(
            lines.size, "20\u{a0}bytes",
            "{view:?}: the fixture's 20-byte file"
        );
        assert!(!lines.modified.is_empty(), "{view:?}: a date modified");
        let survey = cell_named(&test, "Survey.bin").shown_item_lines();
        let survey = survey.unwrap_or_else(|| panic!("{view:?} shows the large file's lines"));
        assert_eq!(
            survey.size, "1.23\u{a0}MB",
            "{view:?}: three digits, as the status bar"
        );
        let folder = cell_named(&test, "Documents").shown_item_lines();
        let folder = folder.unwrap_or_else(|| panic!("{view:?} shows the folder's lines"));
        // Windows 11's Tiles and Content give a folder its name alone.
        assert_eq!(
            (folder.kind.as_str(), folder.size.as_str()),
            ("", ""),
            "{view:?}: a folder has no type or size"
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

/// The group headings the visible grid draws, top to bottom.
fn grid_headings(test: &TestWindow) -> Vec<String> {
    wait_for_frames(&test.window, 3);
    let view = test.window.folder_pane().view_widget();
    let mut headings: Vec<(f32, String)> = descendants::<FileCell>(&view)
        .into_iter()
        .filter(WidgetExt::is_mapped)
        .filter_map(|cell| {
            let text = cell.group_heading()?;
            let bounds = cell.compute_bounds(&view)?;
            Some((bounds.y(), text))
        })
        .collect();
    headings.sort_by(|a, b| a.0.total_cmp(&b.0));
    super::support::with_group_counts(test, headings.into_iter().map(|(_, text)| text).collect())
}

/// The headings the details view draws, top to bottom.
fn details_headings(test: &TestWindow) -> Vec<String> {
    wait_for_frames(&test.window, 3);
    let column_view = test.window.folder_pane().details().column_view().clone();
    let mut labels: Vec<(f32, String)> = descendants::<gtk::Label>(&column_view)
        .into_iter()
        .filter(|label| label.has_css_class("group-title") && label.is_mapped())
        .filter_map(|label| {
            let point = label.compute_point(&column_view, &gtk::graphene::Point::new(0.0, 0.0))?;
            Some((point.y(), label.text().to_string()))
        })
        .collect();
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    super::support::with_group_counts(test, labels.into_iter().map(|(_, text)| text).collect())
}

/// A grouped folder keeps its groups in Tiles and Content, as in Windows
/// 11: each group starts a row under a heading with the title and count
/// Details shows, which takes clicks itself (VIEW-022); a
/// file added to a group counts in its heading; Group by
/// (None) takes the headings away, and the items stay selectable and
/// open as before.
///
/// parity: VIEW-071, VIEW-022
#[gtk::test]
fn tiles_and_content_keep_the_folders_groups_under_headings() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let expected = details_headings(&test);
    assert_eq!(expected, ["File folder (1)", "Text Document (3)"]);
    for view in [FolderView::Tiles, FolderView::Content] {
        show(&test, view);
        assert_eq!(grid_headings(&test), expected, "{view:?}");
        let (folder, first_file) = (cell_bounds(&test, 0), cell_bounds(&test, 1));
        assert!(
            (folder.x() - first_file.x()).abs() < 1.0,
            "{view:?}: the next group starts a row"
        );
        assert!(
            first_file.y() > folder.y() + folder.height() + 16.0,
            "{view:?}: under its own heading"
        );
        let pane = test.window.folder_pane();
        let on_heading = (f64::from(first_file.x()) + 20.0, f64::from(first_file.y()) - 12.0);
        // A heading's row takes clicks itself, selecting its group or
        // folding it with its chevron (VIEW-022), so it is no blank space;
        // the blanks after a short group still are.
        assert!(
            !test
                .window
                .is_blank_space(&pane.view_widget(), on_heading.0, on_heading.1),
            "{view:?}: a click on a heading is the heading's, not blank space"
        );
        assert_eq!(
            pane.owners().position_of(&cell_named(&test, "Notes 2").upcast()),
            Some(1)
        );
        pane.model().select_only(2);
        assert_eq!(test.selected_names(), ["Notes 10.txt"], "{view:?}");
    }
    fixture.write("Notes 3.txt");
    wait_until("the new file's heading count", || {
        grid_headings(&test) == ["File folder (1)", "Text Document (4)"]
    });
    test.activate("group-by", Some("none"));
    wait_until("the headings to go", || grid_headings(&test).is_empty());
    let (first, next) = (cell_bounds(&test, 0), cell_bounds(&test, 1));
    assert!(
        (next.y() - (first.y() + first.height())).abs() < 1.0,
        "with no headings each row follows the last"
    );
}

/// Group by keeps a folder's groups in the icon sizes too, as Windows 11
/// does: Extra large, Large, Medium and Small icons, and the sizes Ctrl+wheel
/// steps through between them, head each group as Details does, each
/// group starts a row under its heading, and a heading row is as tall as
/// a line of title while an item row keeps its tile's height. Before,
/// they listed the items in group order with no heading, so a 7-Zip
/// archive came before the folders. Small icons, which fill rows as in
/// Windows 11, head their groups too.
///
/// parity: VIEW-022, VIEW-005, VIEW-010
#[gtk::test]
fn icon_sizes_keep_the_folders_groups_under_headings() {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Harbor survey")).expect("fixture subfolder");
    for name in [
        "Morning tide.mp3",
        "Dock walkthrough.mp4",
        "Harbor report.pdf",
        "Lighthouse.jpg",
        "Meeting minutes.odt",
        "Archive of logs.zip",
        "Backup set.7z",
        "Budget 2026.txt",
        "ümlaut café 日本語.txt",
    ] {
        fixture.write(name);
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let expected = details_headings(&test);
    assert!(expected.len() >= 8, "a group for each type: {expected:?}");
    let sizes = ["extra-large", "large", "medium", "icons-64", "icons-28", "small"];
    for key in sizes {
        let view = FolderView::from_key(key).expect("an icon size");
        show(&test, view);
        let shown = grid_headings(&test);
        assert!(!shown.is_empty(), "{key}: headed groups");
        assert_eq!(shown[..], expected[..shown.len()], "{key}: as Details heads them");
        let (first, second) = (cell_bounds(&test, 0), cell_bounds(&test, 1));
        assert!(
            (first.x() - second.x()).abs() < 1.0,
            "{key}: the second group starts a row of its own"
        );
        assert!(
            second.y() > first.y() + first.height() + 16.0,
            "{key}: under its own heading"
        );
        let pane = test.window.folder_pane();
        let heights = pane.icon_view().row_heights();
        let grid = pane.view_widget();
        let slot_height = |cell: &FileCell| {
            cell.parent()
                .and_then(|slot| slot.compute_bounds(&grid))
                .map(|bounds| bounds.height())
                .expect("the slot is laid out")
        };
        let heading = descendants::<FileCell>(&grid)
            .into_iter()
            .find(|cell| cell.is_mapped() && cell.group_heading().is_some())
            .expect("a heading on screen");
        #[expect(clippy::cast_precision_loss, reason = "row heights are small")]
        let (heading_px, item_px) = (heights.heading as f32, heights.item as f32);
        assert!(
            (slot_height(&heading) - heading_px).abs() < 1.0,
            "{key}: a heading row is a line of title tall, not a tile: {} vs {heading_px}",
            slot_height(&heading)
        );
        let tile = descendants::<FileCell>(&grid)
            .into_iter()
            .find(|cell| cell.is_mapped() && cell.slot().is_none() && !cell.name().is_empty())
            .expect("a tile on screen");
        assert!(
            (slot_height(&tile) - item_px).abs() < 1.0,
            "{key}: an item row keeps its tile's height: {} vs {item_px}",
            slot_height(&tile)
        );
        pane.model().select_only(1);
        assert_eq!(test.selected_names().len(), 1, "{key}: the listing's positions");
    }
    show(&test, FolderView::from_key("large").expect("Large icons"));
    test.activate("group-by", Some("none"));
    wait_until("the headings to go", || grid_headings(&test).is_empty());
    let (first, next) = (cell_bounds(&test, 0), cell_bounds(&test, 1));
    assert!(
        (next.y() - first.y()).abs() < 1.0,
        "ungrouped, the tiles share a row again"
    );
}

/// Group by in List, as Windows 11 lays it out (checked in the VM): the
/// groups stand side by side as columns, each with its title at the top
/// and no chevron, its items running down under it and on into the next
/// column when they overflow. The keys move between the items: Down
/// from a group's last item goes on to the next group's first, Right to
/// the next column. Before, List showed the group order with no headings.
///
/// parity: VIEW-022, VIEW-008
#[gtk::test]
fn list_heads_each_group_at_the_top_of_its_column() {
    use gtk::gdk;
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Harbor survey")).expect("fixture subfolder");
    for number in 0..60 {
        fixture.write(&format!("Notes {number:02}.txt"));
    }
    for name in ["Lighthouse.jpg", "Morning tide.mp3", "Archive of logs.zip"] {
        fixture.write(name);
    }
    // Before the window opens, which a new pointer would put away.
    let mut pointer = crate::test_support::pointer::Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let expected = details_headings(&test);
    show(&test, FolderView::Compact);
    let shown = grid_headings(&test);
    assert!(!shown.is_empty(), "List heads its groups");
    assert_eq!(shown[..], expected[..shown.len()], "as Details heads them");
    let pane = test.window.folder_pane();
    let grid = pane.view_widget();
    let headings: Vec<(FileCell, gtk::graphene::Rect)> = descendants::<FileCell>(&grid)
        .into_iter()
        .filter(|cell| cell.is_mapped() && cell.group_heading().is_some())
        .filter_map(|cell| cell.compute_bounds(&grid).map(|bounds| (cell, bounds)))
        .collect();
    let top = headings
        .iter()
        .map(|(_, bounds)| bounds.y())
        .fold(f32::MAX, f32::min);
    for (cell, bounds) in &headings {
        assert!((bounds.y() - top).abs() < 1.0, "each heading tops its column");
        let chevron = descendants::<gtk::Image>(cell)
            .into_iter()
            .find(|image| image.has_css_class("group-chevron"))
            .expect("a chevron widget");
        assert!(chevron.width() < 1, "no chevron in List");
    }
    let mut xs: Vec<f32> = headings.iter().map(|(_, bounds)| bounds.x()).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    assert_eq!(xs.len(), headings.len(), "the groups stand side by side");
    // Text Document's 60 notes overflow their column into the next.
    let model = pane.model();
    let lines = pane.icon_view().grid().max_columns();
    assert!(lines < 60, "the notes overflow a column");
    let first_note = test.position_of("Notes 00.txt");
    pane.scroll_into_view(first_note + lines);
    wait_for_frames(&test.window, 4);
    let note = |offset: u32| cell_bounds(&test, first_note + offset);
    let (first, next) = (note(0), note(1));
    assert!(
        (first.x() - next.x()).abs() < 1.0 && next.y() > first.y(),
        "the notes run down"
    );
    let overflow = note(lines - 1);
    assert!(overflow.x() > first.x(), "the notes run on into the next column");
    assert!((overflow.y() - top).abs() < 1.0, "from its top");
    // Keys.
    test.window.folder_pane().focus_view();
    let before = first_note - 1;
    test.window.folder_pane().reveal(before);
    test.window.folder_model().select_only(before);
    let press = |key: gdk::Key| super::selection::press_key(&test, key, gdk::ModifierType::empty());
    press(gdk::Key::Down);
    assert_eq!(
        model.selected_positions(),
        [first_note],
        "Down goes on into the next group"
    );
    press(gdk::Key::Right);
    assert_eq!(
        model.selected_positions(),
        [first_note + lines],
        "Right goes to the same row of the next column"
    );
    press(gdk::Key::Left);
    assert_eq!(model.selected_positions(), [first_note], "Left comes back");
    // A real click on the notes' heading selects the group, as Windows 11's
    // does, and leaves the keyboard on the heading; a real Right then goes
    // on to the same row of the next column, and Down from the heading to
    // the group's first item, rather than Left and Right being taken as
    // Collapse and Expand, which List has not.
    // In front of any window an earlier test left on the display.
    test.window.present();
    wait_for_frames(&test.window, 3);
    let click_heading = |pointer: &mut crate::test_support::pointer::Pointer| {
        let heading = descendants::<FileCell>(&grid)
            .into_iter()
            .find(|cell| {
                cell.is_mapped()
                    && cell
                        .group_heading()
                        .is_some_and(|title| title.starts_with("Text Document"))
            })
            .expect("the notes' heading");
        let bounds = heading.compute_bounds(&grid).expect("the heading is laid out");
        let at = crate::test_support::pointer::screen_point(
            pointer,
            &grid,
            (
                f64::from(bounds.x() + bounds.width() / 2.0),
                f64::from(bounds.y() + bounds.height() / 2.0),
            ),
            super::right_drag::PROBE,
        );
        // Past the double-click time since any click before, so this one
        // is a click of its own, not the second of a double-click, which
        // would fold the group back to the selection before it.
        let double_click = gtk::Settings::default().map_or(400, |settings| settings.gtk_double_click_time());
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(
            u64::try_from(double_click).unwrap_or(400) + 100,
        ));
        pointer.move_to(at.0, at.1);
        pointer.tap(crate::test_support::pointer::Button::Primary);
        wait_until("the notes selected", || {
            let selected = model.selected_positions();
            selected.len() == 60 && selected.first() == Some(&first_note)
        });
        wait_for_frames(&test.window, 4);
        assert!(
            test.window.focused_heading().is_some(),
            "the click leaves the keyboard on the heading"
        );
    };
    click_heading(&mut pointer);
    let wait_for_selection = |expected: u32, what: &str| {
        wait_until(what, || model.selected_positions() == [expected]);
    };
    pointer.tap_key(gdk::Key::Right);
    wait_for_selection(
        first_note + lines - 1,
        "Right from the heading to the same row of the next column",
    );
    pane.reveal(first_note);
    wait_for_frames(&test.window, 4);
    click_heading(&mut pointer);
    pointer.tap_key(gdk::Key::Down);
    wait_for_selection(first_note, "Down from the heading to its first item");
    pointer.move_to(720, 500);
    // A key on a group's second column keeps List's sideways scroll: the
    // other grouped views' keeping of a heading in sight measured rows,
    // and snapped List back towards its left edge.
    let sideways = pane.icon_view().scroll_adjustment();
    let right = (sideways.upper() - sideways.page_size()).max(0.0);
    sideways.set_value(right);
    pane.icon_view().show_heading_above(first_note + lines);
    assert!(
        (sideways.value() - right).abs() < 0.5,
        "List stays scrolled where it was: {} after {right}",
        sideways.value()
    );
}

/// A group heading's accessible name follows its title when the groups
/// change, as the heading itself does, though GTK keeps the heading bound
/// (a heading shows no count, as in Windows 11 since 22H2).
///
/// parity: VIEW-071, VIEW-022
#[gtk::test]
fn a_group_headings_accessible_name_follows_its_title() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    show(&test, FolderView::Content);
    let owners = test.window.folder_pane().owners();
    let labels = || owners.bound_accessible_labels();
    wait_until("the headings' names", || {
        let names = labels();
        names.contains(&"File folder".to_owned()) && names.contains(&"Text Document".to_owned())
    });
    test.activate("group-by", Some("name"));
    wait_until("the new headings", || {
        grid_headings(&test) == ["A – H (1)", "I – P (2)", "Q – Z (1)"]
    });
    let names = labels();
    for title in ["A – H", "I – P", "Q – Z"] {
        assert!(names.contains(&title.to_owned()), "{names:?}");
    }
    assert!(!names.contains(&"File folder".to_owned()), "{names:?}");
    assert!(!names.contains(&"Text Document".to_owned()), "{names:?}");
}

/// A heading row is as tall as Page Up and Page Down count it, and an
/// item row as tall as its tile or Content row, so a page moves as far
/// as the view shows.
///
/// parity: VIEW-071
#[gtk::test]
fn heading_and_item_rows_are_as_tall_as_a_page_counts_them() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    for view in [FolderView::Tiles, FolderView::Content] {
        show(&test, view);
        wait_until("the headings", || !grid_headings(&test).is_empty());
        let pane = test.window.folder_pane();
        let heights = pane.icon_view().row_heights();
        let grid = pane.view_widget();
        let slot_height = |cell: &FileCell| {
            cell.parent()
                .and_then(|slot| slot.compute_bounds(&grid))
                .map(|bounds| bounds.height())
                .expect("the slot is laid out")
        };
        let heading = descendants::<FileCell>(&grid)
            .into_iter()
            .find(|cell| cell.is_mapped() && cell.group_heading().is_some())
            .expect("a heading on screen");
        #[expect(clippy::cast_precision_loss, reason = "row heights are small")]
        let (heading_px, item_px) = (heights.heading as f32, heights.item as f32);
        assert!(
            (slot_height(&heading) - heading_px).abs() < 1.0,
            "{view:?}: heading"
        );
        let item = cell_named(&test, "Notes 2");
        assert!((slot_height(&item) - item_px).abs() < 1.0, "{view:?}: item row");
    }
}

/// A cell that showed a group's heading and shows an item again takes
/// the item check boxes' current click targeting: while they wait for
/// Ctrl, an unchecked box lets clicks through to its row.
///
/// parity: VIEW-071, SEL-014
#[gtk::test]
fn a_cell_that_showed_a_heading_follows_whether_check_boxes_take_clicks() {
    let fixture = Fixture::empty();
    let extensions = [
        "txt", "md", "pdf", "png", "ods", "sh", "svg", "zip", "csv", "html", "json", "xml",
    ];
    for (number, extension) in extensions.iter().cycle().take(36).enumerate() {
        fixture.write(&format!("file {number:02}.{extension}"));
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    show(&test, FolderView::Content);
    let pane = test.window.folder_pane();
    wait_until("the headings", || grid_headings(&test).len() >= 3);
    let grid = pane.view_widget();
    let headings: Vec<FileCell> = descendants::<FileCell>(&grid)
        .into_iter()
        .filter(|cell| cell.group_heading().is_some())
        .collect();
    // The boxes stop waiting for Ctrl while some cells show headings. It
    // goes through the window, which says again whether they take clicks
    // as it gets focus, so a change made around it could be undone.
    let owners = pane.owners();
    assert!(!owners.item_checks_take_clicks(), "the boxes wait for Ctrl");
    test.window.wait_for_ctrl_with_item_checks(false);
    let take = true;
    assert!(owners.item_checks_take_clicks());
    let adjustment = pane.icon_view().scroll_adjustment();
    let mut reused = 0;
    for step in 1..=12 {
        adjustment.set_value(adjustment.upper() * f64::from(step) / 12.0);
        wait_for_frames(&test.window, 2);
        for cell in &headings {
            let shows_item = cell.is_mapped() && cell.group_heading().is_none() && !cell.name().is_empty();
            if shows_item && !cell.item_check().is_active() {
                reused += 1;
                assert_eq!(cell.item_check_targets_clicks(), take, "{}", cell.name());
            }
        }
    }
    assert!(reused > 0, "scrolling shows items in cells that showed headings");
}

/// A group's heading is heard without a file's details, also in a cell
/// that showed a file before: GTK reuses its list items as the view
/// scrolls, and a heading bound where a file was kept that file's type,
/// size and date as its description.
///
/// parity: VIEW-071, ACC-002
#[gtk::test]
fn a_heading_in_a_reused_cell_is_heard_without_a_files_details() {
    let fixture = Fixture::empty();
    let extensions = [
        "txt", "md", "pdf", "png", "ods", "sh", "svg", "zip", "csv", "html", "json", "xml",
    ];
    for (number, extension) in extensions.iter().cycle().take(36).enumerate() {
        fixture.write(&format!("file {number:02}.{extension}"));
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    show(&test, FolderView::Content);
    let pane = test.window.folder_pane();
    wait_until("the headings", || grid_headings(&test).len() >= 3);
    let grid = pane.view_widget();
    let items: Vec<FileCell> = descendants::<FileCell>(&grid)
        .into_iter()
        .filter(|cell| cell.is_mapped() && cell.group_heading().is_none() && !cell.name().is_empty())
        .collect();
    let owners = pane.owners();
    let adjustment = pane.icon_view().scroll_adjustment();
    let mut reused = 0;
    for step in (1..=12).chain((0..12).rev()) {
        adjustment.set_value(adjustment.upper() * f64::from(step) / 12.0);
        wait_for_frames(&test.window, 2);
        reused += items
            .iter()
            .filter(|cell| cell.is_mapped() && cell.group_heading().is_some())
            .count();
        let described: Vec<String> = owners
            .bound_slot_descriptions()
            .into_iter()
            .filter(|description| !description.is_empty())
            .collect();
        assert!(described.is_empty(), "step {step}: {described:?}");
    }
    assert!(reused > 0, "scrolling shows headings in cells that showed items");
}

/// Content gives a picture its dimensions, read from its header off the
/// main thread by the picture guard, which decodes nothing: binding a
/// row used to ask gdk-pixbuf on the main thread, which through glycin
/// decodes the whole picture and aborts on a huge one. A file that only
/// claims to be a picture shows none.
///
/// parity: VIEW-071
#[gtk::test]
fn content_shows_a_pictures_dimensions_read_off_the_main_thread() {
    let fixture = Fixture::empty();
    let picture =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 48).expect("a picture");
    picture.fill(0x3366_99ff);
    picture
        .savev(fixture.path("Harbor.png"), "png", &[])
        .expect("png");
    std::fs::write(fixture.path("Not a photo.png"), "fictional text").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    show(&test, FolderView::Content);

    let harbor = cell_named(&test, "Harbor");
    wait_until("the picture's dimensions", || {
        harbor.shown_picture_dimensions() == "64 x 48"
    });
    let other = cell_named(&test, "Not a photo");
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    assert_eq!(other.shown_picture_dimensions(), "", "no dimensions");
}

/// Grouped List's groups stand side by side as columns, which do not
/// collapse, so neither the folder menu's View nor a heading's menu offers
/// Collapse group, Expand all groups or Collapse all groups there, which
/// did nothing; Tiles still offers them.
///
/// parity: VIEW-022, VIEW-008
#[gtk::test]
fn grouped_list_offers_no_collapse_commands() {
    use crate::folder_view::grid::SlotKind;
    use crate::window::menu_popover::{MenuAction, MenuEntry};
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("group-by", Some("type"));
    let group_commands = |entries: &[MenuEntry]| {
        entries
            .iter()
            .filter(|entry| {
                matches!(entry, MenuEntry::Item(item) if matches!(
                    item.action,
                    MenuAction::Window(
                        WindowAction::CollapseGroup
                            | WindowAction::ExpandGroup
                            | WindowAction::CollapseAllGroups
                            | WindowAction::ExpandAllGroups
                    )
                ))
            })
            .count()
    };
    for (view, offered) in [(FolderView::Tiles, true), (FolderView::Compact, false)] {
        show(&test, view);
        let grid = test.window.folder_pane().view_widget();
        let heading = || {
            descendants::<FileCell>(&grid)
                .into_iter()
                .find_map(|cell| cell.slot().filter(|slot| slot.kind() == SlotKind::Heading))
        };
        wait_until("a heading", || heading().is_some());
        let heading = heading().expect("a heading");
        let in_view = group_commands(&test.window.background_view_menu());
        let on_heading = group_commands(&test.window.heading_menu_for(&heading));
        assert_eq!(in_view > 0, offered, "{view:?}: View has {in_view}");
        assert_eq!(on_heading > 0, offered, "{view:?}: the heading has {on_heading}");
    }
}

/// A view chosen while an item of a collapsed group stays selected, beside
/// one selected far down in a group that shows, reveals that one and gives
/// it the keyboard. The hidden item, first in the listing, used to be the
/// one asked for, which no row shows, so the new view stayed at its top.
///
/// parity: VIEW-022
#[gtk::test]
fn a_new_view_reveals_the_first_selected_item_it_shows() {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Harbor survey")).expect("fixture subfolder");
    for number in 0..80 {
        fixture.write(&format!("Notes {number:02}.txt"));
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    test.activate("group-by", Some("type"));
    let pane = test.window.folder_pane();
    wait_until("the groups", || {
        pane.grouped().is_some_and(|grouped| grouped.layout().is_some())
    });
    let folder = test.position_of("Harbor survey");
    let last = test.position_of("Notes 79.txt");
    pane.model().select_positions(&[folder, last]);
    pane.grouped()
        .expect("a grouped view")
        .set_collapsed("File folder", true);
    pane.focus_view();
    wait_until("the list to have the keyboard", || pane.view_has_focus());

    test.activate("view", Some(FolderView::Content.as_str()));
    wait_until("the last note to have the keyboard", || {
        pane.focused_position() == Some(last)
    });
    let view = pane.view_widget();
    wait_until("the last note to be scrolled into view", || {
        let bounds = pane
            .owners()
            .file_cell_at(last, &view)
            .filter(WidgetExt::is_mapped)
            .and_then(|cell| cell.compute_bounds(&view));
        #[expect(clippy::cast_precision_loss, reason = "widget sizes are small")]
        let height = view.height() as f32;
        bounds.is_some_and(|bounds| bounds.y() >= 0.0 && bounds.y() + bounds.height() <= height)
    });
    let mut selected = pane.model().selected_positions();
    selected.sort_unstable();
    let mut expected = vec![folder, last];
    expected.sort_unstable();
    assert_eq!(selected, expected, "both stay selected");
}

/// Shows `view` in a folder with nothing to lay out.
fn show_empty(test: &TestWindow, view: FolderView) {
    test.activate("view", Some(view.as_str()));
    assert_eq!(test.window.folder_pane().view(), view);
}
