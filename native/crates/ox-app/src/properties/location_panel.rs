// SPDX-License-Identifier: AGPL-3.0-only
//! The Location tab of a standard folder's Properties (PROP-017, PROP-018).
//!
//! Ports `renderLocationPanel` in `v2.0.0:desktop/ui/app.js` over ox-core's
//! [`FolderRelocation`], which ports `v2.0.0:desktop/folder_locations.py`: the
//! Folder location field, a picker of mounted network drives, Check
//! location, Restore default, Use previous, the status line, the warning
//! about existing files, Brave's follow-up for Downloads, the consent box
//! and Apply location, then the network mount assistant. Beyond the
//! Python app, and as Windows 11 does, a change is followed by the offer
//! to move the old folder's files ([`applying`]).
//!
//! Safety rule "explicit consent, for the checked folder": Apply location
//! stays disabled until the consent box is ticked and the field still
//! holds the path the last check returned; any edit needs a new check.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `applying` | Check location, Apply location and what follows a change |

mod applying;

use std::sync::Arc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::folder_locations::{FolderRelocation, StandardFolderLocation};
use ox_core::integration::BraveIntegration;
use ox_core::network::{read_mount_table, MountEntry};
use ox_core::places::KnownFolder;

use super::general_panel::glyph_button;
use super::mount_assistant::mount_assistant;
use crate::dialog::{check_row, labelled_entry, quiet_text};
use crate::icons::{self, Icon};
use crate::window::ButtonStyle;

/// The warning under the status line (`.location-warning`). The Python
/// app never moved files; the native app offers to after a change.
const FILES_STAY: &str = "Apply changes only the setting: existing files are NOT moved unless you choose to \
                          move them afterwards. Your browser may have its own download setting—set it to the \
                          same Linux path. A network folder is unavailable while its server is offline.";
/// The consent box.
const CONSENT: &str = "Change the system folder location; leave existing files in place.";
/// Brave's follow-up box, for Downloads only.
const SYNC_BRAVE: &str = "Also update Brave’s download directory (choose profiles after Apply).";
/// The first line of the mounted network drive picker.
const CHOOSE_MOUNT: &str = "Choose a mounted network drive…";

/// What the status line says and how it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// Plain guidance.
    Plain,
    /// A passed check or an applied change (`.valid`).
    Valid,
    /// A refusal (`.error`).
    Error,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::sync::Arc;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::folder_locations::{CheckedLocation, FolderRelocation, StandardFolderLocation};
    use ox_core::integration::BraveIntegration;
    use ox_core::places::KnownFolder;

    /// Private state of [`super::LocationPanel`].
    #[derive(Debug, Default)]
    pub(crate) struct LocationPanel {
        /// The standard folder; set by `new`.
        pub(super) folder: OnceCell<KnownFolder>,
        /// Checks and moves the folder; set by `new`.
        pub(super) relocation: OnceCell<Arc<FolderRelocation>>,
        /// Brave's integration, for the Downloads follow-up; set by `new`.
        pub(super) brave: OnceCell<BraveIntegration>,
        /// Where the folder is, its default and previous path, once read.
        pub(super) location: RefCell<Option<StandardFolderLocation>>,
        /// The last passed check, until the field changes.
        pub(super) checked: RefCell<Option<CheckedLocation>>,
        /// Whether a change is being applied.
        pub(super) is_applying: Cell<bool>,
        /// Set while the panel itself writes the field, which is no edit.
        pub(super) is_filling_field: Cell<bool>,
        /// Folder location.
        pub(super) field: OnceCell<gtk::Entry>,
        /// Where the mounted network drive picker goes, once mounts are read.
        pub(super) picker_slot: gtk::Box,
        /// Check location, Restore default and Use previous.
        pub(super) controls: gtk::Box,
        /// Check location.
        pub(super) check_button: OnceCell<gtk::Button>,
        /// The status line (`role=status`).
        pub(super) status: gtk::Label,
        /// Brave's follow-up box.
        pub(super) sync_brave: OnceCell<gtk::CheckButton>,
        /// The consent box.
        pub(super) consent: OnceCell<gtk::CheckButton>,
        /// Apply location.
        pub(super) apply_button: OnceCell<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LocationPanel {
        const NAME: &'static str = "OxLocationPanel";
        type Type = super::LocationPanel;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for LocationPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_orientation(gtk::Orientation::Vertical);
        }
    }

    impl WidgetImpl for LocationPanel {}
    impl BoxImpl for LocationPanel {}
}

glib::wrapper! {
    /// The Location tab of a standard folder.
    pub(crate) struct LocationPanel(ObjectSubclass<imp::LocationPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl LocationPanel {
    /// The tab of `folder`, moved through `relocation`. It reads where the
    /// folder is and the mounted network drives off the main thread.
    pub(crate) fn new(
        folder: KnownFolder,
        relocation: Arc<FolderRelocation>,
        brave: BraveIntegration,
    ) -> Self {
        let panel: Self = glib::Object::new();
        let imp = panel.imp();
        imp.folder.set(folder).expect("a new panel has no folder yet");
        imp.relocation
            .set(relocation)
            .expect("a new panel has no relocation yet");
        imp.brave
            .set(brave)
            .expect("a new panel has no Brave integration yet");
        panel.build();
        panel.read_location();
        panel
    }

    fn folder(&self) -> KnownFolder {
        *self.imp().folder.get().expect("new sets the folder")
    }

    fn relocation(&self) -> Arc<FolderRelocation> {
        Arc::clone(self.imp().relocation.get().expect("new sets the relocation"))
    }

    fn field(&self) -> &gtk::Entry {
        self.imp().field.get().expect("build adds the field")
    }

    fn consent(&self) -> &gtk::CheckButton {
        self.imp().consent.get().expect("build adds the consent box")
    }

    /// Lays the tab out, top to bottom, as `renderLocationPanel`.
    fn build(&self) {
        let imp = self.imp();
        let label = self.folder().label();
        let intro = format!(
            "Choose where {label} is stored. Applications that honor Linux’s standard-folder settings will \
             use this location."
        );
        self.append(&quiet_text(&intro));
        let field = labelled_entry(self.upcast_ref(), &ox_core::i18n::gettext("Folder location"), "");
        field.connect_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.field_edited()
        ));
        imp.field.set(field).expect("build runs once");
        self.append(&imp.picker_slot);
        self.build_controls();
        imp.status.set_xalign(0.0);
        imp.status.set_wrap(true);
        imp.status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        imp.status.set_accessible_role(gtk::AccessibleRole::Status);
        self.set_status("", Tone::Plain);
        self.append(&imp.status);
        self.append(&warning());
        self.build_consent();
        self.append(&mount_assistant(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |path: &str| panel.fill_field(path, "After mounting, click Check location.")
        )));
    }

    /// Check location and Restore default; Use previous joins them once
    /// the history is read.
    fn build_controls(&self) {
        let imp = self.imp();
        imp.controls.add_css_class("location-controls");
        let check = glyph_button("Check location", Icon::Checkmark);
        check.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.check_location()
        ));
        let restore = glyph_button("Restore default", Icon::ArrowReset);
        restore.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.restore_default()
        ));
        imp.controls.append(&check);
        imp.controls.append(&restore);
        imp.check_button.set(check).expect("build runs once");
        self.append(&imp.controls);
    }

    /// Brave's follow-up for Downloads, the consent box and Apply location.
    fn build_consent(&self) {
        let imp = self.imp();
        let sync_brave = check_row(SYNC_BRAVE, false);
        sync_brave.set_visible(self.folder() == KnownFolder::Downloads);
        self.append(&sync_brave);
        let consent = check_row(CONSENT, false);
        consent.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.update_apply()
        ));
        self.append(&consent);
        let apply = glyph_button("Apply location", Icon::Checkmark);
        apply.remove_css_class(ButtonStyle::Bordered.css_class());
        apply.add_css_class(ButtonStyle::Accent.css_class());
        apply.set_halign(gtk::Align::Start);
        apply.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.apply_location()
        ));
        self.append(&apply);
        imp.sync_brave.set(sync_brave).expect("build runs once");
        imp.consent.set(consent).expect("build runs once");
        imp.apply_button.set(apply).expect("build runs once");
        self.update_apply();
    }

    /// Reads where the folder is and the mounted network drives, then
    /// fills the field, Use previous and the drive picker.
    fn read_location(&self) {
        let folder = self.folder();
        let relocation = self.relocation();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            async move {
                let read = gio::spawn_blocking(move || {
                    let location = relocation.location_of(folder);
                    let mounts = read_mount_table().unwrap_or_default();
                    (location, mounts)
                });
                let Ok((location, mounts)) = read.await else {
                    return;
                };
                panel.show_location(location, &mounts);
            }
        ));
    }

    fn show_location(&self, location: StandardFolderLocation, mounts: &[MountEntry]) {
        let path = location.path.to_string_lossy().into_owned();
        if location.previous_path.is_some() {
            self.add_use_previous();
        }
        self.imp().location.replace(Some(location));
        self.set_field_text(&path);
        self.add_mount_picker(mounts);
    }

    fn add_use_previous(&self) {
        let use_previous = glyph_button("Use previous", Icon::History);
        use_previous.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.use_previous()
        ));
        self.imp().controls.append(&use_previous);
    }

    /// "Choose a mounted network drive…" and one line per kernel SMB mount
    /// (`source → path`); nothing without such mounts.
    fn add_mount_picker(&self, mounts: &[MountEntry]) {
        let smb_mounts: Vec<&MountEntry> = mounts.iter().filter(|mount| mount.is_smb()).collect();
        if smb_mounts.is_empty() {
            return;
        }
        let mut lines = vec![CHOOSE_MOUNT.to_owned()];
        lines.extend(
            smb_mounts
                .iter()
                .map(|mount| format!("{} → {}", mount.source, mount.path)),
        );
        let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let picker = gtk::DropDown::from_strings(&line_refs);
        picker.add_css_class("mount-picker");
        picker.update_property(&[gtk::accessible::Property::Label("Mounted network drives")]);
        let paths: Vec<String> = smb_mounts.iter().map(|mount| mount.path.clone()).collect();
        picker.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |picker| {
                let Some(index) = picker.selected().checked_sub(1) else {
                    return;
                };
                if let Some(path) = paths.get(index as usize) {
                    panel.fill_field(path, "Check this destination before applying.");
                }
            }
        ));
        self.imp().picker_slot.append(&picker);
    }

    fn restore_default(&self) {
        let location = self.imp().location.borrow().clone();
        let Some(location) = location else {
            return;
        };
        let default_path = location.default_path.to_string_lossy();
        self.fill_field(
            &default_path,
            "Check this folder, then Apply. Existing files stay where they are.",
        );
    }

    fn use_previous(&self) {
        let location = self.imp().location.borrow().clone();
        let Some(previous) = location.and_then(|location| location.previous_path) else {
            return;
        };
        self.fill_field(
            &previous.to_string_lossy(),
            "Check the previous folder, then Apply.",
        );
    }

    /// Puts `path` in the field, which then needs a check, and says `hint`.
    fn fill_field(&self, path: &str, hint: &str) {
        self.set_field_text(path);
        self.forget_check();
        self.set_status(hint, Tone::Plain);
    }

    /// Writes the field without counting it as an edit.
    fn set_field_text(&self, text: &str) {
        self.imp().is_filling_field.set(true);
        self.field().set_text(text);
        self.imp().is_filling_field.set(false);
    }

    /// The user changed the field: the last check no longer applies.
    fn field_edited(&self) {
        if self.imp().is_filling_field.get() {
            return;
        }
        self.forget_check();
        self.set_status("Check the changed destination before applying.", Tone::Plain);
    }

    fn forget_check(&self) {
        self.imp().checked.replace(None);
        self.update_apply();
    }

    /// Enables Apply location only for a consented, checked, unchanged
    /// destination while nothing is being applied (`syncApply`).
    fn update_apply(&self) {
        let Some(apply) = self.imp().apply_button.get() else {
            return;
        };
        let can_apply =
            !self.imp().is_applying.get() && self.consent().is_active() && self.is_field_checked();
        apply.set_sensitive(can_apply);
    }

    /// Whether the field holds the path the last check returned.
    fn is_field_checked(&self) -> bool {
        let checked = self.imp().checked.borrow();
        let text = self.field().text();
        checked
            .as_ref()
            .is_some_and(|checked| checked.path.to_string_lossy() == text.as_str())
    }

    fn set_status(&self, text: &str, tone: Tone) {
        let status = &self.imp().status;
        status.set_text(text);
        status.set_css_classes(&["location-status"]);
        match tone {
            Tone::Plain => {}
            Tone::Valid => status.add_css_class("valid"),
            Tone::Error => status.add_css_class("error"),
        }
    }
}

/// What tests read and change.
#[cfg(test)]
impl LocationPanel {
    /// The Folder location field.
    pub(crate) fn location_field(&self) -> gtk::Entry {
        self.field().clone()
    }

    /// The status line.
    pub(crate) fn status_text(&self) -> String {
        self.imp().status.text().to_string()
    }

    /// Ticks or clears the consent box.
    pub(crate) fn set_consent(&self, is_given: bool) {
        self.consent().set_active(is_given);
    }

    /// Whether Apply location is enabled.
    pub(crate) fn can_apply(&self) -> bool {
        self.imp()
            .apply_button
            .get()
            .is_some_and(gtk::Button::is_sensitive)
    }

    /// Brave's follow-up box, which only Downloads shows.
    pub(crate) fn sync_brave_box(&self) -> gtk::CheckButton {
        self.imp().sync_brave.get().expect("build adds the box").clone()
    }
}

/// The note about existing files (`.location-warning`).
fn warning() -> gtk::Box {
    let warning = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    warning.add_css_class("location-warning");
    let glyph = icons::image(Icon::Info, 18);
    glyph.set_valign(gtk::Align::Start);
    warning.append(&glyph);
    let text = quiet_text(FILES_STAY);
    text.set_hexpand(true);
    // A vertical box reports no baseline, so the row does not align the
    // text's baseline with the glyph's, which GTK 4.14 does only when it
    // measures for a given width and then reports two pixels more.
    let text_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text_column.set_hexpand(true);
    text_column.append(&text);
    warning.append(&text_column);
    warning
}
