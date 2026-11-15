// SPDX-License-Identifier: AGPL-3.0-only
//! What every modal dialog window of the app does: how it is shown and
//! what it does with the keyboard.
//!
//! Ports the Escape handling of `showModal` in `v2.0.0:desktop/ui/app.js`:
//! Escape answers "cancel", as the dialog's Close or Cancel button does.
//!
//! A dialog window is filled before it is shown and fits its parent when
//! it shows ([`fit_to_parent`], which each dialog calls as it is realized
//! and again whenever what it shows changes), so the desktop draws its
//! first frame at the size it keeps, never a frame that is cut off and
//! then grows. A dialog is never taller than the window it belongs to,
//! less a margin: its body scrolls instead, as
//! `.modal{max-height:calc(100vh - 50px)}` did, also when rows arrive after
//! it is shown. The in-window dialogs of [`crate::dialog`] follow the
//! same rule.

use gtk::prelude::*;
use gtk::{gdk, glib};

/// The room kept above and below a capped dialog (`.modal-layer`'s 25px).
const WINDOW_MARGIN: i32 = 25;

/// The body height a dialog is measured with to find how much of it is
/// not its scrolling body.
const PROBE_HEIGHT: i32 = 40;

/// Caps `scroller`, the scrolling body of `dialog`, so the whole dialog is
/// no taller than the window it belongs to, now and when its body grows.
/// A dialog calls this as it is shown, once it is filled, and again when
/// what it shows changes; one that reads what it shows in the background
/// is shown once that has arrived.
pub(crate) fn fit_to_parent(dialog: &impl IsA<gtk::Window>, scroller: &gtk::ScrolledWindow) {
    let dialog = dialog.upcast_ref::<gtk::Window>();
    match height_cap(dialog) {
        Some(cap) => cap_body(dialog, scroller, cap),
        None => scroller.set_max_content_height(-1),
    }
}

/// The tallest a dialog over its parent window may be, or `None` when it
/// has no parent window with a size yet.
fn height_cap(dialog: &gtk::Window) -> Option<i32> {
    let parent = dialog.transient_for()?;
    let height = parent.height();
    (height > 0).then(|| (height - 2 * WINDOW_MARGIN).max(PROBE_HEIGHT * 2))
}

/// Limits `scroller` so `dialog`, at the width it will have, is at most
/// `cap` tall; the body scrolls instead of the dialog outgrowing it. A
/// body shorter than the limit keeps its natural height.
pub(crate) fn cap_body(dialog: &gtk::Window, scroller: &gtk::ScrolledWindow, cap: i32) {
    let width = dialog_width(dialog);
    // With the body held at exactly PROBE_HEIGHT, the rest of the dialog
    // is what is left over: the title, the buttons and the padding.
    let minimum = scroller.min_content_height();
    scroller.set_min_content_height(-1);
    scroller.set_max_content_height(PROBE_HEIGHT);
    scroller.set_min_content_height(PROBE_HEIGHT);
    let rest = dialog.measure(gtk::Orientation::Vertical, width).1 - PROBE_HEIGHT;
    scroller.set_min_content_height(-1);
    scroller.set_max_content_height((cap - rest).max(PROBE_HEIGHT));
    scroller.set_min_content_height(minimum.min(scroller.max_content_height()));
}

/// The width GTK gives `dialog`: its default width when it has one, never
/// less than it needs.
fn dialog_width(dialog: &gtk::Window) -> i32 {
    let (minimum, natural, _, _) = dialog.measure(gtk::Orientation::Horizontal, -1);
    match dialog.default_width() {
        width if width > 0 => width.max(minimum),
        _ => natural,
    }
}

/// Makes Tab and Shift+Tab in `window` go round its controls, as
/// Microsoft lists them for dialog boxes: "Tab: Move forward through
/// options", "Shift+Tab: Move back through options"
/// (<https://support.microsoft.com/en-us/accessibility/windows/keyboard-shortcuts-in-windows>),
/// ACC-004. GTK 4.14, Ubuntu 24.04's, failed twice here:
///
/// - Past the last control of a window with a title bar widget it does
///   not show, as the dialogs have, it gave the keyboard to nothing. So
///   once GTK has moved, the keyboard goes on to the first control (the
///   last, backward) when nothing has it.
/// - A scrolled body holding no control, as a question's lines about a
///   file, claimed Tab without taking the keyboard, even when it was not
///   focusable, so Tab went nowhere from there on. So before GTK moves,
///   each dialog body takes part only where it has something for the
///   keyboard ([`update_body_tab_stop`]).
pub(crate) fn tab_goes_round(window: &impl IsA<gtk::Window>) {
    let window = window.as_ref();
    window.connect_move_focus(|window, direction| {
        if is_tab(direction) {
            update_body_tab_stops(window.upcast_ref());
        }
    });
    window.connect_local("move-focus", true, |values| {
        let window = values.first()?.get::<gtk::Window>().ok()?;
        let direction = values.get(1)?.get::<gtk::DirectionType>().ok()?;
        if is_tab(direction) && GtkWindowExt::focus(&window).is_none() {
            if let Some(content) = window.child() {
                content.child_focus(direction);
            }
        }
        None
    });
}

/// Whether `direction` is Tab's or Shift+Tab's.
fn is_tab(direction: gtk::DirectionType) -> bool {
    matches!(
        direction,
        gtk::DirectionType::TabForward | gtk::DirectionType::TabBackward
    )
}

/// The CSS class of a dialog's scrolled body (`dialog-frame.ui`).
pub(crate) const DIALOG_BODY: &str = "dialog-body-scroller";

/// Updates the Tab stop of every dialog body under `root`.
pub(crate) fn update_body_tab_stops(root: &gtk::Widget) {
    let mut next = root.first_child();
    while let Some(widget) = next {
        match widget.downcast_ref::<gtk::ScrolledWindow>() {
            Some(scroller) if scroller.has_css_class(DIALOG_BODY) => update_body_tab_stop(scroller),
            _ => update_body_tab_stops(&widget),
        }
        next = widget.next_sibling();
    }
}

/// Lets a dialog's scrolled body `scroller` take Tab only for what it has
/// for the keyboard: itself while it has more to show than fits, so the
/// arrow and page keys scroll it (drawn with the ring, dialogs.css), and
/// the controls inside it. A body that fits and holds no control is no
/// stop: GTK would make it one that shows nothing, or one that never
/// takes the keyboard at all.
pub(crate) fn update_body_tab_stop(scroller: &gtk::ScrolledWindow) {
    let adjustment = scroller.vadjustment();
    let scrolls = adjustment.upper() - adjustment.page_size() > 0.5;
    let holds_a_control = scroller.child().is_some_and(|child| holds_a_control(&child));
    scroller.set_focusable(scrolls);
    scroller.set_can_focus(scrolls || holds_a_control);
}

/// Whether a shown, usable control under `widget`, or `widget` itself,
/// takes the keyboard.
fn holds_a_control(widget: &gtk::Widget) -> bool {
    if !widget.is_visible() || !widget.is_sensitive() || !widget.can_focus() {
        return false;
    }
    if widget.is_focusable() {
        return true;
    }
    let mut next = widget.first_child();
    while let Some(child) = next {
        if holds_a_control(&child) {
            return true;
        }
        next = child.next_sibling();
    }
    false
}

/// A controller for a window: F4 on a drop-down list shows its items, as
/// Microsoft lists for dialog boxes ("F4: Display the items in the active
/// list",
/// <https://support.microsoft.com/en-us/accessibility/windows/keyboard-shortcuts-in-windows>).
/// Elsewhere the key goes on, so the browser window's F4 still opens the
/// address bar. GTK opens a drop-down with Space or Enter only.
pub(crate) fn f4_shows_a_list() -> gtk::ShortcutController {
    let action = gtk::CallbackAction::new(|widget, _| {
        let focus = widget.root().and_then(|root| root.focus());
        let drop_down = focus.and_then(|focus| {
            focus
                .downcast_ref::<gtk::DropDown>()
                .cloned()
                .or_else(|| focus.ancestor(gtk::DropDown::static_type()).and_downcast())
        });
        let Some(drop_down) = drop_down.filter(|drop_down| drop_down.is_sensitive()) else {
            return glib::Propagation::Proceed;
        };
        show_items(&drop_down);
        glib::Propagation::Stop
    });
    let trigger = gtk::KeyvalTrigger::new(gdk::Key::F4, gdk::ModifierType::empty());
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
    shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
    shortcuts
}

/// Shows the list of `drop_down`, as a click on it does: its button is a
/// toggle button whose press opens the list.
fn show_items(drop_down: &gtk::DropDown) {
    let mut next = drop_down.first_child();
    while let Some(child) = next {
        if let Some(button) = child.downcast_ref::<gtk::ToggleButton>() {
            button.set_active(true);
            return;
        }
        next = child.next_sibling();
    }
}

/// A controller for a dialog window: Escape closes it through its close
/// request, which a dialog refuses while it must stay open, such as
/// Software updates during an installation.
pub(crate) fn escape_closes() -> gtk::ShortcutController {
    on_escape(|widget| {
        if let Some(window) = widget.downcast_ref::<gtk::Window>() {
            window.close();
        }
        glib::Propagation::Stop
    })
}

/// A controller that runs `dismiss` on the widget it belongs to when
/// Escape is pressed; `dismiss` says whether the key was used.
pub(crate) fn on_escape(
    dismiss: impl Fn(&gtk::Widget) -> glib::Propagation + 'static,
) -> gtk::ShortcutController {
    let action = gtk::CallbackAction::new(move |widget, _| dismiss(widget));
    let trigger = gtk::KeyvalTrigger::new(gdk::Key::Escape, gdk::ModifierType::empty());
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
    shortcuts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{settle, wait_until};

    /// A shown parent window `height` tall.
    fn parent(height: i32) -> gtk::Window {
        let parent = gtk::Window::builder()
            .default_width(640)
            .default_height(height)
            .build();
        parent.present();
        wait_until("the parent window to have its size", || parent.height() == height);
        parent
    }

    /// A dialog over `parent` whose body holds `lines` lines above a
    /// footer, and its scrolling body.
    fn dialog(parent: &gtk::Window, lines: usize) -> (gtk::Window, gtk::ScrolledWindow) {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for line in 0..lines {
            body.append(&gtk::Label::new(Some(&format!("Line {line}"))));
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .vexpand(true)
            .child(&body)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&scroller);
        column.append(&gtk::Button::with_label("Close"));
        let dialog = gtk::Window::builder()
            .transient_for(parent)
            .modal(true)
            .resizable(false)
            .default_width(400)
            .child(&column)
            .build();
        (dialog, scroller)
    }

    #[gtk::test]
    fn a_dialog_taller_than_its_window_is_capped_and_its_body_scrolls() {
        let parent = parent(360);
        let (dialog, scroller) = dialog(&parent, 200);

        fit_to_parent(&dialog, &scroller);
        dialog.present();
        wait_until("the dialog to show", || dialog.height() > 0);
        settle();

        let tallest = 360 - 2 * WINDOW_MARGIN;
        assert!(
            dialog.height() <= tallest,
            "{} fits in {tallest}",
            dialog.height()
        );
        let scrolling = scroller.vadjustment();
        assert!(scrolling.upper() > scrolling.page_size(), "the body scrolls");
        dialog.destroy();
        parent.destroy();
    }

    #[gtk::test]
    fn a_dialog_that_fits_keeps_its_natural_height() {
        let parent = parent(600);
        let (dialog, scroller) = dialog(&parent, 3);
        let natural = dialog.measure(gtk::Orientation::Vertical, 400).1;

        fit_to_parent(&dialog, &scroller);

        assert_eq!(dialog.measure(gtk::Orientation::Vertical, 400).1, natural);
        dialog.destroy();
        parent.destroy();
    }

    #[gtk::test]
    fn a_dialog_filled_after_it_shows_stays_within_its_window() {
        let parent = parent(360);
        let (dialog, scroller) = dialog(&parent, 3);
        fit_to_parent(&dialog, &scroller);
        dialog.present();
        wait_until("the dialog to show", || dialog.height() > 0);

        let body = scroller
            .child()
            .and_then(|viewport| viewport.first_child())
            .and_downcast::<gtk::Box>()
            .expect("the body is a box in a viewport");
        for line in 0..200 {
            body.append(&gtk::Label::new(Some(&format!("Added {line}"))));
        }
        settle();

        let tallest = 360 - 2 * WINDOW_MARGIN;
        assert!(
            dialog.height() <= tallest,
            "{} fits in {tallest}",
            dialog.height()
        );
        dialog.destroy();
        parent.destroy();
    }
}
