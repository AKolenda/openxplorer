// SPDX-License-Identifier: AGPL-3.0-only
//! One window's network state: its sign-in prompts, the queue of dialogs
//! that answers them, and why discovery could not run, if it could not.
//!
//! Ports what `v2.0.0:desktop/winspace.py` creates for each window (`MountPrompts`
//! over the window's `SessionCredentials`) and the discovery state of
//! `v2.0.0:desktop/ui/app.js`. Each window keeps its own credentials in memory,
//! over the keyring every window shares; closing the window wipes them
//! (SAFE-011, TAB-050).

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use ox_core::network::{mount_location, MountPrompts, NetworkError, SessionCredentials, SignInPrompter};

use super::services::NetworkServices;
use super::sign_in_queue::SignInQueue;

/// A mount of a share, running.
pub(crate) type Mounting = Pin<Box<dyn Future<Output = Result<(), NetworkError>>>>;

/// In tests, what mounting a share answers instead of GIO.
#[cfg(test)]
type MountAnswer = Rc<dyn Fn() -> Result<(), NetworkError>>;

/// The network state of one window.
pub(crate) struct WindowNetwork {
    /// Answers `GVfs`'s sign-in questions for the window's mounts.
    prompts: MountPrompts,
    /// Shows the questions one dialog at a time.
    sign_in: Rc<SignInQueue>,
    /// Why the last listing of Network could not discover computers;
    /// `None` when it could. Network's info bar shows it.
    discovery_problem: RefCell<Option<String>>,
    /// Test safety: tests answer mounts themselves, and a test that does
    /// not is refused, so no test mounts a share.
    #[cfg(test)]
    mount_answer: std::cell::RefCell<Option<MountAnswer>>,
}

/// What a mount answers in a test that did not choose an answer.
#[cfg(test)]
fn refuse_test_mount() -> Result<(), NetworkError> {
    let refusal = gtk::glib::Error::new(gtk::gio::IOErrorEnum::NotSupported, "Tests never mount a share.");
    Err(NetworkError::Gio(refusal))
}

impl std::fmt::Debug for WindowNetwork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowNetwork")
            .field("prompts", &self.prompts)
            .field("sign_in", &self.sign_in)
            .field("discovery_problem", &self.discovery_problem)
            .finish_non_exhaustive()
    }
}

impl WindowNetwork {
    /// The network state of `window`, over the keyring of `services`.
    /// Connect the queue's answers with [`SignInQueue::answer_with`].
    pub(crate) fn new(window: &impl IsA<gtk::Window>, services: &NetworkServices) -> Self {
        let credentials = Arc::new(SessionCredentials::new(services.credential_store()));
        let sign_in = SignInQueue::new(window);
        let prompter: Rc<dyn SignInPrompter> = Rc::clone(&sign_in) as Rc<dyn SignInPrompter>;
        Self {
            prompts: MountPrompts::new(credentials, prompter),
            sign_in,
            discovery_problem: RefCell::default(),
            #[cfg(test)]
            mount_answer: std::cell::RefCell::new(Some(Rc::new(refuse_test_mount))),
        }
    }

    /// Mounts the share or volume that holds `uri`, asking for credentials
    /// through the window's sign-in dialog when the server wants them.
    pub(crate) fn mount(&self, uri: &str) -> Mounting {
        #[cfg(test)]
        if let Some(answer) = self.mount_answer.borrow().as_ref() {
            let answer = Rc::clone(answer);
            let uri = uri.to_owned();
            return Box::pin(async move {
                crate::test_support::held::wait_while_held(&crate::test_support::held::mount_key(&uri)).await;
                answer()
            });
        }
        let prompts = self.prompts.clone();
        let uri = uri.to_owned();
        Box::pin(async move { mount_location(&prompts, &uri).await })
    }

    /// Makes every mount answer what `answer` returns, for tests.
    #[cfg(test)]
    pub(crate) fn answer_mounts_with(&self, answer: impl Fn() -> Result<(), NetworkError> + 'static) {
        self.mount_answer.replace(Some(Rc::new(answer)));
    }

    /// The prompts every mount of the window uses.
    pub(crate) fn prompts(&self) -> &MountPrompts {
        &self.prompts
    }

    /// The window's sign-in dialogs.
    pub(crate) fn sign_in(&self) -> &Rc<SignInQueue> {
        &self.sign_in
    }

    /// Why the last listing of Network could not discover computers, or
    /// `None` when it could.
    pub(crate) fn discovery_problem(&self) -> Option<String> {
        self.discovery_problem.borrow().clone()
    }

    /// Records how the last listing of Network went: `problem` says why
    /// discovery could not run, or is `None` when it ran.
    pub(crate) fn set_discovery_problem(&self, problem: Option<String>) {
        self.discovery_problem.replace(problem);
    }

    /// The window closes: every open challenge is dismissed and its mount
    /// aborted, and the window's credentials are wiped from memory. A
    /// listing of Network still running ends with its tab.
    pub(crate) fn close(&self) {
        self.prompts.close();
    }
}
