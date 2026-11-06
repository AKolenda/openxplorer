// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane as Windows 11 lays it out (PROP-009): a photo's own
//! details read by Properties' metadata reader without holding up the
//! pane, several items summed, and the Share and Properties buttons.
//! The photo is made here, with a fictional camera.

use std::fs;
use std::time::Duration;

use gtk::prelude::*;

use crate::media_details::tests::{camera_exif, jpeg_with_exif};
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::portal::EmailPortal;
use crate::window::details_pane::ShownProperty;

/// Opens a window on `fixture` with the details pane shown.
fn window_with_pane(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    if !test.window.details_pane().is_visible() {
        test.activate("details-pane", None);
    }
    test
}

/// The value of the row called `name` among `rows`.
fn value<'a>(rows: &'a [ShownProperty], name: &str) -> Option<&'a str> {
    let row = rows.iter().find(|row| row.name == name)?;
    Some(row.value.as_str())
}

/// The names of `rows`, top to bottom.
fn names(rows: &[ShownProperty]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

/// A fixture with a fictional camera's photo and a text file.
fn photo_fixture() -> Fixture {
    let fixture = Fixture::standard();
    fs::write(
        fixture.path("Harbour.jpg"),
        jpeg_with_exif(64, 48, &camera_exif()),
    )
    .expect("the photo");
    fixture
}

/// A photo's file rows show at once, and its camera's rows follow them
/// once the reader has read the file, in Properties' wording.
///
/// parity: PROP-009, PROP-034
#[gtk::test]
fn a_photos_camera_rows_follow_its_file_rows() {
    let fixture = photo_fixture();
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();

    test.select_named("Harbour.jpg");
    let at_once = pane.shown_properties();
    assert!(value(&at_once, "Type").is_some(), "the file rows at once");
    assert_eq!(value(&at_once, "Camera maker"), None, "the reader waits");
    assert!(pane.reading_metadata());

    wait_until("the camera's rows", || {
        value(&pane.shown_properties(), "Camera model") == Some("FO-100")
    });
    let rows = pane.shown_properties();
    let names = names(&rows);
    let taken = names.iter().position(|name| *name == "Date taken");
    let location = names.iter().position(|name| *name == "File location");
    assert!(
        taken > location && location.is_some(),
        "the photo's rows come after its file rows: {names:?}"
    );
    for (name, expected) in [
        ("Dimensions", "64 x 48"),
        ("Camera maker", "Fictional Optics"),
        ("F-stop", "f/2.8"),
        ("Exposure time", "1/125 sec."),
        ("ISO speed", "ISO-200"),
        ("Focal length", "50 mm"),
    ] {
        assert_eq!(value(&rows, name), Some(expected), "{name}");
    }

    // A redraw for the same photo keeps its rows without reading again.
    test.window.update_details_pane();
    assert_eq!(value(&pane.shown_properties(), "Camera model"), Some("FO-100"));
    assert!(!pane.reading_metadata());
}

/// Moving on before the reader starts reads nothing for the photo, and
/// a photo left while it is read never lends its rows to what follows.
///
/// parity: PROP-009
#[gtk::test]
fn a_selection_moved_on_drops_the_photos_reading() {
    let fixture = photo_fixture();
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();

    test.select_named("Harbour.jpg");
    test.select_named("Notes 2.txt");
    wait_for(Duration::from_millis(600));
    let rows = pane.shown_properties();
    assert_eq!(value(&rows, "Camera maker"), None);
    assert_eq!(value(&rows, "Date taken"), None);
    assert!(!pane.reading_metadata(), "a text file has nothing to read");
    assert_eq!(pane.shown_name(), "Notes 2", "named as the file list names it");
}

/// Several items show "N items selected" and their total size; nothing
/// selected shows the folder's item count again.
///
/// parity: PROP-009
#[gtk::test]
fn several_items_show_their_count_and_total_size() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Plan.txt"), [b'x'; 1500]).expect("a file");
    fs::write(fixture.path("Agenda.txt"), [b'y'; 500]).expect("a file");
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();
    let model = test.window.folder_model();
    let positions: Vec<u32> = ["Plan.txt", "Agenda.txt"]
        .iter()
        .map(|name| {
            let position = test.names().iter().position(|listed| listed == name);
            u32::try_from(position.expect("listed")).expect("a short listing")
        })
        .collect();

    model.select_positions(&positions);
    assert_eq!(pane.shown_name(), "2 items selected");
    assert_eq!(
        value(&pane.shown_properties(), "Total size"),
        Some(ox_core::format::pretty_bytes(2000).as_str())
    );
    model.select_positions(&[]);
    assert!(pane.shown_name().ends_with(" items)"));
}

/// Nothing selected, the pane shows Windows' hint in place of the Details
/// rows, with no Share and no Properties; a selected folder lists Type,
/// File location and Date modified over Properties, drawn with a wrench.
///
/// parity: PROP-009
#[gtk::test]
fn nothing_selected_shows_windows_hint_alone() {
    let fixture = Fixture::standard();
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();
    let folder = fixture.root();
    let title = folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    assert_eq!(pane.shown_name(), format!("{title} (4 items)"));
    assert!(pane.shows_hint());
    assert!(!pane.shows_properties(), "no Details rows");
    assert!(!pane.shows_properties_button() && !pane.shows_share());

    test.select_named("Documents");
    assert!(!pane.shows_hint());
    let names: Vec<String> = pane.shown_properties().into_iter().map(|row| row.name).collect();
    assert_eq!(names, ["Type", "File location", "Date modified"]);
    assert!(pane.shows_properties_button());
    let wrench = crate::test_support::harness::descendants::<gtk::Image>(pane)
        .iter()
        .any(|image| image.icon_name().is_some_and(|name| name.contains("wrench")));
    assert!(wrench, "Properties is drawn with a wrench");
}

/// Share shows in the pane where the desktop can share, for files it
/// can attach; Properties opens the Properties of what the pane shows.
///
/// parity: PROP-009, CMD-037
#[gtk::test]
fn the_pane_offers_share_and_properties() {
    let fixture = Fixture::standard();
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();
    assert!(!pane.shows_share(), "this desktop has no Email portal");

    let portal = EmailPortal::answering(0);
    test.window.follow_share_service(&portal.name());
    wait_until("the window to ask the portal", || {
        test.window.share_service_checked(&portal.name())
    });
    wait_for_frames(&test.window, 2);
    assert!(!pane.shows_share(), "nothing selected, nothing to share");
    test.select_named("Documents");
    assert!(!pane.shows_share(), "a folder cannot be shared");
    test.select_named("Notes 2.txt");
    assert!(pane.shows_share());

    let properties = crate::test_support::harness::descendants::<gtk::Button>(pane)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.properties"))
        .expect("the Properties button");
    assert!(properties.is_visible() && properties.is_sensitive());
    properties.emit_clicked();
    test.wait_for_properties("Notes 2.txt Properties");
}

/// The pane's keyboard stops are its buttons: its names and values can be
/// selected with the pointer but take no Tab, so focus moving on its own
/// (after a menu closes) never lands on a label (see `property_value`).
///
/// parity: PROP-009
#[gtk::test]
fn the_panes_labels_are_no_tab_stops() {
    let fixture = Fixture::standard();
    let test = window_with_pane(&fixture);
    test.select_named("Notes 2.txt");
    let pane = test.window.details_pane();
    let labels = crate::test_support::harness::descendants::<gtk::Label>(pane);
    assert!(
        labels.iter().any(gtk::Label::is_selectable),
        "values can be selected"
    );
    let stops: Vec<String> = labels
        .iter()
        .filter(|label| label.is_selectable() && (label.is_focusable() || label.can_focus()))
        .map(|label| label.text().to_string())
        .collect();
    assert!(stops.is_empty(), "labels that take Tab: {stops:?}");
    assert!(
        pane.child_focus(gtk::DirectionType::TabForward),
        "a Tab into the pane"
    );
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a control has the keyboard");
    assert!(focus.is::<gtk::Button>(), "lands on a button: {focus:?}");
    assert!(focus.is_ancestor(pane));
}

/// With nothing selected in search results, the pane counts the results
/// the list shows, as the status bar does and Windows' pane does, not the
/// folder searched; leaving the search counts the folder again.
///
/// parity: PROP-009
#[gtk::test]
fn search_results_are_counted_as_the_list_shows_them() {
    let fixture = Fixture::standard();
    let test = window_with_pane(&fixture);
    test.start_search_cache();
    let pane = test.window.details_pane();
    assert!(pane.shown_name().ends_with(" (4 items)"), "the folder");

    test.search_for("notes");

    assert_eq!(test.names().len(), 2);
    wait_until("the pane to count the results", || {
        pane.shown_name().ends_with(" (2 items)")
    });
    test.window.search_box().entry().set_text("");
    wait_until("the folder counted again", || {
        pane.shown_name().ends_with(" (4 items)")
    });
}

/// A selected folder whose picture changes in Properties shows the new
/// one in the pane at once, as the file list does: OK on the Customize
/// tab calls `refresh_item_icon`, which the pane follows too.
///
/// parity: PROP-009, PROP-016
#[gtk::test]
fn a_folders_new_picture_shows_in_the_pane_at_once() {
    let fixture = Fixture::standard();
    let pixels = gtk::glib::Bytes::from_owned(vec![0x60_u8; 8 * 8 * 4]);
    let texture = gtk::gdk::MemoryTexture::new(8, 8, gtk::gdk::MemoryFormat::R8g8b8a8, &pixels, 32);
    texture.save_to_png(fixture.path("Star.png")).expect("a picture");
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();
    test.select_named("Documents");
    wait_for(Duration::from_millis(300));
    assert!(
        crate::test_support::harness::descendants::<gtk::Picture>(pane).is_empty(),
        "a plain folder shows its art"
    );

    let folder = fixture.uri_of("Documents");
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let finished = std::rc::Rc::clone(&done);
    let (target, icon) = (folder.clone(), fixture.uri_of("Star.png"));
    gtk::glib::spawn_future_local(async move {
        crate::properties::set_custom_icon(&target, Some(icon))
            .await
            .expect("the metadata is stored");
        finished.set(true);
    });
    wait_until("the change", || done.get());
    test.window.refresh_item_icon(&folder);

    wait_until("the new picture in the pane", || {
        !crate::test_support::harness::descendants::<gtk::Picture>(pane).is_empty()
    });
    assert_eq!(test.selected_names(), ["Documents"], "without selecting again");
}
