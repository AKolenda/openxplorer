// SPDX-License-Identifier: AGPL-3.0-only
//! The transfer panel: what the running operation is doing, how far it
//! is, and Cancel (OPS-019, ARC-011).
//!
//! Ports `#transfer` in `v2.0.0:desktop/ui/index.html`, `.transfer` in
//! `style.css` and `updateTransfer` in `v2.0.0:desktop/ui/app.js`. Each panel
//! shows one write: a file transfer, an extraction, a compression or a
//! restored copy of a previous version. Concurrent transfer jobs each
//! have their own panel and Cancel button; exclusive writes use the
//! window's primary panel.
//! The label starts as the operation's starting text ("Moving to Trash…",
//! "Preparing extraction…") and then follows the worker's reports
//! ("Copy: a.txt (1/3)", "Copying a.txt · 8,192 / 35,000 bytes"). The
//! first bar shows the whole batch; while a file is copied a second,
//! thinner bar shows that file's bytes, so a full file bar is never read
//! as the batch finishing (OPS-020). Both bars are named for assistive
//! technologies and carry the label as their value text. Cancel stops the
//! operation and the label reads "Cancelling…" until it stops; a report
//! that arrives meanwhile does not replace it.
//!
//! While it shows, the panel holds an [`OperationSession`], so the
//! session does not log out or suspend in the middle of the operation
//! (INT-028) and the dock icon shows the progress (INT-027).
//!
//! [`TransferPanel`] is a `GtkBox` subclass whose layout is the template
//! `resources/ui/transfer-panel.ui`. It floats over the folder pane, so
//! browsing goes on around it.

mod rate;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::transfer::{Cancellation, Progress, ProgressScope};

use crate::icons::{self, Icon};
use crate::operation_session::OperationSession;

/// The glyph's edge (`#transfer-icon`).
const GLYPH_SIZE: i32 = 18;

/// The label while the operation stops (`fire('cancel')` in app.js).
const CANCELLING: &str = "Cancelling…";

/// What kind of write the panel shows, which its glyph tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferKind {
    /// Copying, moving, deleting or restoring files.
    Files,
    /// Extracting or compressing a ZIP archive.
    Archive,
}

impl TransferKind {
    /// The glyph shown at the panel's left.
    const fn glyph(self) -> Icon {
        match self {
            TransferKind::Files => Icon::Copy,
            TransferKind::Archive => Icon::FolderZip,
        }
    }
}

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::transfer::Cancellation;

    use crate::operation_session::OperationSession;

    /// Private state of [`super::TransferPanel`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/transfer-panel.ui")]
    pub(crate) struct TransferPanel {
        /// The glyph of the kind of write.
        #[template_child]
        pub(super) glyph: TemplateChild<gtk::Image>,
        /// What the operation is doing.
        #[template_child]
        pub(super) status_label: TemplateChild<gtk::Label>,
        /// How far the batch is.
        #[template_child]
        pub(super) progress_bar: TemplateChild<gtk::ProgressBar>,
        /// How far the file being copied is; hidden between files.
        #[template_child]
        pub(super) file_bar: TemplateChild<gtk::ProgressBar>,
        /// Processed size, speed and estimated remaining time.
        #[template_child]
        pub(super) rate_label: TemplateChild<gtk::Label>,
        /// Pauses or resumes the copy worker.
        #[template_child]
        pub(super) pause_button: TemplateChild<gtk::Button>,
        pub(super) rate: RefCell<super::rate::Rate>,
        /// Stops the operation.
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        /// Stops the running operation; `None` while none runs.
        pub(super) cancel: RefCell<Option<Cancellation>>,
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
            crate::i18n::translate_template(&*self.obj(), "transfer-panel.ui");
            let panel = self.obj();
            self.pause_button.connect_clicked(glib::clone!(
                #[weak]
                panel,
                move |_| panel.toggle_pause()
            ));
            self.cancel_button.connect_clicked(glib::clone!(
                #[weak]
                panel,
                move |_| panel.cancel()
            ));
        }
    }
    impl WidgetImpl for TransferPanel {}
    impl BoxImpl for TransferPanel {}
}

glib::wrapper! {
    /// The floating panel of the running operation.
    pub(crate) struct TransferPanel(ObjectSubclass<imp::TransferPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl TransferPanel {
    /// Shows the panel for a `kind` operation that starts with `label`,
    /// its bar empty, and that `cancel` stops.
    pub(crate) fn start(&self, kind: TransferKind, label: &str, cancel: Cancellation) {
        let imp = self.imp();
        icons::set_icon(&imp.glyph, kind.glyph(), GLYPH_SIZE);
        imp.cancel.replace(Some(cancel));
        imp.cancel_button.set_sensitive(true);
        imp.pause_button.set_visible(kind == TransferKind::Files);
        imp.pause_button.set_sensitive(true);
        imp.pause_button.set_label(ox_core::i18n::gettext_static("Pause"));
        imp.rate.replace(rate::Rate::default());
        imp.rate_label.set_visible(false);
        imp.session.replace(Some(OperationSession::start(self)));
        self.show_progress(&Progress {
            label: label.to_owned(),
            fraction: 0.0,
            scope: ProgressScope::Batch,
            bytes: None,
        });
        self.set_visible(true);
    }

    /// True while an operation runs.
    pub(crate) fn is_busy(&self) -> bool {
        self.imp().cancel.borrow().is_some()
    }

    /// Shows the worker's report: its label, and its fraction, clamped to
    /// 0–1 as `updateTransfer` does, on the batch bar or the file bar.
    /// After Cancel the label keeps saying "Cancelling…".
    pub(crate) fn show_progress(&self, progress: &Progress) {
        let imp = self.imp();
        let label = progress.label.as_str();
        if let Some(bytes) = progress.bytes {
            imp.rate_label.set_text(&imp.rate.borrow_mut().report(bytes));
            imp.rate_label.set_visible(true);
        }
        if !self.is_cancelling() {
            imp.status_label.set_text(label);
        }
        let bar = match progress.scope {
            ProgressScope::Batch => &imp.progress_bar,
            ProgressScope::File => &imp.file_bar,
        };
        bar.set_fraction(progress.fraction.clamp(0.0, 1.0));
        bar.update_property(&[gtk::accessible::Property::ValueText(label)]);
        imp.file_bar.set_visible(progress.scope == ProgressScope::File);
        if progress.scope == ProgressScope::Batch {
            if let Some(session) = imp.session.borrow().as_ref() {
                session.show_progress(progress.fraction);
            }
        }
    }

    /// Whether this panel still displays the job that owns this token.
    pub(crate) fn tracks(&self, cancel: &Cancellation) -> bool {
        self.imp()
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(|current| current.cancellable() == cancel.cancellable())
    }

    /// Stops the running operation, as its Cancel button does, and says
    /// that it is stopping. Does nothing while none runs.
    pub(crate) fn cancel(&self) {
        let imp = self.imp();
        let Some(cancel) = imp.cancel.borrow().clone() else {
            return;
        };
        cancel.cancel();
        imp.status_label.set_text(CANCELLING);
        imp.cancel_button.set_sensitive(false);
        imp.pause_button.set_sensitive(false);
    }

    /// Pause and resume are per job; cancellation always wakes a paused worker.
    pub(crate) fn toggle_pause(&self) {
        let imp = self.imp();
        let Some(cancel) = imp.cancel.borrow().clone() else {
            return;
        };
        if cancel.is_cancelled() {
            return;
        }
        if cancel.is_paused() {
            cancel.resume();
            imp.rate.borrow_mut().resume();
            imp.pause_button.set_label(ox_core::i18n::gettext_static("Pause"));
        } else {
            cancel.pause();
            imp.rate.borrow_mut().pause();
            imp.pause_button
                .set_label(ox_core::i18n::gettext_static("Resume"));
        }
    }

    /// Hides the panel when the operation has ended.
    pub(crate) fn finish(&self) {
        let imp = self.imp();
        imp.cancel.replace(None);
        imp.session.replace(None);
        self.set_visible(false);
    }

    /// Whether the running operation was cancelled.
    fn is_cancelling(&self) -> bool {
        self.imp()
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(Cancellation::is_cancelled)
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

    /// The batch bar's fraction, and the file bar's while it shows, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn fractions(&self) -> (f64, Option<f64>) {
        let imp = self.imp();
        let file = imp.file_bar.is_visible().then(|| imp.file_bar.fraction());
        (imp.progress_bar.fraction(), file)
    }

    /// Presses Cancel, for tests.
    #[cfg(test)]
    pub(crate) fn press_cancel(&self) {
        self.imp().cancel_button.emit_clicked();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report of the whole batch at `fraction`.
    fn batch(label: &str, fraction: f64) -> Progress {
        Progress {
            label: label.to_owned(),
            fraction,
            scope: ProgressScope::Batch,
            bytes: None,
        }
    }

    /// An extraction shows in the same panel as a file operation, with
    /// the archive glyph; its Cancel stops it, and a late report keeps
    /// saying so.
    ///
    /// parity: ARC-011
    #[gtk::test]
    fn an_archive_operation_shows_its_glyph_and_cancel_stops_it() {
        let panel: TransferPanel = glib::Object::new();
        let cancel = Cancellation::new();

        panel.start(TransferKind::Archive, "Preparing extraction…", cancel.clone());
        let glyph = panel.imp().glyph.icon_name();
        let started = (panel.is_visible(), panel.is_busy(), panel.status_text());
        panel.show_progress(&batch("Extracting a.txt", 0.4));
        let reported = panel.status_text();
        panel.press_cancel();
        panel.show_progress(&batch("Extracting b.txt", 0.8));
        let after_cancel = panel.status_text();
        panel.finish();

        assert_eq!(glyph.as_deref(), Some(Icon::FolderZip.name()));
        assert_eq!(started, (true, true, "Preparing extraction…".to_owned()));
        assert_eq!(reported, "Extracting a.txt");
        assert!(cancel.is_cancelled());
        assert_eq!(after_cancel, "Cancelling…");
        assert!(!panel.is_visible() && !panel.is_busy());
    }

    #[gtk::test]
    fn a_file_operation_after_a_cancelled_one_shows_its_glyph_and_can_be_cancelled() {
        let panel: TransferPanel = glib::Object::new();
        panel.start(
            TransferKind::Archive,
            "Preparing compression…",
            Cancellation::new(),
        );
        panel.press_cancel();
        panel.finish();

        panel.start(TransferKind::Files, "Preparing copy…", Cancellation::new());

        assert_eq!(panel.imp().glyph.icon_name().as_deref(), Some(Icon::Copy.name()));
        assert_eq!(panel.status_text(), "Preparing copy…");
        assert!(panel.imp().cancel_button.is_sensitive());
    }
}
