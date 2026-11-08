// SPDX-License-Identifier: AGPL-3.0-only
//! Ctrl+Shift+1 to Ctrl+Shift+8 as a keyboard sends them (CMD-039,
//! VIEW-001).
//!
//! The view keys are application accelerators, `<Primary><Shift>5` and
//! the others ([`super::actions::install_accelerators`]), which the View
//! menu and the keyboard shortcuts window read. A keyboard sends Shift+5
//! as the 5 key's shifted symbol, `%` on a US layout, and GTK does not
//! match that to `<Primary><Shift>5`, so the keys did nothing when typed.
//! The window also takes each digit key's shifted symbol, as the
//! keyboard layout gives it when the window is built, with Ctrl and
//! Shift, and runs the view of that digit, from any focus as the
//! accelerators do.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::folder_pane::FolderView;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The shifted symbols of the digit keys on a US layout, the fallback
/// where the display cannot say.
const US_SHIFTED: [&str; 8] = [
    "exclam",
    "at",
    "numbersign",
    "dollar",
    "percent",
    "asciicircum",
    "ampersand",
    "asterisk",
];

/// The digit of `view`'s Ctrl+Shift key, 1 to 8.
fn digit_of(view: FolderView) -> Option<usize> {
    let (accelerator, _) = view.shortcut()?;
    let digit = accelerator
        .strip_prefix("<Primary><Shift>")?
        .parse::<usize>()
        .ok()?;
    (1..=8).contains(&digit).then_some(digit)
}

/// The names of the keys `digit` gives with Shift held on `display`'s
/// layout: the symbol of the key that types the digit unshifted. Where the
/// layout types the digit only with Shift, as AZERTY does, the
/// accelerator matches as typed and there is none. Without a display, the
/// US symbol.
fn shifted_key_names(display: Option<&gdk::Display>, digit: usize) -> Vec<String> {
    let mut names = Vec::new();
    let digit_key = gdk::Key::from_name(digit.to_string());
    if let (Some(display), Some(digit_key)) = (display, digit_key) {
        for entry in display.map_keyval(digit_key).unwrap_or_default() {
            if entry.level() != 0 {
                continue;
            }
            let shifted =
                display.translate_key(entry.keycode(), gdk::ModifierType::SHIFT_MASK, entry.group());
            if let Some(name) = shifted.and_then(|(key, ..)| key.name()) {
                if name.as_str() != digit.to_string() && !names.iter().any(|known| known == name.as_str()) {
                    names.push(name.to_string());
                }
            }
        }
    }
    if display.is_none() {
        if let Some(name) = digit.checked_sub(1).and_then(|index| US_SHIFTED.get(index)) {
            names.push((*name).to_owned());
        }
    }
    names
}

/// Each view with a Ctrl+Shift key and the triggers of its shifted
/// symbols on `display`, as GTK parses them.
fn view_triggers(display: Option<&gdk::Display>) -> Vec<(FolderView, String)> {
    FolderView::NAMED
        .into_iter()
        .filter_map(|view| Some((view, digit_of(view)?)))
        .map(|(view, digit)| {
            let triggers: Vec<String> = shifted_key_names(display, digit)
                .into_iter()
                .map(|name| format!("<Primary><Shift>{name}"))
                .collect();
            (view, triggers.join("|"))
        })
        .filter(|(_, triggers)| !triggers.is_empty())
        .collect()
}

impl BrowserWindow {
    /// Adds Ctrl+Shift with each view digit's shifted symbol, before the
    /// focused widget sees the key.
    pub(super) fn install_view_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let display = gdk::Display::default();
        for (view, triggers) in view_triggers(display.as_ref()) {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(&triggers) else {
                continue;
            };
            let name = WindowAction::View.detailed_name();
            let target = view.as_str().to_variant();
            let run = gtk::CallbackAction::new(move |widget, _| {
                // GTK fails only for an action the window does not have,
                // and every browser window has the views.
                let _ = widget.activate_action(&name, Some(&target));
                glib::Propagation::Stop
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(run)));
        }
        self.add_controller(shortcuts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a display the US symbols stand in, one for each of the
    /// eight views with a digit, and GTK parses every trigger.
    ///
    /// parity: CMD-039
    #[gtk::test]
    fn every_view_digit_has_its_shifted_symbol() {
        let triggers = view_triggers(None);
        assert_eq!(triggers.len(), 8);
        let compact = triggers
            .iter()
            .find(|(view, _)| *view == FolderView::Compact)
            .map(|(_, triggers)| triggers.as_str());
        assert_eq!(compact, Some("<Primary><Shift>percent"));
        for (_, keys) in view_triggers(gdk::Display::default().as_ref()) {
            assert!(
                gtk::ShortcutTrigger::parse_string(&keys).is_some(),
                "GTK parses {keys}"
            );
        }
    }
}
