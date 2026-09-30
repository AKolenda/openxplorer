// SPDX-License-Identifier: AGPL-3.0-only
//! "Software updates": the installed and available versions, a live
//! status, and Close, Check again, Install update… and Restart now.
//!
//! Ports `updatesDialog` in `desktop/ui/app.js` (UPD-001, UPD-003,
//! UPD-005 to UPD-007). Opening it checks at once; release notes are
//! never shown. "Install update…" shows only when a newer release exists
//! and is enabled only when this build may install it; a build that
//! cannot (a Flatpak, another package, a source build) says what updates
//! it instead. While an update installs the dialog cannot be closed,
//! Escape does nothing and keyboard focus stays on the status line.
//!
//! [`UpdateDialog`] is a `GtkWindow` subclass laid out by the template
//! `resources/ui/update-dialog.ui`. It shows the shared [`Updates`] state,
//! so a check started in another window shows here too.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::update::Activity;

use super::{UpdateState, Updates};

/// Why "Install update…" was refused before asking anything.
const WORK_RUNNING: &str = "Finish file operations before installing the update.";

/// Says whether any window has work running that an installation must
/// not interrupt.
type WorkCheck = Box<dyn Fn() -> Activity>;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::WorkCheck;
    use crate::update::Updates;

    /// Private state of [`super::UpdateDialog`].
    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/update-dialog.ui")]
    pub(crate) struct UpdateDialog {
        /// The scrolling body, capped to the parent window's height.
        #[template_child]
        pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
        /// "Installed: 1.1.4 · Available: 1.2.0".
        #[template_child]
        pub(super) versions_label: TemplateChild<gtk::Label>,
        /// The live status line.
        #[template_child]
        pub(super) status_label: TemplateChild<gtk::Label>,
        /// What updates a build that cannot install itself.
        #[template_child]
        pub(super) hint_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) close_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) check_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) install_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) restart_button: TemplateChild<gtk::Button>,
        /// The application's updates, set when the dialog is made.
        pub(super) updates: OnceCell<Updates>,
        /// Whether any window has work running.
        pub(super) work: OnceCell<WorkCheck>,
        /// The handler on the updates' state, disconnected on dispose.
        pub(super) state_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// A refusal shown instead of the state's status until the state
        /// changes.
        pub(super) refusal: RefCell<Option<&'static str>>,
    }

    impl std::fmt::Debug for UpdateDialog {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("UpdateDialog")
                .field("updates", &self.updates)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UpdateDialog {
        const NAME: &'static str = "OxUpdateDialog";
        type Type = super::UpdateDialog;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
            dialog.init_template();
        }
    }

    impl ObjectImpl for UpdateDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().connect_buttons();
        }

        fn dispose(&self) {
            let handler = self.state_handler.take();
            if let (Some(updates), Some(handler)) = (self.updates.get(), handler) {
                updates.disconnect(handler);
            }
        }
    }

    impl WidgetImpl for UpdateDialog {
        /// Fits the dialog to its parent window before its first frame, as
        /// it is realized when it shows.
        fn realize(&self) {
            crate::modal::fit_to_parent(&*self.obj(), &self.scroller);
            self.parent_realize();
        }
    }

    impl WindowImpl for UpdateDialog {
        /// Close and Escape cannot dismiss the dialog while an update
        /// installs (UPD-005).
        fn close_request(&self) -> glib::Propagation {
            let installing = self
                .updates
                .get()
                .is_some_and(|updates| updates.state().is_installing());
            if installing {
                return glib::Propagation::Stop;
            }
            self.parent_close_request()
        }
    }
}

glib::wrapper! {
    /// The Software updates dialog.
    pub(crate) struct UpdateDialog(ObjectSubclass<imp::UpdateDialog>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager;
}

impl UpdateDialog {
    /// Opens the dialog over `parent` and checks for updates at once,
    /// unless a check or an installation runs or a restart is due.
    /// `work` says whether any window has work running.
    pub(crate) fn present_for(
        parent: &impl IsA<gtk::Window>,
        updates: &Updates,
        work: impl Fn() -> Activity + 'static,
    ) -> Self {
        let dialog: Self = glib::Object::builder().property("transient-for", parent).build();
        crate::window::follow_text_size_keys(&dialog);
        dialog.bind(updates, Box::new(work));
        // The check starts first, so the first frame already says so.
        let state = updates.state();
        if !state.is_busy() && !state.needs_restart() {
            updates.check();
        }
        dialog.present();
        dialog
    }

    fn bind(&self, updates: &Updates, work: WorkCheck) {
        let imp = self.imp();
        imp.updates.set(updates.clone()).expect("a dialog is bound once");
        assert!(imp.work.set(work).is_ok(), "a dialog is bound once");
        let handler = updates.connect_state_changed(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.imp().refusal.take();
                dialog.show_state();
            }
        ));
        imp.state_handler.replace(Some(handler));
        self.show_state();
    }

    fn updates(&self) -> &Updates {
        self.imp()
            .updates
            .get()
            .expect("UpdateDialog::present_for binds the updates")
    }

    fn connect_buttons(&self) {
        let imp = self.imp();
        imp.close_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.close()
        ));
        imp.check_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.updates().check()
        ));
        imp.install_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.install()
        ));
        imp.restart_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.updates().restart(dialog.work())
        ));
        self.add_controller(crate::modal::escape_closes());
    }

    /// Whether any window has work running.
    fn work(&self) -> Activity {
        self.imp().work.get().map_or(Activity::Idle, |work| work())
    }

    /// "Install update…": refused while work runs; otherwise the click
    /// is the user's confirmation, and the dialog keeps keyboard focus on
    /// the status while the update installs.
    fn install(&self) {
        let imp = self.imp();
        if self.work() == Activity::Busy {
            imp.refusal.replace(Some(WORK_RUNNING));
            self.show_state();
            return;
        }
        self.updates().install(Activity::Idle);
        imp.status_label.grab_focus();
    }

    /// Shows the shared state: the texts, which buttons show, and which
    /// are enabled (`sync` in `updatesDialog`).
    fn show_state(&self) {
        let imp = self.imp();
        let state = self.updates().state();
        imp.versions_label
            .set_text(&state.versions_text(Updates::running_version()));
        let status = imp.refusal.borrow().map(str::to_owned);
        imp.status_label
            .set_text(&status.unwrap_or_else(|| state.status_text()));
        let hint = state.installation_hint();
        imp.hint_label.set_visible(hint.is_some());
        imp.hint_label.set_text(hint.unwrap_or_default());
        self.show_buttons(&state);
        crate::modal::fit_to_parent(self, &imp.scroller);
    }

    fn show_buttons(&self, state: &UpdateState) {
        let imp = self.imp();
        let busy = state.is_busy();
        let needs_restart = state.needs_restart();
        let available = state.available();
        imp.close_button.set_sensitive(!state.is_installing());
        imp.check_button.set_sensitive(!busy && !needs_restart);
        imp.install_button
            .set_visible(available.is_some() && !needs_restart);
        let can_install = available.is_some_and(|update| update.installation.can_install());
        imp.install_button.set_sensitive(!busy && can_install);
        imp.restart_button.set_visible(needs_restart);
        imp.restart_button.set_sensitive(!busy);
        if needs_restart && !busy {
            imp.restart_button.grab_focus();
        }
    }

    /// The status line, for tests.
    #[cfg(test)]
    pub(crate) fn status(&self) -> String {
        self.imp().status_label.text().to_string()
    }

    /// What updates a build that cannot install itself, when shown, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn hint(&self) -> Option<String> {
        let hint = &self.imp().hint_label;
        hint.is_visible().then(|| hint.text().to_string())
    }

    /// The buttons shown, by label, for tests.
    #[cfg(test)]
    pub(crate) fn shown_buttons(&self) -> Vec<(String, bool)> {
        let imp = self.imp();
        let buttons = [
            &imp.close_button,
            &imp.check_button,
            &imp.install_button,
            &imp.restart_button,
        ];
        buttons
            .into_iter()
            .filter(|button| button.is_visible())
            .map(|button| {
                (
                    button.label().unwrap_or_default().to_string(),
                    button.is_sensitive(),
                )
            })
            .collect()
    }

    /// Clicks the button labelled `label`, for tests.
    #[cfg(test)]
    pub(crate) fn click(&self, label: &str) {
        let imp = self.imp();
        let buttons = [
            &imp.close_button,
            &imp.check_button,
            &imp.install_button,
            &imp.restart_button,
        ];
        let button = buttons
            .into_iter()
            .find(|button| button.label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("the dialog has a {label} button"));
        button.emit_clicked();
    }
}
