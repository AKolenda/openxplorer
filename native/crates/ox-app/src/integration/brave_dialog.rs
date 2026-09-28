// SPDX-License-Identifier: AGPL-3.0-only
//! "Use this Downloads folder in Brave": pointing the download folder of
//! chosen Brave profiles at the Linux Downloads folder, and putting one
//! profile's previous folder back.
//!
//! Ports `braveDialog` in `desktop/ui/app.js` (INT-019 to INT-021). Every
//! profile starts ticked and the consent check box unticked; Apply to
//! Brave needs the consent and at least one profile, Restore previous the
//! consent and exactly one. The sync and its safety rules (never while
//! Brave runs, check every profile first, back up, change only the two
//! download folders) are ox-core's [`BraveIntegration`].
//!
//! [`BraveDialog`] is a `GtkWindow` subclass laid out by the template
//! `resources/ui/brave-dialog.ui`.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::integration::{
    BraveError, BraveIntegration, BraveProfile, BraveReach, BraveStatus, Confirmation, SyncOutcome,
};

/// Why Apply to Brave did nothing.
const CONSENT_NEEDED: &str = "Confirm the change using the checkbox.";
/// Why Apply to Brave did nothing.
const PROFILE_NEEDED: &str = "Select at least one profile.";
/// Why Restore previous did nothing.
const ONE_PROFILE_NEEDED: &str = "Select one profile and confirm to restore its previous download setting.";

/// Shows a message in the window that opened the dialog.
type Report = Box<dyn Fn(&str)>;

/// The status line for `status` (`load` in `braveDialog`).
pub(crate) fn status_text(status: &BraveStatus) -> String {
    if status.reach == BraveReach::Sandboxed {
        return BraveError::Sandboxed.to_string();
    }
    let state = if status.is_running {
        "Brave is running. Quit it completely, then Recheck."
    } else if status.profiles.is_empty() {
        "No supported native profiles found. Set brave://settings/downloads manually."
    } else {
        "Brave is closed. Select the profiles to update."
    };
    if !status.sandboxed_installs.is_empty() {
        let installs: Vec<&str> = status
            .sandboxed_installs
            .iter()
            .map(|install| install.label())
            .collect();
        let manual = installs.join(" / ");
        return format!("{state} {manual} installations need manual browser settings.");
    }
    state.to_owned()
}

/// What a sync that ended with `outcome` says: the toast when every
/// profile was updated, else the line that stays in the dialog.
pub(crate) fn sync_report(outcome: &SyncOutcome) -> Result<String, String> {
    if outcome.failures.is_empty() {
        return Ok(format!(
            "Brave Downloads updated for {} profile(s).",
            outcome.updated.len()
        ));
    }
    let reasons: Vec<String> = outcome
        .failures
        .iter()
        .map(|failure| failure.error.to_string())
        .collect();
    Err(format!(
        "{} updated. {}",
        outcome.updated.len(),
        reasons.join(" ")
    ))
}

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;
    use ox_core::integration::BraveIntegration;

    use super::Report;

    /// Private state of [`super::BraveDialog`].
    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/brave-dialog.ui")]
    pub(crate) struct BraveDialog {
        #[template_child]
        pub(super) destination_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) profile_list: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) recheck_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) consent_check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) restore_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) apply_button: TemplateChild<gtk::Button>,
        /// The user's Brave integration.
        pub(super) brave: OnceCell<BraveIntegration>,
        /// The folder the profiles will use.
        pub(super) destination: OnceCell<String>,
        /// Shows the outcome in the window.
        pub(super) report: OnceCell<Report>,
        /// One check box per profile, with the profile's ID.
        pub(super) choices: RefCell<Vec<(gtk::CheckButton, String)>>,
    }

    impl std::fmt::Debug for BraveDialog {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("BraveDialog")
                .field("destination", &self.destination)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BraveDialog {
        const NAME: &'static str = "OxBraveDialog";
        type Type = super::BraveDialog;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
            dialog.init_template();
        }
    }

    impl ObjectImpl for BraveDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().connect_buttons();
        }
    }

    impl WidgetImpl for BraveDialog {}
    impl WindowImpl for BraveDialog {}
}

glib::wrapper! {
    /// The Brave download-folder dialog.
    pub(crate) struct BraveDialog(ObjectSubclass<imp::BraveDialog>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager;
}

impl BraveDialog {
    /// Opens the dialog over `parent` to point Brave at `destination`, and
    /// reads the profiles. `report` shows the outcome in the window.
    pub(crate) fn present_for(
        parent: &impl IsA<gtk::Window>,
        brave: BraveIntegration,
        destination: &str,
        report: impl Fn(&str) + 'static,
    ) -> Self {
        let dialog: Self = glib::Object::builder().property("transient-for", parent).build();
        let imp = dialog.imp();
        imp.destination_label.set_text(destination);
        imp.brave
            .set(brave)
            .expect("a new dialog has no Brave integration");
        imp.destination
            .set(destination.to_owned())
            .expect("a new dialog has no destination");
        let report: Report = Box::new(report);
        assert!(imp.report.set(report).is_ok(), "a new dialog has no report");
        dialog.present();
        dialog.load();
        dialog
    }

    fn brave(&self) -> &BraveIntegration {
        self.imp()
            .brave
            .get()
            .expect("BraveDialog::present_for sets the integration")
    }

    fn connect_buttons(&self) {
        let imp = self.imp();
        imp.recheck_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.load()
        ));
        imp.cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.close()
        ));
        imp.restore_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.restore()
        ));
        imp.apply_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.apply()
        ));
        self.add_controller(crate::modal::escape_closes());
    }

    /// Reads the profiles and whether Brave runs ("Recheck profiles").
    fn load(&self) {
        self.imp()
            .status_label
            .set_text("Checking native Brave profiles…");
        let reading = self.brave().run_in_background(BraveIntegration::status);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let status = reading.await;
                dialog.show_status(&status);
            }
        ));
    }

    fn show_status(&self, status: &BraveStatus) {
        let imp = self.imp();
        let list = &*imp.profile_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let choices: Vec<(gtk::CheckButton, String)> = status
            .profiles
            .iter()
            .map(|profile| (profile_check(profile), profile.id.clone()))
            .collect();
        for (check, _) in &choices {
            list.append(check);
        }
        imp.choices.replace(choices);
        imp.status_label.set_text(&status_text(status));
    }

    /// The IDs of the ticked profiles.
    fn selected_profiles(&self) -> Vec<String> {
        let choices = self.imp().choices.borrow();
        let ticked = choices.iter().filter(|(check, _)| check.is_active());
        ticked.map(|(_, id)| id.clone()).collect()
    }

    fn confirmation(&self) -> Confirmation {
        if self.imp().consent_check.is_active() {
            Confirmation::Confirmed
        } else {
            Confirmation::NotConfirmed
        }
    }

    /// Apply to Brave: needs the consent and a profile, then syncs every
    /// ticked profile and reports each failure.
    fn apply(&self) {
        let imp = self.imp();
        if self.confirmation() != Confirmation::Confirmed {
            imp.status_label.set_text(CONSENT_NEEDED);
            return;
        }
        let profiles = self.selected_profiles();
        if profiles.is_empty() {
            imp.status_label.set_text(PROFILE_NEEDED);
            return;
        }
        imp.apply_button.set_sensitive(false);
        let destination = imp.destination.get().cloned().unwrap_or_default();
        let syncing = self
            .brave()
            .run_in_background(move |brave| brave.sync(&profiles, &destination, Confirmation::Confirmed));
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let outcome = syncing.await;
                dialog.imp().apply_button.set_sensitive(true);
                let report = outcome
                    .map_err(|error| error.to_string())
                    .and_then(|outcome| sync_report(&outcome));
                dialog.finish(report);
            }
        ));
    }

    /// Restore previous: needs the consent and exactly one profile.
    fn restore(&self) {
        let profiles = self.selected_profiles();
        let [profile] = profiles.as_slice() else {
            self.imp().status_label.set_text(ONE_PROFILE_NEEDED);
            return;
        };
        if self.confirmation() != Confirmation::Confirmed {
            self.imp().status_label.set_text(ONE_PROFILE_NEEDED);
            return;
        }
        let profile = profile.clone();
        let restoring = self
            .brave()
            .run_in_background(move |brave| brave.restore(&profile, Confirmation::Confirmed));
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let restored = restoring.await;
                let report = restored
                    .map(|_| "Previous Brave download setting restored.".to_owned())
                    .map_err(|error| error.to_string());
                dialog.finish(report);
            }
        ));
    }

    /// Reports a success in the window and closes, or keeps the dialog
    /// open with the failure.
    fn finish(&self, report: Result<String, String>) {
        match report {
            Ok(message) => {
                if let Some(report) = self.imp().report.get() {
                    report(&message);
                }
                self.close();
            }
            Err(message) => self.imp().status_label.set_text(&message),
        }
    }

    /// The status line, for tests.
    #[cfg(test)]
    pub(crate) fn status(&self) -> String {
        self.imp().status_label.text().to_string()
    }

    /// The profile check boxes' labels, for tests.
    #[cfg(test)]
    pub(crate) fn profile_labels(&self) -> Vec<String> {
        let choices = self.imp().choices.borrow();
        let labels = choices.iter().filter_map(|(check, _)| check.label());
        labels.map(|label| label.to_string()).collect()
    }

    /// Clicks Apply to Brave, for tests.
    #[cfg(test)]
    pub(crate) fn click_apply(&self) {
        self.imp().apply_button.emit_clicked();
    }
}

/// The check box of `profile`: "<name> · <channel>", then its download
/// folder or "Uses browser default". Ticked, as in the Python dialog.
fn profile_check(profile: &BraveProfile) -> gtk::CheckButton {
    let folder = if profile.download_path.is_empty() {
        "Uses browser default"
    } else {
        profile.download_path.as_str()
    };
    let label = format!("{} · {}\n{folder}", profile.name, profile.channel.folder_name());
    gtk::CheckButton::builder().label(label).active(true).build()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ox_core::integration::{BraveChannel, ProfileFailure, SandboxedBrave};

    use super::*;

    fn status(profiles: Vec<BraveProfile>, is_running: bool) -> BraveStatus {
        BraveStatus {
            reach: BraveReach::Native,
            profiles,
            is_running,
            sandboxed_installs: Vec::new(),
            backups: PathBuf::from("/home/demo/.config/winspace/brave-backups"),
        }
    }

    fn profile() -> BraveProfile {
        BraveProfile {
            id: "Brave-Browser:Default".to_owned(),
            name: "Personal".to_owned(),
            channel: BraveChannel::Stable,
            directory: PathBuf::from("/home/demo/.config/BraveSoftware/Brave-Browser/Default"),
            download_path: String::new(),
        }
    }

    /// Ported from the status lines of `braveDialog` in `desktop/ui/app.js`.
    ///
    /// parity: INT-019, INT-020
    #[test]
    fn the_status_says_whether_brave_can_be_changed_now() {
        assert_eq!(
            status_text(&status(vec![profile()], true)),
            "Brave is running. Quit it completely, then Recheck."
        );
        assert_eq!(
            status_text(&status(vec![profile()], false)),
            "Brave is closed. Select the profiles to update."
        );
        let mut nothing = status(Vec::new(), false);
        nothing.sandboxed_installs = vec![SandboxedBrave::Flatpak, SandboxedBrave::Snap];
        assert_eq!(
            status_text(&nothing),
            "No supported native profiles found. Set brave://settings/downloads manually. Flatpak / Snap \
             installations need manual browser settings."
        );
        let sandboxed = BraveStatus {
            reach: BraveReach::Sandboxed,
            ..status(Vec::new(), true)
        };
        assert_eq!(status_text(&sandboxed), BraveError::Sandboxed.to_string());
    }

    /// parity: INT-020
    #[test]
    fn a_sync_reports_each_profile_that_failed() {
        let complete = SyncOutcome {
            updated: vec!["Brave-Browser:Default".to_owned()],
            failures: Vec::new(),
            path: PathBuf::from("/home/demo/Downloads"),
            backups: PathBuf::new(),
        };
        assert_eq!(
            sync_report(&complete),
            Ok("Brave Downloads updated for 1 profile(s).".to_owned())
        );
        let partial = SyncOutcome {
            failures: vec![ProfileFailure {
                profile: "Brave-Browser:Profile 1".to_owned(),
                error: BraveError::ChangedDuringSync,
            }],
            ..complete
        };
        assert_eq!(
            sync_report(&partial),
            Err("1 updated. Brave started or its preferences changed. Close Brave and retry.".to_owned())
        );
    }

    #[gtk::test]
    fn a_profile_names_its_channel_and_folder() {
        let check = profile_check(&profile());
        assert_eq!(
            check.label().as_deref(),
            Some("Personal · Brave-Browser\nUses browser default")
        );
        assert!(check.is_active(), "every profile starts ticked");
    }
}
