// SPDX-License-Identifier: AGPL-3.0-only
//! The floating panel of a running extraction, compression or restored
//! copy of a previous version: what it is doing, how far it is, and
//! Cancel.
//!
//! Ports `#transfer` of `desktop/ui/index.html` and `updateTransfer` of
//! `desktop/ui/app.js` as the extraction uses them (ARC-011): it starts
//! with "Preparing extraction…", follows the extractor's reports, and
//! Cancel stops the work, which then leaves nothing behind. It floats over
//! the folder pane (`native/docs/ui-spec.md` §4.15, Transfer), so browsing
//! goes on around it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::transfer::Cancellation;

use crate::icons::{self, Icon};
use crate::window::ButtonStyle;

/// The label once the user cancelled.
const CANCELLING: &str = "Cancelling…";

/// The glyph's size (`#transfer-icon`).
const GLYPH_SIZE: i32 = 18;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::subclass::prelude::*;
    use ox_core::transfer::Cancellation;

    use crate::launcher_progress::LauncherProgress;
    use crate::write_inhibitor::WriteInhibitor;

    /// Private state of [`super::OperationPanel`].
    #[derive(Debug, Default)]
    pub(crate) struct OperationPanel {
        /// What the operation is doing.
        pub(super) label: gtk::Label,
        /// How far it is.
        pub(super) progress: gtk::ProgressBar,
        /// Stops it.
        pub(super) cancel_button: gtk::Button,
        /// The running operation's cancellation.
        pub(super) cancel: RefCell<Option<Cancellation>>,
        /// Keeps the session from logging out or suspending meanwhile
        /// (INT-028).
        pub(super) inhibitor: RefCell<Option<WriteInhibitor>>,
        /// The progress on the dock icon (INT-027).
        pub(super) launcher: RefCell<Option<LauncherProgress>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OperationPanel {
        const NAME: &'static str = "OxOperationPanel";
        type Type = super::OperationPanel;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for OperationPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for OperationPanel {}
    impl BoxImpl for OperationPanel {}
}

glib::wrapper! {
    /// The panel of a running extraction or compression.
    pub(crate) struct OperationPanel(ObjectSubclass<imp::OperationPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for OperationPanel {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl OperationPanel {
    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("operation-panel");
        self.set_visible(false);
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::End);
        self.append(&icons::image(Icon::FolderZip, GLYPH_SIZE));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.set_hexpand(true);
        imp.label.set_xalign(0.0);
        imp.label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        imp.label.set_accessible_role(gtk::AccessibleRole::Status);
        column.append(&imp.label);
        column.append(&imp.progress);
        self.append(&column);
        imp.cancel_button.set_label("Cancel");
        imp.cancel_button.add_css_class(ButtonStyle::Bordered.css_class());
        imp.cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.cancel()
        ));
        self.append(&imp.cancel_button);
    }

    /// True while an operation runs.
    pub(crate) fn is_busy(&self) -> bool {
        self.imp().cancel.borrow().is_some()
    }

    /// Shows the panel for an operation that starts with `label` and stops
    /// when `cancel` is cancelled.
    pub(crate) fn start(&self, label: &str, cancel: Cancellation) {
        let imp = self.imp();
        imp.cancel.replace(Some(cancel));
        imp.cancel_button.set_sensitive(true);
        imp.launcher
            .replace(crate::launcher_progress::LauncherProgress::for_widget(self));
        self.show_progress(label, 0.0);
        self.set_visible(true);
        imp.inhibitor
            .replace(crate::write_inhibitor::WriteInhibitor::hold(self));
    }

    /// Shows `label` and the bar at `fraction` (clamped to 0–1). A report
    /// after Cancel keeps "Cancelling…".
    pub(crate) fn show_progress(&self, label: &str, fraction: f64) {
        let imp = self.imp();
        let is_cancelling = imp
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(Cancellation::is_cancelled);
        if !is_cancelling {
            imp.label.set_text(label);
        }
        imp.progress.set_fraction(fraction.clamp(0.0, 1.0));
        if let Some(launcher) = imp.launcher.borrow().as_ref() {
            launcher.show(fraction);
        }
    }

    /// Stops the running operation.
    fn cancel(&self) {
        let imp = self.imp();
        if let Some(cancel) = imp.cancel.borrow().as_ref() {
            cancel.cancel();
        }
        imp.label.set_text(CANCELLING);
        imp.cancel_button.set_sensitive(false);
    }

    /// Hides the panel once the operation ended.
    pub(crate) fn finish(&self) {
        self.imp().cancel.replace(None);
        self.imp().inhibitor.replace(None);
        self.imp().launcher.replace(None);
        self.set_visible(false);
    }
}
