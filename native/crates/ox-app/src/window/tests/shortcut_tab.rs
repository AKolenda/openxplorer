// SPDX-License-Identifier: AGPL-3.0-only
//! The Shortcut tab of a link's or a launcher's Properties: what it points
//! to, where that is, and, for a launcher, where it starts and its comment,
//! as Windows' Shortcut tab shows a shortcut's.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use gtk::prelude::*;

use super::item_dialogs::{button, tab, texts, value_after};
use crate::properties::PropertiesWindow;
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

/// Opens the Properties of the item called `name` and waits for its
/// Shortcut tab to be read.
fn shortcut_properties(test: &TestWindow, name: &str) -> (PropertiesWindow, gtk::Widget) {
    test.select_named(name);
    test.activate("properties", None);
    let window = test.wait_for_properties(&format!("{name} Properties"));
    let page = tab(&window, "shortcut");
    wait_until("the shortcut to be read", || {
        value_after(&page, "Target type:").is_some()
    });
    (window, page)
}

/// The text field after the name `name` on `page`.
fn field(page: &gtk::Widget, name: &str) -> gtk::Entry {
    let names: Vec<gtk::Label> = descendants::<gtk::Label>(page);
    let label = names
        .iter()
        .find(|label| label.text() == name)
        .unwrap_or_else(|| panic!("a {name} row"));
    // The field the name's underlined letter moves to.
    label
        .mnemonic_widget()
        .and_downcast::<gtk::Entry>()
        .unwrap_or_else(|| panic!("a {name} field"))
}

/// A link to a file has General, Shortcut, Security, Details and Previous
/// Versions, in Windows' order; its Shortcut tab names the target's type,
/// its folder and its path, Open File Location shows the target selected
/// in its folder, and a new Target applied points the link there.
///
/// parity: PROP-004
#[gtk::test]
fn a_links_shortcut_tab_shows_and_changes_its_target() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Plans")).expect("a folder");
    fs::write(fixture.path("Plans/Budget.txt"), b"budget").expect("a file");
    fs::write(fixture.path("Plans/Agenda.txt"), b"agenda").expect("a file");
    std::os::unix::fs::symlink(fixture.path("Plans/Budget.txt"), fixture.path("Budget link"))
        .expect("the link");
    let test = TestWindow::open(&fixture.uri());
    let (window, page) = shortcut_properties(&test, "Budget link");

    assert_eq!(
        window.tab_labels(),
        ["General", "Shortcut", "Security", "Details", "Previous Versions"]
    );
    assert_eq!(value_after(&page, "Target location:").as_deref(), Some("Plans"));
    assert!(value_after(&page, "Target type:").is_some_and(|kind| !kind.is_empty()));
    let target = field(&page, "Target:");
    assert_eq!(
        target.text(),
        fixture.path("Plans/Budget.txt").to_str().expect("UTF-8")
    );
    assert!(!texts(&tab(&window, "general")).contains(&"Opens with:".to_owned()));

    button(&page, "Open File Location").emit_clicked();
    wait_until("the target selected in its folder", || {
        test.window.current_uri().as_deref() == Some(fixture.uri_of("Plans").as_str())
            && test.selected_names() == ["Budget.txt"]
    });

    target.set_text(fixture.path("Plans/Agenda.txt").to_str().expect("UTF-8"));
    assert!(
        window.apply_button().is_sensitive(),
        "a new target waits for Apply"
    );
    window.apply_button().emit_clicked();
    wait_until("the link to point at the new target", || {
        fs::read_link(fixture.path("Budget link")).ok() == Some(fixture.path("Plans/Agenda.txt"))
    });
    assert_eq!(
        fs::read(fixture.path("Plans/Budget.txt")).expect("kept"),
        b"budget"
    );
    wait_until("Target location read again", || {
        value_after(&page, "Target location:").as_deref() == Some("Plans")
    });
    window.close();
}

/// A link to a folder is a shortcut, not the folder: it has no Sharing or
/// Customize tab, as a shortcut has none in Windows, and its General tab
/// names no application to open it with.
///
/// parity: PROP-004
#[gtk::test]
fn a_link_to_a_folder_has_the_tabs_of_a_shortcut() {
    let fixture = Fixture::standard();
    std::os::unix::fs::symlink(fixture.path("Documents"), fixture.path("Docs link")).expect("the link");
    let test = TestWindow::open(&fixture.uri());
    let (window, page) = shortcut_properties(&test, "Docs link");

    assert_eq!(
        window.tab_labels(),
        ["General", "Shortcut", "Security", "Previous Versions"]
    );
    assert_eq!(
        field(&page, "Target:").text(),
        fixture.path("Documents").to_str().expect("UTF-8")
    );
    assert!(!texts(&tab(&window, "general")).contains(&"Opens with:".to_owned()));
    window.close();
}

/// A launcher (a `.desktop` file) has a Shortcut tab with its command as
/// Target, its working folder as Start in and its Comment; changes are
/// written on Apply, keeping its other keys, its translations and its
/// permission to run.
///
/// parity: PROP-004
#[gtk::test]
fn a_launchers_shortcut_tab_edits_its_command_folder_and_comment() {
    let fixture = Fixture::standard();
    let path = fixture.path("Example Editor.desktop");
    let start = fixture.path("Documents");
    fs::write(
        &path,
        format!(
            "[Desktop Entry]\n# A fictional editor\nType=Application\nName=Example Editor\n\
             Name[de]=Beispiel\nComment=Edits notes\nExec=true --example %F\nPath={}\nIcon=text-editor\n",
            start.display()
        ),
    )
    .expect("the launcher");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("runnable");
    let test = TestWindow::open(&fixture.uri());
    let (window, page) = shortcut_properties(&test, "Example Editor.desktop");

    assert_eq!(
        window.tab_labels(),
        ["General", "Shortcut", "Security", "Details", "Previous Versions"]
    );
    assert_eq!(field(&page, "Target:").text(), "true --example %F");
    assert_eq!(field(&page, "Start in:").text(), start.to_str().expect("UTF-8"));
    assert_eq!(field(&page, "Comment:").text(), "Edits notes");
    assert!(!texts(&tab(&window, "general")).contains(&"Opens with:".to_owned()));

    field(&page, "Start in:").set_text("/tmp");
    field(&page, "Comment:").set_text("Edits fictional notes");
    window.apply_button().emit_clicked();
    wait_until("the launcher written", || {
        fs::read_to_string(&path).is_ok_and(|text| text.contains("Path=/tmp\n"))
    });
    let written = fs::read_to_string(&path).expect("the launcher");
    assert!(written.contains("Comment=Edits fictional notes\n"), "{written}");
    assert!(written.contains("Name[de]=Beispiel\n"), "{written}");
    assert!(written.contains("# A fictional editor\n"), "{written}");
    assert!(written.contains("Exec=true --example %F\n"), "{written}");
    assert_eq!(
        fs::metadata(&path).expect("the launcher").permissions().mode() & 0o777,
        0o755
    );
    window.close();
}
