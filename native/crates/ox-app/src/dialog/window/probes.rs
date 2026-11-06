// SPDX-License-Identifier: AGPL-3.0-only
//! What tests read from and do to a [`Dialog`]: its texts and buttons.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::Dialog;

/// The text of `button`: its label, or the label beside its icon.
fn button_text(button: &gtk::Button) -> Option<String> {
    if let Some(label) = button.label() {
        return Some(label.into());
    }
    crate::test_support::harness::descendants::<gtk::Label>(button)
        .first()
        .map(|label| label.text().into())
}

/// Whether `button` shows `icon` before its label, for tests.
fn shows_icon(button: &gtk::Button, icon: crate::icons::Icon) -> bool {
    crate::test_support::harness::descendants::<gtk::Image>(button)
        .iter()
        .any(|image| image.icon_name().as_deref() == Some(icon.name()))
}

impl Dialog {
    /// Whether the button labelled `label` shows `icon`, for tests.
    pub(crate) fn button_shows_icon(&self, label: &str, icon: crate::icons::Icon) -> bool {
        shows_icon(&self.button_labelled(label), icon)
    }

    /// The text of the scrolled box, for tests.
    pub(crate) fn scrolled_text(&self) -> String {
        let scrolled = crate::window::children(&self.fields())
            .find_map(|child| child.downcast::<gtk::ScrolledWindow>().ok());
        let label = scrolled
            .and_then(|scrolled| scrolled.child())
            .and_then(|child| child.downcast::<gtk::Viewport>().ok()?.child())
            .and_then(|child| child.downcast::<gtk::Label>().ok());
        label.map(|label| label.text().to_string()).unwrap_or_default()
    }

    /// The heading, for tests.
    pub(crate) fn title_text(&self) -> String {
        self.imp().frame.title().to_string()
    }

    /// The message, for tests.
    pub(crate) fn message_text(&self) -> String {
        self.imp().frame.message().to_string()
    }

    /// The error line while shown, for tests.
    pub(crate) fn error_text(&self) -> Option<String> {
        let error = self.imp().frame.error_text();
        (!error.is_empty()).then(|| error.to_string())
    }

    /// The button labels in order, for tests.
    pub(crate) fn button_labels(&self) -> Vec<String> {
        let buttons = self.imp().buttons.borrow();
        buttons.iter().filter_map(button_text).collect()
    }

    /// Whether the button labelled `label` can be pressed, for tests.
    pub(crate) fn can_press(&self, label: &str) -> bool {
        self.button_labelled(label).is_sensitive()
    }

    /// Every text the dialog holds, shown or not, in order, for tests.
    pub(crate) fn texts(&self) -> Vec<String> {
        let labels = crate::test_support::harness::descendants::<gtk::Label>(self);
        labels.iter().map(|label| label.text().to_string()).collect()
    }

    /// Whether an answer is being carried out, for tests.
    pub(crate) fn is_busy(&self) -> bool {
        self.imp().running.borrow().is_some()
    }

    /// Presses the button labelled `label`, as a click does, for tests.
    pub(crate) fn press(&self, label: &str) {
        self.button_labelled(label).emit_clicked();
    }

    /// Presses the primary button, the one Enter presses, for tests.
    pub(crate) fn press_primary(&self) {
        let primary = self.default_widget().and_downcast::<gtk::Button>();
        primary.expect("the dialog has a primary button").emit_clicked();
    }

    /// The button labelled `label`, for tests.
    fn button_labelled(&self, label: &str) -> gtk::Button {
        self.imp()
            .buttons
            .borrow()
            .iter()
            .find(|button| button_text(button).as_deref() == Some(label))
            .cloned()
            .unwrap_or_else(|| panic!("the dialog has a {label} button"))
    }
}
