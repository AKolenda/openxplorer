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
fn cap_body(dialog: &gtk::Window, scroller: &gtk::ScrolledWindow, cap: i32) {
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
