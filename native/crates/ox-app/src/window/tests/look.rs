// SPDX-License-Identifier: AGPL-3.0-only
//! The Windows 11 look as the window draws it: the bars in Explorer's
//! order and heights, the palette of each appearance from the first frame,
//! the frame and its identity, the fields, the command bar, menus,
//! dialogs and the toast, against `v2.0.0:desktop/ui/style.css` and the
//! reference captures (see [`super::geometry`] for where the numbers come
//! from).

use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::ContextMenu;

use super::context_menus::{choose_menu_style, position_of};
use super::file_ops_support::open_dialog;
use super::geometry::{bounds, laid_out, Bounds};
use super::icons::{assert_same_colour, css_colour, painted, painted_colour, TRANSITION_TIME};
use crate::test_support::harness::{
    descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::test_support::window_font;
use crate::window::BrowserWindow;

/// Switches `test`'s window to `theme` and waits out the colour
/// transitions.
fn show_theme(test: &TestWindow, theme: &str) {
    test.activate("theme", Some(theme));
    wait_for(TRANSITION_TIME);
    wait_for_frames(&test.window, 2);
}

/// Where `widget` is inside `ancestor`, border included, in whole pixels.
fn bounds_in(widget: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> Bounds {
    let rect = widget
        .compute_bounds(ancestor)
        .unwrap_or_else(graphene::Rect::zero);
    Bounds::from_rect(&rect)
}

/// The surfaces of one appearance: title bar, bars, sidebar and content
/// (C01, C04, C06 and C05 of light.css and dark.css).
struct Surfaces {
    theme: &'static str,
    title: &'static str,
    chrome: &'static str,
    sidebar: &'static str,
    content: &'static str,
}

const SURFACES: [Surfaces; 2] = [
    Surfaces {
        theme: "light",
        title: "#eff1f4",
        chrome: "#f9f9f9",
        sidebar: "#fafafa",
        content: "#ffffff",
    },
    Surfaces {
        theme: "dark",
        title: "#191919",
        chrome: "#282828",
        sidebar: "#252525",
        content: "#202020",
    },
];

/// Asserts that `window` paints the surfaces of `expected`.
fn assert_surfaces(window: &BrowserWindow, expected: &Surfaces) {
    let theme = expected.theme;
    let title_bar = window.titlebar().expect("the tab strip is the title bar");
    let title_colour = painted_colour(&title_bar, title_bar.width() - 300, 4);
    assert_same_colour(
        title_colour,
        css_colour(expected.title),
        &format!("{theme} title bar"),
    );
    let navigation = &*window.imp().navigation_row;
    let chrome_colour = painted_colour(navigation, 3, 3);
    assert_same_colour(
        chrome_colour,
        css_colour(expected.chrome),
        &format!("{theme} bars"),
    );
    let sidebar = window.sidebar();
    let sidebar_colour = painted_colour(sidebar, sidebar.width() - 3, sidebar.height() - 3);
    assert_same_colour(
        sidebar_colour,
        css_colour(expected.sidebar),
        &format!("{theme} sidebar"),
    );
    let files = window.folder_pane();
    let content_colour = painted_colour(files, files.width() / 2, files.height() - 3);
    assert_same_colour(
        content_colour,
        css_colour(expected.content),
        &format!("{theme} file list"),
    );
}

/// From top to bottom: the 42-pixel tab strip, the navigation row, the
/// command bar, the sidebar, file list and details pane side by side, and
/// the 30-pixel status bar, all in the app's font stack at 13 pixels.
///
/// parity: LOOK-001
#[gtk::test]
fn the_bars_and_panes_stack_as_in_explorer_in_the_apps_font() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let window = &test.window;
    let title = bounds(&test, &window.titlebar().expect("the tab strip is the title bar"));
    let navigation = bounds(&test, &*window.imp().navigation_row);
    let commands = bounds(&test, window.command_bar());
    let sidebar = bounds(&test, window.sidebar());
    let files = bounds(&test, window.folder_pane());
    let details = bounds(&test, window.details_pane());
    let status = bounds(&test, window.status_bar());
    assert_eq!((title.y, title.height), (0, 42), "the title bar");
    assert_eq!((navigation.y, navigation.height), (42, 62), "the navigation row");
    assert_eq!((commands.y, commands.height), (104, 55), "the command bar");
    for pane in [sidebar, files, details] {
        assert_eq!((pane.y, pane.y + pane.height), (159, status.y), "{pane:?}");
    }
    assert_eq!((sidebar.x, sidebar.width), (0, 210), "the sidebar");
    assert!(sidebar.right() < files.x, "the resizer sits between");
    assert_eq!(files.right(), details.x, "the details pane follows the files");
    assert_eq!(details.right(), window.width(), "and ends at the edge");
    assert_eq!((status.height, status.y + status.height), (30, window.height()));
    let font = window
        .pango_context()
        .font_description()
        .expect("the window names a font");
    assert_eq!(
        font.to_string(),
        "Segoe UI Variable,Segoe UI,Noto Sans,Arial,sans-serif 13px"
    );
}

/// Each appearance paints the title bar, the bars, the sidebar and the
/// file list in its own tokens, and a later switch repaints them all.
///
/// parity: LOOK-002, LOOK-003
#[gtk::test]
fn each_appearance_paints_the_surfaces_in_its_tokens() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for expected in &SURFACES {
        show_theme(&test, expected.theme);
        assert_surfaces(&test.window, expected);
    }
    let light = include_str!("../../../resources/light.css");
    let dark = include_str!("../../../resources/dark.css");
    assert!(
        light.contains("@define-color ox_accent #0067c0;"),
        "the light accent"
    );
    assert!(
        dark.contains("@define-color ox_accent #74beff;"),
        "the dark accent"
    );
}

/// A new window draws its first frame complete and in the chosen
/// appearance: the whole Explorer layout, never a white page or a
/// placeholder waiting for the interface.
///
/// parity: LOOK-007, LOOK-008
#[gtk::test]
fn a_new_window_draws_its_first_frame_complete_in_the_chosen_appearance() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let first = laid_out(&fixture.uri());
    show_theme(&first, "dark");
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 1);
    let window = &test.window;
    let parts: [&gtk::Widget; 6] = [
        window.imp().navigation_row.upcast_ref(),
        window.command_bar().upcast_ref(),
        window.sidebar().upcast_ref(),
        window.folder_pane().upcast_ref(),
        window.details_pane().upcast_ref(),
        window.status_bar().upcast_ref(),
    ];
    for part in parts {
        let name = part.type_().name();
        assert!(part.is_mapped() && part.height() > 0, "{name} is drawn");
    }
    let spinners = descendants::<gtk::Spinner>(window);
    assert!(
        !spinners.iter().any(WidgetExt::is_mapped),
        "no spinner stands in for the layout"
    );
    assert_surfaces(window, &SURFACES[1]);
}

/// The window keeps its client-side frame, with the tab strip as its
/// title bar and no menu bar, also in a second window.
///
/// parity: LOOK-010
#[gtk::test]
fn the_tab_strip_is_the_title_bar_of_a_decorated_window_without_a_menu_bar() {
    let fixture = Fixture::standard();
    let first = laid_out(&fixture.uri());
    let second = first.open_beside(&fixture.uri_of("Documents"));
    for test in [&first, &second] {
        let window = &test.window;
        assert!(window.is_decorated(), "decorations stay on");
        let is_client_side = window.has_css_class("csd") || window.has_css_class("solid-csd");
        assert!(is_client_side, "{:?}", window.css_classes());
        let title_bar = window.titlebar().expect("a title bar");
        assert!(
            title_bar.is::<gtk::WindowHandle>(),
            "dragging it moves the window"
        );
        assert!(window.tab_strip().is_ancestor(&title_bar), "the tabs are in it");
        assert!(!window.shows_menubar(), "no fallback menu bar");
    }
}

/// New windows open at 1320 by 810 and never shrink below 670 by 470
/// (`set_default_size` and `set_size_request` in winspace.py).
///
/// parity: LOOK-011
#[gtk::test]
fn new_windows_open_at_1320_by_810_and_keep_at_least_670_by_470() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert_eq!(test.window.default_size(), (1320, 810));
    assert_eq!(test.window.size_request(), (670, 470));
    let (minimum_width, ..) = test.window.measure(gtk::Orientation::Horizontal, -1);
    let (minimum_height, ..) = test.window.measure(gtk::Orientation::Vertical, -1);
    assert!(minimum_width >= 670 && minimum_height >= 470);
}

/// The navigation row's search box shows its magnifier at the right, and
/// a focused field gets the 2-pixel accent line along its bottom.
///
/// parity: LOOK-012
#[gtk::test]
fn the_search_icon_sits_at_the_right_and_focus_draws_the_accent_line() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    show_theme(&test, "light");
    let search = test.window.search_box();
    let place = bounds(&test, search);
    let icon = descendants::<gtk::Image>(search)
        .into_iter()
        .find(|image| image.has_css_class("search-icon"))
        .expect("the search box shows a magnifier");
    let icon_place = bounds(&test, &icon);
    assert_eq!(
        icon_place.right(),
        place.right() - 12,
        "11 pixels inside the border"
    );
    search.entry().grab_focus();
    wait_for(TRANSITION_TIME);
    wait_for_frames(&test.window, 2);
    let accent = css_colour("#0067c0");
    for y in [place.height - 1, place.height - 2] {
        let colour = painted_colour(search, place.width / 2, y);
        assert_same_colour(colour, accent, "the focus line");
    }
}

/// The command bar is 55 pixels with its bottom line; Appearance,
/// Settings and Details sit at its right end, the file commands scroll
/// sideways instead of clipping, and the switched-on Details toggle shows
/// the selected colour.
///
/// parity: LOOK-013
#[gtk::test]
fn the_command_bar_keeps_its_right_group_and_shows_details_as_selected() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    show_theme(&test, "light");
    let bar = test.window.command_bar();
    let place = bounds(&test, bar);
    assert_eq!(place.height, 55);
    let details = descendants::<gtk::ToggleButton>(bar)
        .into_iter()
        .find(|button| button.has_css_class("details-toggle"))
        .expect("the Details toggle");
    assert_eq!(
        bounds(&test, &details).right(),
        place.right() - 15,
        "at the right end"
    );
    let scroller = descendants::<gtk::ScrolledWindow>(bar)
        .into_iter()
        .next()
        .expect("the commands scroll");
    assert_eq!(scroller.hscrollbar_policy(), gtk::PolicyType::External);
    assert!(details.is_active(), "the details pane is open");
    let colour = painted_colour(&details, 2, details.height() / 2);
    assert_same_colour(colour, css_colour("#e8f1fb"), "the selected colour");
}

/// Classic menus are 264 pixels with 33-pixel rows at 100% text; the
/// compact menu is 276 pixels with the 34-pixel icon strip.
///
/// parity: LOOK-019
#[gtk::test]
fn classic_and_compact_menus_have_the_current_apps_widths_and_rows() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    wait_for_frames(&test.window, 2);
    let contents = menu.child().expect("the menu has contents");
    // The contents box, inside 3 pixels of padding and a 1-pixel border.
    assert_eq!(contents.width() + 2 * (3 + 1), 264, "the classic width");
    let row = menu.rows().into_iter().next().expect("a row");
    assert_eq!(row.height(), 33, "the classic row");
    menu.popdown();

    choose_menu_style(&test, ContextMenu::Win11);
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    wait_for_frames(&test.window, 2);
    assert!(menu.has_css_class("compact"));
    let contents = menu.child().expect("the menu has contents");
    assert_eq!(contents.width() + 2, 276, "the compact width, inside its border");
    let buttons = descendants::<gtk::Button>(&menu);
    let cut = buttons
        .iter()
        .find(|button| button.tooltip_text().as_deref() == Some("Cut"))
        .expect("the strip has Cut");
    assert_eq!(bounds_in(cut, cut).height, 34, "the strip");
    menu.popdown();
}

/// Menus and the SMB sign-in dialog switch with the appearance: the
/// context menu paints the flyout colour, and the sign-in dialog its
/// caption bar and body, in the tokens of each appearance.
///
/// parity: LOOK-003
#[gtk::test]
fn menus_and_the_sign_in_dialog_paint_each_appearance() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let cases = [
        ("light", "#f9f9f9", "#f9f9f9", "#ffffff"),
        ("dark", "#2c2c2c", "#282828", "#202020"),
    ];
    for (theme, flyout, caption, body) in cases {
        show_theme(&test, theme);
        test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
        let menu = test.window.context_menu();
        wait_until("the menu to be drawn", || menu.is_mapped() && menu.width() > 0);
        wait_for_frames(&test.window, 2);
        let contents = menu.child().expect("the menu has contents");
        let place = bounds_in(&contents, &menu);
        // Inside the menu's 3 pixels of padding, left of its rows.
        let menu_colour = painted_colour(&menu, place.x - 2, place.y + place.height / 2);
        assert_same_colour(menu_colour, css_colour(flyout), &format!("{theme} menu"));
        menu.popdown();

        let prompts = test.window.network().prompts().clone();
        let operation = prompts.create("smb://studio-nas/projects").expect("an SMB share");
        let flags = gtk::gio::AskPasswordFlags::NEED_USERNAME | gtk::gio::AskPasswordFlags::NEED_PASSWORD;
        operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
        let sign_in = test.window.network().sign_in().clone();
        wait_until("the sign-in dialog", || sign_in.shown_dialog().is_some());
        let dialog = sign_in.shown_dialog().expect("the sign-in dialog");
        wait_until("the sign-in dialog to be drawn", || {
            dialog.is_mapped() && dialog.width() > 0
        });
        wait_for_frames(&dialog, 3);
        let caption_colour = painted_colour(&dialog, 10, 20);
        assert_same_colour(caption_colour, css_colour(caption), &format!("{theme} caption"));
        // The body's top padding, below the caption bar's 43 pixels and line.
        let body_colour = painted_colour(&dialog, 10, 56);
        assert_same_colour(body_colour, css_colour(body), &format!("{theme} sign-in body"));
        prompts.finish(&operation, ox_core::network::MountOutcome::Failed);
        wait_until("the sign-in dialog to close", || sign_in.shown_dialog().is_none());
    }
}

/// A question is 510 pixels wide with the accent button that Enter
/// presses, and paints the content colour of each appearance.
///
/// parity: LOOK-020
#[gtk::test]
fn a_dialog_is_510_pixels_with_an_accent_button_in_each_appearance() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for (theme, content) in [("light", "#ffffff"), ("dark", "#202020")] {
        show_theme(&test, theme);
        test.activate("new-folder", None);
        let dialog = open_dialog(&test);
        wait_for_frames(&dialog, 3);
        assert_eq!(painted(&dialog).width(), 510, "the dialog's width");
        assert_same_colour(painted_colour(&dialog, 255, 12), css_colour(content), theme);
        let buttons = descendants::<gtk::Button>(&dialog);
        let save = buttons
            .iter()
            .find(|button| button.has_css_class("accent"))
            .expect("Save is the accent button");
        let save_place = bounds_in(save, &dialog);
        assert!(
            save_place.height >= 32 && save_place.width >= 92,
            "{save_place:?}"
        );
        dialog.close();
    }
}

/// The toast sits 78 pixels above the window's bottom, centred, at most
/// 650 pixels wide, in inverted colours, and is announced as a status.
///
/// parity: LOOK-021
#[gtk::test]
fn the_toast_is_an_inverted_status_78_pixels_above_the_bottom() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    // The toast's background is the text colour of each appearance.
    for (theme, inverted) in [("light", "#1b1b1b"), ("dark", "#f1f1f1")] {
        show_theme(&test, theme);
        let message = "Path copied. Sharing permissions are unchanged. ".repeat(6);
        test.window.show_message(&message);
        wait_for_frames(&test.window, 2);
        let label = descendants::<gtk::Label>(&*test.window.imp().toast)
            .into_iter()
            .next()
            .expect("the toast shows its message");
        assert_eq!(label.accessible_role(), gtk::AccessibleRole::Status);
        // The rounded surface holds the message and the Undo button.
        let surface = label.parent().expect("the toast's surface");
        let place = bounds(&test, &surface);
        assert_eq!(test.window.height() - place.y - place.height, 78);
        // The 80-character limit is 650 pixels in the window's own fonts.
        if window_font(&surface).is_ok() {
            assert!(place.width <= 650, "{place:?}");
        }
        let centre = place.x + place.width / 2;
        assert!((centre - test.window.width() / 2).abs() <= 1, "{place:?}");
        let background = painted_colour(&surface, 4, place.height / 2);
        assert_same_colour(background, css_colour(inverted), &format!("{theme} toast"));
    }
}
