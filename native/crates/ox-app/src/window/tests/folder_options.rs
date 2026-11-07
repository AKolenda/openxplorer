// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options' choices that only it sets (VIEW-021): Browse folders,
//! Privacy and Clear `OpenXplorer` history on the General tab, the status
//! bar, the infotips and typing into the search box on the View tab, and
//! searching names and contents on the Search tab.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{same_location, RECENT_LOCATIONS_URI, RECENT_URI};
use ox_core::search::SearchIn;
use ox_core::settings::{RecentEntry, Settings};

use super::file_ops_support::select_names;
use super::view_styles::{close_folder_options, open_folder_options, option_check, tab_button};
use crate::test_support::harness::{wait_until, Fixture, OpenedWindows, TestWindow};

/// Whether the saved settings hide the navigation pane's place `uri`.
fn hides_place(test: &TestWindow, uri: &str) -> bool {
    let preferences = test.context.settings_data().preferences;
    preferences
        .hidden_sidebar_places
        .iter()
        .any(|place| same_location(place, uri))
}

/// General: Browse folders' "Open each folder in its own window" opens a
/// folder opened from the file list, by double-click or by Open on its
/// context menu, in a new window and leaves this one where it is; Click
/// items as follows offers only Double-click, which is chosen; Privacy's
/// check boxes hide Recent files and Recent locations from the navigation
/// pane; and Clear forgets the recent files and the typed addresses at
/// once, before OK.
///
/// parity: VIEW-020
#[gtk::test]
fn the_general_tab_opens_folders_in_their_own_window_and_clears_history() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    let general = |label: &str| option_check(&options, "General", label);
    assert!(general("Open each folder in the same window").is_active());
    assert!(!general("Single-click to open an item (point to select)").is_sensitive());
    assert!(general("Double-click to open an item (single-click to select)").is_active());
    assert!(general("Show recently used files").is_active());
    assert!(general("Show frequently used folders").is_active());
    general("Open each folder in its own window").set_active(true);
    general("Show recently used files").set_active(false);
    general("Show frequently used folders").set_active(false);

    test.window.address_bar().remember_typed("/srv/Example projects");
    Settings::open(test.settings_directory())
        .remember_open(RecentEntry {
            uri: fixture.uri_of("Notes 2.txt"),
            name: "Notes 2.txt".into(),
            type_label: "Text".into(),
            is_dir: false,
            size: 1,
            modified: 1,
            opened: None,
        })
        .expect("the settings file takes the recent file");
    test.context.reload_settings();
    wait_until("the recent file to be remembered", || {
        !test.context.settings_data().recent.is_empty()
    });
    tab_button(&options, "General", "Clear").emit_clicked();
    wait_until("the recent files to be forgotten", || {
        test.context.settings_data().recent.is_empty()
    });
    assert!(
        test.window.address_bar().typed_history().is_empty(),
        "the typed addresses are forgotten"
    );
    assert!(
        !test
            .context
            .settings_data()
            .preferences
            .open_folders_in_own_window,
        "Clear saves nothing else"
    );

    close_folder_options(&test, &options, "OK");
    wait_until("the choices to be saved", || {
        test.context
            .settings_data()
            .preferences
            .open_folders_in_own_window
            && hides_place(&test, RECENT_URI)
            && hides_place(&test, RECENT_LOCATIONS_URI)
    });
    let double_click = || test.window.activate_from_view(test.position_of("Documents"));
    let open = || {
        select_names(&test, &["Documents"]);
        test.activate("open", None);
    };
    for (way, open_documents) in [("double-click", &double_click as &dyn Fn()), ("Open", &open)] {
        open_documents();
        let opened = OpenedWindows::only(&[&test.window]);
        wait_until("the folder in the new window", || {
            opened.window().current_uri() == Some(fixture.uri_of("Documents"))
        });
        assert_eq!(
            test.window.current_uri(),
            Some(fixture.uri()),
            "{way}: this window stays"
        );
    }
}

/// View: "Show status bar" and "Show pop-up description for folder and
/// desktop items" off hide the status bar and the items' infotips, and
/// "Automatically type into the Search Box" sends text typed in the file
/// list to the search box; Restore Defaults brings them all back.
///
/// parity: VIEW-020
#[gtk::test]
fn the_view_tab_hides_the_status_bar_and_infotips_and_types_into_the_search_box() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(test.window.status_bar().is_visible());
    let options = open_folder_options(&test);
    let view = |label: &str| option_check(&options, "View", label);
    assert!(view("Select the typed item in the view").is_active());
    view("Show status bar").set_active(false);
    view("Show pop-up description for folder and desktop items").set_active(false);
    view("Show preview handlers in preview pane").set_active(false);
    view("Automatically type into the Search Box").set_active(true);
    close_folder_options(&test, &options, "OK");
    wait_until("the status bar and infotips to go", || {
        !test.window.status_bar().is_visible() && test.window.folder_pane().owners().hides_infotips()
    });
    assert!(test.context.settings_data().preferences.hide_preview_handlers);

    test.window.type_text("No");
    let entry = test.window.search_box().entry();
    assert_eq!(entry.text(), "No");
    let focus = GtkWindowExt::focus(&test.window);
    assert!(
        focus.is_some_and(|focus| focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&entry)),
        "the search box has the keyboard"
    );
    test.window.search_box().clear();

    let options = open_folder_options(&test);
    tab_button(&options, "View", "Restore Defaults").emit_clicked();
    close_folder_options(&test, &options, "OK");
    wait_until("the defaults to come back", || {
        let preferences = test.context.settings_data().preferences;
        !preferences.type_into_search_box
            && !preferences.hide_preview_handlers
            && test.window.status_bar().is_visible()
            && !test.window.folder_pane().owners().hides_infotips()
    });
}

/// Search: "Always search file names and contents" makes searches look at
/// names and contents, as the search strip's "Names and contents" does,
/// and off, at names again.
///
/// parity: VIEW-020, SRCH-036
#[gtk::test]
fn the_search_tab_searches_names_and_contents() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let search_in = || test.window.imp().search.borrow().search_in();
    assert_eq!(search_in(), SearchIn::Names);
    let contents = "Always search file names and contents (this might take several minutes)";
    let options = open_folder_options(&test);
    option_check(&options, "Search", contents).set_active(true);
    options.press("Apply");
    wait_until("searches to read contents", || {
        search_in() == SearchIn::NamesAndContents
    });
    tab_button(&options, "Search", "Restore Defaults").emit_clicked();
    assert!(!option_check(&options, "Search", contents).is_active());
    close_folder_options(&test, &options, "OK");
    wait_until("searches to read names again", || search_in() == SearchIn::Names);
}

/// The close button in the title bar of the titled dialog `options`.
fn title_bar_close(options: &crate::properties::PropertiesWindow) -> gtk::Button {
    let bar = options.titlebar().expect("Folder Options has a title bar");
    crate::test_support::harness::descendants::<gtk::Button>(&bar)
        .into_iter()
        .find(|button| button.has_css_class("close"))
        .expect("the title bar has a close button")
}

/// Folder Options is a window of its own, as in Windows 11, not a layer
/// inside the window it was opened from: it has a title bar with its name
/// and a close button, by which it is moved, and it is a modal child of
/// that window alone. It joins that window's window group, so another
/// window stays usable while it is open. Its close button cancels, as
/// Cancel and Escape do, and gives the window back.
///
/// parity: VIEW-020
#[gtk::test]
fn folder_options_is_a_modal_window_of_its_own() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri_of("Documents"));
    let options = open_folder_options(&test);
    let window: &gtk::Window = test.window.upcast_ref();

    assert_eq!(options.transient_for().as_ref(), Some(window));
    assert!(options.is_modal(), "it holds back the window it belongs to");
    assert!(options.parent().is_none(), "it is no widget inside the window");
    // Windows 11 draws it about 380 by 483 pixels at 100 %.
    wait_until("Folder Options to have its size", || options.width() > 0);
    // Without a compositor, as on Xvfb, GTK draws a solid frame of 4
    // pixels a side inside that width.
    assert!(
        (376..=380).contains(&options.width()),
        "as wide as Windows' Folder Options, not {}",
        options.width()
    );
    assert!(
        options.height() < 540,
        "about as tall as Windows' Folder Options, not {}",
        options.height()
    );
    assert!(
        test.window.dialog_layer().shown().is_none(),
        "nothing is shown over the window's content"
    );
    let bar = options.titlebar().expect("Folder Options has a title bar");
    assert!(bar.is::<gtk::WindowHandle>(), "the title bar moves the window");
    let title = crate::test_support::harness::descendants::<gtk::Label>(&bar)
        .into_iter()
        .map(|label| label.text().to_string())
        .collect::<Vec<_>>();
    assert_eq!(title, ["Folder Options"]);

    assert_eq!(
        options.group(),
        test.window.group(),
        "it holds back its own window"
    );
    assert_ne!(other.window.group(), test.window.group(), "and no other window");

    let browse = option_check(&options, "General", "Open each folder in its own window");
    browse.set_active(true);
    title_bar_close(&options).emit_clicked();
    wait_until("Folder Options to close", || !options.is_visible());
    assert!(
        !test
            .context
            .settings_data()
            .preferences
            .open_folders_in_own_window,
        "the close button cancels"
    );
    assert!(!test.window.has_open_dialog(), "the window is usable again");
    assert!(other.window.is_visible());
}

/// Folder Options beside the window it was opened from, light and dark,
/// for review with `$OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`): the
/// window, each tab of Folder Options, and, under a window manager, the
/// screen with Folder Options moved partly off its window.
#[gtk::test]
fn folder_options_as_a_window_is_captured_light_and_dark() {
    use crate::test_support::harness::{capture, capture_dialog, wait_for_frames, ThemeGuard};

    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.activate("theme", Some(theme));
        test.window.set_default_size(760, 540);
        let options = open_folder_options(&test);
        capture(&test.window, &format!("folder-options-window-parent-{theme}.png"));
        for tab in ["view", "search", "general"] {
            options.select_tab(tab);
            capture_dialog(&options, &format!("folder-options-window-{tab}-{theme}.png"));
        }
        if let Some(directory) = std::env::var_os("OX_NATIVE_CAPTURE_DIR") {
            if std::env::var_os("OX_CAPTURE_SCREEN").is_some() {
                // Moved by the window manager, as dragging its title bar does.
                let moved = std::process::Command::new("xdotool")
                    .args([
                        "search",
                        "--sync",
                        "--name",
                        "^Folder Options$",
                        "windowmove",
                        "%@",
                        "1340",
                        "200",
                    ])
                    .status()
                    .is_ok_and(|status| status.success());
                assert!(moved, "the window manager moves Folder Options");
                crate::test_support::harness::wait_for(std::time::Duration::from_millis(500));
                wait_for_frames(&options, 3);
                let path = std::path::Path::new(&directory)
                    .join(format!("folder-options-window-screen-{theme}.png"));
                let saved = std::process::Command::new("import")
                    .args(["-window", "root"])
                    .arg(&path)
                    .status()
                    .is_ok_and(|status| status.success());
                assert!(saved, "the screen is saved");
            }
        }
        close_folder_options(&test, &options, "Cancel");
    }
}
