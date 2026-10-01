// SPDX-License-Identifier: AGPL-3.0-only
//! The file commands' keys, and the selection keys, and where they work
//! (CMD-016, CMD-017, SEL-004, SEL-005).
//!
//! Ports the file keys, Ctrl+A and Escape of `onKey` in `desktop/ui/app.js`:
//! Ctrl+A selects every shown item and Escape clears the selection wherever
//! focus is outside a text field, not only in the folder view. It adds
//! Undo, Redo, Shift+Delete and Copy path's keys (Explorer's Ctrl+Shift+C
//! for "Copy as path", and Dolphin's Ctrl+Alt+C for "Copy Location",
//! CLIP-013). They are not application accelerators: a text field keeps
//! its own Ctrl+C, Ctrl+V, Delete and F2, and on the Settings page or in
//! the address bar they do nothing, as in app.js. The window handles them
//! in the bubble phase, after the focused widget had its turn, and only
//! when focus is elsewhere; menus and dialogs take their own Escape first.
//! Ctrl+A alone is handled in the capture phase, because the sidebar's list
//! would otherwise take it for its own rows. A disabled command ignores its
//! key, except Cut, Copy and Paste, which `onKey` runs directly so that
//! they can say why nothing happens.

use gtk::glib;
use gtk::prelude::*;
use ox_core::clipboard::ClipboardMode;

use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// Each file command and its keys, as GTK parses them, handled after the
/// focused widget.
const FILE_SHORTCUTS: [(WindowAction, &str); 11] = [
    (WindowAction::SelectNone, "Escape"),
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
        let shortcuts = file_shortcuts(gtk::PropagationPhase::Bubble, &FILE_SHORTCUTS);
        self.add_controller(shortcuts);
        let select_all = [(WindowAction::SelectAll, "<Primary>a")];
        self.add_controller(file_shortcuts(gtk::PropagationPhase::Capture, &select_all));
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

    /// False on the Settings page, in a text field, in the address bar,
    /// where the keys edit text or move between crumbs, and on the pane
    /// splitter's handle, whose keys move it.
    pub(in crate::window) fn file_keys_apply(&self) -> bool {
        if self.shows_settings() || self.focus_is_in_text_field() {
            return false;
        }
        let Some(focus) = GtkWindowExt::focus(self) else {
            return true;
        };
        !(focus.is_ancestor(self.address_bar()) || focus.is::<gtk::Paned>())
    }

    /// True while keyboard focus is in a text field: the search box, the
    /// address, or a name being renamed in place.
    pub(in crate::window) fn focus_is_in_text_field(&self) -> bool {
        GtkWindowExt::focus(self).is_some_and(|focus| focus.dynamic_cast_ref::<gtk::Editable>().is_some())
    }
}

/// A controller that runs each of `shortcuts`' commands for its keys in
/// `phase`, where [`BrowserWindow::run_file_shortcut`] lets it.
fn file_shortcuts(
    phase: gtk::PropagationPhase,
    shortcuts: &[(WindowAction, &str)],
) -> gtk::ShortcutController {
    let controller = gtk::ShortcutController::new();
    controller.set_propagation_phase(phase);
    for &(action, keys) in shortcuts {
        let trigger = gtk::ShortcutTrigger::parse_string(keys);
        let run = gtk::CallbackAction::new(move |widget, _| {
            let Some(window) = widget.downcast_ref::<BrowserWindow>() else {
                return glib::Propagation::Proceed;
            };
            window.run_file_shortcut(action)
        });
        controller.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
    }
    controller
}
