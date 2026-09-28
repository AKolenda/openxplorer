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
        if action == WindowAction::Paste {
            self.paste_from_keyboard();
        } else {
            action.activate_from(self, None);
        }
        glib::Propagation::Stop
    }

    /// Ctrl+V: pastes even where the Paste button is disabled, so a search
    /// says why nothing is pasted (`onKey` calls `paste()` directly).
    pub(in crate::window) fn paste_from_keyboard(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window.paste().await;
            }
        ));
    }

    /// False on the Settings page, in a text field and in the address
    /// bar, where the keys edit text or move between crumbs.
    pub(in crate::window) fn file_keys_apply(&self) -> bool {
        if self.shows_settings() || self.focus_is_in_text_field() {
            return false;
        }
        let focus = GtkWindowExt::focus(self);
        !focus.is_some_and(|focus| focus.is_ancestor(self.address_bar()))
    }

    /// True while keyboard focus is in a text field: the search box, the
    /// address, or a name being renamed in place.
    pub(in crate::window) fn focus_is_in_text_field(&self) -> bool {
        GtkWindowExt::focus(self).is_some_and(|focus| focus.dynamic_cast_ref::<gtk::Editable>().is_some())
    }
}
