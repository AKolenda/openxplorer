// SPDX-License-Identifier: AGPL-3.0-only
//! The bar at the bottom of the window while folders are measured
//! (PROP-026).
//!
//! Ports `#size-scan` of `v2.0.0:desktop/ui/index.html` and the texts
//! `receiveFolderSize`, `stopSizeScan` and `scanFolderSizes` of
//! `v2.0.0:desktop/ui/app.js` write into it: which folder of how many is being
//! measured and how much it holds so far, then how the run ended, with
//! Cancel scan while it runs and Dismiss after it. The button runs
//! [`WindowAction::CancelSizeScan`], which cancels a running scan or hides
//! the finished bar. `native/docs/ui-spec.md` §4.15 draws it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format;
use ox_core::sizes::FolderSize;

use crate::window::{ButtonStyle, WindowAction};

/// The label while the scan stops.
const CANCELLING: &str = crate::i18n::message_id("Cancelling size scan…");

/// Where one folder of a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunPosition {
    /// The folder's position in the run, from 0.
    pub index: usize,
    /// How many folders the run measures.
    pub total: usize,
}

/// How a run ended, for the text the bar keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunEnd {
    /// The user cancelled.
    Cancelled,
    /// Every folder was measured: `complete` fully, `partial` partly or
    /// not at all.
    Finished {
        /// Folders whose scan counted everything.
        complete: usize,
        /// Folders whose scan was partial or failed.
        partial: usize,
    },
}

/// `Scanning folder size · 1/3 · Projects · 1.2 MB · 1,024 files`, or
/// `Cancelling… 1/3 · …` once the user cancelled (`receiveFolderSize`).
pub(crate) fn progress_text(
    position: RunPosition,
    name: &str,
    size: &FolderSize,
    is_cancelling: bool,
) -> String {
    let lead = if is_cancelling {
        ox_core::i18n::gettext_static("Cancelling… ")
    } else {
        ox_core::i18n::gettext_static("Scanning folder size · ")
    };
    let counted = ox_core::i18n::format_message(
        "{value1}/{total} · {name} · {value3} · {group_thousands} files",
        &[
            ("value1", &(position.index + 1).to_string()),
            ("total", &position.total.to_string()),
            ("name", name),
            ("value3", &format::pretty_bytes(size.bytes)),
            ("group_thousands", &group_thousands(size.files)),
        ],
    );
    format!("{lead}{counted}")
}

/// `Size scan finished · 2 complete · 1 partial/unavailable · Logical
/// bytes; recalculate after changes`, or `Size scan cancelled · …`.
pub(crate) fn end_text(end: RunEnd) -> String {
    let hint = match end {
        RunEnd::Cancelled => ox_core::i18n::gettext_static("Size scan cancelled").to_owned(),
        RunEnd::Finished { complete, partial: 0 } => ox_core::i18n::format_message(
            "Size scan finished · {complete} complete",
            &[("complete", &complete.to_string())],
        ),
        RunEnd::Finished { complete, partial } => ox_core::i18n::format_message(
            "Size scan finished · {complete} complete · {partial} partial/unavailable",
            &[
                ("complete", &complete.to_string()),
                ("partial", &partial.to_string()),
            ],
        ),
    };
    ox_core::i18n::format_message(
        "{hint} · Logical bytes; recalculate after changes",
        &[("hint", &hint)],
    )
}

/// `1234567` as `1,234,567`, as `toLocaleString` writes it in English.
fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        let remaining = digits.len() - index;
        if index > 0 && remaining.is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::SizeScanStrip`].
    #[derive(Debug, Default)]
    pub(crate) struct SizeScanStrip {
        /// What the run is doing, announced politely.
        pub(super) label: gtk::Label,
        /// Cancel scan, or Dismiss once the run ended.
        pub(super) button: gtk::Button,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SizeScanStrip {
        const NAME: &'static str = "OxSizeScanStrip";
        type Type = super::SizeScanStrip;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for SizeScanStrip {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for SizeScanStrip {}
    impl BoxImpl for SizeScanStrip {}
}

glib::wrapper! {
    /// The bar of a running or finished folder-size scan.
    pub(crate) struct SizeScanStrip(ObjectSubclass<imp::SizeScanStrip>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for SizeScanStrip {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SizeScanStrip {
    /// Lays out the label and the button; the bar starts hidden.
    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("size-scan");
        self.set_visible(false);
        imp.label.set_xalign(0.0);
        imp.label.set_hexpand(true);
        imp.label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        imp.label.set_accessible_role(gtk::AccessibleRole::Status);
        imp.button.add_css_class(ButtonStyle::Bordered.css_class());
        imp.button.set_valign(gtk::Align::Center);
        WindowAction::CancelSizeScan.assign_to(&imp.button);
        self.append(&imp.label);
        self.append(&imp.button);
    }

    /// Shows the bar for a new run, with Cancel scan.
    pub(crate) fn start(&self) {
        let button = &self.imp().button;
        button.set_label(&ox_core::i18n::gettext("Cancel scan"));
        button.set_sensitive(true);
        self.set_visible(true);
    }

    /// Shows `text` about the folder being measured.
    pub(crate) fn show_progress(&self, text: &str) {
        self.imp().label.set_text(text);
    }

    /// Says the run is stopping; Cancel scan cannot be pressed again.
    pub(crate) fn show_cancelling(&self) {
        self.imp()
            .label
            .set_text(ox_core::i18n::gettext_static(CANCELLING));
        self.imp().button.set_sensitive(false);
    }

    /// Shows how the run ended, with Dismiss.
    pub(crate) fn show_end(&self, end: RunEnd) {
        let imp = self.imp();
        imp.label.set_text(&end_text(end));
        imp.button.set_label(&ox_core::i18n::gettext("Dismiss"));
        imp.button.set_sensitive(true);
    }

    /// The text shown, for tests.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.imp().label.text().to_string()
    }

    /// The button's label, for tests.
    #[cfg(test)]
    pub(crate) fn button_label(&self) -> String {
        self.imp().button.label().map(String::from).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ox_core::sizes::ScanStatus;

    use super::*;

    /// parity: PROP-026
    #[test]
    fn the_bar_names_the_folder_its_position_and_the_totals() {
        let size = FolderSize {
            uri: "file:///tmp/ox-test/Projects".to_owned(),
            bytes: 1_048_576,
            files: 1_024,
            folders: 3,
            entries: 1_027,
            skipped: 0,
            errors: 0,
            status: ScanStatus::Scanning,
            elapsed: Duration::ZERO,
            finished_at: None,
        };
        let position = RunPosition { index: 0, total: 3 };

        assert_eq!(
            progress_text(position, "Projects", &size, false),
            "Scanning folder size · 1/3 · Projects · 1.0 MB · 1,024 files"
        );
        assert_eq!(
            progress_text(position, "Projects", &size, true),
            "Cancelling… 1/3 · Projects · 1.0 MB · 1,024 files"
        );
    }

    /// parity: PROP-026
    #[test]
    fn the_end_of_a_run_says_how_many_folders_are_exact() {
        let all_complete = RunEnd::Finished {
            complete: 2,
            partial: 0,
        };
        let some_partial = RunEnd::Finished {
            complete: 2,
            partial: 1,
        };

        assert_eq!(
            end_text(all_complete),
            "Size scan finished · 2 complete · Logical bytes; recalculate after changes"
        );
        assert_eq!(
            end_text(some_partial),
            "Size scan finished · 2 complete · 1 partial/unavailable · Logical bytes; recalculate after \
             changes"
        );
        assert_eq!(
            end_text(RunEnd::Cancelled),
            "Size scan cancelled · Logical bytes; recalculate after changes"
        );
    }
}
