// SPDX-License-Identifier: AGPL-3.0-only
//! The progress window: every running file operation of the application,
//! each an action with its own Pause and Cancel ([`TransferPanel`]), in
//! one small window of its own, as Windows Explorer shows them (OPS-019,
//! OPS-025).
//!
//! Windows 8 and later "present all pending copy jobs in this single
//! dialog", each with pause, resume and stop, and an optional detailed
//! view with a throughput graph (Building Windows 8, "Improving our file
//! management basics: copy, move, rename, and delete", 2011). The
//! window's title puts what distinguishes it first, for the taskbar:
//! "44% complete" for one action, as the Windows UX guidance gives as its
//! example, and "2 Running Actions" for several, as Windows titles it.
//!
//! "More details" / "Fewer details" applies to every action and is
//! remembered across sessions, as in Windows ("'More details' is sticky
//! … across sessions", a Windows team reply under that post), in the
//! `progressDetails` preference. While the window's one action asks
//! Replace or Skip Files, the toggle shows greyed, as Windows 11 shows
//! it: the two files are compared only in the File Conflict window that
//! "Compare info for both files" opens. The remembered choice comes back
//! with the progress once the question is answered.
//!
//! The window shows once an action has run for [`SHOW_DELAY`], so a
//! quick operation shows nothing, and it does not take the focus (the UX
//! guidance: "Don't steal input focus to show a progress update"). It
//! closes when its last action ends. Closing it cancels every action, as
//! each action's Cancel does: finished work stays (OPS-022). It belongs
//! to no browsing window and sets no position: the desktop places it, as
//! Wayland requires, and closing a browsing window still asks first
//! while that window's operations run (TAB-049). While an action started
//! in a file dialog runs, the window is that dialog's modeless child, as
//! Windows' common dialog owns the progress of what it started, so the
//! desktop keeps it above the dialog and lets it be used while the
//! dialog is modal for its caller.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::PreferencesUpdate;

use crate::app_context::AppContext;
use crate::icons::{self, Icon};
use crate::window::conflict_window::FileConflictWindow;
use crate::window::transfer_panel::TransferPanel;

/// How long an action runs before the window shows. Windows shows no
/// progress for an operation that ends at once; the UX guidance counts
/// "operations that take two seconds or longer" as needing it.
pub(crate) const SHOW_DELAY: Duration = Duration::from_secs(1);

/// The edge of the "More details" chevron.
const CHEVRON_SIZE: i32 = 16;

mod imp {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::SHOW_DELAY;
    use crate::app_context::AppContext;
    use crate::window::conflict_window::FileConflictWindow;

    /// Private state of [`super::ProgressWindow`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/progress-window.ui")]
    pub(crate) struct ProgressWindow {
        /// The actions, one under the other.
        #[template_child]
        pub(super) actions: TemplateChild<gtk::Box>,
        /// "More details" / "Fewer details".
        #[template_child]
        pub(super) details_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) details_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub(super) details_label: TemplateChild<gtk::Label>,
        /// The application's state, whose preference keeps the details.
        pub(super) context: glib::WeakRef<AppContext>,
        /// Whether the details show.
        pub(super) details: Cell<bool>,
        /// How long an action runs before the window shows.
        pub(super) show_delay: Cell<Duration>,
        /// The timer that shows the window.
        pub(super) showing: RefCell<Option<glib::SourceId>>,
        /// The window group of this window and its File Conflict window
        /// alone, so that window blocks only this one.
        pub(super) group: gtk::WindowGroup,
        /// The File Conflict window an action opened, while it shows.
        pub(super) conflict_window: glib::WeakRef<FileConflictWindow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProgressWindow {
        const NAME: &'static str = "OxProgressWindow";
        type Type = super::ProgressWindow;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(window: &glib::subclass::InitializingObject<Self>) {
            window.init_template();
        }
    }

    impl ObjectImpl for ProgressWindow {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "progress-window.ui");
            self.show_delay.set(SHOW_DELAY);
            let window = self.obj();
            self.group.add_window(&*window);
            self.details_button.connect_clicked(glib::clone!(
                #[weak]
                window,
                move |_| window.toggle_details()
            ));
        }

        fn dispose(&self) {
            if let Some(timer) = self.showing.take() {
                timer.remove();
            }
        }
    }

    impl WidgetImpl for ProgressWindow {}

    impl WindowImpl for ProgressWindow {
        /// Closing cancels every action; the window closes with the last.
        fn close_request(&self) -> glib::Propagation {
            let window = self.obj();
            let actions = window.actions();
            if actions.is_empty() {
                return glib::Propagation::Proceed;
            }
            for action in actions {
                action.cancel();
            }
            glib::Propagation::Stop
        }
    }
}

glib::wrapper! {
    /// The window of the running file operations.
    pub(crate) struct ProgressWindow(ObjectSubclass<imp::ProgressWindow>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager;
}

impl ProgressWindow {
    /// The progress window of `context`'s application, made when none is
    /// open.
    pub(crate) fn of(context: &AppContext) -> Self {
        if let Some(window) = context.progress_window() {
            return window;
        }
        let window: Self = glib::Object::new();
        window.imp().context.set(Some(context));
        let details = context.settings_data().preferences.progress_details;
        window.set_details(details);
        context.set_progress_window(Some(&window));
        window
    }

    /// Adds `action`, which has just started, under the others; the
    /// window shows once it has run for a moment.
    pub(crate) fn add(&self, action: &TransferPanel) {
        let imp = self.imp();
        if action.parent().as_ref() != Some(imp.actions.upcast_ref()) {
            if let Some(parent) = action.parent().and_downcast::<gtk::Box>() {
                parent.remove(action);
            }
            imp.actions.append(action);
        }
        action.show_in(Some(self));
        action.show_details(imp.details.get());
        self.follow_dialog();
        self.refresh();
        self.show_soon();
    }

    /// Makes the window the child of the file dialog the earliest running
    /// action was started in, or of no window when none was: a browsing
    /// window's progress stands alone (OPS-019). GTK moves a child into
    /// its parent's window group and, once it has no parent, into the
    /// default one, so the window goes back to its own group each time,
    /// where its File Conflict window blocks it alone.
    fn follow_dialog(&self) {
        let dialog = self.actions().iter().find_map(TransferPanel::dialog);
        let parent = dialog.as_ref().map(Cast::upcast_ref::<gtk::Window>);
        if self.transient_for().as_ref() != parent {
            self.set_transient_for(parent);
            self.imp().group.add_window(self);
        }
    }

    /// Takes `action`, which has ended, out; the window closes with its
    /// last action.
    pub(crate) fn remove(&self, action: &TransferPanel) {
        let imp = self.imp();
        action.show_in(None);
        if action.parent().as_ref() == Some(imp.actions.upcast_ref()) {
            imp.actions.remove(action);
        }
        if !self.actions().is_empty() {
            self.follow_dialog();
            self.refresh();
            return;
        }
        if let Some(timer) = imp.showing.take() {
            timer.remove();
        }
        if let Some(context) = imp.context.upgrade() {
            if context.progress_window().as_ref() == Some(self) {
                context.set_progress_window(None);
            }
        }
        self.destroy();
    }

    /// The running actions, in the order they started.
    pub(crate) fn actions(&self) -> Vec<TransferPanel> {
        let mut actions = Vec::new();
        let mut child = self.imp().actions.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(action) = widget.downcast::<TransferPanel>() {
                actions.push(action);
            }
        }
        actions
    }

    /// Titles the window for its actions: "44% complete" for one, or
    /// "Replace or Skip Files" while it asks about taken names and "1
    /// Interrupted Action" while it asks about a full drive or a failed
    /// item, as Windows
    /// titles it, and "2 Running Actions" for several; its icon is the
    /// first action's.
    pub(crate) fn refresh(&self) {
        let actions = self.actions();
        for action in &actions {
            action.show_cancel_beside_others(actions.len() > 1);
        }
        let title = match actions.as_slice() {
            [] => return,
            [action] if action.asks_about_space() || action.asks_about_failure() => {
                ox_core::i18n::ngettext("{count} Interrupted Action", "{count} Interrupted Actions", 1)
                    .replace("{count}", "1")
            }
            [action] if action.is_asking() => ox_core::i18n::gettext("Replace or Skip Files"),
            [action] => action.percent_text(),
            _ => {
                let count = u64::try_from(actions.len()).unwrap_or(u64::MAX);
                ox_core::i18n::ngettext("{count} Running Action", "{count} Running Actions", count)
                    .replace("{count}", &count.to_string())
            }
        };
        if self.title().as_deref() != Some(title.as_str()) {
            self.set_title(Some(&title));
        }
        let icon = actions[0].glyph().name();
        if self.icon_name().as_deref() != Some(icon) {
            self.set_icon_name(Some(icon));
        }
        let asking = matches!(actions.as_slice(), [action] if action.asks_about_names());
        self.offer_details(!asking);
    }

    /// Offers "More details" / "Fewer details", or shows it greyed and
    /// out of the keyboard's reach, worded "More details", while the one
    /// action asks Replace or Skip Files, as Windows 11 does. Offered
    /// again, it words the remembered choice.
    fn offer_details(&self, offered: bool) {
        let imp = self.imp();
        if imp.details_button.is_sensitive() == offered {
            return;
        }
        imp.details_button.set_sensitive(offered);
        self.word_details(offered && imp.details.get());
    }

    /// Shows the window now, without taking the focus, for an action
    /// that already waited its moment before it started, so it does not
    /// wait [`SHOW_DELAY`] a second time.
    pub(crate) fn show_now(&self) {
        if let Some(timer) = self.imp().showing.take() {
            timer.remove();
        }
        if self.is_visible() || self.actions().is_empty() {
            return;
        }
        self.set_visible(true);
        self.forget_focus();
    }

    /// Shows the window once [`SHOW_DELAY`] has passed with an action
    /// still running, without taking the focus.
    fn show_soon(&self) {
        let imp = self.imp();
        if self.is_visible() || imp.showing.borrow().is_some() {
            return;
        }
        let delay = imp.show_delay.get();
        let timer = glib::timeout_add_local_once(
            delay,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().showing.take();
                    window.show_now();
                }
            ),
        );
        imp.showing.replace(Some(timer));
    }

    /// Shows the window at once, and in front, for an action that asks
    /// about taken names: a question waits for the user, unlike progress.
    pub(crate) fn show_question(&self) {
        if let Some(timer) = self.imp().showing.take() {
            timer.remove();
        }
        self.refresh();
        self.present();
        // As in Windows, nothing shows a focus ring until the keyboard
        // is used; Tab reaches the rows.
        self.forget_focus();
    }

    /// The window group the File Conflict windows join.
    pub(crate) fn group_for_questions(&self) -> gtk::WindowGroup {
        self.imp().group.clone()
    }

    /// Records the File Conflict window that shows over this one.
    pub(crate) fn set_conflict_window(&self, window: Option<&FileConflictWindow>) {
        self.imp().conflict_window.set(window);
    }

    /// The File Conflict window that shows over this one, if any, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn conflict_window(&self) -> Option<FileConflictWindow> {
        self.imp().conflict_window.upgrade()
    }

    /// Leaves no widget focused and no focus ring showing, as the Windows
    /// window opens: GTK focuses the first link when a window shows. Tab
    /// still moves into the window, and the keyboard brings the ring back.
    fn forget_focus(&self) {
        GtkWindowExt::set_focus(self, None::<&gtk::Widget>);
        self.set_focus_visible(false);
    }

    /// "More details" or "Fewer details": every action shows or hides
    /// its details, and the choice is kept for next time.
    fn toggle_details(&self) {
        let shown = !self.imp().details.get();
        self.set_details(shown);
        if let Some(context) = self.imp().context.upgrade() {
            let update = PreferencesUpdate {
                progress_details: Some(shown),
                ..PreferencesUpdate::default()
            };
            context.remember_preferences(update);
        }
    }

    /// Shows or hides every action's details, and words the toggle.
    fn set_details(&self, shown: bool) {
        let imp = self.imp();
        imp.details.set(shown);
        self.word_details(shown && imp.details_button.is_sensitive());
        for action in self.actions() {
            action.show_details(shown);
        }
    }

    /// Words the toggle "Fewer details" when `expanded`, else "More
    /// details", with its chevron.
    fn word_details(&self, expanded: bool) {
        let imp = self.imp();
        let (icon, text) = if expanded {
            (Icon::ChevronUp16, ox_core::i18n::gettext_static("Fewer details"))
        } else {
            (Icon::ChevronDown16, ox_core::i18n::gettext_static("More details"))
        };
        icons::set_icon(&imp.details_icon, icon, CHEVRON_SIZE);
        imp.details_label.set_text(text);
        imp.details_button
            .update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
    }

    /// Whether the details show.
    pub(crate) fn shows_details(&self) -> bool {
        self.imp().details.get()
    }

    /// Presses "More details" or "Fewer details", for tests.
    #[cfg(test)]
    pub(crate) fn press_details(&self) {
        self.imp().details_button.emit_clicked();
    }

    /// Whether the toggle can be pressed, for tests.
    #[cfg(test)]
    pub(crate) fn offers_details(&self) -> bool {
        self.imp().details_button.is_sensitive()
    }

    /// The toggle's text, for tests.
    #[cfg(test)]
    pub(crate) fn details_text(&self) -> String {
        self.imp().details_label.text().to_string()
    }

    /// Shows the window after `delay` instead of [`SHOW_DELAY`], for
    /// tests.
    #[cfg(test)]
    pub(crate) fn set_show_delay(&self, delay: Duration) {
        self.imp().show_delay.set(delay);
    }
}

#[cfg(test)]
mod tests;
