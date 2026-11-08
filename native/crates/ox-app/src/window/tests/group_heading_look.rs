// SPDX-License-Identifier: AGPL-3.0-only
//! A group's heading looks as Windows 11 draws it, in every view and with
//! Compact view on or off (VIEW-022): its title in regular weight about a
//! quarter larger than the items' names (15 pixels to their 12 at 100%),
//! and no line after it.

use gtk::pango;
use gtk::prelude::*;

use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::FolderView;

/// A folder and two text files, grouped by type in the window.
fn grouped_window(fixture: &Fixture, compact: bool) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    if test.window.shows_compact_density() != compact {
        test.activate("compact-density", None);
    }
    test.activate("group-by", Some("type"));
    wait_for_frames(&test.window, 4);
    test
}

/// Shows `view` and waits for its headings.
fn show(test: &TestWindow, view: FolderView) {
    test.activate("view", Some(view.as_str()));
    wait_until("the view's headings", || {
        descendants::<gtk::Label>(&test.window.folder_pane().view_widget())
            .iter()
            .any(|label| label.is_mapped() && label.has_css_class("group-title"))
    });
    wait_for_frames(&test.window, 3);
}

/// A label's font size in pixels and its weight.
fn font_of(label: &gtk::Label) -> (f64, pango::Weight) {
    let font = label
        .pango_context()
        .font_description()
        .expect("the label has a font");
    assert!(font.is_size_absolute(), "a size in pixels: {font}");
    (f64::from(font.size()) / f64::from(pango::SCALE), font.weight())
}

/// In Details, Tiles and Content, normal and compact, a heading's title is
/// 15 pixels in regular weight, where the items' names are 12, and nothing
/// draws a line after it.
///
/// parity: VIEW-022, VIEW-067, VIEW-071
#[gtk::test]
fn a_heading_has_windows_11_s_larger_regular_title_and_no_line() {
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
    fixture.write("Notes 1.txt");
    fixture.write("Notes 2.txt");
    for compact in [false, true] {
        let test = grouped_window(&fixture, compact);
        for view in [FolderView::Details, FolderView::Tiles, FolderView::Content] {
            show(&test, view);
            let shown = test.window.folder_pane().view_widget();
            let titles: Vec<gtk::Label> = descendants::<gtk::Label>(&shown)
                .into_iter()
                .filter(|label| label.is_mapped() && label.has_css_class("group-title"))
                .collect();
            assert!(!titles.is_empty(), "{view:?}, compact {compact}: headings");
            for title in &titles {
                assert_eq!(
                    font_of(title),
                    (15.0, pango::Weight::Normal),
                    "{view:?}, compact {compact}: {:?}",
                    title.text()
                );
            }
            let name = descendants::<FileCell>(&shown)
                .into_iter()
                .find(|cell| cell.is_mapped() && cell.name() == "Notes 1")
                .and_then(|cell| {
                    descendants::<gtk::Label>(&cell)
                        .into_iter()
                        .find(|label| label.is_mapped() && label.text() == "Notes 1")
                })
                .expect("an item's name");
            assert_eq!(font_of(&name).0, 12.0, "{view:?}: the items' names");
            let lines: Vec<gtk::Separator> = descendants::<gtk::Separator>(&shown)
                .into_iter()
                .filter(WidgetExt::is_mapped)
                .collect();
            assert!(
                lines.is_empty(),
                "{view:?}, compact {compact}: no line after a heading"
            );
        }
    }
}
