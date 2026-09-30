// SPDX-License-Identifier: AGPL-3.0-only
//! A desktop notification when a file operation ends while no `OpenXplorer`
//! window has focus (INT-026), as Dolphin and Nautilus send one.
//!
//! The in-window toast or "Operation result" dialog stays; the
//! notification adds a way to learn the result from another app.
//! Clicking it, or its Show button, brings the window back, which shows
//! the folder the operation wrote to with the new items selected. One
//! notification ID is reused, so the latest result replaces an earlier
//! one. The desktop entries declare `X-GNOME-UsesNotifications`.

use gtk::gio;
use gtk::prelude::*;
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
}

impl Notice {
    /// What the notification says about `summary`.
    pub(super) fn of(summary: &OperationSummary) -> Self {
        match summary {
            OperationSummary::Toast(text) => Self {
                title: text.clone(),
                body: None,
            },
            OperationSummary::Report(text) => Self {
                title: RESULT_TITLE.to_owned(),
                body: Some(text.clone()),
            },
        }
    }
}

impl BrowserWindow {
    /// Sends a notification about `summary` when no window of the
    /// application has focus.
    pub(super) fn notify_if_in_background(&self, summary: &OperationSummary) {
        let Some(app) = self.application() else {
            return;
        };
        if app.windows().iter().any(GtkWindowExt::is_active) {
            return;
        }
        let notice = Notice::of(summary);
        let notification = gio::Notification::new(&notice.title);
        if let Some(body) = &notice.body {
            notification.set_body(Some(body));
        }
        let focus = AppAction::FocusWindow.detailed_name();
        let window = self.id().to_variant();
        notification.set_default_action_and_target_value(&focus, Some(&window));
        notification.add_button_with_target_value("Show", &focus, Some(&window));
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
    static SENT: std::cell::RefCell<Vec<Notice>> = const { std::cell::RefCell::new(Vec::new()) };
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
        let toast = Notice::of(&OperationSummary::Toast("12 item(s) copied.".to_owned()));
        assert_eq!(toast.title, "12 item(s) copied.");
        assert_eq!(toast.body, None);
        let report = Notice::of(&OperationSummary::Report(
            "1 item could not be copied.".to_owned(),
        ));
        assert_eq!(report.title, RESULT_TITLE);
        assert_eq!(report.body.as_deref(), Some("1 item could not be copied."));
    }
}
