// SPDX-License-Identifier: AGPL-3.0-only
//! "Behave like Windows Explorer": the switch first on the General page
//! sets every setting of the registry at once, each setting's own row
//! shows the change, a row changed on its own marks the switch Custom,
//! and turning it off puts the earlier settings back.

use gtk::prelude::*;
use ox_core::settings::{windows_mode, Preferences, WindowsMode, WINDOWS_SETTINGS};

use super::{switch_of, SettingsTest};
use crate::settings_page::choice_list::ChoiceList;
use crate::settings_page::row::SettingRow;
use crate::settings_page::windows_mode::BEHAVE_LIKE_WINDOWS;
use crate::test_support::harness::wait_until;

/// What a row's control shows: a switch's state, or a drop-down's choice.
fn shown_value(row: &SettingRow) -> String {
    let control = row.controls().into_iter().next().expect("the row has a control");
    if let Some(switch) = control.downcast_ref::<gtk::Switch>() {
        return switch.is_active().to_string();
    }
    let button = control
        .downcast::<gtk::MenuButton>()
        .expect("a switch or a drop-down");
    let list = button
        .popover()
        .and_downcast::<ChoiceList>()
        .expect("a drop-down");
    list.chosen_label()
}

/// What every row of the registry shows, in its order.
fn shown_values(settings: &SettingsTest) -> Vec<String> {
    let rows = WINDOWS_SETTINGS.iter().map(|setting| settings.row(setting.title));
    rows.map(|row| shown_value(&row)).collect()
}

/// Whether the master row shows "Custom" before its switch.
fn shows_custom(row: &SettingRow) -> bool {
    let labels = row
        .controls()
        .into_iter()
        .filter_map(|control| control.downcast::<gtk::Label>().ok());
    labels
        .into_iter()
        .any(|label| label.is_visible() && label.text() == "Custom")
}

/// Off → on: every setting is saved at its Windows value and each row
/// shows it. A row changed on its own then marks the switch Custom, still
/// on. Off puts every setting back as it was before, that one too, and
/// the rows show it.
///
/// parity: SET-020
#[gtk::test]
fn on_sets_every_row_custom_follows_a_change_and_off_restores() {
    let settings = SettingsTest::open();
    let master = settings.row(BEHAVE_LIKE_WINDOWS.title);
    let switch = switch_of(&master);
    let before: Preferences = settings.saved_preferences();
    let shown_before = shown_values(&settings);

    switch.set_active(true);
    wait_until("every Windows value to be saved", || {
        windows_mode(&settings.saved_preferences()) == WindowsMode::On
    });
    wait_until("every row to show its Windows value", || {
        shown_values(&settings)
            .iter()
            .zip(&shown_before)
            .all(|(now, then)| now != then)
    });
    assert!(!shows_custom(&master));
    let stored = std::fs::read_to_string(settings.test.settings_directory().join("settings.json")).unwrap();
    assert!(stored.contains("\"behaveLikeWindows\""), "{stored}");

    let paste = switch_of(&settings.row("Paste pictures as new files"));
    assert!(!paste.is_active(), "Windows has no Paste for a picture");
    paste.set_active(true);
    wait_until("the switch to read Custom", || {
        windows_mode(&settings.saved_preferences()) == WindowsMode::Custom && shows_custom(&master)
    });
    assert!(switch.is_active(), "Custom stays on");
    assert!(switch.has_css_class("mixed"));
    assert_eq!(
        master.shown_description(),
        "Custom: 1 setting was changed since this was turned on."
    );

    switch.set_active(false);
    wait_until("the earlier settings to be saved", || {
        let saved = settings.saved_preferences();
        saved.windows_mode_before.is_none()
            && WINDOWS_SETTINGS
                .iter()
                .all(|setting| (setting.read)(&saved) == (setting.read)(&before))
    });
    wait_until("every row to show its earlier value", || {
        shown_values(&settings) == shown_before
    });
    assert!(!shows_custom(&master));
    assert!(!switch.has_css_class("mixed"));
    let stored = std::fs::read_to_string(settings.test.settings_directory().join("settings.json")).unwrap();
    assert!(!stored.contains("behaveLikeWindows"), "{stored}");
}
