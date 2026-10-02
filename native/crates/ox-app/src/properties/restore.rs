// SPDX-License-Identifier: AGPL-3.0-only
//! The Restore a copy dialog (PROP-025).
//!
//! Ports `restoreVersion` in `v2.0.0:desktop/ui/app.js`: the dialog asks for a
//! destination folder (the home folder by default), refuses one inside a
//! snapshot or backup folder, and answers with the canonical folder. The
//! window then copies the version there with Keep both, so neither the
//! live original nor the snapshot is ever replaced.

use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::normalise_location;
use ox_core::versions::{PreviousVersion, PreviousVersions};

use crate::dialog::{labelled_entry, note, DialogFrame, DialogWidth};
use crate::window::ButtonStyle;

/// What the dialog says the copy does, and does not do.
const RESTORE_NOTE: &str = crate::i18n::message_id(
    "Copies this version into a destination you choose. Existing names are kept; \
                            the restored item receives a copy name if needed. The live original and \
                            snapshot are not replaced.",
);

/// A previous version to restore a copy of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoreRequest {
    /// The version, inside its snapshot.
    pub version_uri: String,
    /// The snapshot's name.
    pub label: String,
    /// The item's name.
    pub name: String,
}

impl RestoreRequest {
    /// The request of the Restore a copy… button of `version`.
    pub(crate) fn of_version(version: &PreviousVersion) -> Self {
        Self {
            version_uri: version.entry.uri.clone(),
            label: version.label.clone(),
            name: version.entry.name.clone(),
        }
    }

    /// The request as a window action's parameter, `(sss)`.
    pub(crate) fn to_variant(&self) -> glib::Variant {
        (self.version_uri.as_str(), self.label.as_str(), self.name.as_str()).to_variant()
    }

    /// The request in a window action's parameter.
    pub(crate) fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let (version_uri, label, name) = variant.get::<(String, String, String)>()?;
        Some(Self {
            version_uri,
            label,
            name,
        })
    }

    /// The dialog for this request, which calls `restore` with the checked
    /// destination folder when the user confirms. `home` is the folder it
    /// suggests and the base of a relative path.
    pub(crate) fn dialog(
        &self,
        versions: Arc<PreviousVersions>,
        home: &str,
        restore: impl Fn(String) + 'static,
    ) -> DialogFrame {
        let frame = DialogFrame::new(&ox_core::i18n::gettext("Restore a copy"), DialogWidth::Standard);
        frame.set_message(&format!("{} · {}", self.label, self.name));
        let body = frame.body();
        body.append(&note(ox_core::i18n::gettext_static(RESTORE_NOTE)));
        let shown_home = glib::filename_from_uri(home)
            .map_or_else(|_| home.to_owned(), |(path, _)| path.display().to_string());
        let destination = labelled_entry(&body, &ox_core::i18n::gettext("Destination folder"), &shown_home);
        frame.add_closing_button(&ox_core::i18n::gettext("Cancel"), ButtonStyle::Bordered, || {});
        // Copy version and Enter in the field both confirm.
        let confirm = Rc::new(glib::clone!(
            #[weak]
            frame,
            #[weak]
            destination,
            move || match checked_destination(&versions, &destination.text(), &glib::home_dir()) {
                Err(message) => frame.show_error(&message),
                Ok(folder) => {
                    frame.close();
                    restore(folder);
                }
            }
        ));
        let copy = frame.add_button(&ox_core::i18n::gettext("Copy version"), ButtonStyle::Accent);
        copy.connect_clicked(glib::clone!(
            #[strong]
            confirm,
            move |_| confirm()
        ));
        destination.connect_activate(move |_| confirm());
        frame
    }
}

/// The canonical destination for `typed`, or the message that refuses it:
/// an address the location rules refuse, or a folder inside a snapshot
/// ("Choose a folder outside the snapshot collection.").
fn checked_destination(
    versions: &PreviousVersions,
    typed: &str,
    home: &std::path::Path,
) -> Result<String, String> {
    let folder = normalise_location(typed, None, home).map_err(|error| error.to_string())?;
    // Safety rule PROP-025: a restored copy never goes into a snapshot or
    // backup folder, which stays read-only.
    versions
        .restore_destination(&folder)
        .map_err(|refusal| refusal.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-025
    #[test]
    fn a_destination_inside_a_snapshot_is_refused() {
        let settings = tempfile::tempdir().expect("a settings folder");
        let versions = PreviousVersions::new(settings.path());
        let home = std::path::Path::new("/home/demo");

        let into_snapshot = checked_destination(&versions, "smb://nas/share/.snapshot/daily", home);
        let into_live_folder = checked_destination(&versions, "/home/demo/Restored", home);

        assert_eq!(
            into_snapshot,
            Err("Choose a folder outside the snapshot collection.".to_owned())
        );
        assert_eq!(into_live_folder, Ok("file:///home/demo/Restored".to_owned()));
    }
}
