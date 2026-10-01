// SPDX-License-Identifier: AGPL-3.0-only
//! The window-wide selection keys: Ctrl+A selects every shown item and
//! Escape clears the selection, wherever keyboard focus is outside a text
//! field. Ctrl+H is one of the window keys (see [`super::window_keys`]).
//!
//! Ports the document-level part of `onKey` in `v2.0.0:desktop/ui/app.js`
//! (SEL-004, SEL-005). The folder views handle Ctrl+A (GTK's own
//! list binding) and Escape (the typed prefix first, see [`super::input`])
//! before these keys reach the window; this controller runs in the bubble
//! phase, after the focused widget had its turn, so it only sees them when
//! focus is elsewhere, such as the sidebar or the command bar. The keys
//! are not application accelerators: GTK runs those in the capture phase,
//! so Ctrl+A would stop selecting the search box's text.

use gtk::glib;
use gtk::prelude::*;

use super::gestures;
use super::BrowserWindow;

/// A window-wide key and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowKey {
    /// Ctrl+A: selects every shown item (`filtered()` in app.js).
    SelectAll,
    /// Escape: clears the selection (`clearSelection`).
    ClearSelection,
}

impl WindowKey {
    /// Every key, with its triggers as GTK parses them.
    const ALL: [(WindowKey, &'static str); 2] = [
        (WindowKey::SelectAll, "<Primary>a"),
        (WindowKey::ClearSelection, "Escape"),
    ];
}

impl BrowserWindow {
    /// Adds the window-wide selection keys, and ends a typed
    /// prefix on every pointer press in the window (SEL-031).
    pub(super) fn install_selection_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Bubble);
        for (key, trigger) in WindowKey::ALL {
            let trigger = gtk::ShortcutTrigger::parse_string(trigger);
            let run = gtk::CallbackAction::new(move |widget, _| {
                let Some(window) = widget.downcast_ref::<BrowserWindow>() else {
                    return glib::Propagation::Proceed;
                };
                window.run_selection_key(key)
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
        }
        self.add_controller(shortcuts);
        self.add_controller(self.prefix_reset_on_any_press());
    }

    /// Runs `key` unless focus is where it means something else: Ctrl+A
    /// and Escape leave text fields, the address bar and the Settings page
    /// alone.
    pub(super) fn run_selection_key(&self, key: WindowKey) -> glib::Propagation {
        if !self.file_keys_apply() {
            return glib::Propagation::Proceed;
        }
        match key {
            WindowKey::SelectAll => self.folder_pane().model().select_all(),
            WindowKey::ClearSelection => self.clear_selection(),
        }
        glib::Propagation::Stop
    }

    /// Clears the selection and the typed prefix (`clearSelection`).
    pub(super) fn clear_selection(&self) {
        self.reset_typeahead();
        self.folder_pane().model().select_none();
    }

    /// A pointer press anywhere in the window starts a new typed prefix,
    /// as the capture-phase `pointerdown` listener of app.js does; the
    /// press itself goes on to its widget.
    fn prefix_reset_on_any_press(&self) -> gtk::GestureClick {
        let press = gtk::GestureClick::new();
        press.set_button(gestures::EVERY_BUTTON);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.reset_typeahead()
        ));
        press
    }
}
