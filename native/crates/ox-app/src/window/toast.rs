// SPDX-License-Identifier: AGPL-3.0-only
//! The toast: a short message at the bottom centre of the window.
//!
//! Ports `toast()` in `desktop/ui/app.js` and `.toast` in
//! `desktop/ui/style.css`. A message stays four seconds; a new one replaces
//! it and starts the time again. Screen readers announce it as a status.
//!
//! [`Toast`] is a widget subclass around its label, so the timer that
//! hides it lives in the widget and ends with it.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

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
    pub struct Toast {
        /// The message, built by `constructed`.
        pub(super) label: OnceCell<gtk::Label>,
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
            let label = super::message_label();
            label.set_parent(&*toast);
            self.label.set(label).expect("constructed runs once per object");
        }

        fn dispose(&self) {
            self.obj().cancel_hide_timer();
            if let Some(label) = self.label.get() {
                label.unparent();
            }
        }
    }

    impl WidgetImpl for Toast {}
}

glib::wrapper! {
    /// The toast, to lay over the window's workspace.
    pub struct Toast(ObjectSubclass<imp::Toast>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Toast {
    /// A hidden toast.
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn label(&self) -> &gtk::Label {
        self.imp().label.get().expect("constructed builds the label")
    }

    /// Shows `message` for [`TOAST_DURATION`], replacing the message shown
    /// now and starting the time again.
    pub fn show(&self, message: &str) {
        self.cancel_hide_timer();
        self.label().set_text(message);
        self.set_visible(true);
        let timer = glib::timeout_add_local_once(
            TOAST_DURATION,
            glib::clone!(
                #[weak(rename_to = toast)]
                self,
                move || {
                    // The source ends with this call; nothing may remove it again.
                    toast.imp().hide_timer.take();
                    toast.set_visible(false);
                }
            ),
        );
        self.imp().hide_timer.replace(Some(timer));
    }

    /// Hides the toast at once, as moving to another folder or tab does.
    pub fn hide(&self) {
        self.cancel_hide_timer();
        self.label().set_text("");
        self.set_visible(false);
    }

    /// The message shown last, for tests.
    #[cfg(test)]
    pub fn text(&self) -> glib::GString {
        self.label().text()
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
        .css_classes(["toast"])
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
}
