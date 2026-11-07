// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar and its menus against `section.commandbar` in
//! `v2.0.0:desktop/ui/index.html` and the menus of `setup()` and `openNewMenu` in
//! `v2.0.0:desktop/ui/app.js`: the same controls in the same order, the same menu
//! items and dividers, and the commands that are not ported yet shown
//! disabled with the milestone that brings them.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{is_triggered_by, press_shortcut, select_names, window_shortcuts};
use super::geometry::laid_out;
use crate::icons::Icon;
use crate::locations::Page;
use crate::settings_page::{Category, SettingsView};
use crate::test_support::harness::{
    application, descendants, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::menu_popover::MenuPopover;
use crate::window::widget_tree::children;
use crate::window::window_action::WindowAction;

/// How a test names a command bar control: "|" for a separator, the
/// visible label of a text command, else the first line of its tooltip.
fn control_name(control: &gtk::Widget) -> Option<String> {
    if control.is::<gtk::Separator>() {
        return Some("|".to_owned());
    }
    // What the button shows, leaving out a menu button's menu.
    let content = match control.downcast_ref::<gtk::MenuButton>() {
        Some(menu_button) => menu_button.child(),
        None => Some(control.clone()),
    };
    let label = content.and_then(|content| descendants::<gtk::Label>(&content).into_iter().next());
    if let Some(label) = label {
        return Some(label.text().to_string());
    }
    let tooltip = control.tooltip_text()?;
    tooltip.lines().next().map(str::to_owned)
}

/// The command bar's shown controls in order, the scrolling file commands
/// included.
pub(super) fn command_bar_controls(test: &TestWindow) -> Vec<gtk::Widget> {
    let bar = test.window.command_bar();
    let mut controls = Vec::new();
    for child in children(bar) {
        let group = descendants::<gtk::Box>(&child)
            .into_iter()
            .find(|widget| widget.has_css_class("command-group"));
        match group {
            // Extract all shows only inside a ZIP opened like a folder.
            Some(group) => controls.extend(children(&group).filter(WidgetExt::is_visible)),
            // The appearance toggle is hidden, as Settings › Appearance
            // chooses the theme; Update shows only while a release waits.
            None if child.is_visible() => controls.push(child),
            None => {}
        }
    }
    controls
}

/// The menu of the command bar control named `name`.
pub(super) fn menu_of(test: &TestWindow, name: &str) -> MenuPopover {
    let control = command_bar_controls(test)
        .into_iter()
        .find(|control| control_name(control).as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the command bar has {name}"));
    let button = control
        .downcast::<gtk::MenuButton>()
        .unwrap_or_else(|_| panic!("{name} opens a menu"));
    button
        .popover()
        .and_downcast::<MenuPopover>()
        .unwrap_or_else(|| panic!("{name} opens an app menu"))
}

/// The names of `test`'s shown command bar controls, in order.
pub(super) fn command_bar_names(test: &TestWindow) -> Vec<String> {
    command_bar_controls(test)
        .iter()
        .filter_map(control_name)
        .collect()
}

/// Share, between Rename and Delete, shows only where the desktop
/// can share (`tests::share`); here no Email portal answers, whatever the
/// machine has.
#[gtk::test]
fn the_command_bar_has_the_current_controls_in_order() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window.follow_share_service(crate::window::share::NO_PORTAL);
    wait_until("the window to ask no portal", || {
        test.window.share_service_checked(crate::window::share::NO_PORTAL)
    });
    wait_for_frames(&test.window, 2);
    let names = command_bar_names(&test);
    assert_eq!(
        names,
        [
            "New",
            "|",
            "Cut (Ctrl+X)",
            "Copy (Ctrl+C)",
            "Paste files (Ctrl+V)",
            "Rename (F2)",
            "Delete (Delete)",
            "|",
            "Sort",
            "View",
            "More options",
            "Settings (Ctrl+,)",
            "Details",
        ]
    );
}

/// The command bar button that runs `action`.
fn command_button(test: &TestWindow, action: &str) -> gtk::Button {
    descendants::<gtk::Button>(test.window.command_bar())
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some(action))
        .unwrap_or_else(|| panic!("a button runs {action}"))
}

/// parity: CMD-001, CMD-002, CMD-016
#[gtk::test]
fn the_edit_commands_follow_the_selection_with_their_python_tooltips() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let edit_commands = [
        ("win.cut", "Cut (Ctrl+X)"),
        ("win.copy", "Copy (Ctrl+C)"),
        ("win.rename", "Rename (F2)"),
        ("win.trash", "Delete (Delete)"),
    ];
    for (action, tooltip) in edit_commands {
        let button = command_button(&test, action);
        assert!(!button.is_sensitive(), "{action} needs a selection");
        assert_eq!(button.tooltip_text().as_deref(), Some(tooltip));
    }
    let paste = command_button(&test, "win.paste");
    assert_eq!(paste.tooltip_text().as_deref(), Some("Paste files (Ctrl+V)"));
    test.window.folder_model().select_only(1);
    for (action, _) in edit_commands {
        assert!(
            command_button(&test, action).is_sensitive(),
            "{action} acts on one item"
        );
    }
    assert!(
        WidgetExt::activate_action(&test.window, "win.copy-path", None).is_ok(),
        "Copy path works now"
    );
}

/// The New menu, laid out as Windows 11's, a divider as `-`.
const NEW_MENU: [&str; 7] = [
    "Folder",
    "Shortcut",
    "-",
    "Bitmap image",
    "Rich Text Document",
    "Text Document",
    "Compressed (zipped) Folder",
];

/// The Sort menu, laid out as Windows Explorer's: three keys, More,
/// one item per direction, Group by, then folders first.
const SORT_MENU: [&str; 11] = [
    "Name",
    "Date modified",
    "Type",
    "More",
    "-",
    "Ascending",
    "Descending",
    "-",
    "Group by",
    "-",
    "Folders first",
];

/// The images of each row `menu` shows that are glyphs, not the marks, a
/// submenu's arrow or the back arrow, by row label.
fn row_glyphs(menu: &MenuPopover) -> Vec<(String, String)> {
    let not_glyphs = [
        Icon::CircleSmallFilled.name(),
        Icon::Checkmark.name(),
        Icon::ChevronRight16.name(),
    ];
    menu.rows()
        .into_iter()
        .flat_map(|row| {
            let label = crate::window::menu_popover::row_label(&row).unwrap_or_default();
            descendants::<gtk::Image>(&row)
                .into_iter()
                .filter_map(|image| image.icon_name().map(|name| name.to_string()))
                .filter(|name| !not_glyphs.contains(&name.as_str()))
                .map(move |name| (label.clone(), name))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The marks `menu` shows, by row label: the mark's icon name.
fn row_marks(menu: &MenuPopover) -> Vec<(String, String)> {
    menu.rows()
        .into_iter()
        .flat_map(|row| {
            let label = crate::window::menu_popover::row_label(&row).unwrap_or_default();
            descendants::<gtk::Image>(&row)
                .into_iter()
                .filter(|image| image.has_css_class("menu-mark"))
                .filter_map(|image| image.icon_name().map(|name| name.to_string()))
                .map(move |name| (label.clone(), name))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The Sort menu is Windows 11's: no row has a glyph, in its More and
/// Group by submenus either; a dot marks the chosen column, direction and
/// grouping. The app's own Folders first stays, after a divider, as a
/// toggle with a check mark.
///
/// parity: VIEW-017, VIEW-022
#[gtk::test]
fn the_sort_menu_has_no_glyphs_and_dots_its_choices_as_windows_does() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("sort", Some("modified"));
    test.activate("direction", Some("descending"));
    let sort = menu_of(&test, "Sort");
    sort.popup();
    wait_for_frames(&test.window, 2);
    assert_eq!(sort.row_labels(), SORT_MENU);
    assert_eq!(row_glyphs(&sort), Vec::<(String, String)>::new());
    let dot = Icon::CircleSmallFilled.name().to_owned();
    let marks = row_marks(&sort);
    assert!(
        marks.contains(&("Date modified".to_owned(), dot.clone())),
        "{marks:?}"
    );
    assert!(
        marks.contains(&("Descending".to_owned(), dot.clone())),
        "{marks:?}"
    );
    if test.action_state("folders-first").as_deref() == Some("true") {
        assert!(
            marks.contains(&("Folders first".to_owned(), Icon::Checkmark.name().to_owned())),
            "{marks:?}"
        );
    }
    assert!(
        marks
            .iter()
            .filter(|(label, _)| label != "Folders first")
            .all(|(_, mark)| *mark == dot),
        "the choices are dotted: {marks:?}"
    );
    for submenu in ["More", "Group by"] {
        let opened = sort.open_submenu_of(submenu);
        wait_for_frames(&test.window, 2);
        assert_eq!(row_glyphs(&opened), Vec::<(String, String)>::new(), "{submenu}");
        let marks = row_marks(&opened);
        assert!(marks.iter().all(|(_, mark)| *mark == dot), "{submenu}: {marks:?}");
        assert!(opened.press_in_list(gdk::Key::Left), "Left closes {submenu}");
        wait_until("the submenu to close", || !opened.is_visible());
    }
    sort.popdown();
}

/// The "…" menu in a folder with nothing to undo, as Windows 11 File
/// Explorer's: Pin, the selection commands with the app's Select items
/// matching…, then Properties and Options (CMD-007).
const MORE_MENU: [&str; 9] = [
    "Pin to Quick access",
    "-",
    "Select all",
    "Select none",
    "Invert selection",
    "Select items matching…",
    "-",
    "Properties",
    "Options",
];

/// The "…" menu in Network, as Windows 11's in This PC: the network drive
/// commands, the selection commands, Properties and Options, and no Pin.
/// Windows' Connect to a media server has no `GVfs` backend to run, so it is
/// not listed.
const MORE_MENU_ON_A_NETWORK_PAGE: [&str; 11] = [
    "Add a network location",
    "Map network drive",
    "Disconnect network drive",
    "-",
    "Select all",
    "Select none",
    "Invert selection",
    "Select items matching…",
    "-",
    "Properties",
    "Options",
];

/// The items that left "…" for Settings, Folder Options and View > Show.
const MOVED_FROM_MORE_MENU: [&str; 12] = [
    "New window",
    "Settings",
    "Default file explorer…",
    "Cache this folder for search",
    "Light appearance",
    "Dark appearance",
    "Use system appearance",
    "Show hidden files",
    "Keyboard shortcuts",
    "Help",
    "License & source",
    "About this build",
];

/// parity: VIEW-013, CMD-007
#[gtk::test]
fn the_menus_list_the_current_items_between_the_same_dividers() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert_eq!(menu_of(&test, "New").row_labels(), NEW_MENU);
    assert_eq!(menu_of(&test, "Sort").row_labels(), SORT_MENU);
    assert_eq!(menu_of(&test, "More options").row_labels(), MORE_MENU);
}

/// The "…" menu in This PC where the desktop has no system information
/// app: Properties, which would open it, is left out rather than greyed.
const MORE_MENU_IN_THIS_PC_WITHOUT_SYSTEM_INFORMATION: [&str; 10] = [
    "Add a network location",
    "Map network drive",
    "Disconnect network drive",
    "-",
    "Select all",
    "Select none",
    "Invert selection",
    "Select items matching…",
    "-",
    "Options",
];

/// "…" in This PC and Network lists Windows 11's network drive commands
/// in place of Pin: Map network drive and Add a network location open the
/// connect dialog, and Disconnect network drive says why it cannot run
/// while no share is connected. Network's Properties says pages have
/// none; This PC's opens the system information app, and is left out here,
/// where the test session has none. With the navigation pane's button
/// gone, as Windows has none, "…" is where This PC maps a drive.
///
/// parity: CMD-007, NET-001, CMD-031
#[gtk::test]
fn the_more_menu_of_this_pc_and_network_has_the_network_drive_commands() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert!(
        crate::integration::system_information().is_none(),
        "the test session has no system information app"
    );
    for page in [Page::ThisPc, Page::Network] {
        test.window.navigate(page.uri()).expect("a page");
        let labels = menu_of(&test, "More options").row_labels();
        let expected: &[&str] = if page == Page::ThisPc {
            &MORE_MENU_IN_THIS_PC_WITHOUT_SYSTEM_INFORMATION
        } else {
            &MORE_MENU_ON_A_NETWORK_PAGE
        };
        assert_eq!(labels, expected, "{page:?}");
        let more = menu_of(&test, "More options");
        more.popup();
        wait_for_frames(&test.window, 2);
        assert!(more.row("Map network drive").is_sensitive(), "{page:?}");
        assert!(more.row("Add a network location").is_sensitive(), "{page:?}");
        let disconnect = more.row("Disconnect network drive");
        assert!(!disconnect.is_sensitive(), "{page:?}");
        assert_eq!(
            disconnect.tooltip_text().unwrap_or_default().lines().nth(1),
            Some("No network drive is connected."),
            "{page:?}"
        );
        if page == Page::Network {
            let properties = more.row("Properties");
            assert_eq!(
                properties.tooltip_text().unwrap_or_default().lines().nth(1),
                Some("Pages such as This PC have no properties."),
                "{page:?}"
            );
        }
        more.popdown();
    }
    test.window.navigate(&fixture.uri()).expect("the folder");
    let labels = menu_of(&test, "More options").row_labels();
    assert_eq!(labels, MORE_MENU);
    for moved in MOVED_FROM_MORE_MENU {
        assert!(!labels.iter().any(|label| label == moved), "{moved} moved");
    }
}

/// Disconnect network drive lists each network mount once, by the root
/// the volume monitor mounted: a saved share inside a mount is not a drive
/// of its own, since disconnecting it ends the whole mount, and the mount
/// is not listed again for the share.
///
/// parity: CMD-007, DEV-006
#[gtk::test]
fn disconnect_network_drive_lists_each_mount_once_by_its_root() {
    use crate::test_support::mounted_volume;
    use crate::volumes::VolumeKind;
    use crate::window::menu_popover::MenuEntry;

    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.save_share("smb://fileserver/share/team", "Team");
    wait_until("the window to read the saved share", || {
        test.window
            .places()
            .network
            .iter()
            .any(|location| location.label == "Team")
    });
    test.window.imp().volumes.replace(vec![mounted_volume(
        "share on fileserver",
        "smb://fileserver/share/",
        VolumeKind::Drive,
    )]);
    test.window.render_places();
    let team_is_connected = test
        .window
        .places()
        .network
        .iter()
        .any(|location| location.label == "Team" && location.is_connected);
    assert!(
        team_is_connected,
        "the saved share inside the mount counts as connected"
    );
    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    let more = menu_of(&test, "More options");
    let disconnect = more
        .items()
        .into_iter()
        .find(|item| item.label == "Disconnect network drive")
        .expect("This PC lists Disconnect network drive");
    let choices: Vec<(String, Option<String>)> = disconnect
        .submenu
        .iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some((
                item.label.clone(),
                item.target.as_ref().and_then(glib::Variant::get::<String>),
            )),
            MenuEntry::Divider => None,
        })
        .collect();
    assert_eq!(
        choices,
        [(
            "share on fileserver".to_owned(),
            Some("smb://fileserver/share/".to_owned())
        )]
    );
}

/// This PC's "…" > Properties opens the desktop's system information app,
/// as Windows' opens the system's properties: here KDE's Info Center,
/// found by its desktop file and launched as an application.
///
/// parity: CMD-007
#[gtk::test]
fn this_pc_properties_opens_the_system_information_app() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let info_center = super::support::install_test_application("org.kde.kinfocenter.desktop");
    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    wait_until("… to offer Properties", || {
        menu_of(&test, "More options")
            .row_labels()
            .contains(&"Properties".to_owned())
    });
    let item = more_item(&test, "Properties");
    assert_eq!(
        item.action,
        crate::window::menu_popover::MenuAction::Window(WindowAction::SystemProperties)
    );
    let more = menu_of(&test, "More options");
    more.popup();
    wait_for_frames(&test.window, 2);
    assert!(more.row("Properties").is_sensitive());
    more.popdown();

    test.activate("system-properties", None);

    assert_eq!(
        test.context.recorded_launches(),
        ["app:org.kde.kinfocenter.desktop"]
    );
    drop(info_center);
}

/// The glyph of `row`, a menu row: the image after its mark slot.
fn row_glyph(row: &gtk::ListBoxRow) -> Option<String> {
    let images = descendants::<gtk::Image>(row);
    images
        .into_iter()
        .filter(|image| !image.has_css_class("menu-mark"))
        .find_map(|image| image.icon_name().map(|name| name.to_string()))
}

/// "…" in a folder shows Undo and Redo only while each can run, named
/// plainly "Undo" and "Redo" as Windows 11's "…" names them (the folder's
/// own menu names the step), with Redo's glyph Undo's mirrored as Windows
/// draws it; and Properties, with Windows' wrench, opens the folder's
/// Properties or the selection's.
///
/// parity: CMD-007, OPS-029, CMD-011
#[gtk::test]
fn the_more_menu_shows_undo_and_redo_only_while_they_can_run() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let more = || menu_of(&test, "More options");
    let undo_redo = || {
        let labels = more().row_labels();
        labels
            .into_iter()
            .filter(|label| label.starts_with("Undo") || label.starts_with("Redo"))
            .collect::<Vec<_>>()
    };
    assert!(undo_redo().is_empty(), "{:?}", more().row_labels());

    for _ in 0..2 {
        test.activate("new-folder", None);
        super::file_ops_support::name_editor(&test).emit_activate();
        wait_until("the name to be kept", || {
            !super::file_ops_support::is_renaming_in_place(&test)
        });
    }
    wait_until("… to offer Undo", || undo_redo() == ["Undo"]);
    let labels = more().row_labels();
    assert_eq!(&labels[..3], ["Undo", "-", "Pin to Quick access"]);

    test.activate("undo", None);
    wait_until("… to offer Undo and Redo", || undo_redo() == ["Undo", "Redo"]);
    let menu = more();
    menu.popup();
    wait_for_frames(&test.window, 2);
    let undo = row_glyph(&menu.row("Undo"));
    let redo_row = menu.row("Redo");
    assert_eq!(undo.as_deref(), Some(Icon::ArrowUndo.name()));
    assert_eq!(row_glyph(&redo_row).as_deref(), Some(Icon::ArrowUndo.name()));
    let mirrored = descendants::<gtk::Image>(&redo_row)
        .into_iter()
        .any(|image| image.has_css_class("mirrored"));
    assert!(mirrored, "Redo shows Undo's glyph mirrored");
    for row in [menu.row("Undo"), redo_row.clone()] {
        let blue = descendants::<gtk::Image>(&row)
            .into_iter()
            .any(|image| image.has_css_class("accent-glyph"));
        assert!(blue, "Undo and Redo are drawn in Windows' blue");
    }
    let properties = menu.row("Properties");
    assert!(properties.is_sensitive());
    assert_eq!(row_glyph(&properties).as_deref(), Some(Icon::Wrench.name()));
    menu.popdown();

    test.activate("undo", None);
    wait_until("… to offer Redo only", || undo_redo() == ["Redo"]);
    assert_eq!(
        more()
            .items_labels()
            .iter()
            .find(|label| label.starts_with("Redo"))
            .map(String::as_str),
        Some("Redo")
    );
}

/// The shortcut texts `menu`'s rows show, opened now.
fn shortcut_texts(menu: &MenuPopover) -> Vec<String> {
    let labels = descendants::<gtk::Label>(&menu.row_list());
    labels
        .into_iter()
        .filter(|label| label.has_css_class("shortcut"))
        .map(|label| label.text().to_string())
        .collect()
}

/// Windows 11's "…", View and View > Show show no shortcut text beside
/// their items, so neither do the app's; the keys still work and the
/// keyboard shortcuts window lists them (CMD-032).
///
/// parity: CMD-007, CMD-006, CMD-016
#[gtk::test]
fn the_more_view_and_show_menus_show_no_shortcut_text_as_in_windows() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("new-folder", None);
    super::file_ops_support::name_editor(&test).emit_activate();
    wait_until("the name to be kept", || {
        !super::file_ops_support::is_renaming_in_place(&test)
    });
    wait_until("… to offer Undo", || {
        menu_of(&test, "More options")
            .row_labels()
            .contains(&"Undo".to_owned())
    });
    for page in [None, Some(Page::ThisPc)] {
        if let Some(page) = page {
            test.window.navigate(page.uri()).expect("a page");
        }
        let more = menu_of(&test, "More options");
        assert_eq!(shortcut_texts(&more), Vec::<String>::new(), "{page:?}");
    }
    let view = menu_of(&test, "View");
    view.popup();
    wait_until("the View menu", || view.is_visible());
    assert_eq!(shortcut_texts(&view), Vec::<String>::new());
    let show = view.open_submenu_of("Show");
    assert_eq!(shortcut_texts(&show), Vec::<String>::new());
    view.popdown();
}

/// "…" > Properties runs `win.properties`, which describes the selection
/// when items are selected and the folder shown otherwise, as Windows 11's
/// does.
///
/// parity: CMD-007, PROP-001
#[gtk::test]
fn properties_in_the_more_menu_describes_the_selection_or_the_folder() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let item = more_item(&test, "Properties");
    assert_eq!(
        item.action,
        crate::window::menu_popover::MenuAction::Window(WindowAction::Properties)
    );
    assert!(item.target.is_none());
    assert!(
        test.window.is_action_enabled(WindowAction::Properties.name()),
        "the folder's"
    );
    select_names(&test, &["Notes 2.txt", "Documents"]);
    assert!(
        test.window.is_action_enabled(WindowAction::Properties.name()),
        "the selection's"
    );
}

/// The "…" item labelled `label`.
fn more_item(test: &TestWindow, label: &str) -> crate::window::menu_popover::MenuItem {
    menu_of(test, "More options")
        .items()
        .into_iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("… has {label}"))
}

/// Whether `test`'s window has `uri` pinned to Quick access.
fn is_pinned(test: &TestWindow, uri: &str) -> bool {
    let pins = test.window.places().quick_access;
    pins.iter()
        .any(|place| ox_core::location::same_location(&place.uri, uri))
}

/// "…" pins the folder shown or unpins it, as Windows 11's toggles
/// between "Pin to Quick access" and "Unpin from Quick access", and
/// follows a pin added or removed elsewhere: from the folder's own
/// item, a right-click menu or the sidebar's Unpin.
///
/// parity: CMD-007, SIDE-009
#[gtk::test]
fn the_more_menu_pins_or_unpins_the_folder_shown() {
    const PIN: &str = "Pin to Quick access";
    const UNPIN: &str = "Unpin from Quick access";
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let folder = fixture.uri();
    let more = || menu_of(&test, "More options");
    let offers = |label: &str| more().row_labels().iter().any(|row| row == label);
    assert!(offers(PIN) && !offers(UNPIN), "{:?}", more().row_labels());

    let button = menu_button(&test, "More options");
    button.popup();
    wait_until("the menu", || more().is_visible());
    more().click_row(PIN);
    wait_until("the folder to be pinned", || is_pinned(&test, &folder));
    wait_until("… to offer Unpin", || offers(UNPIN) && !offers(PIN));

    button.popup();
    wait_until("the menu", || more().is_visible());
    let unpin = more().row(UNPIN);
    assert!(unpin.is_sensitive());
    more().click_row(UNPIN);
    wait_until("the folder to be unpinned", || !is_pinned(&test, &folder));
    wait_until("… to offer Pin again", || offers(PIN) && !offers(UNPIN));

    // Pinned from a right-click menu, then shown: "…" offers Unpin.
    let documents = fixture.uri_of("Documents");
    select_names(&test, &["Documents"]);
    test.activate("pin-selected", None);
    wait_until("Documents to be pinned", || is_pinned(&test, &documents));
    test.window.navigate(&documents).expect("Documents");
    wait_until("… to offer Unpin in Documents", || offers(UNPIN));

    // Unpinned from the sidebar's menu, which runs win.unpin with the pin.
    test.activate("unpin", Some(&documents));
    wait_until("Documents to be unpinned", || !is_pinned(&test, &documents));
    wait_until("… to offer Pin after the sidebar unpins", || {
        offers(PIN) && !offers(UNPIN)
    });
}

/// What left "…" keeps its keys: Ctrl+N opens a new window, Ctrl+, opens
/// Settings, F1 the manual and Ctrl+? the keyboard shortcuts.
///
/// parity: CMD-007, CMD-016
#[gtk::test]
fn the_items_moved_out_of_the_more_menu_keep_their_keys() {
    let fixture = Fixture::standard();
    let _test = laid_out(&fixture.uri());
    for (action, key) in [
        (WindowAction::Help.detailed_name(), "F1"),
        (
            WindowAction::KeyboardShortcuts.detailed_name(),
            "<Control>question",
        ),
        (WindowAction::Settings.detailed_name(), "<Control>comma"),
    ] {
        let keys = application().accels_for_action(&action);
        assert!(keys.iter().any(|bound| bound == key), "{action}: {keys:?}");
    }
    let new_window = crate::application::AppAction::NewWindow.detailed_name();
    let ctrl_n = crate::window::window_keys::window_key_bindings()
        .any(|(action, keys)| action == new_window && keys == "<Primary>n");
    assert!(ctrl_n, "Ctrl+N opens a new window");
}

/// The command bar control named `name` as the menu button it is.
fn menu_button(test: &TestWindow, name: &str) -> gtk::MenuButton {
    command_bar_controls(test)
        .into_iter()
        .find(|control| control_name(control).as_deref() == Some(name))
        .and_then(|control| control.downcast::<gtk::MenuButton>().ok())
        .unwrap_or_else(|| panic!("{name} is a menu button"))
}

/// Every menu of the bar drops down below its button, Enter on a focused
/// button opens its menu without opening the selected file, and closing
/// the menu returns the keyboard to the button.
///
/// parity: CMD-005, CMD-031
#[gtk::test]
fn enter_on_a_command_opens_its_menu_below_it_and_not_the_selection() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for name in ["New", "Sort", "View", "More options"] {
        let popover = menu_button(&test, name).popover().expect("a menu");
        assert_eq!(popover.position(), gtk::PositionType::Bottom, "{name}");
    }
    select_names(&test, &["Notes 2.txt"]);
    let new = menu_button(&test, "New");
    new.grab_focus();

    // Enter goes to the focused button: the views' Enter handlers sit
    // outside it, and no app shortcut or window key handler takes Enter
    // (GtkWindow's own Enter only activates the focused widget).
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a focused widget");
    assert!(focus.is_ancestor(&new), "the button has the focus");
    let takes_enter = window_shortcuts(&test).into_iter().any(|shortcut| {
        let is_ours = shortcut
            .action()
            .is_some_and(|action| action.is::<gtk::NamedAction>() || action.is::<gtk::CallbackAction>());
        let trigger = shortcut.trigger();
        let on_enter = [gdk::Key::Return, gdk::Key::KP_Enter].into_iter().any(|key| {
            trigger
                .as_ref()
                .is_some_and(|trigger| is_triggered_by(trigger, key, gdk::ModifierType::empty()))
        });
        is_ours && on_enter
    });
    assert!(!takes_enter, "no window shortcut of the app takes Enter");
    // Enter on a focused button is its "activate" key binding.
    new.emit_by_name::<()>("activate", &[]);

    // A button shows the press for a moment before it acts.
    let menu = new.popover().expect("New has a menu");
    wait_until("Enter to open New's menu", || menu.is_visible());
    assert!(test.context.recorded_launches().is_empty(), "no file opened");

    // Closing the menu gives the keyboard back to its button (CMD-031).
    menu.popdown();
    wait_until("focus back on New", || {
        gtk::prelude::GtkWindowExt::focus(&test.window).is_some_and(|focus| focus.is_ancestor(&new))
    });
}

/// A submenu opened before its menu has laid out its rows, as Right or
/// Enter can just after a menu opens, and as a newer GTK maps a menu
/// before it lays it out, opens beside its row once the rows are laid
/// out, not level with the menu's top.
///
/// parity: CMD-031
#[gtk::test]
fn a_submenu_opened_before_its_menu_is_laid_out_opens_beside_its_row() {
    use crate::window::menu_popover::{MenuEntry, MenuItem};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let button = super::support::menu_button_with_class(&test, "new-command");
    let menu = super::support::app_menu(&button);
    let entries = || -> Vec<MenuEntry> {
        let mut entries: Vec<MenuEntry> = (0..6)
            .map(|row| MenuItem::new(&format!("Item {row}"), Icon::Folder, WindowAction::NewFolder).into())
            .collect();
        let deep = vec![MenuItem::new("Note", Icon::Document, WindowAction::NewFolder).into()];
        entries.push(MenuItem::submenu("Letters", Icon::Folder, WindowAction::ShowNewMenu, deep).into());
        entries
    };
    menu.set_entries(entries());
    button.popup();
    wait_until("the New menu", || menu.is_mapped());
    // New rows, which the menu has not laid out yet, and Letters opened
    // from the keyboard at once.
    menu.set_entries(entries());
    menu.open_submenu(6, true);
    let letters = menu.open_submenu_menu().expect("Letters opens");
    let row = menu.row("Letters");
    let row_top = || {
        #[expect(clippy::cast_possible_truncation, reason = "menu rows are small")]
        let top = row.compute_bounds(&menu).map(|bounds| bounds.y() as i32);
        top.filter(|_| row.height() > 0)
    };
    wait_until("Letters beside its row", || {
        let (points, pointing) = letters.pointing_to();
        points && row_top().is_some_and(|top| top > 0 && pointing.y() == top)
    });
    menu.popdown();
}

/// Escape or Left in a menu whose submenu the pointer opened, which
/// leaves the keyboard in the menu, closes only that submenu, as on
/// Windows: not the whole menu, and at a nested level not the submenu the
/// keyboard is in.
///
/// parity: CMD-031, OPS-003
#[gtk::test]
fn escape_or_left_closes_only_a_submenu_the_pointer_opened() {
    use crate::window::menu_popover::{MenuEntry, MenuItem};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let button = super::support::menu_button_with_class(&test, "new-command");
    let menu = super::support::app_menu(&button);
    let deep: Vec<MenuEntry> = vec![MenuItem::new("Note", Icon::Document, WindowAction::NewFolder).into()];
    let work = MenuItem::submenu("Work", Icon::Folder, WindowAction::ShowNewMenu, deep);
    let letters = MenuItem::submenu(
        "Letters",
        Icon::Folder,
        WindowAction::ShowNewMenu,
        vec![work.into()],
    );
    menu.set_entries(vec![
        MenuItem::new("Folder", Icon::Folder, WindowAction::NewFolder).into(),
        letters.into(),
    ]);
    button.popup();
    wait_until("the New menu", || menu.is_visible());

    for key in [gdk::Key::Escape, gdk::Key::Left] {
        // The pointer opens Letters: the keyboard stays in the New menu.
        let letters = menu.open_submenu_of("Letters");
        assert!(menu.press_in_list(key), "{key:?} is the submenu's to handle");
        assert!(menu.open_submenu_menu().is_none(), "{key:?} closes Letters");
        assert!(!letters.is_visible());
        assert!(menu.is_visible(), "{key:?} leaves the New menu open");
        assert_eq!(
            gtk::prelude::GtkWindowExt::focus(&test.window).and_downcast::<gtk::ListBoxRow>(),
            Some(menu.row("Letters")),
            "the keyboard is back on Letters"
        );

        // Nested: the pointer opens Work from Letters; the key in Letters
        // closes Work, not Letters.
        let letters = menu.open_submenu_of("Letters");
        let work = letters.open_submenu_of("Work");
        assert!(letters.press_in_list(key));
        assert!(
            letters.open_submenu_menu().is_none() && !work.is_visible(),
            "{key:?} closes Work"
        );
        assert!(letters.is_visible(), "{key:?} leaves Letters open");
        assert_eq!(menu.open_submenu_menu().as_ref(), Some(&letters));
        // Again in Letters, with nothing open from it, it closes Letters.
        assert!(letters.press_in_list(key));
        assert!(menu.open_submenu_menu().is_none() && menu.is_visible());

        // The pointer opens Letters, then Work, then goes back to the New
        // menu's Letters row, taking the keyboard there: the key closes
        // only Work, the innermost.
        let letters = menu.open_submenu_of("Letters");
        let work = letters.open_submenu_of("Work");
        menu.row("Letters").grab_focus();
        assert!(menu.press_in_list(key));
        assert!(!work.is_visible(), "{key:?} closes Work");
        assert!(letters.is_visible(), "{key:?} leaves Letters open");
        assert!(menu.press_in_list(key));
        assert!(menu.open_submenu_menu().is_none() && menu.is_visible());
    }
    menu.popdown();
}

/// The View menu, as Windows 11 File Explorer's: the views from the
/// largest icons to Details, Tiles and Content, the Details pane, Show,
/// then Dolphin's split view, folder tree and terminal where they were
/// before Show, and the text size.
const VIEW_MENU: [&str; 20] = [
    "Extra large icons",
    "Large icons",
    "Medium icons",
    "Small icons",
    "List",
    "Details",
    "Tiles",
    "Content",
    "-",
    "Details pane",
    "Preview pane",
    "Show",
    "-",
    "Split view",
    "Folder tree",
    "Terminal",
    "-",
    "Larger text",
    "Smaller text",
    "Reset text size",
];

/// View > Show, as Windows 11's: the navigation pane, Compact view, item
/// check boxes, File name extensions and Hidden items.
const SHOW_MENU: [&str; 7] = [
    "Navigation pane",
    "-",
    "Compact view",
    "-",
    "Item check boxes",
    "File name extensions",
    "Hidden items",
];

/// The View menu lists Windows 11's items in its order; Details pane is
/// beside the views with a dot while it is open, and View > Show's
/// toggles show a check mark while on. Hidden items is the hidden-files
/// toggle and Ctrl+H's.
///
/// parity: CMD-006, VIEW-023
#[gtk::test]
fn view_show_checks_hidden_items_and_the_details_pane_while_on() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let view = menu_of(&test, "View");
    assert_eq!(view.row_labels(), VIEW_MENU);
    let marks_now = || {
        view.popup();
        wait_until("the View menu", || view.is_visible());
        let pane = view.checked_labels().contains(&"Details pane".to_owned());
        let show = view.open_submenu_of("Show");
        assert_eq!(show.row_labels(), SHOW_MENU);
        assert!(show.row("Hidden items").is_sensitive());
        let hidden = show.checked_labels().contains(&"Hidden items".to_owned());
        view.popdown();
        wait_until("the View menu to close", || !view.is_visible());
        [hidden, pane]
    };
    let before = marks_now();

    test.activate("hidden", None);
    test.activate("details-pane", None);

    assert_eq!(marks_now(), before.map(|was_on| !was_on));
}

/// The mark in the column left of a menu row's glyph, as Windows 11 draws
/// it: a dot for a choice and the Details pane, a check mark for a
/// toggle, and the item's own glyph beside it either way.
///
/// parity: CMD-006, CMD-031
#[gtk::test]
fn menu_marks_are_dots_for_choices_and_checks_for_toggles_beside_the_glyph() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("view", Some("details"));
    let view = menu_of(&test, "View");
    view.popup();
    wait_until("the View menu", || view.is_visible());
    let mark_in = |menu: &crate::window::menu_popover::MenuPopover, label: &str| {
        let row = menu.row(label);
        descendants::<gtk::Image>(&row)
            .into_iter()
            .find(|image| image.has_css_class("menu-mark"))
            .and_then(|image| image.icon_name())
            .map(|name| name.to_string())
    };
    assert_eq!(
        mark_in(&view, "Details").as_deref(),
        Some(Icon::CircleSmallFilled.name())
    );
    assert_eq!(
        row_glyph(&view.row("Details")).as_deref(),
        Some(Icon::TextBulletList.name())
    );
    assert_eq!(
        mark_in(&view, "Large icons"),
        None,
        "an unchosen view has no mark"
    );
    let show = view.open_submenu_of("Show");
    if test.window.is_action_enabled(WindowAction::Sidebar.name()) {
        assert_eq!(
            mark_in(&show, "Navigation pane").as_deref(),
            Some(Icon::Checkmark.name())
        );
        assert_eq!(
            row_glyph(&show.row("Navigation pane")).as_deref(),
            Some(Icon::PanelLeft.name())
        );
    }
    view.popdown();
}

/// The glyphs of "…" and View are Windows 11's: original icons made to
/// match Windows' for Select all, Select none, Invert selection, Options
/// and the network commands (no Fluent glyph has them), and the bundled
/// Fluent icons closest to the rest: Undo's arrow and the pin in Windows'
/// blue, the wrench, the view sizes, the panes and Show's items.
///
/// parity: CMD-007, CMD-006
#[gtk::test]
fn the_more_and_view_menus_show_windows_glyphs() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let glyph_of = |menu: &MenuPopover, label: &str| {
        menu.items()
            .into_iter()
            .find(|item| item.label == label)
            .map_or_else(|| panic!("the menu has {label}"), |item| item.glyph)
    };
    let more = menu_of(&test, "More options");
    for (label, glyph) in [
        ("Pin to Quick access", Icon::Pin),
        ("Select all", Icon::SelectAll24),
        ("Select none", Icon::SelectNone24),
        ("Invert selection", Icon::InvertSelection24),
        ("Properties", Icon::Wrench),
        ("Options", Icon::Options24),
    ] {
        assert_eq!(glyph_of(&more, label), glyph, "{label}");
    }
    // Windows draws Undo, Redo and Pin in blue; the selection glyphs carry
    // their own blue, and the rest are plain.
    let accented = |menu: &MenuPopover| -> Vec<String> {
        menu.items()
            .into_iter()
            .filter(|item| item.glyph_look.accent)
            .map(|item| item.label)
            .collect()
    };
    assert_eq!(accented(&more), ["Pin to Quick access"]);
    more.popup();
    wait_for_frames(&test.window, 2);
    let blue = descendants::<gtk::Image>(&more.row("Pin to Quick access"))
        .into_iter()
        .any(|image| image.has_css_class("accent-glyph"));
    assert!(blue, "Pin's glyph is drawn in Windows' blue");
    more.popdown();
    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    let this_pc = menu_of(&test, "More options");
    assert!(
        accented(&this_pc).is_empty(),
        "This PC's network icons carry their own blue; none is tinted"
    );
    for (label, glyph) in [
        ("Add a network location", Icon::AddNetworkLocation24),
        ("Map network drive", Icon::MapNetworkDrive24),
        ("Disconnect network drive", Icon::DisconnectNetworkDrive24),
        ("Options", Icon::Options24),
    ] {
        assert_eq!(glyph_of(&this_pc, label), glyph, "{label}");
    }
    let view = menu_of(&test, "View");
    for (label, glyph) in [
        ("Extra large icons", Icon::RectangleLandscape),
        ("Large icons", Icon::RectangleLandscape),
        ("Medium icons", Icon::Desktop),
        ("Small icons", Icon::Grid),
        ("List", Icon::TextColumnTwo),
        ("Details", Icon::TextBulletList),
        ("Tiles", Icon::AppsListDetail),
        ("Content", Icon::ContentView),
        ("Details pane", Icon::PanelRight),
    ] {
        assert_eq!(glyph_of(&view, label), glyph, "{label}");
    }
    let show = view
        .items()
        .into_iter()
        .find(|item| item.label == "Show")
        .expect("View has Show");
    let show_glyphs: Vec<(String, Icon)> = show
        .submenu
        .iter()
        .filter_map(|entry| match entry {
            crate::window::menu_popover::MenuEntry::Item(item) => Some((item.label.clone(), item.glyph)),
            crate::window::menu_popover::MenuEntry::Divider => None,
        })
        .collect();
    assert_eq!(
        show_glyphs,
        [
            ("Navigation pane".to_owned(), Icon::PanelLeft),
            ("Compact view".to_owned(), Icon::ArrowAutofitHeight),
            ("Item check boxes".to_owned(), Icon::CheckboxChecked),
            ("File name extensions".to_owned(), Icon::Document),
            ("Hidden items".to_owned(), Icon::Eye),
        ]
    );
}

/// View > Show opens beside the View menu, as on Windows 11: Show has its
/// chevron at the far right, tells screen readers it opens a submenu,
/// stays highlighted while its submenu is open, and the submenu's first
/// item is level with it.
///
/// parity: CMD-006
#[gtk::test]
fn view_show_opens_beside_the_view_menu_as_on_windows() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let on = |name: &str| {
        test.window
            .lookup_action(name)
            .and_then(|action| action.state())
            .and_then(|state| state.get::<bool>())
    };
    let view = menu_of(&test, "View");
    view.popup();
    wait_until("the View menu", || view.is_mapped());
    let row = view.row("Show");
    let content = row.child().expect("content");
    let parts: Vec<gtk::Widget> = crate::window::widget_tree::children(&content).collect();
    assert!(
        parts
            .last()
            .is_some_and(|last| last.has_css_class("submenu-arrow")),
        "a chevron at the far right"
    );
    assert!(
        gtk::test_accessible_has_property(&row, gtk::AccessibleProperty::HasPopup),
        "screen readers hear that Show opens a submenu"
    );
    assert!(gtk::test_accessible_has_state(
        &row,
        gtk::AccessibleState::Expanded
    ));

    let show = view.open_submenu_of("Show");
    assert_eq!(show.row_labels(), SHOW_MENU);
    assert_eq!(
        show.checked_labels().contains(&"Navigation pane".to_owned()),
        on("sidebar") == Some(true),
        "{:?}",
        show.checked_labels()
    );
    assert!(row.has_css_class("submenu-open"), "Show stays highlighted");
    // Beside Show, its first item level with Show.
    assert_eq!(show.opens_from().as_ref(), Some(&view));
    // Once the View menu has laid out its rows: a newer GTK maps a menu
    // before it lays it out.
    wait_until("Show's submenu beside Show", || {
        let (points, pointing) = show.pointing_to();
        #[expect(clippy::cast_possible_truncation, reason = "menu rows are small")]
        let top = row.compute_bounds(&view).map(|bounds| bounds.y() as i32);
        points && row.height() > 0 && top.is_some_and(|top| top > 0 && pointing.y() == top)
    });
    let compact = on("compact-density");
    show.row("Compact view").emit_activate();
    assert_eq!(on("compact-density"), compact.map(|was| !was));
    wait_until("the menus to close", || !view.is_visible() && !show.is_visible());
}

/// The View menu fades in sliding down out of its button, View > Show
/// sideways out of its row, as Windows 11's menus open, and each opening
/// starts the motion afresh.
///
/// parity: LOOK-019
#[gtk::test]
fn the_view_menu_slides_out_of_its_button_and_show_out_of_its_row_each_time() {
    use crate::window::menu_popover::Slide;

    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let view = menu_of(&test, "View");
    view.popup();
    wait_until("the View menu", || view.is_mapped());
    assert_eq!(view.slide(), Some(Slide::Down));
    let first = view.opening_class();
    assert!(first.is_some(), "the opening names its animation");
    let show = view.open_submenu_of("Show");
    assert_eq!(show.slide(), Some(Slide::Right), "out of its row, at its left");
    view.popdown();
    wait_until("the menus to close", || !view.is_visible() && !show.is_visible());

    view.popup();
    wait_until("the View menu again", || view.is_mapped());
    let second = view.opening_class();
    assert!(second.is_some() && second != first, "the motion starts afresh");
    view.popdown();
    wait_until("the menu to close", || !view.is_visible());
}

/// View > Terminal and Ctrl+Shift+F4, Dolphin's Terminal panel keys, open
/// the desktop's terminal in the folder shown: VTE, which Dolphin's panel
/// would need, is not linked.
///
/// parity: OPEN-022
#[gtk::test]
fn the_view_menu_and_ctrl_shift_f4_open_the_terminal_here() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let view = menu_of(&test, "View");
    view.popup();
    wait_for_frames(&test.window, 2);
    assert!(view.row("Terminal").is_sensitive());
    view.popdown();

    let keys = application().accels_for_action(&WindowAction::OpenTerminal.detailed_name());
    assert!(keys.iter().any(|key| key == "<Shift><Control>F4"), "{keys:?}");
}

/// More options: "Pin to Quick access" needs a folder, so a page that
/// lists it, such as Settings, disables it and says why (CMD-031); This
/// PC and Network list the network drive commands in its place, as
/// Windows 11 does.
///
/// parity: CMD-007, CMD-031
#[gtk::test]
fn more_options_needs_a_folder_for_pin() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let more = menu_of(&test, "More options");
    more.popup();
    wait_for_frames(&test.window, 2);
    assert!(more.row("Pin to Quick access").is_sensitive());
    more.popdown();

    test.window.navigate(Page::Settings.uri()).expect("Settings");
    more.popup();
    wait_for_frames(&test.window, 2);
    let pin = more.row("Pin to Quick access");
    assert!(!pin.is_sensitive());
    let tooltip = pin.tooltip_text().unwrap_or_default();
    assert_eq!(
        tooltip.lines().nth(1),
        Some("Only a folder can be pinned to Quick access.")
    );
    more.popdown();

    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    let labels = more.row_labels();
    assert!(
        !labels.iter().any(|label| label.contains("Quick access")),
        "{labels:?}"
    );
}

/// The appearances left "…" for Settings > Appearance, as Windows 11
/// sets its colours in Settings: the bar has no appearance button, and
/// the theme cards follow `win.theme`, which the keyboard and the cards
/// run.
///
/// parity: LOOK-005, CMD-007
#[gtk::test]
fn the_appearances_are_chosen_in_settings_not_in_more_options() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert!(
        !command_bar_names(&test)
            .iter()
            .any(|name| name == "Light" || name == "Dark"),
        "no appearance button in the bar"
    );
    let labels = menu_of(&test, "More options").row_labels();
    for appearance in ["Light appearance", "Dark appearance", "Use system appearance"] {
        assert!(!labels.iter().any(|label| label == appearance), "{labels:?}");
    }
    test.window
        .open_settings(Some(SettingsView::Category(Category::Appearance)));
    let page = &test.window.imp().settings_page;
    for (theme, card) in [("light", "Light"), ("dark", "Dark"), ("system", "System")] {
        test.activate("theme", Some(theme));
        let radios = descendants::<gtk::CheckButton>(&**page).into_iter();
        let mut cards = radios.filter(|radio| radio.has_css_class("theme-radio"));
        let radio = cards
            .find(|radio| radio.label().as_deref() == Some(card))
            .unwrap_or_else(|| panic!("Appearance has a {card} card"));
        wait_until("the card to follow the theme", || radio.is_active());
    }
}

/// parity: VIEW-006
#[gtk::test]
fn the_view_menu_keeps_the_text_size_items_and_checks_the_current_view() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let view = menu_of(&test, "View");
    let labels = view.row_labels();
    let text_size = ["-", "Larger text", "Smaller text", "Reset text size"];
    assert_eq!(&labels[labels.len() - 4..], text_size);
    assert_eq!(labels[0], "Extra large icons");
    test.activate("view", Some("large"));
    view.popup();
    wait_for_frames(&test.window, 2);
    let checked = view.checked_labels();
    view.popdown();
    assert!(checked.contains(&"Large icons".to_owned()), "{checked:?}");
    assert!(!checked.contains(&"Details".to_owned()), "{checked:?}");
}

/// The text on the clipboard of `test`'s window.
fn clipboard_text(test: &TestWindow) -> Option<String> {
    let clipboard = test.window.clipboard();
    let read = glib::MainContext::default().block_on(clipboard.read_text_future());
    read.ok().flatten().map(|text| text.to_string())
}

/// parity: CLIP-012
#[gtk::test]
fn copy_path_copies_the_selected_items_address_or_the_folders() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("copy-path", None);
    let folder = format!("\"{}\"", fixture.root().display());
    assert_eq!(
        clipboard_text(&test).as_deref(),
        Some(folder.as_str()),
        "nothing selected"
    );
    let message = test.window.shown_message();
    assert_eq!(
        message.as_str(),
        "",
        "copying shows no message, as Windows Explorer"
    );
    test.window.folder_model().select_only(1);
    test.activate("copy-path", None);
    let file = format!("\"{}\"", fixture.path("Notes 2.txt").display());
    assert_eq!(
        clipboard_text(&test).as_deref(),
        Some(file.as_str()),
        "one item selected"
    );
}

/// parity: CLIP-012, CLIP-014
#[gtk::test]
fn copy_path_asks_for_a_folder_on_a_page_and_copies_every_selected_item() {
    let fixture = Fixture::standard();
    let test = laid_out(Page::ThisPc.uri());
    test.activate("copy-path", None);
    let message = test.window.shown_message();
    assert_eq!(message.as_str(), "Open a folder first.");
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    test.window.folder_model().select_all();
    test.activate("copy-path", None);
    let lines = clipboard_text(&test).unwrap_or_default().lines().count();
    assert_eq!(lines, test.names().len(), "one path per selected item");
}

/// Explorer's Ctrl+Shift+C ("Copy as path") and Dolphin's Ctrl+Alt+C
/// ("Copy Location") both run Copy path.
///
/// parity: CLIP-013
#[gtk::test]
fn copy_path_runs_on_explorers_and_dolphins_keys() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let control = gdk::ModifierType::CONTROL_MASK;
    select_names(&test, &["Notes 2.txt"]);

    press_shortcut(&test, gdk::Key::c, control | gdk::ModifierType::SHIFT_MASK);
    let explorer_copy = clipboard_text(&test);
    select_names(&test, &["Notes 10.txt"]);
    press_shortcut(&test, gdk::Key::c, control | gdk::ModifierType::ALT_MASK);

    let first = format!("\"{}\"", fixture.path("Notes 2.txt").display());
    let second = format!("\"{}\"", fixture.path("Notes 10.txt").display());
    assert_eq!(explorer_copy, Some(first));
    assert_eq!(clipboard_text(&test), Some(second));
    assert_eq!(test.window.shown_message().as_str(), "", "and says nothing");
}

/// Submenus within submenus, as template folders within folders in the
/// New menu, open beside each other, as Windows 11's do: each beside the
/// row that opens it, with a chevron at the row's right, its check marks,
/// icons and the reasons a disabled item gives. Only the New menu holds a
/// pointer grab. Choosing a deep item closes every menu and runs it.
///
/// parity: CMD-031, OPS-003
#[gtk::test]
fn nested_submenus_open_beside_each_other() {
    use crate::window::menu_popover::{MenuEntry, MenuItem};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("sort", Some("name"));
    let button = super::support::menu_button_with_class(&test, "new-command");
    let menu = super::support::app_menu(&button);
    let deep: Vec<MenuEntry> = vec![
        MenuItem::choice("Name", Icon::ArrowSort, WindowAction::Sort, "name").into(),
        MenuItem::choice("Size", Icon::ArrowSort, WindowAction::Sort, "size").into(),
        MenuItem::new("Not here", Icon::Document, WindowAction::NewFolder)
            .disabled_because(true, "Not in this folder")
            .into(),
    ];
    let work = MenuItem::submenu("Work", Icon::Folder, WindowAction::ShowNewMenu, deep);
    let letters = MenuItem::submenu(
        "Letters",
        Icon::Folder,
        WindowAction::ShowNewMenu,
        vec![work.into()],
    );
    menu.set_entries(vec![
        MenuItem::new("Folder", Icon::Folder, WindowAction::NewFolder).into(),
        MenuEntry::Divider,
        letters.into(),
    ]);
    let shown_menus = || {
        descendants::<MenuPopover>(&test.window)
            .into_iter()
            .filter(|menu| menu.is_visible() || menu.is_mapped())
            .collect::<Vec<_>>()
    };

    button.popup();
    wait_until("the New menu", || menu.is_visible());
    let letters = menu.open_submenu_of("Letters");
    assert_eq!(letters.row_labels(), ["Work"]);
    let work = letters.open_submenu_of("Work");
    assert_eq!(work.row_labels(), ["Name", "Size", "Not here"]);
    assert_eq!(work.checked_labels(), ["Name"]);
    let disabled = work.row("Not here");
    assert!(!disabled.is_sensitive());
    assert_eq!(
        disabled.tooltip_text().as_deref(),
        Some("Not here\nNot in this folder")
    );
    assert_eq!(
        menu.items_labels(),
        ["Folder", "Letters"],
        "the New menu keeps its rows"
    );
    let shown = shown_menus();
    assert_eq!(shown.len(), 3, "the New menu and two submenus");
    let grabbing: Vec<&MenuPopover> = shown.iter().filter(|menu| menu.is_autohide()).collect();
    assert_eq!(grabbing, [&menu], "only the New menu grabs the pointer");
    assert_eq!(work.opens_from().as_ref(), Some(&letters));
    assert_eq!(letters.opens_from().as_ref(), Some(&menu));
    for submenu in [&letters, &work] {
        assert_eq!(submenu.position(), gtk::PositionType::Right, "beside its row");
        assert_eq!(submenu.valign(), gtk::Align::Start, "level with its row");
    }

    // Left closes only Work, Escape only Letters; Right and Enter open
    // them again with the keyboard on their first item.
    assert!(work.press_in_list(gdk::Key::Left));
    assert!(letters.open_submenu_menu().is_none() && letters.is_visible());
    assert!(letters.press_in_list(gdk::Key::Escape));
    assert!(menu.open_submenu_menu().is_none());
    assert!(menu.is_visible(), "Escape in a submenu leaves the menu open");
    menu.row("Letters").grab_focus();
    assert!(menu.press_in_list(gdk::Key::Right));
    let letters = menu.open_submenu_menu().expect("Right opens Letters");
    assert!(letters.press_in_list(gdk::Key::Return));
    let work = letters.open_submenu_menu().expect("Enter opens Work");
    // Once Letters has laid out Work's row, beside which Work shows.
    wait_until("the keyboard on Work's first item", || {
        gtk::prelude::GtkWindowExt::focus(&test.window).and_downcast::<gtk::ListBoxRow>()
            == Some(work.row("Name"))
    });
    work.click_row("Size");
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
    wait_until("the menu to close", || !menu.is_visible());
    assert!(shown_menus().is_empty());
    assert_eq!(menu.items_labels(), ["Folder", "Letters"], "back at the top");
}
