// SPDX-License-Identifier: AGPL-3.0-only
//! Messages read to screen readers without moving focus.
//!
//! GTK 4.14 gives a widget its test accessibility context when the
//! accessibility bus cannot be reached, or when `GTK_A11Y` is `test`.
//! That context has no announce handler, so `gtk_accessible_announce`
//! calls a null function and the app crashes. With `GTK_A11Y` set to
//! `none` a widget has no context at all, and asking for it aborts the
//! app. Nothing listens in either case, so [`announce`] skips both.

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
    let a11y = std::env::var("GTK_A11Y").ok();
    if reaches_screen_readers(a11y.as_deref(), || widget.at_context().type_().name().to_owned()) {
        widget.announce(message, priority);
    }
}

/// Whether an announcement can reach a screen reader, with `GTK_A11Y` set
/// to `a11y`, from a widget whose accessibility context has the type
/// `context` names. With accessibility turned off (`none`) the widget has
/// no context, and the context is never asked for.
fn reaches_screen_readers(a11y: Option<&str>, context: impl FnOnce() -> String) -> bool {
    if a11y.is_some_and(|value| value.eq_ignore_ascii_case("none")) {
        return false;
    }
    context() != TEST_CONTEXT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With `GTK_A11Y=none`, which people set to turn GTK's accessibility
    /// off, GTK 4.14 gives a widget no accessibility context at all: asking
    /// for it aborted the app at the first file drag. The context is never
    /// asked for then; otherwise only the test context is left out.
    #[test]
    fn accessibility_turned_off_is_never_asked_for_a_context() {
        let never = || -> String { panic!("the context is never asked for") };
        assert!(!reaches_screen_readers(Some("none"), never));
        assert!(!reaches_screen_readers(Some("NONE"), never));
        assert!(!reaches_screen_readers(None, || TEST_CONTEXT.to_owned()));
        assert!(reaches_screen_readers(None, || "GtkAtSpiContext".to_owned()));
        assert!(reaches_screen_readers(Some("atspi"), || "GtkAtSpiContext".to_owned()));
    }

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

    /// With `GTK_A11Y=none`, which some users set to quiet the warning
    /// about a missing accessibility bus, GTK gives widgets no
    /// accessibility context at all. Announcing, as type-to-select does
    /// for every key, does nothing instead of stopping the app. GTK reads
    /// the variable once, so the test runs itself again with it set.
    #[gtk::test]
    fn a_widget_without_any_accessibility_context_announces_nothing() {
        if std::env::var_os("GTK_A11Y").as_deref() != Some(std::ffi::OsStr::new("none")) {
            let status = std::process::Command::new(std::env::current_exe().expect("the test binary"))
                .args([
                    "announcement::tests::a_widget_without_any_accessibility_context_announces_nothing",
                    "--exact",
                    "--test-threads=1",
                ])
                .env("GTK_A11Y", "none")
                .status()
                .expect("the test binary runs again");
            assert!(status.success(), "announcing with GTK_A11Y=none failed: {status}");
            return;
        }
        let window = gtk::Window::new();
        let label = gtk::Label::new(Some("Notes"));
        window.set_child(Some(&label));
        announce(&label, "Notes", gtk::AccessibleAnnouncementPriority::Medium);
        window.destroy();
    }
}
