// SPDX-License-Identifier: AGPL-3.0-only
//! What every modal dialog of the app does with the keyboard.
//!
//! Ports the Escape handling of `showModal` in `desktop/ui/app.js`:
//! Escape answers "cancel", as the dialog's Close or Cancel button does.

use gtk::prelude::*;
use gtk::{gdk, glib};

/// A controller for a dialog window: Escape closes it through its close
/// request, which a dialog refuses while it must stay open, such as
/// Software updates during an installation.
pub(crate) fn escape_closes() -> gtk::ShortcutController {
    let close = gtk::CallbackAction::new(|widget, _| {
        if let Some(window) = widget.downcast_ref::<gtk::Window>() {
            window.close();
        }
        glib::Propagation::Stop
    });
    let trigger = gtk::KeyvalTrigger::new(gdk::Key::Escape, gdk::ModifierType::empty());
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(close)));
    shortcuts
}
