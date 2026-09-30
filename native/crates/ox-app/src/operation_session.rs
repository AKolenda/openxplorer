// SPDX-License-Identifier: AGPL-3.0-only
//! What the desktop learns about a running write: the session does not
//! log out or suspend (INT-028) and the dock icon shows the progress
//! (INT-027).
//!
//! The transfer panel and the archive operation panel each hold one
//! [`OperationSession`] from the operation's start to its end.

use gtk::prelude::*;

use crate::launcher_progress::LauncherProgress;
use crate::write_inhibitor::WriteInhibitor;

/// A running operation's inhibitor and dock progress, released when this
/// value is dropped.
#[derive(Debug)]
pub(crate) struct OperationSession {
    /// Keeps the session from logging out or suspending meanwhile.
    inhibitor: Option<WriteInhibitor>,
    /// The operation's share of the progress on the dock icon.
    launcher: Option<LauncherProgress>,
}

impl OperationSession {
    /// Starts the session of an operation shown in `widget`'s window.
    pub(crate) fn start(widget: &impl IsA<gtk::Widget>) -> Self {
        Self {
            inhibitor: WriteInhibitor::hold(widget),
            launcher: LauncherProgress::for_widget(widget),
        }
    }

    /// Shows `fraction` (0–1) on the dock icon.
    pub(crate) fn show_progress(&self, fraction: f64) {
        if let Some(launcher) = &self.launcher {
            launcher.show(fraction);
        }
    }

    /// Whether the session's inhibitor is held, for tests.
    #[cfg(test)]
    pub(crate) fn inhibits_logout(&self) -> bool {
        self.inhibitor.is_some()
    }
}

impl Drop for OperationSession {
    /// Hides the dock progress, then lets the session log out again, so
    /// nothing is left showing once logout is allowed.
    fn drop(&mut self) {
        self.launcher.take();
        self.inhibitor.take();
    }
}
