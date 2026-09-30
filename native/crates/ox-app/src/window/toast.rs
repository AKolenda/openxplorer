// SPDX-License-Identifier: AGPL-3.0-only
//! The toast: a short message at the bottom centre of the window, with an
//! optional action button such as Undo (OPS-032).
//!
//! Ports `toast()` in `desktop/ui/app.js` and `.toast` in
//! `desktop/ui/style.css`. A message stays four seconds; a new one replaces
//! it and starts the time again. Screen readers announce it as a status.
//! The time stops while the pointer rests on the toast or its button has
//! keyboard focus, so the button can be reached (WCAG 2.2.1), and starts
//! again when both leave.
//!
//! [`Toast`] is a widget subclass around its message and button, so the
//! timer that hides it lives in the widget and ends with it.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::window::window_action::WindowAction;

/// How long a toast stays (`setTimeout(..., 4000)` in `toast()`).
const TOAST_DURATION: Duration = Duration::from_secs(4);

/// The widest a message gets before it wraps, in characters.
const MAX_WIDTH_CHARS: i32 = 80;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::Toast`].
    #[derive(Debug, Default)]
    pub(crate) struct Toast {
        /// The rounded box of the message and the button.
        pub(super) surface: OnceCell<gtk::Box>,
        /// The message, built by `constructed`.
        pub(super) label: OnceCell<gtk::Label>,
        /// The action button, hidden while the message has none.
        pub(super) action: OnceCell<gtk::Button>,
        /// Tells whether the pointer rests on the toast.
        pub(super) motion: OnceCell<gtk::EventControllerMotion>,
        /// Tells whether the keyboard focus is inside the toast.
        pub(super) focus: OnceCell<gtk::EventControllerFocus>,
        /// The pending hide, while one runs. The timer clears it when it
        /// fires: removing a source that already ran is a `GLib` error,
        /// which `SourceId::remove` turns into a panic.
        pub(super) hide_timer: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Toast {
        const NAME: &'static str = "OxToast";
        type Type = super::Toast;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Toast {
        fn constructed(&self) {
            self.parent_constructed();
            let toast = self.obj();
            toast.set_halign(gtk::Align::Center);
            toast.set_valign(gtk::Align::End);
            toast.set_visible(false);
            let surface = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(16)
                .css_classes(["toast"])
                .build();
            let label = super::message_label();
            let action = gtk::Button::builder()
                .css_classes(["toast-action"])
                .valign(gtk::Align::Center)
                .visible(false)
                .build();
            surface.append(&label);
            surface.append(&action);
            surface.set_parent(&*toast);
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(glib::clone!(
                #[weak]
                toast,
                move |_, _, _| toast.cancel_hide_timer()
            ));
            motion.connect_leave(glib::clone!(
                #[weak]
                toast,
                move |_| toast.resume_unless_held()
            ));
            let focus = gtk::EventControllerFocus::new();
            focus.connect_enter(glib::clone!(
                #[weak]
                toast,
                move |_| toast.cancel_hide_timer()
            ));
            focus.connect_leave(glib::clone!(
                #[weak]
                toast,
                move |_| toast.resume_unless_held()
            ));
            surface.add_controller(motion.clone());
            surface.add_controller(focus.clone());
            self.surface
                .set(surface)
                .expect("constructed runs once per object");
            self.label.set(label).expect("constructed runs once per object");
            self.action.set(action).expect("constructed runs once per object");
            self.motion.set(motion).expect("constructed runs once per object");
            self.focus.set(focus).expect("constructed runs once per object");
        }

        fn dispose(&self) {
            self.obj().cancel_hide_timer();
            if let Some(surface) = self.surface.get() {
                surface.unparent();
            }
        }
    }

    impl WidgetImpl for Toast {}
}

glib::wrapper! {
    /// The toast, to lay over the window's workspace.
    pub(crate) struct Toast(ObjectSubclass<imp::Toast>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Toast {
    /// A hidden toast, for tests; a window gets its toast from its
    /// template.
    #[cfg(test)]
    fn new() -> Self {
        glib::Object::new()
    }

    fn label(&self) -> &gtk::Label {
        self.imp().label.get().expect("constructed builds the label")
    }

    fn action_button(&self) -> &gtk::Button {
        self.imp().action.get().expect("constructed builds the button")
    }

    /// Shows `message` for [`TOAST_DURATION`], replacing the message shown
    /// now and starting the time again.
    pub(crate) fn show(&self, message: &str) {
        self.withdraw_action();
        self.show_text(message);
    }

    /// Shows `message` with a button labelled `label` that runs `action`,
    /// such as Undo after an operation (OPS-032). The button follows the
    /// action: it is disabled while the action is.
    pub(crate) fn show_with_action(&self, message: &str, label: &str, action: WindowAction) {
        let button = self.action_button();
        button.set_label(label);
        action.assign_to(button);
        button.set_visible(true);
        self.show_text(message);
    }

    /// Takes the action button away and keeps the message, as when the
    /// step it would undo is no longer the newest.
    pub(crate) fn withdraw_action(&self) {
        let button = self.action_button();
        button.set_visible(false);
        button.set_action_name(None);
    }

    /// Hides the toast at once, as moving to another folder or tab does.
    pub(crate) fn hide(&self) {
        self.cancel_hide_timer();
        self.withdraw_action();
        self.label().set_text("");
        self.set_visible(false);
    }

    /// The message shown last, for tests.
    #[cfg(test)]
    pub(crate) fn text(&self) -> glib::GString {
        self.label().text()
    }

    /// The action button's label while it shows, for tests.
    #[cfg(test)]
    pub(crate) fn action_label(&self) -> Option<glib::GString> {
        let button = self.action_button();
        button.is_visible().then(|| button.label()).flatten()
    }

    /// Clicks the action button, for tests.
    #[cfg(test)]
    pub(crate) fn press_action(&self) {
        self.action_button().emit_clicked();
    }

    /// Shows `message` and starts the time.
    fn show_text(&self, message: &str) {
        self.label().set_text(message);
        self.set_visible(true);
        self.start_hide_timer();
    }

    /// Starts the time after which the toast hides, from the beginning.
    fn start_hide_timer(&self) {
        self.cancel_hide_timer();
        let timer = glib::timeout_add_local_once(
            TOAST_DURATION,
            glib::clone!(
                #[weak(rename_to = toast)]
                self,
                move || {
                    // The source ends with this call; nothing may remove it again.
                    toast.imp().hide_timer.take();
                    toast.set_visible(false);
                    toast.withdraw_action();
                }
            ),
        );
        self.imp().hide_timer.replace(Some(timer));
    }

    /// Starts the time again once neither the pointer nor the keyboard
    /// focus is on the shown toast.
    fn resume_unless_held(&self) {
        let imp = self.imp();
        let has_pointer = imp
            .motion
            .get()
            .is_some_and(gtk::EventControllerMotion::contains_pointer);
        let has_focus = imp
            .focus
            .get()
            .is_some_and(gtk::EventControllerFocus::contains_focus);
        if self.is_visible() && !has_pointer && !has_focus {
            self.start_hide_timer();
        }
    }

    fn cancel_hide_timer(&self) {
        if let Some(timer) = self.imp().hide_timer.take() {
            timer.remove();
        }
    }
}

/// The message label: centred, wrapping and selectable, so a long error
/// can be read in full and copied.
fn message_label() -> gtk::Label {
    let label = gtk::Label::builder()
        .wrap(true)
        .max_width_chars(MAX_WIDTH_CHARS)
        .justify(gtk::Justification::Center)
        .selectable(true)
        .accessible_role(gtk::AccessibleRole::Status)
        .css_classes(["toast-message"])
        .build();
    label.set_can_target(true);
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::wait_until;

    #[gtk::test]
    fn a_message_after_the_last_one_expired_replaces_it_and_can_be_dropped() {
        let toast = Toast::new();
        toast.show("Pinned to Quick access. No files were moved.");
        assert!(toast.is_visible());
        wait_until("the toast to hide itself", || !toast.is_visible());
        toast.show("Path copied. Sharing permissions are unchanged.");
        assert!(toast.is_visible(), "a new message shows again");
        assert_eq!(toast.text(), "Path copied. Sharing permissions are unchanged.");
        wait_until("the second toast to hide itself", || !toast.is_visible());
        drop(toast);
    }

    #[gtk::test]
    fn hiding_the_toast_removes_the_message_at_once() {
        let toast = Toast::new();
        toast.show("Enter an SMB server or share.");
        toast.hide();
        assert!(!toast.is_visible());
        assert_eq!(toast.text(), "");
    }

    /// parity: OPS-032
    #[gtk::test]
    fn an_action_shows_with_its_message_and_a_plain_message_drops_it() {
        let toast = Toast::new();

        toast.show_with_action("1 item(s) sent to Trash.", "Undo", WindowAction::Undo);
        let with_action = toast.action_label();
        toast.show("Path copied. Sharing permissions are unchanged.");

        assert_eq!(with_action.as_deref(), Some("Undo"));
        assert_eq!(toast.action_label(), None);
    }
}
