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
use crate::test_support::harness::{descendants, wait_until, Fixture, OpenedWindows, TestWindow};

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
/// items as follows has Double-click chosen, as in Windows; Privacy's
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

/// General: Click items as follows starts at Windows' Double-click, its
/// underline choices grayed out until Single-click is chosen; OK turns
/// single-click on in every open window at once, underlining every name
/// with "consistent with my browser", and Restore Defaults brings
/// Double-click back, which every window follows too.
///
/// parity: SEL-018, VIEW-020
#[gtk::test]
fn click_items_as_follows_turns_single_click_on_in_every_window() {
    use crate::window::single_click::ClickMode;

    const SINGLE: &str = "Single-click to open an item (point to select)";
    const DOUBLE: &str = "Double-click to open an item (single-click to select)";
    const ALWAYS: &str = "Underline icon titles consistent with my browser";
    const ON_POINT: &str = "Underline icon titles only when I point at them";
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri_of("Documents"));
    let follow = |mode: ClickMode| {
        for window in [&test.window, &other.window] {
            wait_until("every window to follow", || window.click_mode() == mode);
            let view = window.folder_pane().view_widget();
            let single = matches!(mode, ClickMode::Single { .. });
            let always = mode
                == ClickMode::Single {
                    underline_always: true,
                };
            assert_eq!(view.has_css_class("single-click"), single);
            assert_eq!(view.has_css_class("underlined-titles"), always);
        }
    };

    let options = open_folder_options(&test);
    let general = |label: &str| option_check(&options, "General", label);
    assert!(general(DOUBLE).is_active(), "Windows' default");
    assert!(general(ON_POINT).is_active(), "Windows' default underline");
    for underline in [ALWAYS, ON_POINT] {
        assert!(
            !general(underline).is_sensitive(),
            "{underline}: grayed for Double-click"
        );
    }
    general(SINGLE).set_active(true);
    for underline in [ALWAYS, ON_POINT] {
        assert!(
            general(underline).is_sensitive(),
            "{underline}: chosen with Single-click"
        );
    }
    general(ALWAYS).set_active(true);
    close_folder_options(&test, &options, "OK");
    wait_until("the choices to be saved", || {
        let preferences = test.context.settings_data().preferences;
        preferences.single_click_open && preferences.underline_titles_always
    });
    follow(ClickMode::Single {
        underline_always: true,
    });

    let options = open_folder_options(&test);
    let general = |label: &str| option_check(&options, "General", label);
    assert!(general(SINGLE).is_active() && general(ALWAYS).is_active());
    tab_button(&options, "General", "Restore Defaults").emit_clicked();
    assert!(general(DOUBLE).is_active() && general(ON_POINT).is_active());
    assert!(!general(ALWAYS).is_sensitive());
    close_folder_options(&test, &options, "OK");
    wait_until("the defaults to be saved", || {
        let preferences = test.context.settings_data().preferences;
        !preferences.single_click_open && !preferences.underline_titles_always
    });
    follow(ClickMode::Double);
}

/// Privacy's "Show recently used files" and "Show frequently used
/// folders" choose whether the navigation pane lists Recent files and
/// Recent locations, which it lists only while Settings' "Show more items
/// in the navigation pane" is on (SIDE-034). While it is on, the default,
/// they work as before. While it is off, both are grayed out and say where
/// to turn the rows on; Folder Options does not turn that switch on.
///
/// parity: VIEW-020, SIDE-034
#[gtk::test]
fn privacy_check_boxes_follow_the_navigation_pane_switch() {
    const LABELS: [&str; 2] = ["Show recently used files", "Show frequently used folders"];
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    for label in LABELS {
        let check = option_check(&options, "General", label);
        assert!(check.is_sensitive(), "{label} works with the switch on");
        assert_eq!(check.tooltip_text(), None, "{label}");
    }
    let recent_rows = || {
        let labels = test.window.sidebar().labels();
        ["Recent files", "Recent locations"].map(|row| labels.iter().any(|label| label == row))
    };
    wait_until("the Recent rows", || recent_rows() == [true, true]);
    option_check(&options, "General", LABELS[0]).set_active(false);
    close_folder_options(&test, &options, "OK");
    wait_until("Recent files to leave the pane", || {
        recent_rows() == [false, true]
    });

    test.set_more_navigation_pane_items(false);
    let options = open_folder_options(&test);
    for label in LABELS {
        let check = option_check(&options, "General", label);
        assert!(!check.is_sensitive(), "{label} is grayed out");
        let tooltip = check.tooltip_text().map(String::from).unwrap_or_default();
        assert!(
            tooltip.contains("Show more items in the navigation pane")
                && tooltip.contains("Settings > Appearance"),
            "{label}: {tooltip:?}"
        );
    }
    close_folder_options(&test, &options, "Cancel");
    assert!(
        !test
            .context
            .settings_data()
            .preferences
            .more_navigation_pane_items,
        "Folder Options leaves the switch off"
    );
}

/// Privacy's check boxes follow "Show more items in the navigation pane"
/// while Folder Options is open, whichever window or process changes it:
/// grayed out, saying why, as soon as it is turned off, and usable again,
/// without the tooltip, as soon as it is turned back on (SIDE-034).
///
/// parity: SIDE-034
#[gtk::test]
fn privacy_check_boxes_follow_the_switch_while_folder_options_is_open() {
    const LABELS: [&str; 2] = ["Show recently used files", "Show frequently used folders"];
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    let checks = LABELS.map(|label| option_check(&options, "General", label));
    assert!(checks.iter().all(gtk::prelude::WidgetExt::is_sensitive));

    test.set_more_navigation_pane_items(false);
    wait_until("Privacy to be grayed out", || {
        checks.iter().all(|check| !check.is_sensitive())
    });
    for check in &checks {
        let tooltip = check.tooltip_text().map(String::from).unwrap_or_default();
        assert!(
            tooltip.contains("Show more items in the navigation pane"),
            "{tooltip:?}"
        );
    }

    test.set_more_navigation_pane_items(true);
    wait_until("Privacy to work again", || {
        checks.iter().all(gtk::prelude::WidgetExt::is_sensitive)
    });
    for check in &checks {
        assert_eq!(check.tooltip_text(), None);
    }
    close_folder_options(&test, &options, "Cancel");
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

/// Search: "Include system directories" starts checked and "Include
/// compressed files" unchecked, as in Windows; Apply saves them, and
/// Restore Defaults puts them back (#231).
#[gtk::test]
fn the_search_tab_includes_system_directories_and_compressed_files() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let system = "Include system directories";
    let compressed = "Include compressed files (ZIP, 7Z, TAR...)";
    let options = open_folder_options(&test);
    assert!(option_check(&options, "Search", system).is_active());
    assert!(!option_check(&options, "Search", compressed).is_active());
    option_check(&options, "Search", system).set_active(false);
    option_check(&options, "Search", compressed).set_active(true);
    options.press("Apply");
    let saved = || test.window.context().settings_data().preferences;
    wait_until("the search options to be saved", || {
        !saved().search_system_directories && saved().search_compressed_files
    });
    tab_button(&options, "Search", "Restore Defaults").emit_clicked();
    assert!(option_check(&options, "Search", system).is_active());
    assert!(!option_check(&options, "Search", compressed).is_active());
    close_folder_options(&test, &options, "OK");
    wait_until("the defaults to be saved", || {
        saved().search_system_directories && !saved().search_compressed_files
    });
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

/// Counts the primary presses that reach `window`, before any of its
/// widgets takes them.
fn count_presses(window: &impl IsA<gtk::Widget>) -> std::rc::Rc<std::cell::Cell<u32>> {
    let presses = std::rc::Rc::new(std::cell::Cell::new(0));
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let counted = std::rc::Rc::clone(&presses);
    click.connect_pressed(move |_, _, _, _| counted.set(counted.get() + 1));
    window.add_controller(click);
    presses
}

/// Clicks the primary button at (`x`, `y`) in `widget`, with the real
/// pointer.
fn click_in(
    pointer: &mut crate::test_support::pointer::Pointer,
    widget: &impl IsA<gtk::Widget>,
    point: (f64, f64),
) {
    use crate::test_support::pointer::{screen_point, Button};
    let (x, y) = screen_point(pointer, widget, point, (10, 10));
    pointer.move_to(x, y);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    crate::test_support::harness::settle();
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(200));
}

/// With real clicks, as in Windows: while Folder Options is open, the
/// Explorer window it belongs to takes no click, another Explorer window
/// still does, and Folder Options' own options take theirs.
///
/// parity: VIEW-020
#[gtk::test]
fn folder_options_holds_back_its_own_window_alone_under_real_clicks() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri_of("Documents"));
    // Without a window manager every window opens at the screen's corner:
    // the other window lies wider underneath, this one over it, and
    // Folder Options over that.
    other.window.set_default_size(1100, 760);
    test.window.set_default_size(640, 460);
    test.window.present();
    wait_until("the windows to have their sizes", || {
        other.window.width() >= 1000 && test.window.width() <= 700
    });
    let options = open_folder_options(&test);
    wait_until("Folder Options to show", || options.width() > 0);
    let own = count_presses(&test.window);
    let beside = count_presses(&other.window);
    let mut pointer = crate::test_support::pointer::Pointer::connect();

    click_in(&mut pointer, &test.window, (560.0, 380.0));
    assert_eq!(own.get(), 0, "its own window takes no click");

    click_in(&mut pointer, &other.window, (900.0, 600.0));
    assert_eq!(beside.get(), 1, "another Explorer window takes its click");

    let own_window = option_check(&options, "General", "Open each folder in its own window");
    let middle = (8.0, f64::from(own_window.height()) / 2.0);
    click_in(&mut pointer, &own_window, middle);
    assert!(own_window.is_active(), "Folder Options takes its own clicks");

    close_folder_options(&test, &options, "Cancel");
    click_in(&mut pointer, &test.window, (560.0, 380.0));
    assert_eq!(own.get(), 1, "its window takes clicks again once it closes");
    // Back to the screen's middle, where the display starts it, as
    // picker.rs's real-pointer test ends: GTK 4.14 crashes in
    // gdk_surface_request_motion when a later test closes a menu under a
    // pointer left over its windows.
    pointer.move_to(720, 500);
}

/// While Folder Options is open, a preference saved elsewhere (Settings,
/// View > Show, Ctrl+H or another window) shows in every choice the user
/// has not changed here, the start place too; a choice the user changed
/// keeps the change, and OK saves it without undoing the others.
///
/// parity: VIEW-020, VIEW-023, TAB-055
#[gtk::test]
fn folder_options_follows_preferences_changed_while_it_is_open() {
    use ox_core::settings::PreferencesUpdate;

    use super::view_styles::{change_preferences, options_tab};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    let view = |label: &str| option_check(&options, "View", label);
    let start = descendants::<gtk::DropDown>(&options_tab(&options, "General")).remove(0);
    assert!(view("Don't show hidden files, folders, or drives").is_active());
    assert!(!view("Display the full path in the title bar").is_active());
    view("Show status bar").set_active(false);

    change_preferences(
        &test,
        &PreferencesUpdate {
            show_hidden: Some(true),
            full_path_in_title: Some(true),
            hide_status_bar: Some(false),
            startup_folder: Some(fixture.uri_of("Documents")),
            ..PreferencesUpdate::default()
        },
    );
    wait_until("Folder Options to show the saved choices", || {
        view("Show hidden files, folders, and drives").is_active()
            && view("Display the full path in the title bar").is_active()
    });
    assert!(
        !view("Show status bar").is_active(),
        "the choice changed here keeps the user's change"
    );
    wait_until("the folder chosen in Settings to be the start place", || {
        start.selected() == 2
    });

    close_folder_options(&test, &options, "OK");
    wait_until("the status bar choice to be saved", || {
        test.context.settings_data().preferences.hide_status_bar
    });
    let preferences = test.context.settings_data().preferences;
    assert!(preferences.show_hidden, "OK keeps what was saved elsewhere");
    assert!(preferences.full_path_in_title);
    assert!(preferences
        .startup_folder
        .is_some_and(|folder| folder.ends_with("Documents")));
}

/// The mnemonic key of `widget`'s label, a check box's or a push
/// button's, if it has one, in lower case, as Alt and the letter press it.
fn access_key(widget: &gtk::Widget) -> Option<gtk::gdk::Key> {
    let label = descendants::<gtk::Label>(widget).into_iter().next()?;
    let key = label.mnemonic_keyval();
    // A label without one answers VoidSymbol, or 0 in some GTK versions.
    let key = key.to_lower();
    (key.to_unicode().is_some_and(char::is_alphanumeric)).then_some(key)
}

/// Folder Options' choices and push buttons have Windows' access keys:
/// Alt and the underlined letter choose an option or press a button, and
/// no two controls of a tab share a letter. Click items as follows has
/// none, as only its chosen Double-click can be used.
///
/// parity: VIEW-020, ACC-006
#[gtk::test]
fn folder_options_choices_and_buttons_have_access_keys() {
    use gtk::gdk::Key;

    use super::view_styles::options_tab;

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    let general = |label: &str| option_check(&options, "General", label).upcast::<gtk::Widget>();
    let button = |tab: &str, label: &str| tab_button(&options, tab, label).upcast::<gtk::Widget>();
    let expected = [
        (general("Open each folder in the same window"), Key::a),
        (general("Open each folder in its own window"), Key::w),
        (button("General", "Clear"), Key::l),
        (button("General", "Restore Defaults"), Key::d),
        (button("View", "Apply to Folders"), Key::o),
        (button("View", "Reset Folders"), Key::r),
        (button("View", "Restore Defaults"), Key::d),
        (button("Search", "Restore Defaults"), Key::d),
    ];
    for (widget, key) in &expected {
        assert_eq!(access_key(widget), Some(*key), "{widget:?}");
    }
    for tab in ["General", "View", "Search"] {
        let page = options_tab(&options, tab);
        let mut keys: Vec<_> = descendants::<gtk::Widget>(&page)
            .iter()
            .filter(|widget| widget.is::<gtk::CheckButton>() || widget.is::<gtk::Button>())
            .filter(|widget| widget.is_sensitive())
            .filter_map(access_key)
            .collect();
        let count = keys.len();
        keys.sort_by_key(gtk::gdk::Key::to_unicode);
        keys.dedup();
        assert_eq!(keys.len(), count, "the {tab} tab's access keys are distinct");
    }

    // An access key chooses its option, as Alt and the letter do.
    let own_window = option_check(&options, "General", "Open each folder in its own window");
    own_window.mnemonic_activate(false);
    assert!(own_window.is_active());
    close_folder_options(&test, &options, "Cancel");
}

/// Under the real keyboard, Alt and an option's letter choose it, Enter
/// with a choice focused presses OK, the default button, which saves and
/// closes Folder Options, as in Windows, and Escape closes it without
/// saving.
///
/// parity: VIEW-020, ACC-006
#[gtk::test]
fn real_keys_choose_options_and_enter_presses_ok() {
    use gtk::gdk::Key;

    use super::view_styles::folder_options_over;
    use crate::test_support::harness::wait_for_frames;
    use crate::test_support::pointer::Pointer;

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let mut keys = Pointer::connect();
    let own_window = || {
        test.context
            .settings_data()
            .preferences
            .open_folders_in_own_window
    };
    let alt = |keys: &mut Pointer, key: Key| {
        keys.hold_key(Key::Alt_L);
        keys.tap_key(key);
        keys.let_go_key(Key::Alt_L);
    };

    let options = open_folder_options(&test);
    wait_for_frames(&options, 3);
    alt(&mut keys, Key::w);
    wait_until("Alt+W to choose Open each folder in its own window", || {
        option_check(&options, "General", "Open each folder in its own window").is_active()
    });
    keys.tap_key(Key::Return);
    wait_until("Enter to press OK", || folder_options_over(&test).is_none());
    wait_until("the choice to be saved", own_window);

    let options = open_folder_options(&test);
    wait_for_frames(&options, 3);
    alt(&mut keys, Key::a);
    wait_until("Alt+A to choose Open each folder in the same window", || {
        option_check(&options, "General", "Open each folder in the same window").is_active()
    });
    keys.tap_key(Key::Escape);
    wait_until("Escape to close Folder Options", || {
        folder_options_over(&test).is_none()
    });
    assert!(own_window(), "Escape saves nothing");
    keys.move_to(720, 500);
}
