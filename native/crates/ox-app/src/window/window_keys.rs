// SPDX-License-Identifier: AGPL-3.0-only
//! The tab, window and history keys, and where they work (CMD-017,
//! TAB-005).
//!
//! Ports the keys `onKey` in `desktop/ui/app.js` handles after its
//! `if(input)return`: Ctrl+H, Ctrl+N, Ctrl+T, Ctrl+W, Ctrl+Tab and
//! Ctrl+Shift+Tab, and Alt+Left, Alt+Right and Alt+Up. A text field keeps
//! them, and while a dialog is open they do nothing. The one exception is
//! `state.modalOwner`: while the active tab's Properties dialog is open,
//! Ctrl+Tab and Ctrl+Shift+Tab switch tabs from any focus, the dialog's
//! own fields included, and the dialog is suspended with its tab.
//!
//! These are not application accelerators, which GTK runs before the
//! focused widget sees the key. The window handles them in the capture
//! phase, before the focused widget, and lets the key through to the
//! field when they do not apply.

use gtk::glib;
use gtk::prelude::*;

use crate::application::AppAction;

use super::window_action::WindowAction;
use super::BrowserWindow;

/// A command one of these keys runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyCommand {
    /// A window action.
    Window(WindowAction),
    /// Ctrl+N: the application's New window.
    NewWindow,
}

impl KeyCommand {
    /// Whether the command switches tabs, which works even from the
    /// active tab's Properties dialog.
    fn is_tab_switch(self) -> bool {
        matches!(
            self,
            KeyCommand::Window(WindowAction::NextTab | WindowAction::PreviousTab)
        )
    }

    /// The action the command runs, as widgets name it.
    fn detailed_name(self) -> String {
        match self {
            KeyCommand::Window(action) => action.detailed_name(),
            KeyCommand::NewWindow => AppAction::NewWindow.detailed_name(),
        }
    }
}

/// Each command and its keys, as GTK parses them. Ctrl+Page Down and
/// Ctrl+Page Up are the tab keys of GNOME apps and browsers, added to
/// app.js's Ctrl+Tab.
const WINDOW_KEYS: [(KeyCommand, &str); 9] = [
    (KeyCommand::Window(WindowAction::Hidden), "<Primary>h"),
    (KeyCommand::NewWindow, "<Primary>n"),
    (KeyCommand::Window(WindowAction::NewTab), "<Primary>t"),
    (KeyCommand::Window(WindowAction::CloseTab), "<Primary>w"),
    (
        KeyCommand::Window(WindowAction::NextTab),
        "<Primary>Tab|<Primary>KP_Tab|<Primary>Page_Down",
    ),
    (
        KeyCommand::Window(WindowAction::PreviousTab),
        "<Primary><Shift>ISO_Left_Tab|<Primary><Shift>Tab|<Primary>Page_Up",
    ),
    (KeyCommand::Window(WindowAction::Back), "<Alt>Left"),
    (KeyCommand::Window(WindowAction::Forward), "<Alt>Right"),
    (KeyCommand::Window(WindowAction::Up), "<Alt>Up"),
];

impl BrowserWindow {
    /// Adds the tab, window and history keys to the window.
    pub(super) fn install_window_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (command, keys) in WINDOW_KEYS {
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            let run = gtk::CallbackAction::new(move |widget, _| {
                let Some(window) = widget.downcast_ref::<BrowserWindow>() else {
                    return glib::Propagation::Proceed;
                };
                window.run_window_key(command)
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
        }
        self.add_controller(shortcuts);
    }

    /// Runs `command` for its key, unless focus is where the key belongs
    /// to someone else.
    fn run_window_key(&self, command: KeyCommand) -> glib::Propagation {
        if !self.window_key_applies(command) {
            return glib::Propagation::Proceed;
        }
        // GTK fails only for an action no ancestor has; the window and
        // the application register all of these.
        let _ = WidgetExt::activate_action(self, &command.detailed_name(), None);
        glib::Propagation::Stop
    }

    /// Whether `command`'s key acts now: not from a text field and not
    /// while a dialog is open, except that tab switching works from the
    /// active tab's Properties dialog.
    fn window_key_applies(&self, command: KeyCommand) -> bool {
        if command.is_tab_switch() && self.shows_dialog_of_active_tab() {
            return true;
        }
        if self.dialog_layer().shown().is_some() {
            return false;
        }
        !self.focus_is_in_text_field()
    }

    /// Whether Ctrl+Tab switches tabs now, for tests.
    #[cfg(test)]
    pub(super) fn tab_keys_apply(&self) -> bool {
        self.window_key_applies(KeyCommand::Window(WindowAction::NextTab))
    }

    /// Whether Ctrl+T opens a tab now, for tests.
    #[cfg(test)]
    pub(super) fn new_tab_key_applies(&self) -> bool {
        self.window_key_applies(KeyCommand::Window(WindowAction::NewTab))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys of `command` in [`WINDOW_KEYS`].
    fn keys_of(command: KeyCommand) -> &'static str {
        WINDOW_KEYS
            .iter()
            .find(|(listed, _)| *listed == command)
            .map_or_else(|| panic!("{command:?} has keys"), |(_, keys)| *keys)
    }

    /// parity: TAB-001, TAB-002, TAB-005, TAB-043
    #[gtk::test]
    fn the_tab_and_window_keys_are_those_of_on_key() {
        assert_eq!(keys_of(KeyCommand::Window(WindowAction::NewTab)), "<Primary>t");
        assert_eq!(keys_of(KeyCommand::Window(WindowAction::CloseTab)), "<Primary>w");
        assert_eq!(keys_of(KeyCommand::NewWindow), "<Primary>n");
        assert!(keys_of(KeyCommand::Window(WindowAction::NextTab)).starts_with("<Primary>Tab"));
        assert!(keys_of(KeyCommand::Window(WindowAction::PreviousTab)).contains("<Primary><Shift>Tab"));
        for (_, keys) in WINDOW_KEYS {
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            assert!(trigger.is_some(), "GTK parses {keys}");
        }
    }
}
