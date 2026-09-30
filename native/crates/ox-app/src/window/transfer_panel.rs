// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer panel: what the running file operation is doing, how far
//! it is, and Cancel (OPS-019).
//!
//! Ports `#transfer` in `desktop/ui/index.html`, `.transfer` in
//! `style.css` and `updateTransfer` in `desktop/ui/app.js`. The label
//! starts as the operation's starting text ("Moving to Trash…") and then
//! follows the transfer engine's reports ("Copy: a.txt (1/3)", "Copying
//! a.txt · 8,192 / 35,000 bytes"), which say whether the bar shows one
//! file or the whole batch. Cancel runs [`WindowAction::CancelOperation`]
//! and the label reads "Cancelling…" until the operation stops.
//!
//! While it shows, the panel holds an [`OperationSession`], so the
//! session does not log out or suspend in the middle of the operation
//! (INT-028) and the dock icon shows the progress (INT-027).
//!
//! [`TransferPanel`] is a `GtkBox` subclass whose layout is the template
//! `resources/ui/transfer-panel.ui`. It floats over the folder pane, so
//! browsing goes on around it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::window_action::WindowAction;
use crate::operation_session::OperationSession;

/// The copy glyph's edge (`#transfer-icon`).
const GLYPH_SIZE: i32 = 18;

/// The label while the operation stops (`fire('cancel')` in app.js).
const CANCELLING: &str = "Cancelling…";

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use crate::icons::{self, Icon};
    use crate::operation_session::OperationSession;

    /// Private state of [`super::TransferPanel`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/transfer-panel.ui")]
    pub(crate) struct TransferPanel {
        /// The copy glyph.
        #[template_child]
        pub(super) glyph: TemplateChild<gtk::Image>,
        /// What the operation is doing.
        #[template_child]
        pub(super) status_label: TemplateChild<gtk::Label>,
        /// How far it is.
        #[template_child]
        pub(super) progress_bar: TemplateChild<gtk::ProgressBar>,
        /// Stops the operation.
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        /// The inhibitor and dock progress of the running operation.
        pub(super) session: RefCell<Option<OperationSession>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TransferPanel {
        const NAME: &'static str = "OxTransferPanel";
        type Type = super::TransferPanel;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(panel: &glib::subclass::InitializingObject<Self>) {
            panel.init_template();
        }
    }

    impl ObjectImpl for TransferPanel {
        fn constructed(&self) {
            self.parent_constructed();
            icons::set_icon(&self.glyph, Icon::Copy, super::GLYPH_SIZE);
            super::WindowAction::CancelOperation.assign_to(&*self.cancel_button);
        }
    }

    impl WidgetImpl for TransferPanel {}
    impl BoxImpl for TransferPanel {}
}

glib::wrapper! {
    /// The floating panel of the running file operation.
    pub(crate) struct TransferPanel(ObjectSubclass<imp::TransferPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl TransferPanel {
    /// Shows the panel for an operation that starts with `label`, its bar
    /// empty.
    pub(super) fn start(&self, label: &str) {
        self.imp().session.replace(Some(OperationSession::start(self)));
        self.show_progress(label, 0.0);
        self.set_visible(true);
    }

    /// Shows the engine's report: `label` and the bar at `fraction`,
    /// clamped to 0–1 as `updateTransfer` does.
    pub(super) fn show_progress(&self, label: &str, fraction: f64) {
        let imp = self.imp();
        imp.status_label.set_text(label);
        imp.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
        imp.progress_bar
            .update_property(&[gtk::accessible::Property::ValueText(label)]);
        if let Some(session) = imp.session.borrow().as_ref() {
            session.show_progress(fraction);
        }
    }

    /// Says that the operation is stopping. The window shows no later
    /// report of a cancelled operation.
    pub(super) fn show_cancelling(&self) {
        self.imp().status_label.set_text(CANCELLING);
    }

    /// Hides the panel when the operation has ended.
    pub(super) fn finish(&self) {
        self.set_visible(false);
        self.imp().session.replace(None);
    }

    /// Whether the panel holds the session's inhibitor, for tests.
    #[cfg(test)]
    pub(crate) fn inhibits_logout(&self) -> bool {
        self.imp()
            .session
            .borrow()
            .as_ref()
            .is_some_and(OperationSession::inhibits_logout)
    }

    /// The label shown, for tests.
    #[cfg(test)]
    pub(crate) fn status_text(&self) -> String {
        self.imp().status_label.text().to_string()
    }
}
