// SPDX-License-Identifier: AGPL-3.0-only
//! The toast: a short message at the bottom centre of the window.
//!
//! Ports `toast()` in `desktop/ui/app.js` and `.toast` in
//! `desktop/ui/style.css`. A message stays four seconds; a new one replaces
//! it and starts the time again. Screen readers announce it as a status.
//! `GtkLabel` cannot be subclassed, so [`Toast`] owns its label and timer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

/// How long a toast stays (`setTimeout(..., 4000)` in `toast()`).
const TOAST_DURATION: Duration = Duration::from_secs(4);

/// The toast's label and the timer that hides it.
#[derive(Debug)]
pub(super) struct Toast {
    label: gtk::Label,
    /// The pending hide, shared with the timer, which clears it when it
    /// fires: removing a source that already ran is a `GLib` error, which
    /// `SourceId::remove` turns into a panic.
    hide_timer: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Toast {
    /// A hidden toast.
    pub fn new() -> Self {
        let label = gtk::Label::builder()
            .wrap(true)
            .max_width_chars(80)
            .justify(gtk::Justification::Center)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .selectable(true)
            .visible(false)
            .accessible_role(gtk::AccessibleRole::Status)
            .css_classes(["toast"])
            .build();
        label.set_can_target(true);
        Self {
            label,
            hide_timer: Rc::default(),
        }
    }

    /// The toast, to lay over the window's workspace.
    pub fn widget(&self) -> &gtk::Label {
        &self.label
    }

    /// Shows `message` for [`TOAST_DURATION`]; an empty message hides the
    /// toast at once.
    pub fn show(&self, message: &str) {
        self.cancel_hide_timer();
        self.label.set_text(message);
        self.label.set_visible(!message.is_empty());
        if message.is_empty() {
            return;
        }
        let label = self.label.downgrade();
        let hide_timer = Rc::clone(&self.hide_timer);
        let timer = glib::timeout_add_local_once(TOAST_DURATION, move || {
            // The source ends with this call; nothing may remove it again.
            hide_timer.take();
            if let Some(label) = label.upgrade() {
                label.set_visible(false);
            }
        });
        self.hide_timer.replace(Some(timer));
    }

    /// The message shown last, for tests.
    #[cfg(test)]
    pub fn text(&self) -> glib::GString {
        self.label.text()
    }

    fn cancel_hide_timer(&self) {
        if let Some(timer) = self.hide_timer.take() {
            timer.remove();
        }
    }
}

impl Drop for Toast {
    fn drop(&mut self) {
        self.cancel_hide_timer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::wait_until;

    #[gtk::test]
    fn a_message_after_the_last_one_expired_replaces_it_and_can_be_dropped() {
        let toast = Toast::new();
        toast.show("Pinned to Quick access. No files were moved.");
        assert!(toast.widget().is_visible());
        wait_until("the toast to hide itself", || !toast.widget().is_visible());
        toast.show("Path copied. Sharing permissions are unchanged.");
        assert!(toast.widget().is_visible(), "a new message shows again");
        assert_eq!(toast.text(), "Path copied. Sharing permissions are unchanged.");
        wait_until("the second toast to hide itself", || !toast.widget().is_visible());
        drop(toast);
    }
}
