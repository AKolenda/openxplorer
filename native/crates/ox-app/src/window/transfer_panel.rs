// SPDX-License-Identifier: AGPL-3.0-only
//! One action of the progress window: what the running operation is
//! doing, how far it is, Pause and Cancel (OPS-019, OPS-021, ARC-011).
//!
//! It began as a port of `#transfer` in `v2.0.0:desktop/ui/index.html`
//! and `updateTransfer` in `v2.0.0:desktop/ui/app.js`, a panel floating
//! over the folder pane; it is now an action of the separate
//! [`ProgressWindow`], as in Windows Explorer, where every running
//! operation of every window is one action of one window. Each action
//! shows one write: a file transfer, an extraction, a compression, a
//! restored copy of a previous version or another exclusive write.
//! Concurrent transfer jobs each have their own action, with Pause and
//! Cancel; exclusive writes use the window's primary one.
//!
//! The action says what it does ("Copying 1,718 items from Downloads to
//! Backup", each folder a link that opens it in a new window) once
//! [`TransferPanel::describe`] is told; until then, and for writes that
//! are never described, its first line is the operation's starting text
//! ("Recycling…", "Preparing extraction…"). Under it, "44%
//! complete" in large type, Pause and Cancel, and the bar. The
//! percentage follows the bytes when the free-space check measured them,
//! else the items, and never goes back.
//!
//! "More details" (the window's toggle) replaces the bar with the speed
//! graph ([`speed_graph`]) and adds the name of the item handled, the
//! time remaining and the items remaining ([`rate`]). As in Windows there
//! is no bar of the file being copied: a file's own progress never moves
//! the action's, so a full file is never read as the batch finishing
//! (OPS-020). Without a name, the line shows the worker's own
//! report ("Checking free space…", "Extracting a.txt").
//!
//! Pause turns the bar and graph the caution colour and becomes Resume,
//! as the Windows UX guidance asks. Cancel stops the operation and the
//! percentage reads "Cancelling…" until it stops; a report that arrives
//! meanwhile does not replace it. The bars, the graph and the buttons
//! are named for assistive technologies, and the bar carries the
//! worker's report as its value text. Nothing moves frame by frame: the
//! bar and the graph change only when the operation reports, and Pause
//! or Resume recolours the bar, the graph's area and its speed line on
//! the next frame, with no fade to catch halfway (ACC-010).
//!
//! While it runs, the action holds an [`OperationSession`] for the
//! window that started it, so the session does not log out or suspend in
//! the middle of the operation (INT-028) and the dock icon shows the
//! progress (INT-027).
//!
//! [`TransferPanel`] is a `GtkBox` subclass whose layout is the template
//! `resources/ui/transfer-panel.ui`.

mod action;
mod failure;
mod question;
mod rate;
mod speed_graph;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::transfer::{Cancellation, Progress, ProgressScope};

pub(crate) use action::{location_name, Action, Verb};
pub(crate) use failure::{FailureChoice, FailureView, FirstButton};
pub(crate) use question::SpaceTexts;

use crate::icons::{self, Icon};
use crate::operation_session::OperationSession;
use crate::window::progress_window::ProgressWindow;
use crate::window::BrowserWindow;

/// The edge of the Pause and Cancel glyphs.
const GLYPH_SIZE: i32 = 16;

/// What the action says while the operation stops (`fire('cancel')` in
/// app.js).
const CANCELLING: &str = crate::i18n::message_id("Cancelling…");

/// What kind of write the panel shows, which its glyph tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferKind {
    /// Copying, moving, deleting or restoring files.
    Files,
    /// Extracting or compressing a ZIP archive.
    Archive,
    /// Creating a new item, such as New ▸ Text document on a slow share,
    /// which cannot pause.
    Create,
}

impl TransferKind {
    /// The glyph of the progress window while it shows this kind.
    pub(crate) const fn glyph(self) -> Icon {
        match self {
            TransferKind::Files => Icon::Copy,
            TransferKind::Archive => Icon::FolderZip,
            TransferKind::Create => Icon::DocumentAdd,
        }
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::transfer::{Cancellation, ItemCount};

    use super::action::Action;
    use super::question::Row;
    use super::rate::{Rate, Reading};
    use super::speed_graph::SpeedGraph;
    use super::TransferKind;
    use crate::operation_session::OperationSession;
    use crate::window::conflict_window::FileConflictWindow;
    use crate::window::progress_window::ProgressWindow;
    use crate::window::BrowserWindow;

    /// Private state of [`super::TransferPanel`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/transfer-panel.ui")]
    pub(crate) struct TransferPanel {
        /// What the action does, with its folders as links.
        #[template_child]
        pub(super) header_label: TemplateChild<gtk::Label>,
        /// "44% complete", or "Cancelling…"; while the action asks, what
        /// it found ("The destination already has a file named …").
        #[template_child]
        pub(super) percent_label: TemplateChild<gtk::Label>,
        /// "Replace or Skip Files": the three command rows.
        #[template_child]
        pub(super) question: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) replace_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) replace_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub(super) replace_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) skip_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) skip_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub(super) skip_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) decide_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) decide_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub(super) decide_label: TemplateChild<gtk::Label>,
        /// "There is not enough space on …": what is needed, the drive's
        /// space, Try Again and Cancel.
        #[template_child]
        pub(super) space_question: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) space_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) space_drive_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) space_retry_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) space_cancel_button: TemplateChild<gtk::Button>,
        /// How far the action is, while the details are hidden.
        #[template_child]
        pub(super) progress_bar: TemplateChild<gtk::ProgressBar>,
        /// The speed graph, the name, the time and the items remaining.
        #[template_child]
        pub(super) details: TemplateChild<gtk::Box>,
        /// How fast it went, which takes the bar's place in the details.
        #[template_child]
        pub(super) graph: TemplateChild<SpeedGraph>,
        /// "Name: a.txt", or the worker's report.
        #[template_child]
        pub(super) name_label: TemplateChild<gtk::Label>,
        /// "Time remaining: About 25 seconds".
        #[template_child]
        pub(super) time_label: TemplateChild<gtk::Label>,
        /// "Items remaining: 1,030 (313 MB)".
        #[template_child]
        pub(super) items_label: TemplateChild<gtk::Label>,
        /// Pauses or resumes the copy worker.
        #[template_child]
        pub(super) pause_button: TemplateChild<gtk::Button>,
        /// Its glyph: Pause or Play.
        #[template_child]
        pub(super) pause_icon: TemplateChild<gtk::Image>,
        /// Stops the operation.
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        /// Its glyph.
        #[template_child]
        pub(super) cancel_icon: TemplateChild<gtk::Image>,
        /// The clock of the speed and time remaining.
        pub(super) rate: RefCell<Rate>,
        /// What the bytes said last.
        pub(super) reading: Cell<Option<Reading>>,
        /// The items done and in all, as last reported.
        pub(super) items: Cell<Option<ItemCount>>,
        /// The worker's last report, or the starting text.
        pub(super) status: RefCell<String>,
        /// What the action does, once described.
        pub(super) action: RefCell<Option<Action>>,
        /// What kind of write it is.
        pub(super) kind: Cell<Option<TransferKind>>,
        /// How far the action is (0–1); it never goes back.
        pub(super) fraction: Cell<f64>,
        /// Whether a byte total gives the percentage.
        pub(super) by_bytes: Cell<bool>,
        /// Stops the running operation; `None` while none runs.
        pub(super) cancel: RefCell<Option<Cancellation>>,
        /// The inhibitor and dock progress of the running operation.
        pub(super) session: RefCell<Option<OperationSession>>,
        /// The window that runs the operation.
        pub(super) owner: glib::WeakRef<BrowserWindow>,
        /// The progress window the action shows in.
        pub(super) window: glib::WeakRef<ProgressWindow>,
        /// Receives the row chosen while the action asks.
        pub(super) question_answer: RefCell<Option<async_channel::Sender<Option<Row>>>>,
        /// What "More details" compares while the action asks.
        pub(super) compared: RefCell<Option<gtk::Box>>,
        /// Receives Try Again (true) or Cancel while the action asks
        /// about a full drive.
        pub(super) space_answer: RefCell<Option<async_channel::Sender<bool>>>,
        /// The File Conflict window the question opened, while it shows.
        pub(super) conflict_window: glib::WeakRef<FileConflictWindow>,
        /// The question about a failed item, while it shows.
        pub(super) failure_question: RefCell<Option<gtk::Box>>,
        /// Its More details.
        pub(super) failure_details: RefCell<Option<gtk::Widget>>,
        /// Receives the answer about a failed item.
        pub(super) failure_answer: RefCell<Option<async_channel::Sender<super::failure::FailureChoice>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TransferPanel {
        const NAME: &'static str = "OxTransferPanel";
        type Type = super::TransferPanel;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            SpeedGraph::ensure_type();
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
            crate::icons::set_icon(
                &self.cancel_icon,
                crate::icons::Icon::Dismiss16,
                super::GLYPH_SIZE,
            );
            panel.show_pause_state(false);
            panel.connect_question();
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
            self.header_label.connect_activate_link(glib::clone!(
                #[weak]
                panel,
                #[upgrade_or]
                glib::Propagation::Stop,
                move |_, uri| {
                    panel.open_location(uri);
                    glib::Propagation::Stop
                }
            ));
        }
    }
    impl WidgetImpl for TransferPanel {}
    impl BoxImpl for TransferPanel {}
}

glib::wrapper! {
    /// One action of the progress window.
    pub(crate) struct TransferPanel(ObjectSubclass<imp::TransferPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl TransferPanel {
    /// An action of `owner`'s operations, which shows in the progress
    /// window of `owner`'s application while it runs.
    pub(crate) fn for_window(owner: &BrowserWindow) -> Self {
        let panel: Self = glib::Object::new();
        panel.imp().owner.set(Some(owner));
        panel
    }

    /// Shows the panel for a `kind` operation that starts with `label`,
    /// its bar empty, and that `cancel` stops.
    pub(crate) fn start(&self, kind: TransferKind, label: &str, cancel: Cancellation) {
        let imp = self.imp();
        imp.kind.set(Some(kind));
        imp.cancel.replace(Some(cancel));
        imp.cancel_button.set_sensitive(true);
        imp.pause_button.set_visible(kind == TransferKind::Files);
        imp.pause_button.set_sensitive(true);
        self.show_pause_state(false);
        imp.rate.replace(rate::Rate::default());
        imp.reading.set(None);
        imp.items.set(None);
        imp.action.replace(None);
        imp.fraction.set(0.0);
        imp.by_bytes.set(false);
        imp.graph.reset();
        imp.time_label.set_visible(false);
        imp.items_label.set_visible(false);
        imp.header_label.set_use_markup(false);
        imp.header_label.set_text(label);
        imp.progress_bar.set_fraction(0.0);
        self.show_percent();
        let owner = imp.owner.upgrade();
        if let Some(owner) = &owner {
            ProgressWindow::of(owner.context()).add(self);
        }
        let session = match &owner {
            Some(owner) => OperationSession::start(owner),
            None => OperationSession::start(self),
        };
        session.show_progress(0.0);
        imp.session.replace(Some(session));
        self.show_progress(&Progress {
            label: label.to_owned(),
            fraction: 0.0,
            scope: ProgressScope::Batch,
            bytes: None,
            items: None,
            name: None,
        });
        self.set_visible(true);
    }

    /// [`Self::start`] for an operation that already waited a moment
    /// before it showed anything, such as a slow New ▸ Text document: its
    /// progress window shows at once instead of waiting again.
    pub(crate) fn start_after_waiting(&self, kind: TransferKind, label: &str, cancel: Cancellation) {
        self.start(kind, label, cancel);
        if let Some(window) = self.imp().window.upgrade() {
            window.show_now();
        }
    }

    /// True while an operation runs.
    pub(crate) fn is_busy(&self) -> bool {
        self.imp().cancel.borrow().is_some()
    }

    /// True while the operation `cancel` stops is the one shown.
    pub(crate) fn runs(&self, cancel: &Cancellation) -> bool {
        self.imp()
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(|shown| shown.cancellable() == cancel.cancellable())
    }

    /// Shows the worker's report: its label, and its fraction, clamped to
    /// 0–1 as `updateTransfer` does, on the action's bar, never a file's.
    /// The label is the details' line, unless the report names its item,
    /// and the bar's value text. Byte reports give the speed, the time
    /// and the items remaining. After Cancel the action keeps saying
    /// "Cancelling…".
    pub(crate) fn show_progress(&self, progress: &Progress) {
        let imp = self.imp();
        let label = progress.label.as_str();
        imp.status.replace(label.to_owned());
        if let Some(bytes) = progress.bytes {
            let reading = imp.rate.borrow_mut().report(bytes);
            imp.reading.set(Some(reading));
            if let Some(total) = bytes.batch_size.filter(|total| *total > 0) {
                imp.by_bytes.set(true);
                self.advance(progress_share(bytes.batch_written, total));
            }
        }
        if let Some(items) = progress.items {
            imp.items.set(Some(items));
            self.count_items(items.total);
            // Without a byte total, as for a same-drive move, the items
            // done move the action.
            if !imp.by_bytes.get() && items.total > 0 {
                self.advance(progress_share(items.done, items.total));
            }
        }
        // A file's own fraction moves nothing: a full file is never shown
        // as the action finishing (OPS-020).
        let batch = progress.scope == ProgressScope::Batch;
        if batch && (!imp.by_bytes.get() || progress.fraction >= 1.0) {
            self.advance(progress.fraction.clamp(0.0, 1.0));
        }
        imp.progress_bar
            .update_property(&[gtk::accessible::Property::ValueText(label)]);
        let name = progress.name.as_deref().map_or_else(
            || label.to_owned(),
            |name| ox_core::i18n::format_message("Name: {name}", &[("name", name)]),
        );
        imp.name_label.set_text(&name);
        self.show_details_text();
    }

    /// Says what the action does, from now until it ends.
    pub(crate) fn describe(&self, action: Action) {
        let imp = self.imp();
        let markup = action.markup();
        imp.action.replace(Some(action));
        imp.header_label.set_markup(&markup);
        if let Some(items) = imp.items.get() {
            self.count_items(items.total);
        }
        self.refresh_window();
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
        imp.cancel_button.set_sensitive(false);
        imp.pause_button.set_sensitive(false);
        self.stop_asking();
        self.show_percent();
        self.refresh_window();
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
            self.show_pause_state(false);
        } else {
            cancel.pause();
            imp.rate.borrow_mut().pause();
            self.show_pause_state(true);
        }
    }

    /// Hides the panel when the operation has ended, and takes it out of
    /// the progress window, which closes with its last action.
    pub(crate) fn finish(&self) {
        let imp = self.imp();
        self.stop_asking();
        imp.cancel.replace(None);
        imp.session.replace(None);
        self.set_visible(false);
        self.remove_css_class("paused");
        if let Some(window) = imp.window.upgrade() {
            window.remove(self);
        }
    }

    /// Shows or hides "More details": the graph in the bar's place, the
    /// name, and the time and items remaining.
    /// While the action asks, its bar stays hidden either way.
    pub(crate) fn show_details(&self, shown: bool) {
        // While the action asks about a failed item, More details is
        // where the item is and where it was going.
        if self.asks_about_failure() {
            self.show_failure_details(shown);
            return;
        }
        let imp = self.imp();
        imp.details.set_visible(shown);
        imp.progress_bar.set_visible(!shown && !self.is_asking());
    }

    /// "44% complete", or "Cancelling…" once cancelled, as the progress
    /// window's title says for a single action.
    pub(crate) fn percent_text(&self) -> String {
        if self.is_cancelling() {
            return ox_core::i18n::gettext_static(CANCELLING).to_owned();
        }
        ox_core::i18n::format_message(
            "{percent}% complete",
            &[("percent", &percent_of(self.imp().fraction.get()).to_string())],
        )
    }

    /// The glyph of the kind of write, for the progress window's icon.
    pub(crate) fn glyph(&self) -> Icon {
        self.imp().kind.get().unwrap_or(TransferKind::Files).glyph()
    }

    /// Records the progress window this action shows in.
    pub(super) fn show_in(&self, window: Option<&ProgressWindow>) {
        self.imp().window.set(window);
    }

    /// Opens the folder of a link in a new window of the app, as clicking
    /// a folder of the Windows progress window opens it.
    pub(crate) fn open_location(&self, uri: &str) {
        if let Some(owner) = self.imp().owner.upgrade() {
            owner.open_in_new_window(uri);
        }
    }

    /// The count of the description becomes `total` items: once the
    /// free-space check counted the files of the folders, it counts
    /// those, as Windows does.
    fn count_items(&self, total: u64) {
        let imp = self.imp();
        let markup = {
            let mut action = imp.action.borrow_mut();
            let Some(action) = action.as_mut().filter(|action| action.count.is_some()) else {
                return;
            };
            if action.count == Some(total) || total == 0 {
                return;
            }
            action.count = Some(total);
            action.markup()
        };
        imp.header_label.set_markup(&markup);
    }

    /// Moves the action on to `fraction`, never back.
    fn advance(&self, fraction: f64) {
        let imp = self.imp();
        if fraction <= imp.fraction.get() {
            return;
        }
        imp.fraction.set(fraction);
        imp.progress_bar.set_fraction(fraction);
        if let Some(session) = imp.session.borrow().as_ref() {
            session.show_progress(fraction);
        }
        self.show_percent();
        self.refresh_window();
    }

    /// The percentage line, and the graph's progress.
    fn show_percent(&self) {
        let imp = self.imp();
        // While the action asks, the percentage's place holds what it
        // found; a report that arrives meanwhile waits for the answer.
        if !self.is_asking() {
            imp.percent_label.set_text(&self.percent_text());
        }
        let speed = imp
            .reading
            .get()
            .and_then(|reading| reading.speed)
            .map(|speed| (speed, rate::speed_text(speed)));
        imp.graph.show(imp.fraction.get(), speed);
    }

    /// The time and items remaining of the details.
    fn show_details_text(&self) {
        let imp = self.imp();
        let reading = imp.reading.get();
        let speed = reading
            .and_then(|reading| reading.speed)
            .map(|speed| (speed, rate::speed_text(speed)));
        imp.graph.show(imp.fraction.get(), speed);
        let knows_total = reading.is_some_and(|reading| reading.knows_total);
        imp.time_label.set_visible(knows_total);
        if knows_total {
            imp.time_label
                .set_text(&rate::time_text(reading.and_then(|reading| reading.time_left)));
        }
        let items = imp.items.get();
        imp.items_label.set_visible(items.is_some());
        if let Some(items) = items {
            let left = items.total.saturating_sub(items.done);
            imp.items_label
                .set_text(&rate::items_text(left, reading.as_ref()));
        }
    }

    /// Shows Pause, or Resume while `paused`, and the caution colour of a
    /// paused action.
    fn show_pause_state(&self, paused: bool) {
        let imp = self.imp();
        let (icon, name) = if paused {
            (Icon::Play16, ox_core::i18n::gettext_static("Resume"))
        } else {
            (Icon::Pause16, ox_core::i18n::gettext_static("Pause"))
        };
        icons::set_icon(&imp.pause_icon, icon, GLYPH_SIZE);
        imp.pause_button.set_tooltip_text(Some(name));
        imp.pause_button
            .update_property(&[gtk::accessible::Property::Label(name)]);
        if paused {
            self.add_css_class("paused");
        } else {
            self.remove_css_class("paused");
        }
        // The graph's picture is drawn in the skin's colours: draw it again
        // in the new ones now, as no report comes while the action is
        // paused.
        imp.graph.recolour();
    }

    /// Tells the progress window that the action changed.
    fn refresh_window(&self) {
        if let Some(window) = self.imp().window.upgrade() {
            window.refresh();
        }
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

    /// What the action reports: the worker's last report, or
    /// "Cancelling…" once cancelled, for tests.
    #[cfg(test)]
    pub(crate) fn status_text(&self) -> String {
        if self.is_cancelling() {
            return self.percent_text();
        }
        self.imp().status.borrow().clone()
    }

    /// The text of the description, for tests.
    #[cfg(test)]
    pub(crate) fn header_text(&self) -> String {
        self.imp().header_label.text().to_string()
    }

    /// The action's progress, for tests.
    #[cfg(test)]
    pub(crate) fn fraction(&self) -> f64 {
        self.imp().fraction.get()
    }

    /// Presses Cancel, for tests.
    #[cfg(test)]
    pub(crate) fn press_cancel(&self) {
        self.imp().cancel_button.emit_clicked();
    }

    /// Whether Cancel shows, for tests.
    #[cfg(test)]
    pub(crate) fn shows_cancel(&self) -> bool {
        self.imp().cancel_button.get_visible()
    }

    /// Presses Pause or Resume, for tests.
    #[cfg(test)]
    pub(crate) fn press_pause(&self) {
        self.imp().pause_button.emit_clicked();
    }

    /// Follows the description's link to `uri`, for tests.
    #[cfg(test)]
    pub(crate) fn follow_link(&self, uri: &str) {
        let _: bool = self.imp().header_label.emit_by_name("activate-link", &[&uri]);
    }
}

/// `part` of `whole` as a fraction, at most 1.
#[expect(
    clippy::cast_precision_loss,
    reason = "a progress bar needs far less precision than f64 keeps"
)]
fn progress_share(part: u64, whole: u64) -> f64 {
    (part as f64 / whole as f64).min(1.0)
}

/// The whole percentage of `fraction`, rounded down so 100 % means done.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a percentage between 0 and 100"
)]
fn percent_of(fraction: f64) -> u32 {
    (fraction.clamp(0.0, 1.0) * 100.0).floor() as u32
}

#[cfg(test)]
mod tests {
    use ox_core::transfer::{ByteProgress, ItemCount};

    use super::*;

    /// A report of the whole batch at `fraction`.
    fn batch(label: &str, fraction: f64) -> Progress {
        Progress {
            label: label.to_owned(),
            fraction,
            scope: ProgressScope::Batch,
            bytes: None,
            items: None,
            name: None,
        }
    }

    /// A report of `written` of `size` bytes while copying `name`, with
    /// `done` of `total` items finished.
    fn bytes(name: &str, written: u64, size: u64, done: u64, total: u64) -> Progress {
        Progress {
            label: format!("Copying {name}"),
            fraction: 0.5,
            scope: ProgressScope::File,
            bytes: Some(ByteProgress {
                file_written: 1,
                file_size: 2,
                batch_written: written,
                batch_size: Some(size),
                file_started: false,
            }),
            items: Some(ItemCount { done, total }),
            name: Some(name.to_owned()),
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
        let glyph = panel.glyph();
        let started = (panel.is_visible(), panel.is_busy(), panel.status_text());
        let pause_shown = panel.imp().pause_button.is_visible();
        panel.show_progress(&batch("Extracting a.txt", 0.4));
        let reported = panel.status_text();
        panel.press_cancel();
        panel.show_progress(&batch("Extracting b.txt", 0.8));
        let after_cancel = panel.status_text();
        panel.finish();

        assert_eq!(glyph, Icon::FolderZip);
        assert_eq!(started, (true, true, "Preparing extraction…".to_owned()));
        assert!(!pause_shown, "an extraction cannot pause");
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

        assert_eq!(panel.glyph(), Icon::Copy);
        assert_eq!(panel.status_text(), "Preparing copy…");
        assert_eq!(panel.header_text(), "Preparing copy…");
        assert_eq!(panel.percent_text(), "0% complete");
        assert!(panel.imp().cancel_button.is_sensitive());
    }

    /// A described action says what it does with links to its folders,
    /// "44% complete" beside Pause and Cancel, and counts the files once
    /// the free-space check counted them.
    ///
    /// parity: OPS-019
    #[gtk::test]
    fn an_action_says_what_it_does_and_how_far_it_is() {
        let panel: TransferPanel = glib::Object::new();
        let uris: Vec<String> = (0..3)
            .map(|n| format!("file:///home/sample/Downloads/{n}.txt"))
            .collect();
        panel.start(TransferKind::Files, "Preparing copy…", Cancellation::new());
        panel.describe(Action::of(Verb::Copying, &uris, Some("file:///media/sample/Backup")).unwrap());
        let described = panel.header_text();

        panel.show_progress(&batch("Copy: 0.txt (1/3)", 0.44));
        let percent = panel.percent_text();
        panel.show_progress(&bytes("0.txt", 10, 1000, 0, 1718));

        assert_eq!(described, "Copying 3 items from Downloads to Backup");
        assert_eq!(percent, "44% complete");
        assert_eq!(
            panel.header_text(),
            "Copying 1,718 items from Downloads to Backup"
        );
        assert_eq!(
            panel.percent_text(),
            "44% complete",
            "never back to the bytes' 1 %"
        );
        panel.show_progress(&bytes("0.txt", 500, 1000, 1, 1718));
        assert_eq!(
            panel.percent_text(),
            "50% complete",
            "the bytes give the percentage"
        );
        panel.show_progress(&batch("Copy: 1.txt (2/3)", 0.33));
        assert_eq!(panel.percent_text(), "50% complete");
        panel.show_progress(&batch("3 item(s) completed", 1.0));
        assert_eq!(panel.percent_text(), "100% complete");
        panel.finish();
    }

    /// Compressing one large file moves the bar, the "N% complete" title,
    /// the speed and the details block by block, as a copy does, rather
    /// than staying at 0 % until the ZIP is finished.
    ///
    /// parity: ARC-023, OPS-021
    #[gtk::test]
    fn compressing_one_large_file_moves_the_bar_and_fills_the_details() {
        use std::sync::{Arc, Mutex};

        use ox_core::archive::{CompressionRequest, ZipCompressor};

        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("harbour-survey.iso");
        // Bytes that do not compress well, in many 64 KiB blocks.
        let mut state = 0x9e37_79b9_u32;
        let data: Vec<u8> = (0..2_000_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect();
        std::fs::write(&file, data).unwrap();
        let reports: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let collected = Arc::clone(&reports);
        ZipCompressor::new()
            .with_progress(move |progress| collected.lock().unwrap().push(progress))
            .compress(
                &CompressionRequest {
                    uris: vec![gio::File::for_path(&file).uri().to_string()],
                    destination_uri: gio::File::for_path(root.path()).uri().to_string(),
                    archive_name: "harbour-survey.zip".to_owned(),
                },
                &Cancellation::new(),
            )
            .unwrap();
        let reports = reports.lock().unwrap();
        let panel: TransferPanel = glib::Object::new();
        panel.start(
            TransferKind::Archive,
            "Preparing compression…",
            Cancellation::new(),
        );
        panel.show_details(true);
        let imp = panel.imp();

        let (first_half, second_half) = reports.split_at(reports.len() / 2);
        let first_bytes = first_half
            .iter()
            .position(|report| report.bytes.is_some())
            .expect("byte reports in the first half");
        let (before, after) = first_half.split_at(first_bytes + 1);
        for report in before {
            panel.show_progress(report);
        }
        // The speed is known once a quarter of a second has passed.
        std::thread::sleep(std::time::Duration::from_millis(300));
        for report in after {
            panel.show_progress(report);
        }

        let halfway = panel.percent_text();
        assert_ne!(halfway, "0% complete");
        assert_ne!(halfway, "100% complete");
        assert_eq!(imp.name_label.text(), "Name: harbour-survey.iso");
        let items = imp.items_label.text().to_string();
        assert!(
            imp.items_label.is_visible() && items.starts_with("Items remaining: 1 ("),
            "{items}"
        );
        assert!(imp.time_label.is_visible(), "the time remaining");
        let (graphed, speed) = imp.graph.shown();
        assert!(
            graphed > 0.0 && speed.is_some_and(|speed| speed > 0),
            "the speed graph"
        );
        for report in second_half {
            panel.show_progress(report);
        }
        assert_eq!(panel.percent_text(), "100% complete");
        assert_eq!(imp.items_label.text(), "Items remaining: 0 (0 bytes)");
        panel.finish();
    }

    /// An action without a byte total, such as a same-drive move, moves
    /// on with its items rather than staying at 0 % until it ends.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn an_action_without_a_byte_total_moves_on_with_its_items() {
        let panel: TransferPanel = glib::Object::new();
        panel.start(TransferKind::Files, "Moving items…", Cancellation::new());

        panel.show_progress(&Progress {
            items: Some(ItemCount { done: 3, total: 10 }),
            ..batch("Move: 3.txt (4/10)", 0.0)
        });

        assert_eq!(panel.percent_text(), "30% complete");
        panel.finish();
    }

    /// The details name the item and say the time and items remaining;
    /// a report without a name shows the worker's own words.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn the_details_name_the_item_and_say_what_remains() {
        let panel: TransferPanel = glib::Object::new();
        panel.start(TransferKind::Files, "Preparing copy…", Cancellation::new());
        panel.show_details(true);
        let imp = panel.imp();

        panel.show_progress(&batch("Checking free space…", 0.0));
        let checking = imp.name_label.text().to_string();
        panel.show_progress(&bytes("Holiday.jpg", 0, 328_204_288 + 10, 688, 1718));

        assert_eq!(checking, "Checking free space…");
        assert_eq!(imp.name_label.text(), "Name: Holiday.jpg");
        assert_eq!(imp.items_label.text(), "Items remaining: 1,030 (313 MB)");
        assert_eq!(imp.time_label.text(), "Time remaining: Calculating…");
        assert!(imp.graph.is_visible() && imp.details.is_visible());
        assert!(!imp.progress_bar.is_visible(), "the graph takes the bar's place");
        panel.show_details(false);
        assert!(imp.progress_bar.is_visible() && !imp.details.is_visible());
        panel.finish();
    }

    /// The bar and the graph's speed line are where each report puts
    /// them at once, with or without animations: nothing moves frame by
    /// frame, so reduced motion has nothing to stop.
    ///
    /// parity: ACC-010
    #[gtk::test]
    fn the_bar_and_the_speed_line_never_move_on_their_own() {
        let settings = gtk::Settings::default().expect("a display");
        let before = settings.is_gtk_enable_animations();
        let panel: TransferPanel = glib::Object::new();
        let window = gtk::Window::builder().child(&panel).build();
        window.set_visible(true);
        panel.start(TransferKind::Files, "Preparing copy…", Cancellation::new());
        panel.show_details(true);
        let imp = panel.imp();
        crate::test_support::harness::wait_until("the graph to show", || imp.graph.is_mapped());

        let mut seen = Vec::new();
        for (animations, written) in [(false, 400), (true, 700)] {
            settings.set_gtk_enable_animations(animations);
            panel.show_progress(&bytes("a.txt", written, 1000, 0, 2));
            seen.push((imp.progress_bar.fraction(), imp.graph.shown().0));
        }
        settings.set_gtk_enable_animations(before);
        panel.finish();
        window.destroy();

        assert_eq!(seen, [(0.4, 0.4), (0.7, 0.7)]);
    }

    /// The straight RGBA colour of `widget`, rendered by GTK, at (`x`,
    /// `y`), fractions of its size.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a pixel inside a small test widget"
    )]
    fn rendered_pixel(widget: &impl IsA<gtk::Widget>, x: f64, y: f64) -> [u8; 4] {
        let widget = widget.upcast_ref::<gtk::Widget>();
        let (width, height) = (widget.width(), widget.height());
        let paintable = gtk::WidgetPaintable::new(Some(widget));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
        let node = snapshot.to_node().expect("the widget draws something");
        let renderer = widget
            .native()
            .and_then(|native| native.renderer())
            .expect("a realized window");
        let texture = renderer.render_texture(&node, None);
        let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
        downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let column = ((x * f64::from(texture.width())) as usize).min(texture.width() as usize - 1);
        let row = ((y * f64::from(texture.height())) as usize).min(texture.height() as usize - 1);
        let at = row * stride + column * 4;
        bytes[at..at + 4].try_into().expect("four channels")
    }

    /// Whether the RGB of `pixel` is `expected`, give or take rounding.
    fn is_colour(pixel: [u8; 4], expected: [u8; 3]) -> bool {
        pixel[..3]
            .iter()
            .zip(expected)
            .all(|(channel, expected)| channel.abs_diff(expected) <= 3)
    }

    /// A shown panel in its own window, with a copy half done and
    /// `details` open or closed, and the action's caution and accent
    /// colours, read from the skin's swatches.
    fn paused_fixture(
        details: bool,
    ) -> (
        crate::test_support::harness::TestWindow,
        gtk::Window,
        TransferPanel,
    ) {
        // The browsing window loads the skin, whose colours the action uses.
        let skinned = crate::test_support::harness::TestWindow::without_tabs();
        let panel: TransferPanel = glib::Object::new();
        let window = gtk::Window::builder().child(&panel).default_width(400).build();
        window.set_visible(true);
        panel.start(TransferKind::Files, "Preparing copy…", Cancellation::new());
        panel.show_details(details);
        panel.show_progress(&bytes("a.txt", 250, 1000, 0, 2));
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
        panel.show_progress(&bytes("a.txt", 500, 1000, 0, 2));
        crate::test_support::harness::wait_until("the panel to be drawn", || {
            panel.is_mapped() && (!details || panel.imp().graph.texture_size().is_some())
        });
        crate::test_support::harness::wait_for_frames(&window, 4);
        (skinned, window, panel)
    }

    /// The colour of the graph's area well inside the part done, and of
    /// its speed line.
    fn graph_colours(panel: &TransferPanel) -> ([u8; 4], [u8; 4]) {
        let graph = &panel.imp().graph;
        let line = graph.line_fraction().expect("a speed is known");
        let area = graph.texture_pixel(0.2, 0.97).expect("the graph is drawn");
        let line = graph.texture_pixel(0.8, line).expect("the graph is drawn");
        (area, line)
    }

    /// Pause turns the graph's area and its speed line the caution
    /// colour at once, with More details open.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn pause_turns_the_graph_the_caution_colour_at_once() {
        let _theme = crate::test_support::harness::ThemeGuard::keep();
        let mut seen = Vec::new();
        for theme in ["light", "dark"] {
            let (skinned, window, panel) = paused_fixture(true);
            skinned.activate("theme", Some(theme));
            crate::test_support::harness::wait_for_frames(&window, 3);
            let accent = panel.imp().graph.area_rgb();

            panel.press_pause();
            crate::test_support::harness::wait_for_frames(&window, 2);
            let caution = panel.imp().graph.area_rgb();
            let (area, line) = graph_colours(&panel);
            panel.finish();
            window.destroy();
            seen.push((theme, accent, caution, area, line));
        }

        for (theme, accent, caution, area, line) in seen {
            assert_ne!(caution, accent, "{theme}: the skin has a caution colour");
            assert!(
                is_colour(area, caution),
                "{theme}: the area is {area:?}, not {caution:?}"
            );
            assert!(
                is_colour(line, caution),
                "{theme}: the line is {line:?}, not {caution:?}"
            );
        }
    }

    /// Resume turns the graph back to the accent colour and its line back
    /// to the text colour at once.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn resume_turns_the_graph_back_at_once() {
        let (_skinned, window, panel) = paused_fixture(true);
        let (area_before, line_before) = graph_colours(&panel);
        panel.press_pause();
        crate::test_support::harness::wait_for_frames(&window, 3);
        let caution = panel.imp().graph.area_rgb();
        let (paused_area, _) = graph_colours(&panel);

        panel.press_pause();
        crate::test_support::harness::wait_for_frames(&window, 2);
        let (area, line) = graph_colours(&panel);
        panel.finish();
        window.destroy();

        assert!(is_colour(paused_area, caution), "paused first: {paused_area:?}");
        assert_eq!(area, area_before, "the area is the accent colour again");
        assert_eq!(line, line_before, "the line is the text colour again");
    }

    /// With the details closed, Pause turns the bar's fill the caution
    /// colour on the next frame, with no fade caught halfway, and Resume
    /// turns it back the same way.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn pause_turns_the_bar_the_caution_colour_at_once() {
        let (_skinned, window, panel) = paused_fixture(false);
        let bar = panel.imp().progress_bar.clone();
        let accent = rendered_pixel(&bar, 0.25, 0.5);

        panel.press_pause();
        crate::test_support::harness::wait_for_frames(&window, 1);
        let paused = rendered_pixel(&bar, 0.25, 0.5);
        let caution = panel.imp().graph.area_rgb();
        panel.press_pause();
        crate::test_support::harness::wait_for_frames(&window, 1);
        let resumed = rendered_pixel(&bar, 0.25, 0.5);
        panel.finish();
        window.destroy();

        assert!(
            is_colour(paused, caution),
            "the paused bar is {paused:?}, not {caution:?}"
        );
        assert_eq!(resumed, accent, "the bar is the accent colour again");
    }

    /// The details have no bar of the file being copied, as Windows has
    /// none: the graph is the action's only bar there, and the action's
    /// only other bar is the plain one of Fewer details.
    ///
    /// parity: OPS-020
    #[gtk::test]
    fn the_details_have_no_current_file_bar() {
        fn bars(widget: &gtk::Widget) -> usize {
            let own = usize::from(widget.is::<gtk::ProgressBar>());
            let mut count = own;
            let mut child = widget.first_child();
            while let Some(next) = child {
                count += bars(&next);
                child = next.next_sibling();
            }
            count
        }
        let panel: TransferPanel = glib::Object::new();
        panel.start(TransferKind::Files, "Preparing copy…", Cancellation::new());
        panel.show_details(true);
        panel.show_progress(&bytes("a.txt", 5, 10, 0, 1));

        assert_eq!(bars(panel.upcast_ref()), 1, "only the action's own bar");
        panel.finish();
    }

    /// Pause becomes Resume, for the eye and for assistive technologies,
    /// and the action turns the caution colour; Resume reverses it.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn pause_becomes_resume_and_marks_the_action_paused() {
        let panel: TransferPanel = glib::Object::new();
        let cancel = Cancellation::new();
        panel.start(TransferKind::Files, "Preparing copy…", cancel.clone());
        let imp = panel.imp();

        panel.press_pause();
        let paused = (
            cancel.is_paused(),
            imp.pause_button.tooltip_text().map(|text| text.to_string()),
            panel.has_css_class("paused"),
            imp.pause_icon.icon_name().map(|name| name.to_string()),
        );
        panel.press_pause();

        assert_eq!(
            paused,
            (
                true,
                Some("Resume".to_owned()),
                true,
                Some(Icon::Play16.name().to_owned())
            )
        );
        assert!(!cancel.is_paused() && !panel.has_css_class("paused"));
        assert_eq!(imp.pause_button.tooltip_text().as_deref(), Some("Pause"));
        panel.finish();
    }
}
