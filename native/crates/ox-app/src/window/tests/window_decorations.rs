// SPDX-License-Identifier: AGPL-3.0-only
//! System-frame layout, tab gestures, and drop geometry on a private display.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};

/// Saves the opt-in before a window is constructed, as on the next launch.
fn open_system_window(fixture: &Fixture) -> TestWindow {
    TestWindow::open_prepared(&fixture.uri(), |context| {
        context.update_preferences(
            PreferencesUpdate {
                system_window_decorations: Some(true),
                ..PreferencesUpdate::default()
            },
            |result| result.expect("fixture settings are writable"),
        );
        wait_until("system decorations to be read", || {
            context.settings_data().preferences.system_window_decorations
        });
    })
}

#[gtk::test]
fn system_window_decorations_keep_tabs_in_content_without_duplicate_buttons() {
    let fixture = Fixture::standard();
    let test = open_system_window(&fixture);
    let imp = test.window.imp();
    assert_eq!(imp.content_box.first_child(), Some(imp.tab_bar.get().upcast()));
    assert!(!imp.start_caption_buttons.is_visible());
    assert!(!imp.end_caption_buttons.is_visible());
    assert!(test.window.is_decorated());
    assert!(!imp.tab_bar.has_css_class("titlebar"));
    let controllers = imp.tab_bar.observe_controllers();
    assert!(controllers
        .iter::<gtk::glib::Object>()
        .filter_map(Result::ok)
        .any(|controller| controller.is::<gtk::DragSource>()));
    assert!(
        Settings::open(test.settings_directory())
            .data()
            .preferences
            .system_window_decorations
    );
    test.activate("new-tab", None);
    assert_eq!(test.window.tab_count(), 2);
    test.activate("settings", None);
    assert!(test.window.shows_settings());
    assert!(imp.tab_bar.is_visible());
}

#[gtk::test]
fn system_window_decorations_drop_geometry_tracks_the_content_tab_row() {
    let fixture = Fixture::standard();
    let test = open_system_window(&fixture);
    wait_for_frames(&test.window, 3);
    let bar = test
        .window
        .imp()
        .tab_bar
        .compute_bounds(&test.window)
        .expect("mapped tab row");
    let strip = test
        .window
        .tab_strip()
        .compute_bounds(&test.window)
        .expect("mapped strip");
    let x = strip.x() + strip.width() / 2.0;
    assert!(test
        .window
        .is_in_tab_row(gtk::graphene::Point::new(x, bar.y() + bar.height() / 2.0)));
    assert!(!test
        .window
        .is_in_tab_row(gtk::graphene::Point::new(x, bar.y() - 1.0)));
    assert!(!test
        .window
        .is_in_tab_row(gtk::graphene::Point::new(x, bar.y() + bar.height() + 1.0)));
}

#[gtk::test]
fn system_window_decorations_preserve_real_pointer_tab_reordering() {
    use crate::test_support::pointer::{screen_point, Button, Pointer};
    use crate::window::widget_tree::children;

    let fixture = Fixture::standard();
    let test = open_system_window(&fixture);
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a fixture folder");
    test.wait_for_listing("the second tab");
    wait_for_frames(&test.window, 3);
    let ids: Vec<_> = test
        .window
        .imp()
        .session
        .borrow()
        .tabs()
        .iter()
        .map(|tab| tab.id)
        .collect();
    let tabs: Vec<_> = children(&test.window.tab_strip().tab_list()).collect();
    let mut pointer = Pointer::connect();
    let from = screen_point(&mut pointer, &tabs[0], (60.0, 18.0), (8, 8));
    let to = screen_point(&mut pointer, &tabs[1], (150.0, 18.0), (8, 8));
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(200));
    pointer.glide(from, to, 12);
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(200));
    pointer.release(Button::Primary);
    wait_until("the system-frame tabs to reorder", || {
        test.window.imp().session.borrow().tabs()[0].id == ids[1]
    });
    assert_eq!(test.window.imp().session.borrow().tabs()[1].id, ids[0]);
}
