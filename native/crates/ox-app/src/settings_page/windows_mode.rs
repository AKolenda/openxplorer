// SPDX-License-Identifier: AGPL-3.0-only
//! "Behave like Windows Explorer", first on the General page: one switch
//! that sets every setting of [`ox_core::settings::WINDOWS_SETTINGS`] to the value that works
//! as Windows 11 File Explorer does, and turned off puts the user's
//! earlier values back ([`ox_core::settings::windows_mode`]).
//!
//! It changes no behavior by itself; each of its settings keeps its own
//! row, which shows the change at once. A setting changed on its own row
//! while the switch is on leaves the switch on, marked "Custom", as
//! Windows marks a partly ticked box, and the line under the title says
//! how many settings differ.

use gtk::glib;
use gtk::prelude::*;
use ox_core::i18n::{gettext, ngettext};
use ox_core::settings::{differing, turn_off, turn_on, windows_mode, Preferences, WindowsMode};

use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;

/// The row's text. Its description is the ⓘ's; the line under the title
/// is [`explanation`].
pub(super) const BEHAVE_LIKE_WINDOWS: RowText = RowText {
    title: "Behave like Windows Explorer",
    description: "Sets every setting that has a Windows 11 File Explorer behavior to it, all at once. \
                  Each setting still has its own switch. Turning this off puts back the settings you \
                  had before you turned it on.",
    keywords: "windows explorer mode like windows behave all settings at once master switch file \
               explorer 11 defaults compatibility",
};

/// The line under the title while the switch is off or fully on.
const EXPLANATION: &str =
    crate::i18n::message_id("Uses Windows 11 File Explorer's behavior wherever OpenXplorer offers a choice.");

/// The CSS class of the switch while it reads Custom.
const MIXED: &str = "mixed";

/// The line under the title for `preferences`: the plain explanation, or
/// how many settings differ while the switch reads Custom.
pub(super) fn explanation(preferences: &Preferences) -> String {
    match windows_mode(preferences) {
        WindowsMode::Off | WindowsMode::On => gettext(EXPLANATION),
        WindowsMode::Custom => {
            let count = differing(preferences);
            ngettext(
                "Custom: {count} setting was changed since this was turned on.",
                "Custom: {count} settings were changed since this was turned on.",
                count as u64,
            )
            .replace("{count}", &count.to_string())
        }
    }
}

/// The group with the one row, put first on the General page.
pub(super) fn group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&gettext("Windows Explorer"));
    group.add_row(&row(page));
    group
}

/// The row: its switch, "Custom" before it while a setting differs, and
/// the line under its title.
fn row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(BEHAVE_LIKE_WINDOWS);
    let details = gettext(BEHAVE_LIKE_WINDOWS.description);
    let custom = parts::value_label(&gettext("Custom"));
    custom.set_visible(false);
    row.add_control(&custom, ControlName::OwnLabel);
    let switch = parts::switch();
    row.add_control(&switch, ControlName::RowTitle);
    page.follow_preferences(glib::clone!(
        #[weak]
        row,
        #[weak]
        switch,
        #[weak]
        custom,
        #[strong]
        details,
        move |preferences| {
            let mode = windows_mode(preferences);
            switch.set_active(mode != WindowsMode::Off);
            let mixed = mode == WindowsMode::Custom;
            custom.set_visible(mixed);
            if mixed {
                switch.add_css_class(MIXED);
            } else {
                switch.remove_css_class(MIXED);
            }
            let line = explanation(preferences);
            row.set_description(&line);
            let description = format!("{line} {details}");
            switch.update_property(&[gtk::accessible::Property::Description(&description)]);
        }
    ));
    switch.connect_active_notify(glib::clone!(
        #[weak]
        page,
        move |switch| {
            if !page.is_user_change() {
                return;
            }
            let preferences = page.context().settings_data().preferences;
            let update = if switch.is_active() {
                turn_on(&preferences)
            } else {
                turn_off(&preferences)
            };
            page.save_preferences(update);
        }
    ));
    row
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ox_core::settings::WINDOWS_SETTINGS;

    /// Settings whose titles name Windows but that the switch leaves
    /// alone, each with why. Any other title with the word "Windows", in
    /// any letter case, must have its line in [`WINDOWS_SETTINGS`].
    #[rustfmt::skip]
    const NOT_SET_BY_THE_SWITCH: [(&str, &str); 9] = [
        ("Behave like Windows Explorer", "the switch itself"),
        ("Show letters on mapped network folders", "deliberately not part of the switch"),
        ("Icons from Windows icon files", "needs icon files the user supplies; names Windows as a place"),
        ("Windows icon files", "names Windows only as where the icon files come from"),
        ("Windows symbol font", "names Windows only as where the font comes from"),
        ("Open new windows in split view", "the app's own windows, not Microsoft Windows"),
        ("Open windows", "the app's own windows, not Microsoft Windows"),
        ("Type addresses instead of breadcrumbs in new windows", "the app's own windows, not Microsoft Windows"),
        ("Move tabs between windows", "the app's own windows, not Microsoft Windows"),
    ];

    /// Every setting row title of the Rust files under `folder`, with the
    /// file it is in: the one-line `title: "…"` of each `RowText`.
    fn row_titles(folder: &Path, found: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(folder)
            .expect("the sources")
            .map(|entry| entry.expect("a source entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                row_titles(&path, found);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            for line in text.lines() {
                let Some(rest) = line.trim_start().strip_prefix("title: \"") else {
                    continue;
                };
                if let Some(end) = rest.find('"') {
                    found.push((rest[..end].to_owned(), path.display().to_string()));
                }
            }
        }
    }

    /// Whether `title` has the word "Windows", in any letter case.
    fn names_windows(title: &str) -> bool {
        let words = title.split(|c: char| !c.is_alphanumeric());
        words.into_iter().any(|word| word.eq_ignore_ascii_case("windows"))
    }

    /// The guard: a setting whose title names Windows, in any letter
    /// case, must have its line in [`WINDOWS_SETTINGS`], so "Behave like
    /// Windows Explorer" sets it too, or be one of the listed exceptions.
    /// Every exception and every line names a row that exists.
    ///
    /// parity: SET-020
    #[test]
    fn every_setting_titled_like_windows_is_in_the_registry() {
        let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        row_titles(&sources, &mut found);
        let registered = |title: &str| WINDOWS_SETTINGS.iter().any(|setting| setting.title == title);
        let excepted = |title: &str| NOT_SET_BY_THE_SWITCH.iter().any(|(name, _)| *name == title);
        let named: Vec<_> = found.iter().filter(|(title, _)| names_windows(title)).collect();
        assert!(
            named.iter().any(|(title, _)| title == "F6 moves like Windows"),
            "the scan finds the titles that are there"
        );
        let missing: Vec<_> = named
            .iter()
            .filter(|(title, _)| !registered(title) && !excepted(title))
            .collect();
        assert!(
            missing.is_empty(),
            "add a line to WINDOWS_SETTINGS (ox-core settings/windows_mode.rs), or an exception with \
             its reason here, for: {missing:?}"
        );
        let titles: Vec<&str> = found.iter().map(|(title, _)| title.as_str()).collect();
        let stale: Vec<&str> = NOT_SET_BY_THE_SWITCH
            .iter()
            .map(|(name, _)| *name)
            .chain(WINDOWS_SETTINGS.iter().map(|setting| setting.title))
            .filter(|name| !titles.contains(name))
            .collect();
        assert!(stale.is_empty(), "no row is titled: {stale:?}");
    }

    /// The word must stand alone: "Windows'" and "windows" count, a
    /// longer word does not.
    #[test]
    fn the_guard_matches_the_word_windows_in_any_case() {
        assert!(names_windows("F6 moves like Windows"));
        assert!(names_windows("Name column starts at Windows' width (256 px)"));
        assert!(names_windows("Open windows"));
        assert!(!names_windows("Folders from other apps open in a new window"));
        assert!(!names_windows("Windowsill"));
    }
}
