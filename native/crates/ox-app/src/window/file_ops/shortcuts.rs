// SPDX-License-Identifier: AGPL-3.0-only
//! The file commands' keys, and Ctrl+A, and where they work (CMD-016,
//! CMD-017, SEL-004).
//!
//! Ports the file keys and Ctrl+A of `onKey` in `desktop/ui/app.js`: Ctrl+A
//! selects every shown item wherever focus is outside a text field, not
//! only in the folder view. It adds Undo,
//! Redo, Shift+Delete and Copy path's keys (Explorer's Ctrl+Shift+C for
//! "Copy as path", and Dolphin's Ctrl+Alt+C for "Copy Location",
//! CLIP-013). They are not application accelerators: a text field keeps
//! its own Ctrl+C, Ctrl+V, Delete and F2, and on the Settings page or in
//! the address bar they do nothing, as in app.js. The window handles them
//! in the bubble phase, after the focused widget had its turn, and only
//! when focus is elsewhere. A disabled command ignores its key, except
//! Cut, Copy and Paste, which `onKey` runs directly so that they can say
//! why nothing happens.

use gtk::glib;
use gtk::prelude::*;
use ox_core::clipboard::ClipboardMode;

use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// Each file command and its keys, as GTK parses them.
const FILE_SHORTCUTS: [(WindowAction, &str); 11] = [
    (WindowAction::SelectAll, "<Primary>a"),
    (WindowAction::Cut, "<Primary>x"),
    (WindowAction::Copy, "<Primary>c"),
    (WindowAction::Paste, "<Primary>v"),
    (WindowAction::CopyPath, "<Primary><Shift>c|<Primary><Alt>c"),
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
        match action {
            WindowAction::Cut => self.copy_selection(ClipboardMode::Cut),
            WindowAction::Copy => self.copy_selection(ClipboardMode::Copy),
            WindowAction::Paste => self.paste_from_keyboard(),
            _ => action.activate_from(self, None),
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
