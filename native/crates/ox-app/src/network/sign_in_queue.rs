// SPDX-License-Identifier: AGPL-3.0-only
//! One sign-in dialog at a time for a window's network challenges.
//!
//! Ports `receiveAuth`, `pumpAuth`, `dismissAuth` and `answerAuth` in
//! `desktop/ui/app.js`. ox-core's
//! [`MountPrompts`](ox_core::network::MountPrompts) asks through the
//! [`SignInPrompter`] this queue implements:
//!
//! - Challenges that arrive while one is shown wait in order; a duplicate
//!   is ignored, and dismissing a waiting challenge drops it (NET-010).
//! - The dialog is modal over the window, or over the window's own
//!   dialog, such as Map network location, which stays open underneath
//!   (NET-009).
//! - An answer the service refuses keeps the dialog open with the reason;
//!   an accepted one dismisses it, and the next challenge follows.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;
use ox_core::network::{Answer, Challenge, ChallengeId, SignInError, SignInPrompter};

use crate::dialogs::SignInDialog;

/// Answers a challenge: the window passes it to its prompts.
type AnswerChallenge = dyn Fn(ChallengeId, Answer) -> Result<(), SignInError>;

/// Shows a passing notice in the window.
type ShowNotice = dyn Fn(&str);

/// The dialog on screen and the challenge it shows.
struct ShownChallenge {
    id: ChallengeId,
    dialog: SignInDialog,
}

/// A window's sign-in challenges: the one on screen and those waiting.
pub(crate) struct SignInQueue {
    /// The queue itself, which the dialogs' handlers hold weakly.
    this: Weak<SignInQueue>,
    /// The window the dialogs belong to.
    window: glib::WeakRef<gtk::Window>,
    /// Challenges waiting for the one on screen, oldest first.
    waiting: RefCell<VecDeque<Challenge>>,
    /// The challenge on screen, if any.
    shown: RefCell<Option<ShownChallenge>>,
    /// Passes answers to the window's prompts.
    answer: RefCell<Option<Box<AnswerChallenge>>>,
    /// Shows notices such as the keyring's.
    notices: RefCell<Option<Box<ShowNotice>>>,
}

impl SignInQueue {
    /// A queue that shows the challenges of `window`.
    pub(crate) fn new(window: &impl IsA<gtk::Window>) -> Rc<Self> {
        let window = window.as_ref().downgrade();
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            window,
            waiting: RefCell::default(),
            shown: RefCell::default(),
            answer: RefCell::default(),
            notices: RefCell::default(),
        })
    }

    /// Passes the user's answers to `answer`, which checks them.
    pub(crate) fn answer_with(
        &self,
        answer: impl Fn(ChallengeId, Answer) -> Result<(), SignInError> + 'static,
    ) {
        self.answer.replace(Some(Box::new(answer)));
    }

    /// Shows the prompts' notices with `show`.
    pub(crate) fn show_notices_with(&self, show: impl Fn(&str) + 'static) {
        self.notices.replace(Some(Box::new(show)));
    }

    /// The dialog on screen, for tests.
    #[cfg(test)]
    pub(crate) fn shown_dialog(&self) -> Option<SignInDialog> {
        let shown = self.shown.borrow();
        shown.as_ref().map(|shown| shown.dialog.clone())
    }

    /// How many challenges wait behind the one on screen, for tests.
    #[cfg(test)]
    pub(crate) fn waiting_count(&self) -> usize {
        self.waiting.borrow().len()
    }

    /// The challenge on screen, for tests.
    #[cfg(test)]
    pub(crate) fn shown_id(&self) -> Option<ChallengeId> {
        self.shown.borrow().as_ref().map(|shown| shown.id)
    }

    /// Whether `id` is on screen or waiting.
    fn is_queued(&self, id: ChallengeId) -> bool {
        let is_shown = self.shown.borrow().as_ref().is_some_and(|shown| shown.id == id);
        is_shown || self.waiting.borrow().iter().any(|waiting| waiting.id == id)
    }

    /// Shows the oldest waiting challenge when none is on screen.
    fn show_next(&self) {
        if self.shown.borrow().is_some() {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(challenge) = self.waiting.borrow_mut().pop_front() else {
            return;
        };
        let dialog = SignInDialog::new(&dialog_parent(&window), &challenge);
        let queue = self.this.clone();
        let id = challenge.id;
        dialog.connect_answered(move |dialog, answer| {
            if let Some(queue) = queue.upgrade() {
                queue.submit(id, dialog, answer);
            }
        });
        let shown = ShownChallenge {
            id,
            dialog: dialog.clone(),
        };
        self.shown.replace(Some(shown));
        dialog.present();
    }

    /// Passes `answer` to challenge `id`. A refused answer keeps the
    /// dialog open with the reason; a Cancel that cannot reach the
    /// prompts closes it anyway, so the user is never stuck.
    fn submit(&self, id: ChallengeId, dialog: &SignInDialog, answer: Answer) {
        let is_cancel = answer == Answer::Cancel;
        let result = {
            let answer_challenge = self.answer.borrow();
            match answer_challenge.as_deref() {
                Some(answer_challenge) => answer_challenge(id, answer),
                None => Err(SignInError::Expired),
            }
        };
        let Err(error) = result else {
            return;
        };
        if is_cancel {
            self.dismiss_challenge(id);
        } else {
            dialog.show_error(&error.to_string());
        }
    }
}

impl SignInPrompter for SignInQueue {
    fn show_challenge(&self, challenge: &Challenge) {
        if self.is_queued(challenge.id) {
            return;
        }
        self.waiting.borrow_mut().push_back(challenge.clone());
        self.show_next();
    }

    fn dismiss_challenge(&self, id: ChallengeId) {
        self.waiting.borrow_mut().retain(|waiting| waiting.id != id);
        let is_shown = self.shown.borrow().as_ref().is_some_and(|shown| shown.id == id);
        if !is_shown {
            return;
        }
        let shown = self.shown.take();
        if let Some(shown) = shown {
            shown.dialog.dismiss();
        }
        self.show_next();
    }

    fn show_notice(&self, message: &str) {
        if let Some(show) = self.notices.borrow().as_deref() {
            show(message);
        }
    }
}

impl fmt::Debug for SignInQueue {
    /// Shows which challenges are queued, never what they ask.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = self.shown.borrow().as_ref().map(|shown| shown.id);
        let waiting: Vec<ChallengeId> = self
            .waiting
            .borrow()
            .iter()
            .map(|challenge| challenge.id)
            .collect();
        formatter
            .debug_struct("SignInQueue")
            .field("shown", &shown)
            .field("waiting", &waiting)
            .finish_non_exhaustive()
    }
}

/// The window a sign-in dialog belongs to: the dialog `window` shows,
/// such as Map network location, else `window` itself, so the dialog
/// underneath stays as it was (NET-009).
fn dialog_parent(window: &gtk::Window) -> gtk::Window {
    let is_over_window = |candidate: &gtk::Window| {
        candidate.is_visible()
            && candidate.is_modal()
            && candidate.transient_for().as_ref() == Some(window)
            && !candidate.is::<SignInDialog>()
    };
    let toplevels = gtk::Window::list_toplevels();
    let dialogs = toplevels
        .into_iter()
        .filter_map(|toplevel| toplevel.downcast::<gtk::Window>().ok());
    dialogs
        .filter(is_over_window)
        .last()
        .unwrap_or_else(|| window.clone())
}

#[cfg(test)]
mod tests;
