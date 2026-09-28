// SPDX-License-Identifier: AGPL-3.0-only
//! One row of the Previous versions list (PROP-019, PROP-020).
//!
//! Ports the loop over `data.versions` in `renderVersionsPanel` of
//! `desktop/ui/app.js`: the item's icon, the snapshot's name over the
//! collection it came from, the date and time read from the snapshot's
//! name with a tooltip saying where they come from, then Browse (folders)
//! and Restore a copy…. The buttons run window actions, so the window
//! opens the tab or the Restore dialog.

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::LocationContext;
use ox_core::versions::{PreviousVersion, SnapshotDate, DATE_UNAVAILABLE, NO_DATE_EXPLANATION};

use crate::icons::{self, Art, ArtImage, Icon};
use crate::window::{ButtonStyle, WindowAction};

/// The size of a version's icon (`fileIcon(version, 23)`).
const ROW_ART_SIZE: i32 = 23;

/// The glyph size in a row's buttons (`.version-row button svg`).
const BUTTON_GLYPH: i32 = 13;

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

/// The row showing `version`, whose collection is named through
/// `locations`.
pub(super) fn version_row(version: &PreviousVersion, locations: &LocationContext) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["version-row"])
        .build();
    row.set_accessible_role(gtk::AccessibleRole::ListItem);
    row.append(&identity(version, locations));
    row.append(&date_cell(version));
    row.append(&actions(version));
    row
}

/// The icon, the snapshot's name and the collection it came from.
fn identity(version: &PreviousVersion, locations: &LocationContext) -> gtk::Box {
    let identity = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .css_classes(["version-identity"])
        .build();
    identity.append(&ArtImage::new(Art::for_entry(&version.entry), ROW_ART_SIZE));
    let source = locations.display_location(&version.collection);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["version-text"])
        .tooltip_text(format!("{}\n{source}", version.label))
        .build();
    text.append(&ellipsized(&version.label, "version-label"));
    text.append(&ellipsized(&source, "version-source"));
    identity.append(&text);
    identity
}

/// A one-line label with class `class`, ellipsized when it is too long.
fn ellipsized(text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes([class])
        .build()
}

/// The date and time from the snapshot's name, or "Date unavailable",
/// with a tooltip saying where they come from.
fn date_cell(version: &PreviousVersion) -> gtk::Box {
    let cell = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["version-date"])
        .build();
    let Some(date) = version.date() else {
        cell.set_tooltip_text(Some(NO_DATE_EXPLANATION));
        cell.append(&ellipsized(DATE_UNAVAILABLE, "version-calendar-date"));
        return cell;
    };
    cell.set_tooltip_text(Some(date.explanation()));
    cell.append(&ellipsized(&medium_date(&date), "version-calendar-date"));
    let time = date.time_text();
    if !time.is_empty() {
        cell.append(&ellipsized(&time, "version-time"));
    }
    // A screen reader hears the exact date, as `<time datetime>` gives it.
    cell.update_property(&[gtk::accessible::Property::Description(&date.iso_8601())]);
    cell
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

/// Browse (folders only) and Restore a copy….
fn actions(version: &PreviousVersion) -> gtk::Box {
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["version-actions"])
        .build();
    if version.entry.is_dir {
        let browse = row_button("Browse", Icon::Folder);
        let target = SnapshotTarget::of_version(version).to_variant();
        WindowAction::BrowseSnapshot.assign_with_target_to(&browse, &target);
        actions.append(&browse);
    }
    let restore = row_button("Restore a copy…", Icon::Copy);
    let request = super::RestoreRequest::of_version(version).to_variant();
    WindowAction::RestoreVersion.assign_with_target_to(&restore, &request);
    actions.append(&restore);
    actions
}

/// A small bordered button with `glyph` and `label`.
fn row_button(label: &str, glyph: Icon) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    content.append(&icons::image(glyph, BUTTON_GLYPH));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class(ButtonStyle::Bordered.css_class());
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}
