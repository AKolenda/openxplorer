// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties window keeps Windows 11's width for every item: a long
//! name, location or value wraps or ends in "…" instead of widening it.

use std::fs;

use gtk::glib;
use gtk::glib::subclass::types::ObjectSubclassIsExt;
use gtk::prelude::*;

use super::item_dialogs::general;
use crate::properties::PropertiesWindow;
use crate::test_support::harness::{
    capture_dialog, descendants, wait_for_frames, Fixture, TestWindow, ThemeGuard,
};

/// A fictional name far longer than the window is wide.
const LONG_NAME: &str =
    "Discover Effective Calming Techniques_ Deep Breathing and Quiet Evenings – Example.com_files";

/// Every tab a Properties window may have, by page name.
const TABS: [&str; 8] = [
    "general",
    "hardware",
    "sharing",
    "security",
    "location",
    "details",
    "versions",
    "customize",
];

/// The window's width once its General tab is filled and drawn.
fn shown_width(window: &PropertiesWindow) -> i32 {
    general(window);
    wait_for_frames(window, 4);
    window.width()
}

/// The shown widgets of `window` wider than it, for the failure message.
fn spilling_widgets(window: &PropertiesWindow) -> Vec<String> {
    let width = window.width();
    descendants::<gtk::Widget>(window)
        .into_iter()
        .filter(|widget| widget.is_mapped() && widget.width() > width)
        .map(|widget| {
            format!(
                "{} {:?} {}",
                widget.type_().name(),
                widget.css_classes(),
                widget.width()
            )
        })
        .collect()
}

/// Shows every tab `window` has and checks that each keeps `width` and
/// that nothing on it is wider than the window.
fn assert_every_tab_keeps(window: &PropertiesWindow, width: i32, what: &str) {
    for name in TABS {
        if window.tab_page(name).is_none() {
            continue;
        }
        window.select_tab(name);
        wait_for_frames(window, 4);
        assert_eq!(
            window.width(),
            width,
            "{what}: the {name} tab keeps the window's width"
        );
        let spilling = spilling_widgets(window);
        assert!(
            spilling.is_empty(),
            "{what}: on the {name} tab, wider than the window: {spilling:?}"
        );
    }
}

/// Opens Properties of the selection in `test`, titled `title`.
fn properties_of_selection(test: &TestWindow, title: &str) -> PropertiesWindow {
    test.activate("properties", None);
    test.wait_for_properties(title)
}

/// A long name, a long location and several items with long names give a
/// window as wide and as tall as short names do with the same tabs, on
/// every tab: Windows' fixed size. The name box keeps the width and
/// scrolls its text; the title
/// ends in "…".
///
/// parity: PROP-001
#[gtk::test]
fn a_long_name_or_location_never_widens_properties() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    fs::write(fixture.path("Short.txt"), "fictional").expect("a file");
    fs::create_dir(fixture.path(LONG_NAME)).expect("a folder");
    fs::write(fixture.path(&format!("{LONG_NAME}.txt")), "fictional").expect("a file");
    let deep = [LONG_NAME, LONG_NAME, LONG_NAME].join("/");
    fs::create_dir_all(fixture.path(&format!("Deep/{deep}"))).expect("a deep folder");
    let test = TestWindow::open(&fixture.uri());

    test.select_named("Short");
    let short = properties_of_selection(&test, "Short Properties");
    let width = shown_width(&short);
    let height = short.height();
    assert!(width > 0);
    let short_name = descendants::<gtk::Entry>(&short)
        .into_iter()
        .next()
        .expect("the name box");
    let name_width = short_name.width();
    assert_every_tab_keeps(&short, width, "a short name");
    short.close();

    test.select_named(LONG_NAME);
    let folder = properties_of_selection(&test, &format!("{LONG_NAME} Properties"));
    assert_eq!(shown_width(&folder), width, "a long folder name");
    assert_eq!(folder.height(), height, "nor makes it taller");
    let long_name = descendants::<gtk::Entry>(&folder)
        .into_iter()
        .next()
        .expect("the name box");
    assert_eq!(long_name.width(), name_width, "the name box keeps its width");
    assert_eq!(long_name.text(), LONG_NAME, "and holds the whole name");
    assert_every_tab_keeps(&folder, width, "a long folder name");
    folder.close();

    // A file has tabs of its own, and a tab row wider than the window
    // (a folder's five tabs in a wide font) widens it whatever the name,
    // so a file is compared with a file of a short name.
    test.select_named("Short.txt");
    let short_file = properties_of_selection(&test, "Short.txt Properties");
    let file_width = shown_width(&short_file);
    short_file.close();
    let file_name = format!("{LONG_NAME}.txt");
    test.select_named(&file_name);
    let file = properties_of_selection(&test, &format!("{file_name} Properties"));
    assert_eq!(shown_width(&file), file_width, "a long file name");
    assert_every_tab_keeps(&file, file_width, "a long file name");
    file.close();

    test.show(&fixture.uri_of(&format!("Deep/{}", [LONG_NAME, LONG_NAME].join("/"))));
    test.wait_for_listing("the deep folder");
    test.select_named(LONG_NAME);
    let nested = properties_of_selection(&test, &format!("{LONG_NAME} Properties"));
    assert_eq!(shown_width(&nested), width, "a long location");
    assert_eq!(nested.height(), height, "a long location keeps the height too");
    assert_every_tab_keeps(&nested, width, "a long location");
    nested.close();

    // Several items have tabs of their own too.
    test.show(&fixture.uri());
    test.wait_for_listing("the fixture");
    let short_pair = properties_of_two(&test, "Short", "Short.txt");
    let several_width = shown_width(&short_pair);
    let several_height = short_pair.height();
    short_pair.close();
    let long_pair = properties_of_two(&test, LONG_NAME, &file_name);
    assert_eq!(
        shown_width(&long_pair),
        several_width,
        "several items with long names"
    );
    assert_eq!(long_pair.height(), several_height, "nor makes them taller");
    assert_every_tab_keeps(&long_pair, several_width, "several items");
    long_pair.close();
}

/// Opens Properties of the items named `first` and `second` in `test`.
fn properties_of_two(test: &TestWindow, first: &str, second: &str) -> PropertiesWindow {
    super::file_ops_support::select_names(test, &[first, second]);
    // Windows titles them by their two names, in the listing's order.
    let is_both = |title: &str| {
        title == format!("{first}, {second} Properties") || title == format!("{second}, {first} Properties")
    };
    test.activate("properties", None);
    crate::test_support::harness::wait_until("Properties of the two items", || {
        test.window
            .properties_windows()
            .iter()
            .any(|window| is_both(&window.title_text()))
    });
    test.window
        .properties_windows()
        .into_iter()
        .find(|window| is_both(&window.title_text()))
        .expect("Properties of the two items")
}

/// General with a short and a very long fictional name, light and dark,
/// for review with `OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`):
/// `properties-width-{short,long}-{light,dark}.png`.
///
/// parity: PROP-001
#[gtk::test]
fn properties_with_a_long_name_are_captured() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Holiday 2025")).expect("a folder");
    fs::create_dir(fixture.path(LONG_NAME)).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        for (name, label) in [("Holiday 2025", "short"), (LONG_NAME, "long")] {
            test.select_named(name);
            let window = properties_of_selection(&test, &format!("{name} Properties"));
            shown_width(&window);
            capture_dialog(&window, &format!("properties-width-{label}-{theme}.png"));
            window.close();
        }
    }
}

/// Properties of a folder, a file, several items and a drive (the root,
/// Local Disk) all keep one width, on every tab. The system drive has
/// no Customize tab, as in Windows 11, and its five tabs fit one row.
///
/// parity: PROP-001
#[gtk::test]
fn every_kind_of_properties_keeps_the_width() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    fs::write(fixture.path("Short.txt"), "fictional").expect("a file");
    let test = TestWindow::open(&fixture.uri());

    test.select_named("Short");
    let folder = properties_of_selection(&test, "Short Properties");
    let width = shown_width(&folder);
    assert_every_tab_keeps(&folder, width, "a folder");
    folder.close();

    test.select_named("Short.txt");
    let file = properties_of_selection(&test, "Short.txt Properties");
    assert_eq!(shown_width(&file), width, "a file");
    assert_every_tab_keeps(&file, width, "a file");
    file.close();

    let several = properties_of_two(&test, "Short", "Short.txt");
    assert_eq!(shown_width(&several), width, "several items");
    assert_every_tab_keeps(&several, width, "several items");
    several.close();

    test.activate("properties-of", Some("file:///"));
    let drive = test.wait_for_properties("Local Disk Properties");
    wait_for_frames(&drive, 6);
    assert_eq!(drive.width(), width, "a drive");
    assert_eq!(
        drive.tab_labels(),
        ["General", "Hardware", "Sharing", "Security", "Previous Versions"],
        "a drive has no Customize tab"
    );
    assert_eq!(tab_rows(&drive), [5], "a drive's five tabs fit one row");
    assert_every_tab_keeps(&drive, width, "a drive");
    drive.close();
}

/// File systems that are no disk: the kernel's own and memory drives.
const NOT_A_DISK: [&str; 9] = [
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "securityfs",
    "debugfs",
    "tracefs",
];

/// A drive other than the system drive that keeps a Customize choice,
/// when this computer has one: the root of a local mount, writable and
/// not the root, a disk's file system (such as /boot/efi or /home) before
/// a memory drive (such as /dev/shm).
fn other_drive() -> Option<String> {
    let mounts = ox_core::network::read_mount_table().unwrap_or_default();
    let candidates: Vec<String> = mounts
        .iter()
        .filter(|mount| {
            let filesystem = mount.filesystem.as_str();
            mount.root == "/"
                && mount.path != "/"
                && !NOT_A_DISK.contains(&filesystem)
                && !ox_core::location::is_network_filesystem(filesystem)
                && !filesystem.starts_with("fuse")
                && !mount.options.split(',').any(|option| option == "ro")
        })
        .filter(|mount| std::path::Path::new(&mount.path).is_dir())
        .map(|mount| {
            let uri = gtk::gio::File::for_path(&mount.path).uri().to_string();
            (mount.filesystem == "tmpfs", uri)
        })
        .fold(Vec::new(), |mut found, (is_memory, uri)| {
            if is_memory {
                found.push(uri);
            } else {
                found.insert(0, uri);
            }
            found
        });
    candidates.into_iter().next()
}

/// Opens Properties of the drive `uri` from `test`, listed on This PC as
/// the volume monitor would list it, so it shows as a drive however a
/// folder that is a mount point shows.
fn drive_properties(test: &TestWindow, uri: &str) -> PropertiesWindow {
    test.window
        .imp()
        .volumes
        .replace(vec![crate::test_support::mounted_volume(
            "Data",
            uri,
            crate::volumes::VolumeKind::Drive,
        )]);
    test.activate("properties-of", Some(uri));
    crate::test_support::harness::wait_until("the drive's Properties", || {
        !test.window.properties_windows().is_empty()
    });
    let window = test.window.properties_windows().remove(0);
    crate::test_support::harness::wait_until("the drive's details", || {
        super::item_dialogs::value_after(&window, "Type:").is_some()
    });
    wait_for_frames(&window, 6);
    window
}

/// A drive other than the system drive has Customize, as in Windows 11,
/// after Previous Versions; its six tabs share two rows evenly, and the
/// window keeps Windows' width.
///
/// parity: PROP-001
#[gtk::test]
fn another_drive_has_customize_and_its_tabs_share_the_rows() {
    let Some(uri) = other_drive() else {
        eprintln!("skipped: no writable local drive other than the root here");
        return;
    };
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Short");
    let folder = properties_of_selection(&test, "Short Properties");
    let width = shown_width(&folder);
    folder.close();

    let drive = drive_properties(&test, &uri);
    let labels = drive.tab_labels();
    assert_eq!(labels.first().map(String::as_str), Some("General"));
    assert_eq!(labels.last().map(String::as_str), Some("Customize"), "{labels:?}");
    assert!(
        labels.iter().any(|label| label == "Hardware"),
        "a drive: {labels:?}"
    );
    assert_eq!(drive.width(), width, "the width stays");
    let rows = tab_rows(&drive);
    let most = rows.iter().copied().max().unwrap_or(0);
    let fewest = rows.iter().copied().min().unwrap_or(0);
    assert!(most - fewest <= 1, "the rows share the tabs evenly: {rows:?}");
    assert_every_tab_keeps(&drive, width, "another drive");
    drive.close();
}

/// Properties of every kind of item, open one after another and shown
/// by kind, in `theme` when given: a file, a shortcut, two files, a
/// folder, the system drive, a mount point opened by its path, another
/// drive (when the computer has one) and seven tabs in two rows.
fn every_kind(theme: Option<&str>, each: &mut dyn FnMut(&str, &PropertiesWindow)) {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Holiday 2025")).expect("a folder");
    fs::write(fixture.path("Packing list.txt"), "fictional").expect("a file");
    fs::write(fixture.path("Tickets.txt"), "fictional").expect("a file");
    std::os::unix::fs::symlink(
        fixture.path("Tickets.txt"),
        fixture.path("Tickets.txt - Shortcut"),
    )
    .expect("a shortcut");
    let test = &TestWindow::open(&fixture.uri());
    if let Some(theme) = theme {
        test.activate("theme", Some(theme));
    }
    let mut show = |kind: &str, window: PropertiesWindow| {
        wait_for_frames(&window, 6);
        each(kind, &window);
        window.close();
    };
    test.select_named("Packing list.txt");
    show(
        "file",
        properties_of_selection(test, "Packing list.txt Properties"),
    );
    test.select_named("Tickets.txt - Shortcut");
    show(
        "shortcut",
        properties_of_selection(test, "Tickets.txt - Shortcut Properties"),
    );
    show(
        "several",
        properties_of_two(test, "Packing list.txt", "Tickets.txt"),
    );
    test.select_named("Holiday 2025");
    show("folder", properties_of_selection(test, "Holiday 2025 Properties"));
    show("drive", drive_properties(test, "file:///"));
    if let Some(uri) = other_drive() {
        // By its path, as a folder's mount point opens from elsewhere.
        test.window.imp().volumes.replace(Vec::new());
        test.activate("properties-of", Some(&uri));
        crate::test_support::harness::wait_until("the mount point's Properties", || {
            !test.window.properties_windows().is_empty()
        });
        show("mount-point", test.window.properties_windows().remove(0));
        show("other-drive", drive_properties(test, &uri));
    }
    show("wrapped", seven_tab_properties(test));
}

/// Every Properties window has Windows' one fixed size, whatever it
/// shows: a file, a shortcut, several items, a folder, a drive or a mount
/// point, with one row of tabs or two, on every tab. Windows sizes a
/// property sheet once and never changes it.
///
/// parity: PROP-001
#[gtk::test]
fn every_kind_of_properties_has_one_size() {
    let mut sizes = Vec::new();
    every_kind(None, &mut |kind, window| {
        let scrolling = window.scroller().vadjustment();
        let mut look = |what: String| {
            sizes.push((what.clone(), (window.width(), window.height())));
            assert!(
                scrolling.upper() <= scrolling.page_size() + 1.0,
                "{what}: fits without scrolling ({} of {})",
                scrolling.upper(),
                scrolling.page_size()
            );
        };
        look(kind.to_owned());
        for name in TABS {
            if window.tab_page(name).is_none() {
                continue;
            }
            window.select_tab(name);
            wait_for_frames(window, 4);
            look(format!("{kind} {name}"));
        }
    });
    let expected = (PropertiesWindow::WIDTH, sizes[0].1 .1);
    let others: Vec<_> = sizes.iter().filter(|(_, size)| *size != expected).collect();
    assert!(others.is_empty(), "every window is {expected:?}: {others:?}");
}

/// Every kind of Properties, light and dark, for review side by side with
/// `OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`):
/// `properties-size-{kind}-{light,dark}.png`.
///
/// parity: PROP-001
#[gtk::test]
fn every_kind_of_properties_size_is_captured() {
    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        every_kind(Some(theme), &mut |kind, window| {
            capture_dialog(window, &format!("properties-size-{kind}-{theme}.png"));
        });
    }
}

/// A window built on the Properties window that is not an item's
/// Properties, as Folder Options and the Security tab's dialogs are,
/// keeps its own size: it takes its content's height, not every item's
/// Properties' fixed one, and a stylesheet that gives its notebook a
/// margin of its own, as Folder Options' does, does not add a second
/// margin to the tab area's and widen it.
///
/// parity: PROP-001
#[gtk::test]
fn other_windows_on_the_properties_window_keep_their_own_size() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Short");
    let folder = properties_of_selection(&test, "Short Properties");
    let width = shown_width(&folder);
    let sheet_height = folder.height();
    folder.close();

    // Folder Options' rule at the end of the Properties stylesheet, where
    // it gives the notebook its own margin, as the skin would load it.
    let style = gtk::CssProvider::new();
    style.load_from_string(&format!(
        "{}\n.ox-folder-options notebook.properties-notebook {{ margin: 6px 8px 0 8px; }}",
        include_str!("../../../resources/skin/properties.css")
    ));
    let display = gtk::gdk::Display::default().expect("a display");
    gtk::style_context_add_provider_for_display(&display, &style, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let options = PropertiesWindow::new("Folder Options", None, Some(&test.window));
    options.add_css_class("ox-folder-options");
    for label in ["General", "View", "Search"] {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&gtk::Label::new(Some(label)));
        options.add_tab(&label.to_lowercase(), label, &page);
    }
    options.present();
    wait_for_frames(&options, 6);
    let (shown_width, shown_height) = (options.width(), options.height());
    options.close();
    gtk::style_context_remove_provider_for_display(&display, &style);

    // Folder Options' own margin is narrower than Properties': one
    // margin around the tabs, never the notebook's and the tab area's.
    assert!(
        shown_width <= width,
        "one margin around the tabs, not two: {shown_width} wide, Properties {width}"
    );
    assert!(
        shown_height < sheet_height,
        "it takes its content's height ({shown_height}), not Properties' ({sheet_height})"
    );
}

/// How many tabs each row of `window`'s tabs holds, top to bottom. The
/// tab shown stands a little taller than its row, so tabs whose tops are
/// within a few pixels share a row.
fn tab_rows(window: &PropertiesWindow) -> Vec<usize> {
    let mut tops: Vec<f32> = window
        .tab_buttons()
        .iter()
        .filter(|tab| tab.is_visible())
        .filter_map(|tab| tab.compute_bounds(window))
        .map(|bounds| bounds.y())
        .collect();
    tops.sort_by(f32::total_cmp);
    let mut rows: Vec<(f32, usize)> = Vec::new();
    for top in tops {
        match rows.last_mut() {
            Some((first, count)) if top - *first < 8.0 => *count += 1,
            _ => rows.push((top, 1)),
        }
    }
    rows.into_iter().map(|(_, count)| count).collect()
}

/// A Properties window with Windows' seven drive tabs (General, Tools,
/// Hardware, Sharing, Security, Previous Versions, Quota), too many for
/// one row at Windows' width.
fn seven_tab_properties(test: &TestWindow) -> PropertiesWindow {
    let window = PropertiesWindow::new("Data (D:) Properties", None, Some(&test.window));
    window.hold_sheet_size();
    for label in [
        "General",
        "Tools",
        "Hardware",
        "Sharing",
        "Security",
        "Previous Versions",
        "Quota",
    ] {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&gtk::Label::new(Some(label)));
        window.add_tab(&label.to_lowercase(), label, &page);
    }
    window.present();
    wait_for_frames(&window, 6);
    window
}

/// Tabs too many for one row wrap as Windows' multi-row tabs do: the
/// rows share the tabs evenly, each row is stretched to the full width,
/// never one tab alone on a row, and the row of the tab shown sits next
/// to the page; the window keeps Windows' width.
///
/// parity: PROP-001
#[gtk::test]
fn wrapped_tabs_share_the_rows_evenly() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Short");
    let folder = properties_of_selection(&test, "Short Properties");
    let width = shown_width(&folder);
    folder.close();

    let window = seven_tab_properties(&test);
    assert_eq!(window.width(), width, "the width stays");
    // As Windows' C: drive: Security, Previous Versions and Quota behind,
    // General, Tools, Hardware and Sharing in front, next to the page.
    assert_eq!(tab_rows(&window), [3, 4], "the rows share the tabs evenly");
    // Every row reaches both edges of the row of tabs.
    let tabs = window.tab_buttons();
    let row_area = tabs[0].parent().expect("the row of tabs");
    let row_width = f32::from(i16::try_from(row_area.width()).expect("a row's width"));
    let mut lefts = 0;
    let mut rights = 0;
    for bounds in tabs.iter().filter_map(|tab| tab.compute_bounds(&row_area)) {
        lefts += usize::from(bounds.x() <= 0.0);
        rights += usize::from(bounds.x() + bounds.width() >= row_width - 1.0);
    }
    assert_eq!((lefts, rights), (2, 2), "each row is stretched to the full width");

    // Showing Quota, on the other row, brings its row next to the page.
    let general_top = |window: &PropertiesWindow| {
        window.tab_buttons()[0]
            .compute_bounds(window)
            .expect("bounds")
            .y()
    };
    let before = general_top(&window);
    tabs[6].set_active(true);
    wait_for_frames(&window, 4);
    assert!(general_top(&window) < before - 8.0, "General's row moves up");
    window.close();
}

/// The row of tabs is one stop of the Tab key, on the tab shown, which
/// screen readers hear as the selected tab of a tab list; Left, Right,
/// Home and End move between tabs, and Ctrl+Page Down and Ctrl+Tab show
/// the next one from anywhere in the window.
///
/// parity: PROP-001
#[gtk::test]
fn the_tab_row_is_one_stop_and_moves_with_the_keyboard() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Short")).expect("a folder");
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Short");
    let window = properties_of_selection(&test, "Short Properties");
    general(&window);
    let tabs = window.tab_buttons();
    assert_eq!(window.tab_labels()[0], "General");
    assert!(tabs[0].has_focus(), "the tab shown has the keyboard on opening");
    assert_eq!(tabs[0].accessible_role(), gtk::AccessibleRole::Tab);
    let focusable: Vec<bool> = tabs.iter().map(WidgetExt::is_focusable).collect();
    assert_eq!(
        focusable.iter().filter(|stop| **stop).count(),
        1,
        "one stop: {focusable:?}"
    );

    let strip = tabs[0].parent().expect("the row of tabs");
    assert_eq!(strip.accessible_role(), gtk::AccessibleRole::TabList);
    let press = |key: gtk::gdk::Key| {
        let keys = strip
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .expect("the row's keys");
        keys.emit_by_name::<bool>("key-pressed", &[&key, &0_u32, &gtk::gdk::ModifierType::empty()])
    };
    press(gtk::gdk::Key::Right);
    assert_eq!(window.selected_tab().as_deref(), Some("sharing"));
    assert!(tabs[1].has_focus() && tabs[1].is_active());
    press(gtk::gdk::Key::End);
    assert_eq!(window.selected_tab().as_deref(), Some("customize"));
    press(gtk::gdk::Key::Home);
    assert_eq!(window.selected_tab().as_deref(), Some("general"));
    press(gtk::gdk::Key::Left);
    assert_eq!(
        window.selected_tab().as_deref(),
        Some("customize"),
        "round from the first"
    );

    let shortcut = |keyval: gtk::gdk::Key| {
        let shortcuts: Vec<gtk::Shortcut> = window
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
            .flat_map(|controller| {
                controller
                    .iter::<glib::Object>()
                    .filter_map(Result::ok)
                    .filter_map(|item| item.downcast::<gtk::Shortcut>().ok())
                    .collect::<Vec<_>>()
            })
            .collect();
        let found = shortcuts
            .into_iter()
            .find(|shortcut| {
                shortcut
                    .trigger()
                    .and_downcast::<gtk::KeyvalTrigger>()
                    .is_some_and(|trigger| {
                        trigger.keyval() == keyval
                            && trigger.modifiers() == gtk::gdk::ModifierType::CONTROL_MASK
                    })
            })
            .expect("a Ctrl shortcut");
        found
            .action()
            .expect("an action")
            .activate(gtk::ShortcutActionFlags::empty(), &window, None);
    };
    shortcut(gtk::gdk::Key::Page_Down);
    assert_eq!(
        window.selected_tab().as_deref(),
        Some("general"),
        "round from the last"
    );
    shortcut(gtk::gdk::Key::Tab);
    assert_eq!(window.selected_tab().as_deref(), Some("sharing"));
    window.close();
}

/// General of a folder, a file, several items, the system drive, another
/// drive (when the computer has one) and seven tabs in two rows, light and
/// dark, for review with `OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`):
/// `properties-kind-{folder,file,several,drive,other-drive,wrapped}-{light,dark}.png`.
///
/// parity: PROP-001
#[gtk::test]
fn every_kind_of_properties_is_captured() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Holiday 2025")).expect("a folder");
    fs::write(fixture.path("Packing list.txt"), "fictional").expect("a file");
    let test = TestWindow::open(&fixture.uri());
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        test.select_named("Holiday 2025");
        let folder = properties_of_selection(&test, "Holiday 2025 Properties");
        shown_width(&folder);
        capture_dialog(&folder, &format!("properties-kind-folder-{theme}.png"));
        folder.close();
        test.select_named("Packing list.txt");
        let file = properties_of_selection(&test, "Packing list.txt Properties");
        shown_width(&file);
        capture_dialog(&file, &format!("properties-kind-file-{theme}.png"));
        file.close();
        let several = properties_of_two(&test, "Holiday 2025", "Packing list.txt");
        wait_for_frames(&several, 6);
        capture_dialog(&several, &format!("properties-kind-several-{theme}.png"));
        several.close();
        test.activate("properties-of", Some("file:///"));
        let drive = test.wait_for_properties("Local Disk Properties");
        crate::test_support::harness::wait_until("the drive's details", || {
            super::item_dialogs::value_after(&drive, "Type:").is_some()
        });
        wait_for_frames(&drive, 6);
        capture_dialog(&drive, &format!("properties-kind-drive-{theme}.png"));
        drive.close();
        if let Some(uri) = other_drive() {
            let other = drive_properties(&test, &uri);
            capture_dialog(&other, &format!("properties-kind-other-drive-{theme}.png"));
            other.close();
        }
        let wrapped = seven_tab_properties(&test);
        capture_dialog(&wrapped, &format!("properties-kind-wrapped-{theme}.png"));
        wrapped.close();
    }
}
