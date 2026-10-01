// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar and its menus against `section.commandbar` in
//! `v2.0.0:desktop/ui/index.html` and the menus of `setup()` and `openNewMenu` in
//! `v2.0.0:desktop/ui/app.js`: the same controls in the same order, the same menu
//! items and dividers, and the commands that are not ported yet shown
//! disabled with the milestone that brings them.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{is_triggered_by, press_shortcut, select_names, window_shortcuts};
use super::geometry::laid_out;
use crate::icons::Icon;
use crate::locations::Page;
use crate::test_support::harness::{
    descendants, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::menu_popover::MenuPopover;
use crate::window::widget_tree::children;

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

/// The command bar's controls in order, the scrolling file commands
/// included.
fn command_bar_controls(test: &TestWindow) -> Vec<gtk::Widget> {
    let bar = test.window.command_bar();
    let mut controls = Vec::new();
    for child in children(bar) {
        let group = descendants::<gtk::Box>(&child)
            .into_iter()
            .find(|widget| widget.has_css_class("command-group"));
        match group {
            Some(group) => controls.extend(children(&group)),
            None => controls.push(child),
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

#[gtk::test]
fn the_command_bar_has_the_current_controls_in_order() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let names: Vec<String> = command_bar_controls(&test)
        .iter()
        .filter_map(control_name)
        .collect();
    assert_eq!(
        names,
        [
            "New",
            "|",
            "Cut (Ctrl+X)",
            "Copy (Ctrl+C)",
            "Paste files (Ctrl+V)",
            "Rename (F2)",
            "Copy path (does not change sharing permissions)",
            "Move to Trash (Delete)",
            "|",
            "Sort",
            "View",
            "More options",
            "Light",
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
        ("win.trash", "Move to Trash (Delete)"),
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

/// The Sort menu: the columns, then one item per direction.
const SORT_MENU: [&str; 7] = [
    "Name",
    "Date modified",
    "Type",
    "Size",
    "-",
    "Ascending",
    "Descending",
];

/// The appearance button's menu (`appearanceMenu`).
const APPEARANCE_MENU: [&str; 3] = ["Light appearance", "Dark appearance", "Use system appearance"];

/// How the More options menu starts.
const MORE_MENU_START: [&str; 11] = [
    "New window",
    "Settings",
    "Default file explorer…",
    "Cache this folder for search",
    "Map network location",
    "Pin current folder",
    "-",
    "Light appearance",
    "Dark appearance",
    "Use system appearance",
    "Show hidden files",
];

/// How the More options menu ends.
const MORE_MENU_END: [&str; 3] = ["-", "License & source", "About this build"];

/// parity: VIEW-013
#[gtk::test]
fn the_menus_list_the_current_items_between_the_same_dividers() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert_eq!(menu_of(&test, "New").row_labels(), NEW_MENU);
    assert_eq!(menu_of(&test, "Sort").row_labels(), SORT_MENU);
    assert_eq!(menu_of(&test, "Light").row_labels(), APPEARANCE_MENU);
    let more = menu_of(&test, "More options").row_labels();
    assert_eq!(&more[..MORE_MENU_START.len()], MORE_MENU_START);
    assert_eq!(&more[more.len() - MORE_MENU_END.len()..], MORE_MENU_END);
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
    for name in ["New", "Sort", "View", "More options", "Light"] {
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

/// The toggles of the View menu show their check while they are on.
///
/// parity: CMD-006
#[gtk::test]
fn the_view_menu_checks_hidden_files_and_the_details_pane_while_on() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let view = menu_of(&test, "View");
    let toggles = ["Show hidden files", "Details pane"];
    let checked_now = || {
        view.popup();
        wait_for_frames(&test.window, 2);
        let checked = view.checked_labels();
        view.popdown();
        toggles.map(|toggle| checked.contains(&toggle.to_owned()))
    };
    let before = checked_now();

    test.activate("hidden", None);
    test.activate("details-pane", None);

    assert_eq!(checked_now(), before.map(|was_checked| !was_checked));
}

/// More options: "Pin current folder" and the cache toggle need a
/// folder, so pages disable them and say why; "License & source" shows
/// the code glyph app.js lacked.
///
/// parity: CMD-007, CMD-031
#[gtk::test]
fn more_options_needs_a_folder_for_pin_and_cache() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let more = menu_of(&test, "More options");
    more.popup();
    wait_for_frames(&test.window, 2);
    assert!(more.row("Pin current folder").is_sensitive());
    assert!(more.row("Cache this folder for search").is_sensitive());
    more.popdown();

    for page in [Page::ThisPc, Page::Settings] {
        test.window.navigate(page.uri()).expect("a page");
        more.popup();
        wait_for_frames(&test.window, 2);
        let pin = more.row("Pin current folder");
        assert!(!pin.is_sensitive(), "{page:?}");
        let cache = more.row("Cache this folder for search");
        assert!(!cache.is_sensitive(), "{page:?}");
        // Each says why (CMD-031).
        for (row, reason) in [
            (pin, "Only a folder can be pinned to Quick access."),
            (
                cache,
                "Only a folder on a drive or share can be cached for search.",
            ),
        ] {
            let tooltip = row.tooltip_text().unwrap_or_default();
            assert_eq!(tooltip.lines().nth(1), Some(reason), "{page:?}");
        }
        more.popdown();
    }

    let license = more.row("License & source");
    let glyph = license
        .child()
        .and_then(|content| content.first_child())
        .and_downcast::<gtk::Image>()
        .expect("a row starts with its glyph");
    assert_eq!(glyph.icon_name().as_deref(), Some(Icon::Code.name()));
}

/// The Appearance button shows a sun and "Light" or a moon and "Dark"
/// for the drawn appearance, names the choice in its tooltip, and its
/// menu checks the current choice (`applyTheme` in app.js).
///
/// parity: LOOK-005
#[gtk::test]
fn the_appearance_button_shows_the_drawn_appearance_and_its_menu_checks_the_choice() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let button = descendants::<gtk::MenuButton>(test.window.command_bar())
        .into_iter()
        .find(|button| button.has_css_class("theme-toggle"))
        .expect("the command bar has the Appearance button");
    let cases = [
        ("light", "Light", Icon::WeatherSunny, "Light appearance"),
        ("dark", "Dark", Icon::WeatherMoon, "Dark appearance"),
    ];
    for (theme, label, glyph, checked) in cases {
        test.activate("theme", Some(theme));
        let shown = descendants::<gtk::Label>(&button);
        assert_eq!(shown.first().map(gtk::Label::text).as_deref(), Some(label));
        let image = descendants::<gtk::Image>(&button);
        let icon_name = image.first().and_then(gtk::Image::icon_name);
        assert_eq!(icon_name.as_deref(), Some(glyph.name()), "{theme}");
        let tooltip = button.tooltip_text().unwrap_or_default();
        assert_eq!(tooltip, format!("Appearance: {theme}. Click to change."));
        let menu = button
            .popover()
            .and_downcast::<MenuPopover>()
            .expect("an app menu");
        assert_eq!(menu.row_labels(), APPEARANCE_MENU);
        menu.popup();
        wait_for_frames(&test.window, 2);
        let checked_labels = menu.checked_labels();
        menu.popdown();
        assert_eq!(checked_labels, [checked]);
    }
    test.activate("theme", Some("system"));
    let tooltip = button.tooltip_text().unwrap_or_default();
    assert!(tooltip.starts_with("Appearance: System ("), "{tooltip}");
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
    assert_eq!(labels[0], "Details");
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
    let folder = fixture.root().display().to_string();
    assert_eq!(
        clipboard_text(&test).as_deref(),
        Some(folder.as_str()),
        "nothing selected"
    );
    let message = test.window.shown_message();
    assert_eq!(
        message.as_str(),
        "Path copied. Sharing permissions are unchanged."
    );
    test.window.folder_model().select_only(1);
    test.activate("copy-path", None);
    let file = fixture.path("Notes 2.txt").display().to_string();
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

    let first = fixture.path("Notes 2.txt").display().to_string();
    let second = fixture.path("Notes 10.txt").display().to_string();
    assert_eq!(explorer_copy, Some(first));
    assert_eq!(clipboard_text(&test), Some(second));
    assert_eq!(
        test.window.shown_message().as_str(),
        "Path copied. Sharing permissions are unchanged."
    );
}
