// SPDX-License-Identifier: AGPL-3.0-only
//! A desktop notification when a file operation ends while no `OpenXplorer`
//! window has focus (INT-026), as Dolphin and Nautilus send one.
//!
//! The in-window toast or "Operation result" dialog stays; the
//! notification adds a way to learn the result from another app.
//! Clicking it brings the window back; its Show button also opens the
//! destination there, with the items the operation created selected
//! (`app.show-destination`). One
//! notification ID is reused, so the latest result replaces an earlier
//! one. The desktop entries declare `X-GNOME-UsesNotifications`.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::ops::{OperationSummary, RESULT_TITLE};

use super::BrowserWindow;
use crate::application::AppAction;

/// The notification's ID: a newer result replaces the older one.
#[cfg_attr(
    test,
    allow(dead_code, reason = "tests record notifications instead of sending them")
)]
const NOTIFICATION_ID: &str = "file-operation";

/// The text of a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Notice {
    /// The headline: the toast's text, or "Operation result".
    pub(super) title: String,
    /// The report under it, for an operation that did not fully succeed.
    pub(super) body: Option<String>,
    /// The window that clicking it or its Show button brings back, the
    /// target of `app.focus-window`.
    pub(super) window: u32,
    /// Where the operation wrote, which Show opens.
    pub(super) destination: Destination,
}

/// Where an operation wrote: what the notification's Show button opens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Destination {
    /// The folder written to, when the items are not known.
    pub(super) folder: Option<String>,
    /// The items the operation created or moved there, shown selected.
    pub(super) items: Vec<String>,
}

impl Destination {
    /// The items an operation created, revealed in their folders.
    pub(super) fn items(items: Vec<String>) -> Self {
        Self { folder: None, items }
    }

    /// The folder an operation wrote to.
    pub(super) fn folder(folder: &str) -> Self {
        Self {
            folder: Some(folder.to_owned()),
            items: Vec::new(),
        }
    }

    /// Whether there is anything to open.
    fn is_known(&self) -> bool {
        self.folder.is_some() || !self.items.is_empty()
    }
}

impl Notice {
    /// What the notification says about `summary`, leading back to the
    /// window whose ID is `window` and to `destination`.
    pub(super) fn of(summary: &OperationSummary, window: u32, destination: Destination) -> Self {
        let (title, body) = match summary {
            OperationSummary::Toast(text) => (text.clone(), None),
            OperationSummary::Report(text) => (
                ox_core::i18n::gettext_static(RESULT_TITLE).to_owned(),
                Some(text.clone()),
            ),
        };
        Self {
            title,
            body,
            window,
            destination,
        }
    }

    /// The action and target of the Show button: `app.show-destination`
    /// with the window, the folder (empty when unknown) and the items, or
    /// `app.focus-window` when the operation wrote nowhere to show, as a
    /// deletion.
    pub(super) fn show_action(&self) -> (String, glib::Variant) {
        let destination = &self.destination;
        if !destination.is_known() {
            return (AppAction::FocusWindow.detailed_name(), self.window.to_variant());
        }
        let folder = destination.folder.clone().unwrap_or_default();
        let target = (self.window, folder, destination.items.clone()).to_variant();
        (AppAction::ShowDestination.detailed_name(), target)
    }
}

impl BrowserWindow {
    /// Sends a notification about `summary` when no window of the
    /// application has focus; its Show button opens `destination`.
    pub(super) fn notify_if_in_background(&self, summary: &OperationSummary, destination: Destination) {
        let Some(app) = self.application() else {
            return;
        };
        if app.windows().iter().any(GtkWindowExt::is_active) {
            return;
        }
        let notice = Notice::of(summary, self.id(), destination);
        let notification = gio::Notification::new(&notice.title);
        if let Some(body) = &notice.body {
            notification.set_body(Some(body));
        }
        let focus = AppAction::FocusWindow.detailed_name();
        let window = notice.window.to_variant();
        notification.set_default_action_and_target_value(&focus, Some(&window));
        let (show, target) = notice.show_action();
        notification.add_button_with_target_value("Show", &show, Some(&target));
        send(&app, &notification, notice);
    }
}

/// Sends `notification` through the desktop.
#[cfg(not(test))]
fn send(app: &gtk::Application, notification: &gio::Notification, _notice: Notice) {
    app.send_notification(Some(NOTIFICATION_ID), notification);
}

#[cfg(test)]
thread_local! {
    /// What tests would have sent; they never reach a notification server.
    pub(super) static SENT: std::cell::RefCell<Vec<Notice>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Records what would be sent, so a test session starts no notification
/// daemon.
#[cfg(test)]
fn send(_app: &gtk::Application, _notification: &gio::Notification, notice: Notice) {
    SENT.with(|sent| sent.borrow_mut().push(notice));
}

/// Takes what was sent so far, for tests.
#[cfg(test)]
pub(super) fn take_sent() -> Vec<Notice> {
    SENT.with(std::cell::RefCell::take)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full success says the toast's words; anything else is headed
    /// "Operation result" with the report under it.
    ///
    /// parity: INT-026
    #[test]
    fn the_notice_says_what_the_toast_or_report_says() {
        let toast = Notice::of(
            &OperationSummary::Toast("12 item(s) copied.".to_owned()),
            3,
            Destination::default(),
        );
        assert_eq!(toast.title, "12 item(s) copied.");
        assert_eq!(toast.body, None);
        let report = Notice::of(
            &OperationSummary::Report("1 item could not be copied.".to_owned()),
            3,
            Destination::default(),
        );
        assert_eq!(report.title, RESULT_TITLE);
        assert_eq!(report.body.as_deref(), Some("1 item could not be copied."));
        assert_eq!(report.show_action().0, "app.focus-window", "nothing to open");
        let folder = Notice::of(
            &OperationSummary::Toast("Done.".to_owned()),
            3,
            Destination::folder("file:///tmp/demo"),
        );
        let (action, target) = folder.show_action();
        assert_eq!(action, "app.show-destination");
        assert_eq!(
            target.get::<(u32, String, Vec<String>)>(),
            Some((3, "file:///tmp/demo".to_owned(), Vec::new()))
        );
    }
}
