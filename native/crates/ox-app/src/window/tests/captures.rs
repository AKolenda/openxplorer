// SPDX-License-Identifier: AGPL-3.0-only
//! Screenshots of the window for visual review.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set (passed through the isolated check
//! environment), these save `native-browsing-light.png`,
//! `native-browsing-narrow.png`, `native-browsing-dark.png`,
//! `native-tabs-3-light.png`, `native-tabs-3-dark.png`,
//! `native-this-pc.png`, `native-network.png`, `native-menu-new-*.png`,
//! `native-menu-context-*.png`, `native-states-*.png` and
//! `group-collapse-{details,tiles,content}-{selected,collapsed,menu}-{light,dark}.png`
//! there. Without
//! it, they only prove the window lays out in both themes and on both
//! pages.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use super::support::{app_menu, menu_button_with_class};
use crate::locations::Page;
use crate::test_support::harness::{
    capture, capture_popover, descendants, wait_for, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::widget_tree::children;

/// Longer than the skin's 83 ms colour transitions (ui-spec.md M01).
const TRANSITION_TIME: Duration = Duration::from_millis(150);

/// The theme keys, as `win.theme` takes them.
const THEMES: [&str; 2] = ["light", "dark"];

#[gtk::test]
fn the_window_is_captured_light_narrow_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(1);
    test.activate("theme", Some("light"));
    capture(&test.window, "native-browsing-light.png");
    test.window.set_default_size(1000, 720);
    capture(&test.window, "native-browsing-narrow.png");
    test.window.set_default_size(1320, 810);
    test.activate("theme", Some("dark"));
    capture(&test.window, "native-browsing-dark.png");
}

/// Three tabs with the middle one in front, as the reference
/// `current-*-tabs-3.png` shows them.
#[gtk::test]
fn three_tabs_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    let middle = test.active_tab().expect("a tab in front");
    test.window.add_tab(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    test.activate_tab(middle);
    test.wait_for_listing("the middle tab");
    test.activate("theme", Some("light"));
    capture(&test.window, "native-tabs-3-light.png");
    test.activate("theme", Some("dark"));
    capture(&test.window, "native-tabs-3-dark.png");
}

/// Presses `button` once at the middle of `widget` and lets it go, as far
/// as the visible view's own gestures for that button go.
fn click_on(test: &TestWindow, widget: &gtk::Widget, button: u32) {
    let view = test.window.folder_pane().view_widget();
    #[expect(clippy::cast_precision_loss, reason = "widget sizes are small")]
    let middle = gtk::graphene::Point::new(widget.width() as f32 / 2.0, widget.height() as f32 / 2.0);
    let point = widget
        .compute_point(&view, &middle)
        .expect("the widget is in the view");
    let (x, y) = (f64::from(point.x()), f64::from(point.y()));
    let controllers = view.observe_controllers();
    let gestures: Vec<gtk::GestureClick> = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .filter(|gesture| gesture.button() == button)
        .collect();
    for gesture in &gestures {
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
    }
    for gesture in &gestures {
        gesture.emit_by_name::<()>("released", &[&1_i32, &x, &y]);
    }
}

/// The title of the shown heading whose group is titled `title`.
fn heading_title(test: &TestWindow, title: &str) -> gtk::Widget {
    let view = test.window.folder_pane().view_widget();
    descendants::<gtk::Label>(&view)
        .into_iter()
        .filter(|label| label.has_css_class("group-title") && label.is_mapped())
        .find(|label| label.text() == title)
        .expect("the heading is on screen")
        .upcast()
}

/// Details, Tiles and Content of a folder grouped by type, in both
/// themes: a heading clicked (its group's items selected, the heading
/// outlined), that group collapsed with its items still selected (the
/// heading filled), and the heading's right-click menu. The pointer's
/// hover needs a real pointer, so it is checked in a real session.
///
/// parity: VIEW-022, VIEW-071
#[gtk::test]
fn grouped_views_with_a_collapsed_group_are_captured() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::empty();
    std::fs::create_dir(fixture.path("Photos")).expect("a fixture folder");
    for name in [
        "Budget 2026.ods",
        "Launch plan.md",
        "Meeting notes.txt",
        "Packing list.txt",
        "Roadmap.txt",
        "Brochure draft.pdf",
        "Release notes.md",
    ] {
        fixture.write(name);
    }
    let test = TestWindow::open(&fixture.uri());
    test.window.set_default_size(1200, 760);
    test.activate("group-by", Some("type"));
    for view in ["details", "tiles", "content"] {
        test.activate("view", Some(view));
        wait_for(TRANSITION_TIME);
        let grouped = test.window.folder_pane().grouped().expect("a grouped view");
        let model = test.window.folder_pane().model();
        let launch = model
            .position_of_uri(&fixture.uri_of("Launch plan.md"))
            .expect("Launch plan is listed");
        let layout = grouped.layout().expect("laid out");
        let index = layout.group_of(launch).expect("in a group");
        let markdown = layout.groups()[index].title.clone();
        let heading = grouped.heading(index).expect("a heading");
        let notes: Vec<u32> = (layout.groups()[index].start..layout.groups()[index].end).collect();
        for theme in THEMES {
            test.activate("theme", Some(theme));
            model.select_none();
            click_on(&test, &heading_title(&test, &markdown), gtk::gdk::BUTTON_PRIMARY);
            assert_eq!(
                model.selected_positions(),
                notes,
                "{view}: the title selects the group"
            );
            wait_for(TRANSITION_TIME);
            capture(
                &test.window,
                &format!("group-collapse-{view}-selected-{theme}.png"),
            );

            grouped.set_collapsed(&markdown, true);
            test.window.keep_heading_focus(&heading);
            assert!(!grouped.layout().expect("laid out").shows(launch), "{view}");
            assert_eq!(
                model.selected_positions(),
                notes,
                "{view}: hidden items stay selected"
            );
            wait_for(TRANSITION_TIME);
            capture(
                &test.window,
                &format!("group-collapse-{view}-collapsed-{theme}.png"),
            );
            grouped.set_collapsed(&markdown, false);

            click_on(
                &test,
                &heading_title(&test, &markdown),
                gtk::gdk::BUTTON_SECONDARY,
            );
            let menu = test.window.context_menu();
            wait_until("the heading menu to open", || menu.is_visible());
            capture_popover(
                &test.window,
                menu.upcast_ref(),
                &format!("group-collapse-{view}-menu-{theme}.png"),
            );
            menu.popdown();
        }
    }
}

#[gtk::test]
fn the_landing_pages_are_captured() {
    let _theme = ThemeGuard::keep();
    let test = TestWindow::open(Page::ThisPc.uri());
    test.activate("theme", Some("light"));
    capture(&test.window, "native-this-pc.png");
    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    test.wait_for_listing("the Network page");
    capture(&test.window, "native-network.png");
}

/// The New menu and the file list's context menu, in both themes.
#[gtk::test]
fn the_menus_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let new_button = menu_button_with_class(&test, "new-command");
    let new_menu = app_menu(&new_button);
    for theme in THEMES {
        test.activate("theme", Some(theme));
        new_button.popup();
        capture_popover(
            &test.window,
            new_menu.upcast_ref(),
            &format!("native-menu-new-{theme}.png"),
        );
        new_button.popdown();
        wait_until("the New menu to close", || !new_menu.is_mapped());
        test.window.right_click(Some(0));
        let context_menu = test.window.context_menu();
        capture_popover(
            &test.window,
            context_menu.upcast_ref(),
            &format!("native-menu-context-{theme}.png"),
        );
        context_menu.popdown();
        wait_until("the context menu to close", || !context_menu.is_mapped());
    }
}

/// The command bar's "…" in a folder, with Undo and Redo both to run, and
/// in This PC, and View with its Show submenu, in both themes, to compare
/// with Windows 11's (CMD-006, CMD-007).
#[gtk::test]
fn the_more_view_and_sort_menus_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for _ in 0..2 {
        test.activate("new-folder", None);
        super::file_ops_support::name_editor(&test).emit_activate();
        wait_until("the name to be kept", || {
            !super::file_ops_support::is_renaming_in_place(&test)
        });
    }
    test.activate("undo", None);
    let more_button = menu_button_with_class(&test, "more-command");
    let more = app_menu(&more_button);
    wait_until("… to offer Undo and Redo", || {
        let labels = more.row_labels();
        labels.contains(&"Undo".to_owned()) && labels.contains(&"Redo".to_owned())
    });
    let view_button = menu_button_with_class(&test, "view-command");
    let view = app_menu(&view_button);
    let sort_button = menu_button_with_class(&test, "sort-command");
    let sort = app_menu(&sort_button);
    test.activate("sort", Some("modified"));
    test.activate("direction", Some("descending"));
    for theme in THEMES {
        test.activate("theme", Some(theme));
        test.window.navigate(&fixture.uri()).expect("the folder");
        test.wait_for_listing("the folder");
        for (page, name) in [(None, "folder"), (Some(Page::ThisPc), "this-pc")] {
            if let Some(page) = page {
                test.window.navigate(page.uri()).expect("a page");
            }
            more_button.popup();
            capture_popover(
                &test.window,
                more.upcast_ref(),
                &format!("native-menu-more-{name}-{theme}.png"),
            );
            more_button.popdown();
            wait_until("… to close", || !more.is_mapped());
        }
        test.window.navigate(&fixture.uri()).expect("the folder");
        test.wait_for_listing("the folder");
        view_button.popup();
        capture_popover(
            &test.window,
            view.upcast_ref(),
            &format!("native-menu-view-{theme}.png"),
        );
        let show = view.open_submenu_of("Show");
        capture_popover(
            &test.window,
            show.upcast_ref(),
            &format!("native-menu-view-show-{theme}.png"),
        );
        view_button.popdown();
        wait_until("View to close", || !view.is_mapped());
        // Sort, as Windows 11 draws it: no glyphs, dots on the choices,
        // and Type under the pointer.
        sort_button.popup();
        let hovered = sort.row("Type");
        hovered.set_state_flags(gtk::StateFlags::PRELIGHT, false);
        wait_for(TRANSITION_TIME);
        capture_popover(
            &test.window,
            sort.upcast_ref(),
            &format!("native-menu-sort-{theme}.png"),
        );
        hovered.unset_state_flags(gtk::StateFlags::PRELIGHT);
        sort_button.popdown();
        wait_until("Sort to close", || !sort.is_mapped());
    }
}

/// Hover on a caption button, "+", an inactive tab and a file row, with
/// Sort pressed and a row selected, in both themes. (A list box manages its
/// rows' hover itself, so the sidebar cannot be hovered this way.)
#[gtk::test]
fn hover_and_pressed_states_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let folder_tab = test.active_tab().expect("a tab in front");
    test.window.add_tab(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    test.activate_tab(folder_tab);
    test.wait_for_listing("the folder tab");
    test.window.folder_model().select_only(1);
    let sort = menu_button_with_class(&test, "sort-command");
    let sort_button = sort.first_child().expect("a menu button holds a button");
    sort_button.set_state_flags(gtk::StateFlags::ACTIVE, false);
    for theme in THEMES {
        test.activate("theme", Some(theme));
        // Hovered again for each theme, as a user would.
        for widget in hovered_widgets(&test) {
            widget.set_state_flags(gtk::StateFlags::PRELIGHT, false);
        }
        wait_for(TRANSITION_TIME);
        capture(&test.window, &format!("native-states-{theme}.png"));
    }
}

/// One widget of each kind the states capture hovers.
fn hovered_widgets(test: &TestWindow) -> Vec<gtk::Widget> {
    let buttons = descendants::<gtk::Button>(&test.window);
    let minimize = buttons.iter().find(|button| button.has_css_class("minimize"));
    let new_tab = buttons
        .iter()
        .find(|button| button.action_name().as_deref() == Some("win.new-tab"));
    let tab_list = test.window.tab_strip().tab_list();
    let inactive_tab = children(&tab_list).find(|tab| !tab.has_css_class("active"));
    let file_row = descendants::<gtk::Widget>(&test.window.folder_pane().view_widget())
        .into_iter()
        .filter(|widget| widget.css_name() == "row")
        .nth(2);
    [
        minimize.map(|button| button.clone().upcast()),
        new_tab.map(|button| button.clone().upcast()),
        inactive_tab,
        file_row,
    ]
    .into_iter()
    .flatten()
    .collect()
}
