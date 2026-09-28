// SPDX-License-Identifier: AGPL-3.0-only
//! Ports `CredentialsTests` of `desktop/tests/test_v05.py` and the keyring
//! races of `AdditionalSecurityTests` in
//! `desktop/tests/test_terminal_security.py`, against an in-memory
//! keyring.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::*;
use crate::network::credential::Password;
use crate::network::keyring::KeyringCollection;
use crate::network::test_support::MemoryKeyring;

/// The account of `CredentialsTests.setUp`.
fn sam() -> Credential {
    Credential {
        username: "sam".into(),
        domain: "WORKGROUP".into(),
        password: Password::from("not-a-real-password"),
        scope: CredentialScope::Session,
    }
}

/// A store on a fresh in-memory keyring, and the keyring.
fn store_with_keyring() -> (SessionCredentials, Arc<MemoryKeyring>) {
    let keyring = Arc::new(MemoryKeyring::default());
    let store = SessionCredentials::new(keyring.clone());
    (store, keyring)
}

/// Saves `credential` for the server of `uri` under its current generation.
fn persist(store: &SessionCredentials, uri: &str, credential: &Credential) {
    let generation = store.generation(uri);
    store
        .persist(uri, credential, generation)
        .expect("the memory keyring saves");
}

fn load(store: &SessionCredentials, uri: &str) -> Option<Credential> {
    store.load(uri).expect("the memory keyring can be searched")
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_session_collection_unchecked`
///
/// parity: NET-011, NET-015
#[test]
fn unchecked_remember_saves_in_the_session_collection() {
    let (store, keyring) = store_with_keyring();

    persist(&store, "smb://nas/a", &sam());

    assert_eq!(keyring.last_saved_collection(), Some(KeyringCollection::Session));
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_permanent_checked`
///
/// parity: NET-011, NET-015
#[test]
fn checked_remember_saves_in_the_default_collection() {
    let (store, keyring) = store_with_keyring();
    let remembered = Credential {
        scope: CredentialScope::Permanent,
        ..sam()
    };

    persist(&store, "smb://nas/a", &remembered);

    assert_eq!(keyring.last_saved_collection(), Some(KeyringCollection::Default));
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_cross_share_load`
///
/// parity: NET-014
#[test]
fn another_share_on_the_server_loads_the_saved_credential() {
    let (store, _keyring) = store_with_keyring();

    persist(&store, "smb://nas/a", &sam());

    assert_eq!(load(&store, "smb://nas/b"), Some(sam()));
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_other_window_load`
///
/// The Python windows each had a `SessionCredentials`; here another store
/// on the same keyring stands for another `OpenXplorer` process.
///
/// parity: NET-014
#[test]
fn another_store_on_the_same_keyring_loads_the_saved_credential() {
    let (store, keyring) = store_with_keyring();
    persist(&store, "smb://nas/a", &sam());

    let other = SessionCredentials::new(keyring);

    assert_eq!(load(&other, "smb://nas/b"), Some(sam()));
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_other_host_no_load`
///
/// parity: NET-014
#[test]
fn another_server_does_not_load_the_credential() {
    let (store, _keyring) = store_with_keyring();

    persist(&store, "smb://nas/a", &sam());

    assert_eq!(load(&store, "smb://other/a"), None);
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_session_overrides_older_account`
///
/// parity: NET-011, NET-014
#[test]
fn a_session_entry_wins_over_an_older_permanent_account() {
    let (store, _keyring) = store_with_keyring();
    let old_account = Credential {
        username: "old".into(),
        scope: CredentialScope::Permanent,
        ..sam()
    };
    persist(&store, "smb://nas/a", &old_account);

    persist(&store, "smb://nas/b", &sam());

    let loaded = load(&store, "smb://nas/a").expect("a saved credential");
    assert_eq!(loaded.username, "sam");
}

/// A permanent save removes the server's stale session entry
/// (`_persist_current` in `session_credentials.py`).
///
/// parity: NET-015
#[test]
fn a_permanent_save_removes_the_stale_session_entry() {
    let (store, keyring) = store_with_keyring();
    persist(&store, "smb://nas/a", &sam());
    let remembered = Credential {
        username: "new".into(),
        scope: CredentialScope::Permanent,
        ..sam()
    };

    persist(&store, "smb://nas/a", &remembered);

    assert_eq!(keyring.texts(), [remembered.to_keyring_text()]);
    let fresh = SessionCredentials::new(keyring);
    assert_eq!(load(&fresh, "smb://nas/a"), Some(remembered));
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_forget_clears_only_matching_host`
///
/// parity: NET-020, NET-021
#[test]
fn forgetting_clears_only_the_matching_server() {
    let (store, _keyring) = store_with_keyring();
    persist(&store, "smb://nas/a", &sam());
    persist(&store, "smb://other/a", &sam());

    let removed = store.forget("smb://nas/b", ForgetScope::AllScopes);

    assert_eq!(removed, Ok(true));
    assert_eq!(load(&store, "smb://nas/a"), None);
    assert!(load(&store, "smb://other/a").is_some());
}

/// Sign out without "Forget saved credentials" removes session entries
/// only (`forget(uri, permanent=False)`).
///
/// parity: NET-021
#[test]
fn forgetting_session_entries_keeps_the_permanent_account() {
    let (store, _keyring) = store_with_keyring();
    let remembered = Credential {
        scope: CredentialScope::Permanent,
        ..sam()
    };
    persist(&store, "smb://nas/a", &remembered);

    let removed = store.forget("smb://nas/a", ForgetScope::SessionOnly);

    assert_eq!(removed, Ok(false));
    assert_eq!(load(&store, "smb://nas/a"), Some(remembered));
}

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_no_plaintext_fallback`
///
/// parity: NET-011, SAFE-011
#[test]
fn without_a_keyring_nothing_is_saved_and_memory_still_works() {
    let store = SessionCredentials::memory_only();
    store.accept_memory("smb://nas/a", &sam());

    let saved = store.persist("smb://nas/a", &sam(), store.generation("smb://nas/a"));

    assert_eq!(saved, Err(KeyringError::Unavailable));
    assert_eq!(store.peek("smb://nas/b"), Some(sam()));
}

/// A saved credential leaves memory, so the next sign-in reads the
/// keyring and sees a Sign out made by another process.
#[test]
fn a_saved_credential_leaves_memory() {
    let (store, _keyring) = store_with_keyring();
    store.accept_memory("smb://nas/a", &sam());

    persist(&store, "smb://nas/a", &sam());

    assert_eq!(store.peek("smb://nas/a"), None);
}

/// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_stale_credential_write_after_signout_discarded`
///
/// parity: NET-020, NET-021, SAFE-012
#[test]
fn a_save_started_before_sign_out_is_discarded() {
    let (store, keyring) = store_with_keyring();
    let uri = "smb://security-test-nas/Projects";
    let generation = store.generation(uri);
    store.accept_memory(uri, &sam());

    store.forget_memory(uri);
    store
        .forget(uri, ForgetScope::AllScopes)
        .expect("the memory keyring clears");
    store
        .persist(uri, &sam(), generation)
        .expect("a stale save is skipped");

    assert!(keyring.is_empty());
    assert_eq!(store.peek(uri), None);
}

/// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_forget_waits_for_inflight_keyring_save`
///
/// parity: NET-020, NET-021, SAFE-012
#[test]
fn forgetting_waits_for_a_save_in_flight() {
    let (store, keyring) = store_with_keyring();
    let store = Arc::new(store);
    let uri = "smb://security-test-nas/Projects";
    let (started, save_started) = mpsc::channel();
    let (release, release_save) = mpsc::channel::<()>();
    let release_save = Mutex::new(release_save);
    keyring.set_store_hook(move || {
        started.send(()).expect("the test waits for the save");
        let _released = locked(&release_save).recv_timeout(Duration::from_secs(2));
    });
    let generation = store.generation(uri);

    let writer = thread::spawn({
        let store = Arc::clone(&store);
        move || store.persist(uri, &sam(), generation)
    });
    save_started
        .recv_timeout(Duration::from_secs(1))
        .expect("the save starts");
    let clearer = thread::spawn({
        let store = Arc::clone(&store);
        move || store.forget(uri, ForgetScope::AllScopes)
    });
    release.send(()).expect("the save waits for release");

    let saved = writer.join().expect("the writer finishes");
    let forgotten = clearer.join().expect("the clearer finishes");
    assert_eq!(saved, Ok(()));
    assert_eq!(forgotten, Ok(true));
    assert!(keyring.is_empty());
}

/// A keyring lookup that raced Sign out does not bring the credential
/// back (`generation != self.generation(uri)` in `load`).
///
/// parity: SAFE-012
#[test]
fn a_lookup_that_raced_sign_out_loads_nothing() {
    let (store, keyring) = store_with_keyring();
    let store = Arc::new(store);
    let uri = "smb://nas/a";
    persist(&store, uri, &sam());
    let signing_out = Arc::downgrade(&store);
    keyring.set_lookup_hook(move || {
        if let Some(store) = signing_out.upgrade() {
            store.forget_memory(uri);
        }
    });

    assert_eq!(load(&store, uri), None);
    assert_eq!(store.peek(uri), None);
}

/// parity: SAFE-011
#[test]
fn debug_output_names_servers_but_never_passwords() {
    let (store, _keyring) = store_with_keyring();
    store.accept_memory("smb://nas/a", &sam());

    let debug = format!("{store:?}");

    assert!(debug.contains("nas:445"), "{debug}");
    assert!(!debug.contains("not-a-real-password"), "{debug}");
}
