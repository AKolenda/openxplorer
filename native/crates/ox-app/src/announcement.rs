// SPDX-License-Identifier: AGPL-3.0-only
//! Messages read to screen readers without moving focus.
//!
//! GTK 4.14 gives a widget its test accessibility context when the
//! accessibility bus cannot be reached, or when `GTK_A11Y` is `none` or
//! `test`. That context has no announce handler, so
//! `gtk_accessible_announce` calls a null function and the app crashes.
//! Nothing listens to such a widget, so [`announce`] skips it.

use gtk::prelude::*;

/// The type name of GTK's accessibility context that reaches no screen
/// reader.
const TEST_CONTEXT: &str = "GtkTestATContext";

/// Reads `message` to screen readers from `widget`, at `priority`, when an
/// accessibility bus can carry it.
pub(crate) fn announce(
    widget: &impl IsA<gtk::Accessible>,
    message: &str,
    priority: gtk::AccessibleAnnouncementPriority,
) {
    if widget.at_context().type_().name() != TEST_CONTEXT {
        widget.announce(message, priority);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Announcing from a widget whose accessibility context reaches no
    /// screen reader does nothing instead of crashing.
    #[gtk::test]
    fn a_widget_without_an_accessibility_bus_announces_nothing() {
        let label = gtk::Label::new(Some("Dragging 2 items"));
        announce(
            &label,
            "Dragging 2 items",
            gtk::AccessibleAnnouncementPriority::Medium,
        );
    }
}
