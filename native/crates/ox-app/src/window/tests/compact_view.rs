// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's Compact view in a real window: off by default, View >
//! Compact view draws the details rows and the sidebar's rows closer and
//! saves the choice, and a choice saved elsewhere (Settings or another
//! window) is taken up at once (VIEW-067).

use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::geometry::laid_out;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// Whether the window's Compact view toggle is checked.
fn toggle_is_on(test: &TestWindow) -> Option<bool> {
    test.window.lookup_action("compact-view")?.state()?.get::<bool>()
}

/// The height of the first row of the details list.
fn details_row_height(test: &TestWindow) -> i32 {
    let view = test.window.folder_pane().details().column_view();
    let row = descendants::<gtk::Widget>(view)
        .into_iter()
        .find(|widget| {
            widget.css_name() == "row"
                && widget
                    .parent()
                    .is_some_and(|parent| parent.css_name() == "listview")
        })
        .expect("the details list has a row");
    row.height()
}

/// The height of the first row of the sidebar's lists.
fn sidebar_row_height(test: &TestWindow) -> i32 {
    let sidebar = descendants::<gtk::Widget>(&test.window)
        .into_iter()
        .find(|widget| widget.has_css_class("sidebar"))
        .expect("the window has a sidebar");
    let row = descendants::<gtk::ListBoxRow>(&sidebar)
        .into_iter()
        .find(|row| row.is_visible() && row.height() > 0)
        .expect("the sidebar has a row");
    row.height()
}

/// View > Compact view draws the details rows and the sidebar's rows
/// closer, 24 and 26 pixels at 100% text, and saves the choice; turning
/// it off brings the usual heights back.
///
/// parity: VIEW-067
#[gtk::test]
fn compact_view_draws_closer_rows_and_is_saved() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    assert!(!test.window.shows_compact_view(), "off by default");
    assert_eq!(toggle_is_on(&test), Some(false));
    let usual_row = details_row_height(&test);
    let usual_side = sidebar_row_height(&test);

    test.activate("compact-view", None);
    wait_for_frames(&test.window, 3);
    assert!(test.window.shows_compact_view());
    wait_until("the choice to be saved", || {
        test.context.settings_data().preferences.compact_view
    });
    let compact_row = details_row_height(&test);
    let compact_side = sidebar_row_height(&test);
    assert_eq!(compact_row, 24, "details rows, from {usual_row}");
    assert_eq!(compact_side, 26, "sidebar rows, from {usual_side}");
    assert!(compact_row < usual_row && compact_side < usual_side);

    test.activate("compact-view", None);
    wait_for_frames(&test.window, 3);
    assert!(!test.window.shows_compact_view());
    wait_until("the choice to be saved", || {
        !test.context.settings_data().preferences.compact_view
    });
    assert_eq!(details_row_height(&test), usual_row);
    assert_eq!(sidebar_row_height(&test), usual_side);
}

/// Compact view turned on in Settings, or by another window, reaches an
/// open window at once: its toggle checks and its rows close up.
///
/// parity: VIEW-067
#[gtk::test]
fn a_compact_view_saved_elsewhere_is_taken_up_at_once() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let usual_row = details_row_height(&test);

    let on = PreferencesUpdate {
        compact_view: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&on)
        .expect("the settings file takes the change");
    test.context.reload_settings();
    wait_until("the window to follow", || test.window.shows_compact_view());
    assert_eq!(toggle_is_on(&test), Some(true));
    wait_for_frames(&test.window, 3);
    assert!(details_row_height(&test) < usual_row);
}
