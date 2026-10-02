// SPDX-License-Identifier: AGPL-3.0-only
//! The Snapshot source form of the Previous versions tab (PROP-023).
//!
//! Ports `renderSnapshotSource` in `v2.0.0:desktop/ui/app.js`: the user maps a
//! live folder (the item's folder, or an SMB share's root) to a folder of
//! dated snapshots, in one of two layouts, and saves or removes the
//! mapping. The sources file is written on a GIO worker thread; errors
//! stay in the form.

use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{parent_location, split_location, LocationContext, LocationKind};
use ox_core::versions::{PreviousVersions, SnapshotLayout};

use crate::dialog::{labelled_entry, quiet_text};
use crate::window::ButtonStyle;

/// What the form is for.
const SOURCE_INTRO: &str = "Map the current live folder to a directory containing dated snapshots. Each \
                            snapshot must contain the same relative paths. These may also be existing \
                            backup folders; OpenXplorer does not certify them as immutable.";

/// The layouts the form offers, with their labels.
const LAYOUTS: [(SnapshotLayout, &str); 2] = [
    (SnapshotLayout::Direct, "snapshot-name / relative path"),
    (SnapshotLayout::Snapper, "snapshot-id / snapshot / relative path"),
];

/// What the form's buttons do once they are pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceChange {
    /// Save the mapping (Save source).
    Save,
    /// Remove the mapping of the live folder (Remove mapping).
    Remove,
}

/// The item the form suggests a mapping for.
#[derive(Debug, Clone)]
pub(super) struct SourceItem {
    /// The item's URI.
    pub uri: String,
    /// True for a folder.
    pub is_folder: bool,
}

impl SourceItem {
    /// The suggested live folder: the item's folder, or for SMB the
    /// share's root.
    fn live_folder(&self) -> String {
        let folder = if self.is_folder {
            self.uri.clone()
        } else {
            parent_location(&self.uri).unwrap_or_else(|| self.uri.clone())
        };
        share_root(&self.uri).unwrap_or(folder)
    }
}

/// `smb://server/share` for an item on a share.
fn share_root(uri: &str) -> Option<String> {
    let parts = split_location(uri).ok()?;
    if parts.kind() != LocationKind::Smb {
        return None;
    }
    let share = parts.path.split('/').find(|segment| !segment.is_empty())?;
    Some(format!("smb://{}/{share}", parts.authority))
}

/// The collection folder suggested for `live`, shown as `shown_live`
/// shows it: `/.snapshot` after a path, `\.snapshot` after a UNC name.
fn suggested_collection(shown_live: &str) -> String {
    let trimmed = shown_live.trim_end_matches(['/', '\\']);
    if shown_live.starts_with("\\\\") {
        format!("{trimmed}\\.snapshot")
    } else {
        format!("{trimmed}/.snapshot")
    }
}

/// Fills `panel` with the form. `finished` runs on Back, and after a
/// mapping was saved or removed, to show the versions list again.
pub(super) fn fill_source_form(
    panel: &gtk::Box,
    item: &SourceItem,
    versions: &Arc<PreviousVersions>,
    locations: &LocationContext,
    finished: impl Fn() + Clone + 'static,
) {
    panel.append(&quiet_text(SOURCE_INTRO));
    let shown_live = locations.display_location(&item.live_folder());
    let live = labelled_entry(panel, &ox_core::i18n::gettext("Live folder"), &shown_live);
    let collection = labelled_entry(
        panel,
        &ox_core::i18n::gettext("Snapshot collection folder"),
        &suggested_collection(&shown_live),
    );
    let labels: Vec<&str> = LAYOUTS.iter().map(|(_, label)| *label).collect();
    let layout = gtk::DropDown::from_strings(&labels);
    layout.update_property(&[gtk::accessible::Property::Label("Snapshot layout")]);
    panel.append(&layout);
    let error = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dialog-error"])
        .build();
    panel.append(&error);
    let form = SourceForm {
        live,
        collection,
        layout,
        error,
        versions: Arc::clone(versions),
    };
    panel.append(&form.buttons(finished));
}

/// The form's fields, read when a button is pressed.
#[derive(Debug, Clone)]
struct SourceForm {
    live: gtk::Entry,
    collection: gtk::Entry,
    layout: gtk::DropDown,
    error: gtk::Label,
    versions: Arc<PreviousVersions>,
}

impl SourceForm {
    /// Back, Save source and Remove mapping.
    fn buttons(&self, finished: impl Fn() + Clone + 'static) -> gtk::Box {
        let row = gtk::Box::builder()
            .css_classes(["snapshot-source-actions"])
            .build();
        let back = gtk::Button::with_label(&ox_core::i18n::gettext("Back"));
        back.add_css_class(ButtonStyle::Bordered.css_class());
        back.connect_clicked({
            let finished = finished.clone();
            move |_| finished()
        });
        row.append(&back);
        for (label, change, style) in [
            ("Save source", SourceChange::Save, ButtonStyle::Accent),
            ("Remove mapping", SourceChange::Remove, ButtonStyle::Bordered),
        ] {
            let button = gtk::Button::with_label(label);
            button.add_css_class(style.css_class());
            let form = self.clone();
            let finished = finished.clone();
            button.connect_clicked(move |_| form.apply(change, finished.clone()));
            row.append(&button);
        }
        row
    }

    /// Saves or removes the mapping on a worker thread; `finished` runs
    /// when it worked, and the error shows when it did not.
    fn apply(&self, change: SourceChange, finished: impl Fn() + 'static) {
        let live = self.live.text().to_string();
        let collection = self.collection.text().to_string();
        let position = usize::try_from(self.layout.selected()).unwrap_or_default();
        let layout = LAYOUTS
            .get(position)
            .map_or(SnapshotLayout::Direct, |(layout, _)| *layout);
        let versions = Arc::clone(&self.versions);
        let error = self.error.clone();
        glib::spawn_future_local(async move {
            let saved = gio::spawn_blocking(move || match change {
                SourceChange::Save => versions.configure(&live, &collection, layout),
                SourceChange::Remove => versions.remove_source(&live),
            })
            .await;
            match saved {
                Ok(Ok(_)) => finished(),
                Ok(Err(refusal)) => error.set_text(&refusal.to_string()),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-023
    #[test]
    fn the_form_suggests_the_share_root_and_its_snapshot_folder() {
        let file_on_share = SourceItem {
            uri: "smb://nas/projects/2026/plan.odt".to_owned(),
            is_folder: false,
        };
        let local_folder = SourceItem {
            uri: "file:///home/demo/Projects".to_owned(),
            is_folder: true,
        };

        assert_eq!(file_on_share.live_folder(), "smb://nas/projects");
        assert_eq!(local_folder.live_folder(), "file:///home/demo/Projects");
        assert_eq!(
            suggested_collection("\\\\nas\\projects"),
            "\\\\nas\\projects\\.snapshot"
        );
        assert_eq!(
            suggested_collection("/home/demo/Projects"),
            "/home/demo/Projects/.snapshot"
        );
    }
}
