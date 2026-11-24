// SPDX-License-Identifier: AGPL-3.0-only
//! "Column header style" (VIEW-014): the row saves the choice to
//! `settings.json`, the app's own style unstored as the default, and the
//! window draws its column titles in the saved style.

use ox_core::settings::ColumnHeaderStyle;

use super::{choices_of, SettingsTest};
use crate::test_support::harness::wait_until;

/// The row offers the app's own style first, as the default, and saves
/// Windows' as `columnHeaderStyle: "windows"`, which the window follows;
/// the app's own style again removes the key.
///
/// parity: VIEW-014
#[gtk::test]
fn the_column_header_style_row_saves_the_choice() {
    let settings = SettingsTest::open();
    let stored = || {
        std::fs::read_to_string(settings.test.settings_directory().join("settings.json")).unwrap_or_default()
    };
    let row = settings.row("Column header style");
    assert_eq!(
        settings.saved_preferences().column_header_style,
        ColumnHeaderStyle::OpenXplorer
    );

    choices_of(&row).choose_labelled("Windows");
    wait_until("Windows' style to be saved", || {
        settings.saved_preferences().column_header_style == ColumnHeaderStyle::Windows
    });
    assert!(
        stored().contains("\"columnHeaderStyle\": \"windows\""),
        "{}",
        stored()
    );
    wait_until("the window to draw Windows' titles", || {
        settings.test.window.column_header_style() == ColumnHeaderStyle::Windows
    });

    choices_of(&row).choose_labelled("OpenXplorer (default)");
    wait_until("the app's own style to be saved", || {
        settings.saved_preferences().column_header_style == ColumnHeaderStyle::OpenXplorer
    });
    assert!(
        !stored().contains("columnHeaderStyle"),
        "the default is not stored"
    );
    wait_until("the window to draw its own titles", || {
        settings.test.window.column_header_style() == ColumnHeaderStyle::OpenXplorer
    });
}
