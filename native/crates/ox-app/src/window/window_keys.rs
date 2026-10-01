// SPDX-License-Identifier: AGPL-3.0-only
//! The tab, window and history keys, and where they work (CMD-017,
//! TAB-005).
//!
//! Ports the keys `onKey` in `v2.0.0:desktop/ui/app.js` handles after its
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
    /// Alt+digit: the tab of that number, 0 for the last.
    TabNumber(u32),
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
            KeyCommand::TabNumber(_) => WindowAction::ShowTabNumber.detailed_name(),
        }
    }

    /// The action's parameter, if it takes one.
    fn target(self) -> Option<glib::Variant> {
        match self {
            KeyCommand::TabNumber(number) => Some(number.to_variant()),
            KeyCommand::Window(_) | KeyCommand::NewWindow => None,
        }
    }
}

/// Each command and its keys, as GTK parses them. Ctrl+Page Down and
/// Ctrl+Page Up are the tab keys of GNOME apps and browsers, and Ctrl+]
/// and Ctrl+[ Dolphin's, added to app.js's Ctrl+Tab; Alt+digit and
/// Ctrl+Shift+T are Dolphin's too.
const WINDOW_KEYS: [(KeyCommand, &str); 21] = [
    (KeyCommand::Window(WindowAction::Hidden), "<Primary>h"),
    (KeyCommand::Window(WindowAction::Sidebar), "F9"),
    (KeyCommand::NewWindow, "<Primary>n"),
    (KeyCommand::Window(WindowAction::NewTab), "<Primary>t"),
    (KeyCommand::Window(WindowAction::CloseTab), "<Primary>w"),
    (
        KeyCommand::Window(WindowAction::ReopenClosedTab),
        "<Primary><Shift>t",
    ),
    (
        KeyCommand::Window(WindowAction::NextTab),
        "<Primary>Tab|<Primary>KP_Tab|<Primary>Page_Down|<Primary>bracketright",
    ),
    (
        KeyCommand::Window(WindowAction::PreviousTab),
        "<Primary><Shift>ISO_Left_Tab|<Primary><Shift>Tab|<Primary>Page_Up|<Primary>bracketleft",
    ),
    (KeyCommand::TabNumber(1), "<Alt>1|<Alt>KP_1"),
    (KeyCommand::TabNumber(2), "<Alt>2|<Alt>KP_2"),
    (KeyCommand::TabNumber(3), "<Alt>3|<Alt>KP_3"),
    (KeyCommand::TabNumber(4), "<Alt>4|<Alt>KP_4"),
    (KeyCommand::TabNumber(5), "<Alt>5|<Alt>KP_5"),
    (KeyCommand::TabNumber(6), "<Alt>6|<Alt>KP_6"),
    (KeyCommand::TabNumber(7), "<Alt>7|<Alt>KP_7"),
    (KeyCommand::TabNumber(8), "<Alt>8|<Alt>KP_8"),
    (KeyCommand::TabNumber(9), "<Alt>9|<Alt>KP_9"),
    (KeyCommand::TabNumber(0), "<Alt>0|<Alt>KP_0"),
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
        let _ = WidgetExt::activate_action(self, &command.detailed_name(), command.target().as_ref());
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

    /// parity: TAB-001, TAB-002, TAB-005, TAB-006, TAB-007, TAB-016, TAB-043
    #[gtk::test]
    fn the_tab_and_window_keys_are_those_of_on_key() {
        assert_eq!(keys_of(KeyCommand::Window(WindowAction::NewTab)), "<Primary>t");
        assert_eq!(keys_of(KeyCommand::Window(WindowAction::CloseTab)), "<Primary>w");
        assert_eq!(keys_of(KeyCommand::NewWindow), "<Primary>n");
        assert!(keys_of(KeyCommand::Window(WindowAction::NextTab)).starts_with("<Primary>Tab"));
        assert!(keys_of(KeyCommand::Window(WindowAction::PreviousTab)).contains("<Primary><Shift>Tab"));
        assert!(keys_of(KeyCommand::Window(WindowAction::NextTab)).ends_with("|<Primary>bracketright"));
        assert!(keys_of(KeyCommand::Window(WindowAction::PreviousTab)).ends_with("|<Primary>bracketleft"));
        assert_eq!(
            keys_of(KeyCommand::Window(WindowAction::ReopenClosedTab)),
            "<Primary><Shift>t"
        );
        assert!(keys_of(KeyCommand::TabNumber(1)).starts_with("<Alt>1"));
        assert!(keys_of(KeyCommand::TabNumber(0)).starts_with("<Alt>0"));
        for (_, keys) in WINDOW_KEYS {
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            assert!(trigger.is_some(), "GTK parses {keys}");
        }
    }
}
