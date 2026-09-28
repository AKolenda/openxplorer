// SPDX-License-Identifier: AGPL-3.0-only
//! Fixtures shared by the network tests: an in-memory keyring standing in
//! for the Secret Service as `FakeSecret` in `desktop/tests/test_v05.py`
//! does for libsecret, a sign-in dialog that records what it was asked to
//! show, and sign-in prompts on a private main context.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::keyring::{Keyring, KeyringCollection, KeyringError, NewSecret, SecretAttributes};
use super::prompts::{Challenge, ChallengeId, MountPrompts, SignInPrompter, CHALLENGE_LIFETIME};
use super::session_credentials::SessionCredentials;

/// One stored item.
#[derive(Debug, Clone)]
struct Item {
    attributes: SecretAttributes,
    text: String,
}

/// Called at the start of a keyring call, for tests that hold a save in
/// flight or race a lookup.
type Hook = Box<dyn Fn() + Send + Sync>;

/// A keyring that keeps its items in memory and records every save.
#[derive(Default)]
pub(crate) struct MemoryKeyring {
    items: Mutex<Vec<Item>>,
    saves: Mutex<Vec<(KeyringCollection, SecretAttributes)>>,
    store_hook: Mutex<Option<Hook>>,
    lookup_hook: Mutex<Option<Hook>>,
}

impl MemoryKeyring {
    /// The collection and attributes of every save, oldest first.
    pub(crate) fn saves(&self) -> Vec<(KeyringCollection, SecretAttributes)> {
        locked(&self.saves).clone()
    }

    /// The collection of the latest save.
    pub(crate) fn last_saved_collection(&self) -> Option<KeyringCollection> {
        self.saves().last().map(|(collection, _)| *collection)
    }

    /// True when no item is stored.
    pub(crate) fn is_empty(&self) -> bool {
        locked(&self.items).is_empty()
    }

    /// The text of every stored item, for checks that a secret was or was
    /// not written.
    pub(crate) fn texts(&self) -> Vec<String> {
        locked(&self.items).iter().map(|item| item.text.clone()).collect()
    }

    /// True when an item matching `query` is stored.
    pub(crate) fn contains(&self, query: &SecretAttributes) -> bool {
        locked(&self.items)
            .iter()
            .any(|item| query.matches(&item.attributes))
    }

    /// Runs `hook` at the start of every store.
    pub(crate) fn set_store_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        *locked(&self.store_hook) = Some(Box::new(hook));
    }

    /// Runs `hook` at the start of every lookup.
    pub(crate) fn set_lookup_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        *locked(&self.lookup_hook) = Some(Box::new(hook));
    }
}

impl Keyring for MemoryKeyring {
    fn lookup(&self, query: &SecretAttributes) -> Result<Option<String>, KeyringError> {
        if let Some(hook) = locked(&self.lookup_hook).as_ref() {
            hook();
        }
        let items = locked(&self.items);
        let found = items.iter().find(|item| query.matches(&item.attributes));
        Ok(found.map(|item| item.text.clone()))
    }

    fn store(&self, secret: &NewSecret<'_>) -> Result<(), KeyringError> {
        if let Some(hook) = locked(&self.store_hook).as_ref() {
            hook();
        }
        let mut items = locked(&self.items);
        items.retain(|item| item.attributes != *secret.attributes);
        items.push(Item {
            attributes: secret.attributes.clone(),
            text: secret.text.to_owned(),
        });
        locked(&self.saves).push((secret.collection, secret.attributes.clone()));
        Ok(())
    }

    fn clear(&self, query: &SecretAttributes) -> Result<bool, KeyringError> {
        let mut items = locked(&self.items);
        let before = items.len();
        items.retain(|item| !query.matches(&item.attributes));
        Ok(items.len() < before)
    }
}

/// A sign-in dialog that records everything the prompts asked it to do.
#[derive(Debug, Default)]
pub(crate) struct RecordingPrompter {
    shown: RefCell<Vec<Challenge>>,
    dismissed: RefCell<Vec<ChallengeId>>,
    notices: RefCell<Vec<String>>,
}

impl SignInPrompter for RecordingPrompter {
    fn show_challenge(&self, challenge: &Challenge) {
        self.shown.borrow_mut().push(challenge.clone());
    }

    fn dismiss_challenge(&self, id: ChallengeId) {
        self.dismissed.borrow_mut().push(id);
    }

    fn show_notice(&self, message: &str) {
        self.notices.borrow_mut().push(message.to_owned());
    }
}

impl RecordingPrompter {
    /// How many challenges were shown.
    pub(crate) fn shown_count(&self) -> usize {
        self.shown.borrow().len()
    }

    /// The latest challenge shown.
    pub(crate) fn last_shown(&self) -> Challenge {
        let shown = self.shown.borrow();
        shown.last().cloned().expect("a challenge was shown")
    }

    /// True when the dialog of `id` was removed.
    pub(crate) fn is_dismissed(&self, id: ChallengeId) -> bool {
        self.dismissed.borrow().contains(&id)
    }

    /// Every notice shown, oldest first.
    pub(crate) fn notices(&self) -> Vec<String> {
        self.notices.borrow().clone()
    }
}

/// Sign-in prompts on an in-memory keyring, run on a private main context.
pub(crate) struct PromptsFixture {
    pub(crate) context: glib::MainContext,
    pub(crate) keyring: Arc<MemoryKeyring>,
    pub(crate) credentials: Arc<SessionCredentials>,
    pub(crate) prompter: Rc<RecordingPrompter>,
    pub(crate) prompts: MountPrompts,
}

/// Runs `test` with fresh prompts whose challenges live `lifetime`, on a
/// new main context made the thread default.
pub(crate) fn with_prompts_living(lifetime: Duration, test: impl FnOnce(&PromptsFixture)) {
    let keyring = Arc::new(MemoryKeyring::default());
    let credentials = Arc::new(SessionCredentials::new(keyring.clone()));
    with_credentials_and_lifetime(keyring, credentials, lifetime, test);
}

/// Runs `test` with fresh prompts on an in-memory keyring.
pub(crate) fn with_prompts(test: impl FnOnce(&PromptsFixture)) {
    with_prompts_living(CHALLENGE_LIFETIME, test);
}

/// Runs `test` with prompts whose credential store has no keyring; the
/// fixture's keyring stays empty.
pub(crate) fn with_memory_only_prompts(test: impl FnOnce(&PromptsFixture)) {
    let keyring = Arc::new(MemoryKeyring::default());
    let credentials = Arc::new(SessionCredentials::memory_only());
    with_credentials_and_lifetime(keyring, credentials, CHALLENGE_LIFETIME, test);
}

fn with_credentials_and_lifetime(
    keyring: Arc<MemoryKeyring>,
    credentials: Arc<SessionCredentials>,
    lifetime: Duration,
    test: impl FnOnce(&PromptsFixture),
) {
    let context = glib::MainContext::new();
    let prompter = Rc::new(RecordingPrompter::default());
    context
        .with_thread_default(|| {
            let prompts =
                MountPrompts::with_challenge_lifetime(Arc::clone(&credentials), prompter.clone(), lifetime);
            let fixture = PromptsFixture {
                context: context.clone(),
                keyring,
                credentials,
                prompter,
                prompts,
            };
            test(&fixture);
            fixture.prompts.close();
        })
        .expect("a new main context can be made the thread default");
}

impl PromptsFixture {
    /// Runs the main context until `condition` holds.
    pub(crate) fn wait_until(&self, what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            if !self.context.iteration(false) {
                thread::sleep(Duration::from_millis(2));
            }
        }
    }

    /// Runs `future` to completion on the fixture's main context.
    pub(crate) fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.context.block_on(future)
    }
}

/// Locks `mutex`, ignoring poisoning: a failed test must not hide the
/// assertion of another.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
