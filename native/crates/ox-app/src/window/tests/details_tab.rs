// SPDX-License-Identifier: AGPL-3.0-only
//! The Details tab of a file's Properties in a real window, over a photo,
//! a song and a text file made here with fictional tags.

use std::fs;

use gtk::glib;
use gtk::prelude::*;
use gtk::{gdk, glib::translate::IntoGlib};

use super::item_dialogs::tab;
use crate::media_details::tests::{camera_exif, jpeg_with_exif, write_song};
use crate::properties::PropertiesWindow;
use crate::test_support::harness::{descendants, settle, wait_until, Fixture, TestWindow};

/// The rows of the Details list.
fn rows(window: &PropertiesWindow) -> Vec<gtk::ListBoxRow> {
    let page = tab(window, "details");
    let Some(list) = descendants::<gtk::ListBox>(&page).into_iter().next() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    let mut child = list.first_child();
    while let Some(widget) = child {
        if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>() {
            rows.push(row);
        }
        child = widget.next_sibling();
    }
    rows
}

/// Each row's text: a heading, or `Name: value`; stars read as `*` and
/// `-`.
fn lines(window: &PropertiesWindow) -> Vec<String> {
    rows(window)
        .iter()
        .map(|row| {
            let texts: Vec<String> = descendants::<gtk::Label>(row)
                .iter()
                .map(|label| label.text().to_string())
                .collect();
            if row.has_css_class("details-heading-row") {
                return texts.join("");
            }
            let stars: String = descendants::<gtk::Image>(row)
                .iter()
                .map(|image| {
                    let filled = image.icon_name().is_some_and(|name| name.contains("filled"));
                    if filled {
                        '*'
                    } else {
                        '-'
                    }
                })
                .collect();
            let value = texts.get(1).cloned().unwrap_or(stars);
            format!("{}: {value}", texts.first().cloned().unwrap_or_default())
        })
        .collect()
}

/// The row whose name is `name`.
fn row_named(window: &PropertiesWindow, name: &str) -> gtk::ListBoxRow {
    rows(window)
        .into_iter()
        .find(|row| row.widget_name() == name)
        .unwrap_or_else(|| panic!("a {name} row"))
}

/// Opens the Properties of `name` in `test` on its Details tab.
fn open_details(test: &TestWindow, name: &str) -> PropertiesWindow {
    test.select_named(name);
    test.activate("properties", None);
    let window = test.wait_for_properties(&format!("{name} Properties"));
    // The window puts the keyboard on its tabs when it first maps.
    wait_until("the window to show", || window.is_mapped());
    settle();
    window.select_tab("details");
    window
}

/// Presses `key` with `modifiers` on the Details list.
fn press(window: &PropertiesWindow, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let page = tab(window, "details");
    let list = descendants::<gtk::ListBox>(&page).remove(0);
    let controller = list
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("the list's key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &modifiers])
}

/// A photo's Details tab sits between Security and Previous Versions and
/// lists its EXIF under Windows' headings, with its stars; Ctrl+C copies
/// the cursor's value and Enter lets its text be selected.
///
/// parity: PROP-034
#[gtk::test]
fn a_photos_details_tab_lists_its_exif() {
    let fixture = Fixture::empty();
    fs::write(
        fixture.path("Harbour.jpg"),
        jpeg_with_exif(64, 48, &camera_exif()),
    )
    .expect("the photo");
    let test = TestWindow::open(&fixture.uri());

    let window = open_details(&test, "Harbour.jpg");

    assert_eq!(
        window.tab_labels(),
        ["General", "Security", "Details", "Previous Versions"]
    );
    wait_until("the photo's details", || {
        lines(&window).iter().any(|line| line == "Camera")
    });
    let lines = lines(&window);
    let headings: Vec<&str> = rows(&window)
        .iter()
        .zip(&lines)
        .filter(|(row, _)| row.has_css_class("details-heading-row"))
        .map(|(_, line)| line.as_str())
        .collect();
    assert_eq!(
        headings,
        [
            "Description",
            "Origin",
            "Image",
            "Camera",
            "Advanced photo",
            "File"
        ]
    );
    for expected in [
        "Title: Harbour at dusk",
        "Rating: ****-",
        "Authors: A. Example",
        "Program name: Example Editor 2",
        "Dimensions: 64 x 48",
        "Width: 64 pixels",
        "Horizontal resolution: 300 dpi",
        "Bit depth: 24",
        "Camera maker: Fictional Optics",
        "Camera model: FO-100",
        "F-stop: f/2.8",
        "Exposure time: 1/125 sec.",
        "ISO speed: ISO-200",
        "Exposure bias: -0.7 step",
        "Focal length: 50 mm",
        "Metering mode: Pattern",
        "Flash mode: No flash, compulsory",
        "Lens model: FO 50mm F2.8",
        "EXIF version: 0232",
        "Name: Harbour.jpg",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected} in {lines:#?}"
        );
    }

    let model = row_named(&window, "Camera model");
    let list = model.parent().and_downcast::<gtk::ListBox>().expect("the list");
    list.select_row(Some(&model));
    model.grab_focus();
    assert!(press(&window, gdk::Key::c, gdk::ModifierType::CONTROL_MASK));
    let clipboard = window.clipboard();
    let copied = glib::MainContext::default().block_on(clipboard.read_text_future());
    assert_eq!(copied.ok().flatten().as_deref(), Some("FO-100"));

    model.emit_activate();
    let value = descendants::<gtk::Label>(&model).remove(1);
    assert!(value.is_focus(), "Enter puts the keyboard in the value");
    assert!(value.is_selectable(), "its text can be selected and copied");
    // The selection itself is not checked: GTK drops a label's selection
    // whenever another client owns X11's primary selection, which the test
    // display hands around at random.
}

/// A song's Details list its tags, stream and empty stars; a text file
/// has only its File rows.
///
/// parity: PROP-034
#[gtk::test]
fn a_songs_details_and_a_documents_file_rows() {
    let fixture = Fixture::empty();
    write_song(&fixture.path("Lighthouse Waltz.ogg"));
    fixture.write("Notes.txt");
    let test = TestWindow::open(&fixture.uri());

    let song = open_details(&test, "Lighthouse Waltz.ogg");
    wait_until("the song's details", || {
        lines(&song).iter().any(|line| line == "Media")
    });
    let lines_shown = lines(&song);
    for expected in [
        "Title: Lighthouse Waltz",
        "Rating: -----",
        "Contributing artists: The Example Quartet",
        "Album artist: Example Ensemble",
        "Album: Coastal Sketches",
        "Year: 2025",
        "#: 3",
        "Genre: Folk",
        "Length: 00:00:02",
        "Channels: 2 (stereo)",
        "Audio sample rate: 44.100 kHz",
        "Composers: B. Placeholder",
    ] {
        assert!(
            lines_shown.iter().any(|line| line == expected),
            "{expected} in {lines_shown:#?}"
        );
    }
    song.close();

    let notes = open_details(&test, "Notes.txt");
    wait_until("the file rows", || !rows(&notes).is_empty());
    let shown = lines(&notes);
    assert_eq!(shown.first().map(String::as_str), Some("File"));
    assert!(shown.contains(&"Name: Notes.txt".to_owned()), "{shown:#?}");
    assert!(shown.iter().any(|line| line.starts_with("Size: ")), "{shown:#?}");
}

/// A folder has no Details tab, as in Windows.
///
/// parity: PROP-034
#[gtk::test]
fn a_folder_has_no_details_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Documents");
    test.activate("properties", None);
    let window = test.wait_for_properties("Documents Properties");
    assert!(!window.tab_labels().contains(&"Details".to_owned()));
}

/// Properties opened straight on Details, before the item is read, still
/// read the file's own rows.
///
/// parity: PROP-034
#[gtk::test]
fn properties_opened_on_details_read_the_media_rows() {
    let fixture = Fixture::empty();
    fs::write(
        fixture.path("Harbour.jpg"),
        jpeg_with_exif(64, 48, &camera_exif()),
    )
    .expect("the photo");
    let test = TestWindow::open(&fixture.uri());

    test.window.open_properties_of(
        &fixture.uri_of("Harbour.jpg"),
        crate::properties::PropertiesTab::Details,
    );

    let window = test.wait_for_properties("Harbour.jpg Properties");
    assert_eq!(window.selected_tab().as_deref(), Some("details"));
    wait_until("the photo's details", || {
        lines(&window).iter().any(|line| line == "Camera model: FO-100")
    });
}
