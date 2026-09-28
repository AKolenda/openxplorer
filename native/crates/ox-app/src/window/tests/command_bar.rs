// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar and its menus against `section.commandbar` in
//! `desktop/ui/index.html` and the menus of `setup()` and `openNewMenu` in
//! `desktop/ui/app.js`: the same controls in the same order, the same menu
//! items and dividers, and the commands that are not ported yet shown
//! disabled with the milestone that brings them.

use gtk::glib;
use gtk::prelude::*;

use super::geometry::laid_out;
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, Fixture, TestWindow};
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
    let bar = &test.window.chrome().commands.root;
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

#[gtk::test]
fn unported_file_commands_are_disabled_and_name_their_milestone() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for action in ["win.cut", "win.copy", "win.paste", "win.rename", "win.trash"] {
        let button = descendants::<gtk::Button>(&test.window.chrome().commands.root)
            .into_iter()
            .find(|button| button.action_name().as_deref() == Some(action))
            .unwrap_or_else(|| panic!("a button runs {action}"));
        assert!(!button.is_sensitive(), "{action} waits for its workflow");
        let tooltip = button.tooltip_text().unwrap_or_default();
        assert!(tooltip.ends_with("arrives with file operations."), "{tooltip}");
    }
    assert!(
        WidgetExt::activate_action(&test.window, "win.copy-path", None).is_ok(),
        "Copy path works now"
    );
}

#[gtk::test]
fn the_menus_list_the_current_items_between_the_same_dividers() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let new = [
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
    ];
    assert_eq!(menu_of(&test, "New").row_labels(), new);
    let sort = [
        "Name",
        "Date modified",
        "Type",
        "Size",
        "-",
        "Ascending",
        "Descending",
    ];
    assert_eq!(menu_of(&test, "Sort").row_labels(), sort);
    let appearance = ["Light appearance", "Dark appearance", "Use system appearance"];
    assert_eq!(menu_of(&test, "Light").row_labels(), appearance);
    let more = menu_of(&test, "More options").row_labels();
    let expected_start = [
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
    assert_eq!(&more[..expected_start.len()], expected_start);
    assert_eq!(
        &more[more.len() - 3..],
        ["-", "License & source", "About this build"]
    );
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
    let message = test.window.chrome().message.text();
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

#[gtk::test]
fn copy_path_asks_for_a_folder_on_a_page_and_one_item_at_most() {
    let fixture = Fixture::standard();
    let test = laid_out(Page::ThisPc.uri());
    test.activate("copy-path", None);
    let message = test.window.chrome().message.text();
    assert_eq!(message.as_str(), "Open a folder first.");
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    test.window.folder_model().select_all();
    assert!(!test.window.is_action_enabled("copy-path"), "one path at a time");
}
