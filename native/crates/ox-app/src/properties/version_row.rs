// SPDX-License-Identifier: AGPL-3.0-only
//! One row of the Previous Versions list (PROP-019, PROP-020), as Windows
//! 11 lists a version: the item's icon and name under Name, and under Date
//! modified the date and time read from the snapshot's name (never the
//! snapshot folder's own time), with a tooltip naming the snapshot, where
//! it came from and where its date comes from. Rows are grouped under
//! Windows' headings: Today, Yesterday, Earlier this week, Last week,
//! Earlier this month, Last month, Earlier this year and A long time ago.

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::LocationContext;
use ox_core::versions::{PreviousVersion, SnapshotDate, DATE_UNAVAILABLE, NO_DATE_EXPLANATION};

use crate::icons::{Art, ArtImage};

/// The size of a version's icon.
const ROW_ART_SIZE: i32 = 16;

/// A snapshot folder to browse in a tab: where it is, the snapshot's own
/// folder that the tab's badge covers, and the snapshot's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotTarget {
    /// The folder to open, inside the snapshot.
    pub uri: String,
    /// The snapshot's own folder (`version.snapshotRoot || version.uri`).
    pub root: String,
    /// The snapshot's name, which carries its date.
    pub label: String,
}

impl SnapshotTarget {
    /// The target of the Browse button of `version`.
    pub(crate) fn of_version(version: &PreviousVersion) -> Self {
        let root = if version.snapshot_root.is_empty() {
            version.entry.uri.clone()
        } else {
            version.snapshot_root.clone()
        };
        Self {
            uri: version.entry.uri.clone(),
            root,
            label: version.label.clone(),
        }
    }

    /// The target as a window action's parameter, `(sss)`.
    pub(crate) fn to_variant(&self) -> glib::Variant {
        (self.uri.as_str(), self.root.as_str(), self.label.as_str()).to_variant()
    }

    /// The target in a window action's parameter.
    pub(crate) fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let (uri, root, label) = variant.get::<(String, String, String)>()?;
        Some(Self { uri, root, label })
    }
}

/// The date as a medium date in the user's language, such as `5 Sep
/// 2026`. The web UI used the browser's `Intl` medium format; `GLib` has no
/// CLDR formats, so the day, the locale's abbreviated month and the year
/// are written in that order.
pub(super) fn medium_date(date: &SnapshotDate) -> String {
    let day = glib::DateTime::from_utc(
        i32::from(date.year),
        i32::from(date.month),
        i32::from(date.day),
        0,
        0,
        0.0,
    );
    let formatted = day.and_then(|day| day.format("%-d %b %Y"));
    formatted.map_or_else(|_| date.date_text(), String::from)
}

/// The row showing `version` of the item called `name`, whose collection
/// is named through `locations`.
pub(super) fn version_row(version: &PreviousVersion, name: &str, locations: &LocationContext) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["version-row"])
        .build();
    let cell = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    cell.set_hexpand(true);
    cell.append(&ArtImage::new(Art::for_entry(&version.entry), ROW_ART_SIZE));
    cell.append(&ellipsized(name));
    row.append(&cell);
    let date = version.date();
    let date_text = date.as_ref().map_or_else(
        || ox_core::i18n::gettext_static(DATE_UNAVAILABLE).to_owned(),
        date_modified_text,
    );
    let when = ellipsized(&date_text);
    when.add_css_class("version-date");
    row.append(&when);
    let source = locations.display_location(&version.collection);
    let explanation = date.as_ref().map_or(
        ox_core::i18n::gettext_static(NO_DATE_EXPLANATION),
        SnapshotDate::explanation,
    );
    row.set_tooltip_text(Some(&format!("{}\n{source}\n{explanation}", version.label)));
    row.update_property(&[
        gtk::accessible::Property::Label(&format!("{name}, {date_text}")),
        gtk::accessible::Property::Description(&format!("{} · {source}", version.label)),
    ]);
    row
}

/// A one-line label, ellipsized when it is too long.
fn ellipsized(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build()
}

/// `10/20/2026 03:16`: the locale's date and the time as the snapshot's
/// name writes it.
fn date_modified_text(date: &SnapshotDate) -> String {
    let day = glib::DateTime::from_local(
        i32::from(date.year),
        i32::from(date.month),
        i32::from(date.day),
        12,
        0,
        0.0,
    );
    let day_text = day
        .ok()
        .and_then(|day| ox_core::format::format_date(&day))
        .unwrap_or_else(|| date.date_text());
    let time = date.time_text();
    if time.is_empty() {
        day_text
    } else {
        format!("{day_text} {time}")
    }
}

/// Windows' heading for a version dated `date`, seen on `today`.
pub(super) fn group_heading(date: Option<&SnapshotDate>, today: &glib::DateTime) -> &'static str {
    let Some(date) = date else {
        return ox_core::i18n::gettext_static(DATE_UNAVAILABLE);
    };
    let Ok(day) = glib::DateTime::from_local(
        i32::from(date.year),
        i32::from(date.month),
        i32::from(date.day),
        0,
        0,
        0.0,
    ) else {
        return ox_core::i18n::gettext_static(DATE_UNAVAILABLE);
    };
    let Ok(midnight) =
        glib::DateTime::from_local(today.year(), today.month(), today.day_of_month(), 0, 0, 0.0)
    else {
        return ox_core::i18n::gettext_static(DATE_UNAVAILABLE);
    };
    let days_ago = midnight.difference(&day).as_days();
    // Monday starts the week, as ISO numbers it.
    let into_week = i64::from(today.day_of_week() - 1);
    let heading = if days_ago < 0 {
        crate::i18n::message_id("Later")
    } else if days_ago == 0 {
        crate::i18n::message_id("Today")
    } else if days_ago == 1 {
        crate::i18n::message_id("Yesterday")
    } else if days_ago <= into_week {
        crate::i18n::message_id("Earlier this week")
    } else if days_ago <= into_week + 7 {
        crate::i18n::message_id("Last week")
    } else if day.year() == today.year() && day.month() == today.month() {
        crate::i18n::message_id("Earlier this month")
    } else if (day.year() * 12 + day.month()) + 1 == today.year() * 12 + today.month() {
        crate::i18n::message_id("Last month")
    } else if day.year() == today.year() {
        crate::i18n::message_id("Earlier this year")
    } else {
        crate::i18n::message_id("A long time ago")
    };
    ox_core::i18n::gettext_static(heading)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u16, month: u8, day: u8) -> SnapshotDate {
        SnapshotDate {
            year,
            month,
            day,
            time: None,
            is_utc: false,
        }
    }

    /// Versions group under Windows' headings, counted from today.
    ///
    /// parity: PROP-019
    #[test]
    fn versions_group_under_windows_headings() {
        // Saturday, October 31, 2026.
        let today = glib::DateTime::from_local(2026, 10, 31, 16, 0, 0.0).expect("a date");
        let heading = |year, month, day| group_heading(Some(&date(year, month, day)), &today);
        assert_eq!(heading(2026, 10, 31), "Today");
        assert_eq!(heading(2026, 10, 30), "Yesterday");
        assert_eq!(heading(2026, 10, 26), "Earlier this week");
        assert_eq!(heading(2026, 10, 20), "Last week");
        assert_eq!(heading(2026, 10, 3), "Earlier this month");
        assert_eq!(heading(2026, 9, 5), "Last month");
        assert_eq!(heading(2026, 2, 1), "Earlier this year");
        assert_eq!(heading(2025, 12, 31), "A long time ago");
        assert_eq!(group_heading(None, &today), "Date unavailable");
    }
}
