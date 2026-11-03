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
            // The appearance toggle and Settings are hidden, as Windows 11
            // Explorer's bar has neither.
            None if child.is_visible() => controls.push(child),
            None => {}
        }
    }
    controls
}

/// The menu of the command bar control named `name`.
fn menu_of(test: &TestWindow, name: &str) -> MenuPopover {
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

/// The New menu of `openNewMenu`, then New ▸ Link, a divider as `-`.
const NEW_MENU: [&str; 12] = [
    "Folder",
    "Text document",
    "File…",
    "-",
    "Markdown document",
    "CSV file",
    "JSON file",
    "HTML document",
    "-",
    "From template…",
    "-",
    "Link to file or folder…",
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

/// The "…" menu in This PC and Network, as Windows 11's in This PC: the
/// network drive commands, the selection commands, Properties and
/// Options, and no Pin. Windows' Connect to a media server has no `GVfs`
/// backend to run, so it is not listed.
const MORE_MENU_ON_A_NETWORK_PAGE: [&str; 11] = [
    "Map network drive",
    "Add a network location",
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

/// "…" in This PC and Network lists Windows 11's network drive commands
/// in place of Pin: Map network drive and Add a network location open the
/// connect dialog, and Disconnect network drive says why it cannot run
/// while no share is connected. Properties says pages have none. The
/// sidebar keeps its Map network location button.
///
/// parity: CMD-007, NET-001, CMD-031
#[gtk::test]
fn the_more_menu_of_this_pc_and_network_has_the_network_drive_commands() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for page in [Page::ThisPc, Page::Network] {
        test.window.navigate(page.uri()).expect("a page");
        let labels = menu_of(&test, "More options").row_labels();
        assert_eq!(labels, MORE_MENU_ON_A_NETWORK_PAGE, "{page:?}");
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
        let properties = more.row("Properties");
        assert_eq!(
            properties.tooltip_text().unwrap_or_default().lines().nth(1),
            Some("Pages such as This PC have no properties."),
            "{page:?}"
        );
        more.popdown();
    }
    test.window.navigate(&fixture.uri()).expect("the folder");
    let labels = menu_of(&test, "More options").row_labels();
    assert_eq!(labels, MORE_MENU);
    for moved in MOVED_FROM_MORE_MENU {
        assert!(!labels.iter().any(|label| label == moved), "{moved} moved");
    }
    let sidebar_maps = descendants::<gtk::Button>(test.window.sidebar())
        .into_iter()
        .any(|button| {
            button.action_name().as_deref() == Some(&*WindowAction::MapNetworkLocation.detailed_name())
        });
    assert!(sidebar_maps, "the sidebar keeps Map network location");
}

/// The glyph of `row`, a menu row: the image after its mark slot.
fn row_glyph(row: &gtk::ListBoxRow) -> Option<String> {
    let images = descendants::<gtk::Image>(row);
    images
        .into_iter()
        .filter(|image| !image.has_css_class("menu-mark"))
        .find_map(|image| image.icon_name().map(|name| name.to_string()))
}

/// "…" in a folder shows Undo and Redo only while each can run, naming the
/// step as the folder's menu does, with Redo's glyph Undo's mirrored as
/// Windows 11 draws it; and Properties, with Windows' wrench, opens the
/// folder's Properties or the selection's.
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
    wait_until("… to offer Undo", || undo_redo() == ["Undo New"]);
    let labels = more().row_labels();
    assert_eq!(&labels[..3], ["Undo New", "-", "Pin to Quick access"]);

    test.activate("undo", None);
    wait_until("… to offer Undo and Redo", || {
        undo_redo() == ["Undo New", "Redo New"]
    });
    let menu = more();
    menu.popup();
    wait_for_frames(&test.window, 2);
    let undo = row_glyph(&menu.row("Undo New"));
    let redo_row = menu.row("Redo New");
    assert_eq!(undo.as_deref(), Some(Icon::ArrowUndo.name()));
    assert_eq!(row_glyph(&redo_row).as_deref(), Some(Icon::ArrowUndo.name()));
    let mirrored = descendants::<gtk::Image>(&redo_row)
        .into_iter()
        .any(|image| image.has_css_class("mirrored"));
    assert!(mirrored, "Redo shows Undo's glyph mirrored");
    let properties = menu.row("Properties");
    assert!(properties.is_sensitive());
    assert_eq!(row_glyph(&properties).as_deref(), Some(Icon::Wrench.name()));
    menu.popdown();

    test.activate("undo", None);
    wait_until("… to offer Redo only", || undo_redo() == ["Redo New"]);
    assert_eq!(
        more()
            .items_labels()
            .iter()
            .find(|label| label.starts_with("Redo"))
            .map(String::as_str),
        Some("Redo New")
    );
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

/// The View menu, as Windows 11 File Explorer's: the views from the
/// largest icons to Details, the Details pane, Show, then Dolphin's split
/// view, folder tree and terminal where they were before Show, and the
/// text size. Windows' Tiles, Content and Preview pane are views and a
/// pane the app does not have.
const VIEW_MENU: [&str; 17] = [
    "Extra large icons",
    "Large icons",
    "Medium icons",
    "Small icons",
    "List",
    "Details",
    "-",
    "Details pane",
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

/// View > Show, as Windows 11's with the items the app has: the
/// navigation pane, Compact view, item check boxes and Hidden items.
/// Windows' File name extensions has no setting in the app.
const SHOW_MENU: [&str; 8] = [
    "Show",
    "-",
    "Navigation pane",
    "-",
    "Compact view",
    "-",
    "Item check boxes",
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
        view.click_row("Show");
        assert_eq!(view.row_labels(), SHOW_MENU);
        assert!(view.row("Hidden items").is_sensitive());
        let hidden = view.checked_labels().contains(&"Hidden items".to_owned());
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
    let mark_of = |label: &str| {
        let row = view.row(label);
        descendants::<gtk::Image>(&row)
            .into_iter()
            .find(|image| image.has_css_class("menu-mark"))
            .and_then(|image| image.icon_name())
            .map(|name| name.to_string())
    };
    assert_eq!(
        mark_of("Details").as_deref(),
        Some(Icon::CircleSmallFilled.name())
    );
    assert_eq!(
        row_glyph(&view.row("Details")).as_deref(),
        Some(Icon::TextBulletList.name())
    );
    assert_eq!(mark_of("Large icons"), None, "an unchosen view has no mark");
    view.click_row("Show");
    if test.window.is_action_enabled(WindowAction::Sidebar.name()) {
        assert_eq!(
            mark_of("Navigation pane").as_deref(),
            Some(Icon::Checkmark.name())
        );
        assert_eq!(
            row_glyph(&view.row("Navigation pane")).as_deref(),
            Some(Icon::PanelLeft.name())
        );
    }
    view.popdown();
}

/// The glyphs of "…" and View are Windows 11's, from the bundled Fluent
/// icons: Undo's arrow, the pin, select all and none, invert, the wrench
/// and gear, the view sizes, the panes and Show's items.
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
        ("Select all", Icon::SelectAllOn),
        ("Select none", Icon::Grid),
        ("Invert selection", Icon::SelectObject),
        ("Properties", Icon::Wrench),
        ("Options", Icon::Settings),
    ] {
        assert_eq!(glyph_of(&more, label), glyph, "{label}");
    }
    let view = menu_of(&test, "View");
    for (label, glyph) in [
        ("Extra large icons", Icon::RectangleLandscape),
        ("Large icons", Icon::RectangleLandscape),
        ("Medium icons", Icon::Desktop),
        ("Small icons", Icon::Grid),
        ("List", Icon::TextColumnTwo),
        ("Details", Icon::TextBulletList),
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
            ("Hidden items".to_owned(), Icon::Eye),
        ]
    );
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

/// More options: "Pin to Quick access" needs a folder, so pages disable
/// it and say why (CMD-031).
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

    for page in [Page::ThisPc, Page::Settings] {
        test.window.navigate(page.uri()).expect("a page");
        more.popup();
        wait_for_frames(&test.window, 2);
        let pin = more.row("Pin to Quick access");
        assert!(!pin.is_sensitive(), "{page:?}");
        let tooltip = pin.tooltip_text().unwrap_or_default();
        assert_eq!(
            tooltip.lines().nth(1),
            Some("Only a folder can be pinned to Quick access."),
            "{page:?}"
        );
        more.popdown();
    }
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
/// New menu, show in the one popup: each level under a row that goes
/// back to the level above, with its check marks, icons and the reasons
/// a disabled item gives; choosing a deep item closes the menu and runs
/// it.
///
/// parity: CMD-031, OPS-003
#[gtk::test]
fn nested_submenus_show_one_level_at_a_time_in_one_popup() {
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
            .count()
    };

    button.popup();
    wait_until("the New menu", || menu.is_visible());
    // Clicks go through the list's own click handling, which goes on
    // after the rows it clicked are replaced.
    menu.click_row("Letters");
    assert_eq!(menu.row_labels(), ["Letters", "-", "Work"]);
    menu.click_row("Work");
    assert_eq!(menu.row_labels(), ["Work", "-", "Name", "Size", "Not here"]);
    assert_eq!(menu.checked_labels(), ["Name"]);
    let disabled = menu.row("Not here");
    assert!(!disabled.is_sensitive());
    assert_eq!(
        disabled.tooltip_text().as_deref(),
        Some("Not here\nNot in this folder")
    );
    assert_eq!(shown_menus(), 1, "one popup only");

    // Back one level with Left, then Escape, then down again with
    // Right and Enter, then choose.
    assert!(menu.press_in_list(gdk::Key::Left));
    assert_eq!(menu.row_labels(), ["Letters", "-", "Work"]);
    assert!(menu.press_in_list(gdk::Key::Escape));
    assert_eq!(menu.items_labels(), ["Folder", "Letters"]);
    assert!(menu.is_visible(), "Escape in a submenu leaves the menu open");
    menu.row("Letters").grab_focus();
    assert!(menu.press_in_list(gdk::Key::Right));
    assert_eq!(menu.row_labels(), ["Letters", "-", "Work"]);
    assert!(menu.press_in_list(gdk::Key::Right));
    assert_eq!(menu.row_labels().first().map(String::as_str), Some("Work"));
    menu.click_row("Size");
    assert_eq!(test.action_state("sort").as_deref(), Some("size"));
    wait_until("the menu to close", || !menu.is_visible());
    assert_eq!(shown_menus(), 0);
    assert_eq!(menu.items_labels(), ["Folder", "Letters"], "back at the top");
}
