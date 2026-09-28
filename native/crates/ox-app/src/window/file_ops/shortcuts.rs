// SPDX-License-Identifier: AGPL-3.0-only
//! The file commands' keys, and where they work (CMD-016, CMD-017).
//!
//! Ports the file keys of `onKey` in `desktop/ui/app.js` and adds Undo,
//! Redo and Shift+Delete. They are not application accelerators: a text
//! field keeps its own Ctrl+C, Ctrl+V, Delete and F2, and on the Settings
//! page or in the address bar they do nothing, as in app.js. The window
//! handles them in the bubble phase, after the focused widget had its
//! turn, and only when focus is elsewhere; a disabled command ignores its
//! key.

use gtk::glib;
use gtk::prelude::*;

use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// Each file command and its keys, as GTK parses them.
const FILE_SHORTCUTS: [(WindowAction, &str); 9] = [
    (WindowAction::Cut, "<Primary>x"),
    (WindowAction::Copy, "<Primary>c"),
    (WindowAction::Paste, "<Primary>v"),
    (WindowAction::Rename, "F2"),
    (WindowAction::Trash, "Delete|KP_Delete"),
    (WindowAction::DeletePermanently, "<Shift>Delete|<Shift>KP_Delete"),
    (WindowAction::NewFolder, "<Primary><Shift>n"),
    (WindowAction::Undo, "<Primary>z"),
    (WindowAction::Redo, "<Primary><Shift>z|<Primary>y"),
];

impl BrowserWindow {
    /// Adds the file commands' keys to the window.
    pub(super) fn install_file_shortcuts(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Bubble);
        for (action, keys) in FILE_SHORTCUTS {
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            let run = gtk::CallbackAction::new(move |widget, _| {
                let Some(window) = widget.downcast_ref::<BrowserWindow>() else {
                    return glib::Propagation::Proceed;
                };
                window.run_file_shortcut(action)
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
        }
        self.add_controller(shortcuts);
    }

    /// Runs `action` for its key, unless focus is where the key means
    /// something else.
    fn run_file_shortcut(&self, action: WindowAction) -> glib::Propagation {
        if !self.file_keys_apply() {
            return glib::Propagation::Proceed;
        }
        action.activate_from(self, None);
        glib::Propagation::Stop
    }

    /// False on the Settings page, in a text field and in the address
    /// bar, where the keys edit text or move between crumbs.
    fn file_keys_apply(&self) -> bool {
        if self.shows_settings() {
            return false;
        }
        let Some(focus) = GtkWindowExt::focus(self) else {
            return true;
        };
        let in_text = focus.is::<gtk::Text>() || focus.dynamic_cast_ref::<gtk::Editable>().is_some();
        !in_text && !focus.is_ancestor(self.address_bar())
    }
}
