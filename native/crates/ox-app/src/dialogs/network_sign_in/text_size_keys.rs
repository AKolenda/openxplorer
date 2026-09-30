// SPDX-License-Identifier: AGPL-3.0-only
//! Text size while the sign-in dialog is open.
//!
//! The dialog is modal, so the window's shortcuts do not reach it, as
//! app.js blocks every shortcut under the sign-in layer. Text size is the
//! exception (NET-009): `desktop/ui/app.js:986` installs `textSizeKeys`
//! ahead of the sign-in layer, so Ctrl+plus, Ctrl+minus and Ctrl+0 still
//! resize the text of the window that owns the dialog.

use gtk::glib;
use gtk::prelude::*;

use super::SignInDialog;
use crate::text_size::Step;

/// Runs the window action `name` (such as `win.text-larger`) on the first
/// window in the chain of transient parents from `dialog` that has it: the
/// dialog may sit on another dialog, such as Map network location.
fn activate_in_owner(dialog: &gtk::Window, name: &str) -> glib::Propagation {
    let mut owner = dialog.transient_for();
    while let Some(window) = owner {
        if window.activate_action(name, None).is_ok() {
            return glib::Propagation::Stop;
        }
        owner = window.transient_for();
    }
    glib::Propagation::Proceed
}

/// A shortcut that performs `step` in the owning window when
/// `accelerator` is pressed in the dialog.
fn text_size_shortcut(step: Step, accelerator: &str) -> gtk::Shortcut {
    let action_name = format!("win.{}", step.action_name());
    let action = gtk::CallbackAction::new(move |widget, _| match widget.downcast_ref::<gtk::Window>() {
        Some(dialog) => activate_in_owner(dialog, &action_name),
        None => glib::Propagation::Proceed,
    });
    let trigger = gtk::ShortcutTrigger::parse_string(accelerator);
    gtk::Shortcut::new(trigger, Some(action))
}

impl SignInDialog {
    /// Lets the text-size keys through to the window that owns the dialog.
    pub(super) fn forward_text_size_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        for step in Step::ALL {
            for accelerator in step.accelerators() {
                shortcuts.add_shortcut(text_size_shortcut(step, &accelerator));
            }
        }
        self.add_controller(shortcuts);
    }
}
