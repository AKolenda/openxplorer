// SPDX-License-Identifier: AGPL-3.0-only
//! The questions about one failed item ("Destination Folder Access
//! Denied", "File Access Denied", "File In Use", "Item Not Found",
//! "Interrupted Action"), asked in place of the action's progress, as
//! Windows 8 and later ask them inside the progress window (OPS-047).
//!
//! Like "Replace or Skip Files" and "There is not enough space"
//! ([`super::question`]), the action keeps its description; Windows'
//! caption takes the percentage's place and the window is titled "1
//! Interrupted Action". Under it: Windows' words, the item's icon, name
//! and facts, "Do this for all current items" while other items remain,
//! then the buttons: Try Again, or Continue with the administrator shield
//! where Windows asks for an administrator, Skip and Cancel. The progress
//! window's own More details shows where the item is and where it was
//! going, and stays as the window remembers it. Once answered, the action
//! shows its progress again.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::TransferPanel;
use crate::dialog::PropertyGrid;
use crate::icons::{self, Icon};

/// The edge of the shield on Continue.
const SHIELD_SIZE: i32 = 16;

/// The first button of a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FirstButton {
    /// "Try Again", or another plain label such as "Yes".
    Plain(String),
    /// "Continue" with the administrator shield.
    Administrator(String),
}

/// What a question about a failed item shows.
#[derive(Debug)]
pub(crate) struct FailureView {
    /// Windows' caption, in the percentage's place.
    pub(crate) caption: String,
    /// Windows' instruction and any lines under it.
    pub(crate) text: String,
    /// The item's icon, name and facts.
    pub(crate) card: gtk::Widget,
    /// What More details shows: names and values, such as Source and
    /// Destination.
    pub(crate) details: Vec<(String, String)>,
    /// Whether "Do this for all current items" is offered.
    pub(crate) for_all: bool,
    /// The first button.
    pub(crate) first: FirstButton,
    /// The second button's label: "Skip", or "No".
    pub(crate) second: String,
    /// Whether the question has a Cancel button of its own.
    pub(crate) cancel: bool,
}

/// What the user answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureChoice {
    /// Try Again, Continue or Yes.
    First,
    /// Skip or No.
    Second,
    /// Cancel, or the action was cancelled.
    Cancel,
}

impl TransferPanel {
    /// Asks `view` in place of the progress, and waits: the button pressed
    /// and whether "Do this for all current items" was checked.
    pub(crate) async fn ask_about_failure(&self, view: FailureView) -> (FailureChoice, bool) {
        let imp = self.imp();
        self.show_asking(&view.caption);
        let question = gtk::Box::new(gtk::Orientation::Vertical, 8);
        question.add_css_class("failure-question");
        let text = gtk::Label::new(Some(&view.text));
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        question.append(&text);
        question.append(&view.card);
        let details = PropertyGrid::new();
        for (name, value) in &view.details {
            details.add_row(name, value);
        }
        let details = details.widget().clone();
        details.add_css_class("failure-details");
        details.set_visible(imp.window.upgrade().is_some_and(|window| window.shows_details()));
        question.append(&details);
        let for_all = view.for_all.then(|| {
            let check =
                gtk::CheckButton::with_label(&ox_core::i18n::gettext("Do this for all current items"));
            question.append(&check);
            check
        });
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        buttons.set_margin_top(4);
        let (sender, receiver) = async_channel::bounded(1);
        let first = match &view.first {
            FirstButton::Plain(label) => gtk::Button::with_label(label),
            FirstButton::Administrator(label) => shield_button(label),
        };
        first.add_css_class("accent");
        let second = gtk::Button::with_label(&view.second);
        second.add_css_class("bordered");
        let mut answers = vec![(first, FailureChoice::First), (second, FailureChoice::Second)];
        if view.cancel {
            let cancel = gtk::Button::with_label(&ox_core::i18n::gettext("Cancel"));
            cancel.add_css_class("bordered");
            answers.push((cancel, FailureChoice::Cancel));
        }
        for (button, choice) in answers {
            button.add_css_class("file-conflicts-button");
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_| panel.answer_failure(choice)
            ));
            buttons.append(&button);
        }
        question.append(&buttons);
        self.insert_child_after(&question, Some(&*imp.space_question));
        imp.failure_question.replace(Some(question));
        imp.failure_details.replace(Some(details.upcast()));
        imp.failure_answer.replace(Some(sender));
        if imp
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(ox_core::transfer::Cancellation::is_cancelled)
        {
            self.answer_failure(FailureChoice::Cancel);
        }
        if let Some(window) = imp.window.upgrade() {
            window.show_question();
        }
        let choice = receiver.recv().await.unwrap_or(FailureChoice::Cancel);
        let for_all = for_all.is_some_and(|check| check.is_active());
        self.end_question();
        (choice, for_all)
    }

    /// Answers the question about a failed item, if one waits.
    pub(super) fn answer_failure(&self, choice: FailureChoice) {
        if let Some(sender) = self.imp().failure_answer.take() {
            // The receiver waits for exactly one answer.
            let _ = sender.try_send(choice);
        }
    }

    /// True while the action asks about a failed item.
    pub(crate) fn asks_about_failure(&self) -> bool {
        self.imp().failure_question.borrow().is_some()
    }

    /// Shows or hides the failure question's More details.
    pub(super) fn show_failure_details(&self, shown: bool) {
        if let Some(details) = self.imp().failure_details.borrow().as_ref() {
            details.set_visible(shown);
        }
    }

    /// Takes the failure question away, once answered.
    pub(super) fn remove_failure_question(&self) {
        let imp = self.imp();
        imp.failure_answer.replace(None);
        imp.failure_details.replace(None);
        if let Some(question) = imp.failure_question.take() {
            self.remove(&question);
        }
    }

    /// The question about a failed item while it shows, for tests.
    #[cfg(test)]
    pub(crate) fn failure_question(&self) -> Option<gtk::Box> {
        self.imp().failure_question.borrow().clone()
    }

    /// The caption the action shows in the percentage's place, for tests.
    #[cfg(test)]
    pub(crate) fn caption_shown(&self) -> String {
        self.imp().percent_label.text().into()
    }

    /// Every text of the question about a failed item, in order, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn failure_texts(&self) -> Vec<String> {
        let question = self.failure_question().expect("a question");
        crate::window::widget_tree::descendants::<gtk::Label>(&question)
            .iter()
            .map(|label| label.text().to_string())
            .collect()
    }

    /// The buttons of the question about a failed item, by their text,
    /// for tests.
    #[cfg(test)]
    fn failure_buttons(&self) -> Vec<(String, gtk::Button)> {
        let question = self.failure_question().expect("a question");
        crate::window::widget_tree::descendants::<gtk::Button>(&question)
            .into_iter()
            .filter(|button| button.has_css_class("file-conflicts-button"))
            .map(|button| {
                let text = button.label().map_or_else(
                    || {
                        crate::window::widget_tree::descendants::<gtk::Label>(&button)
                            .first()
                            .map(|label| label.text().to_string())
                            .unwrap_or_default()
                    },
                    String::from,
                );
                (text, button)
            })
            .collect()
    }

    /// The labels of the question's buttons, in order, for tests.
    #[cfg(test)]
    pub(crate) fn failure_button_labels(&self) -> Vec<String> {
        self.failure_buttons().into_iter().map(|(text, _)| text).collect()
    }

    /// Whether the question's first button shows the administrator
    /// shield, for tests.
    #[cfg(test)]
    pub(crate) fn failure_shows_shield(&self) -> bool {
        self.failure_buttons().first().is_some_and(|(_, button)| {
            crate::window::widget_tree::descendants::<gtk::Image>(button)
                .iter()
                .any(|image| image.icon_name().as_deref() == Some(Icon::ShieldLock.name()))
        })
    }

    /// Checks or clears "Do this for all current items", for tests; false
    /// when the question has none.
    #[cfg(test)]
    pub(crate) fn set_failure_for_all(&self, active: bool) -> bool {
        let question = self.failure_question().expect("a question");
        let check = crate::window::widget_tree::descendants::<gtk::CheckButton>(&question)
            .into_iter()
            .next();
        if let Some(check) = &check {
            check.set_active(active);
        }
        check.is_some()
    }

    /// Whether the question's More details shows, for tests.
    #[cfg(test)]
    pub(crate) fn failure_details_shown(&self) -> bool {
        self.imp()
            .failure_details
            .borrow()
            .as_ref()
            .is_some_and(gtk::Widget::is_visible)
    }

    /// Presses the question's button labelled `label`, for tests.
    #[cfg(test)]
    pub(crate) fn press_failure(&self, label: &str) {
        let (_, button) = self
            .failure_buttons()
            .into_iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("the question has a {label} button"));
        button.emit_clicked();
    }
}

/// A button with the administrator shield before `label`, which still
/// names it for screen readers.
fn shield_button(label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.set_halign(gtk::Align::Center);
    content.append(&icons::image(Icon::ShieldLock, SHIELD_SIZE));
    content.append(&gtk::Label::new(Some(label)));
    button.set_child(Some(&content));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}
