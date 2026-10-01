// SPDX-License-Identifier: AGPL-3.0-only
//! Ports `CredentialsTests` of `v2.0.0:desktop/tests/test_v05.py` and the keyring
//! races of `AdditionalSecurityTests` in
//! `v2.0.0:desktop/tests/test_terminal_security.py`, against an in-memory
//! keyring.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::network::credential::Password;
use crate::network::keyring::{KeyringCollection, SecretAttributes};
use crate::network::test_support::{store_with_keyring, window_credentials, KeyringChange, MemoryKeyring};

/// The account of `CredentialsTests.setUp`.
fn sam() -> Credential {
    Credential {
        username: "sam".into(),
        domain: "WORKGROUP".into(),
        password: Password::from("not-a-real-password"),
        scope: CredentialScope::Session,
    }
}

fn remembered_sam() -> Credential {
    Credential {
        scope: CredentialScope::Permanent,
        ..sam()
    }
}

/// One window's credentials on a fresh in-memory keyring, and the keyring.
fn window_with_keyring() -> (Arc<SessionCredentials>, Arc<MemoryKeyring>) {
    let (store, keyring) = store_with_keyring();
    (window_credentials(&store), keyring)
}

/// Saves `credential` for the server of `uri` under its current generation.
fn persist(credentials: &SessionCredentials, uri: &str, credential: &Credential) {
    let generation = credentials.generation(uri);
    credentials
        .persist(uri, credential, generation)
        .expect("the memory keyring saves");
}

fn load(credentials: &SessionCredentials, uri: &str) -> Option<Credential> {
    credentials.load(uri).expect("the memory keyring can be searched")
}

/// Every save of a keyring, held inside the keyring before it stores
/// anything.
struct HeldSaves {
    /// Receives a message when a save starts.
    started: mpsc::Receiver<()>,
    /// Lets the held save store its item.
    release: mpsc::Sender<()>,
}

impl HeldSaves {
    fn on(keyring: &MemoryKeyring) -> Self {
        let (started, save_started) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        keyring.set_store_hook(move || {
            started.send(()).expect("the test waits for the save");
            let _released = locked(&released).recv_timeout(Duration::from_secs(5));
        });
        Self {
            started: save_started,
            release,
        }
    }
}

/// Waits, on this thread, until `condition` holds.
fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

/// The attributes of `OpenXplorer`'s entry for `nas` in `scope`.
fn nas_entry(scope: CredentialScope) -> SecretAttributes {
    SecretAttributes::for_schema(crate::network::SMB_CREDENTIAL_SCHEMA)
        .with("server", "nas")
        .with("port", "445")
        .with("scope", scope.as_str())
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_session_collection_unchecked`
///
/// parity: NET-011, NET-015
#[test]
fn unchecked_remember_saves_in_the_session_collection() {
    let (credentials, keyring) = window_with_keyring();

    persist(&credentials, "smb://nas/a", &sam());

    assert_eq!(keyring.last_saved_collection(), Some(KeyringCollection::Session));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_permanent_checked`
///
/// parity: NET-011, NET-015
#[test]
fn checked_remember_saves_in_the_default_collection() {
    let (credentials, keyring) = window_with_keyring();

    persist(&credentials, "smb://nas/a", &remembered_sam());

    assert_eq!(keyring.last_saved_collection(), Some(KeyringCollection::Default));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_cross_share_load`
///
/// parity: NET-014
#[test]
fn another_share_on_the_server_loads_the_saved_credential() {
    let (credentials, _keyring) = window_with_keyring();

    persist(&credentials, "smb://nas/a", &sam());

    assert_eq!(load(&credentials, "smb://nas/b"), Some(sam()));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_other_window_load`
///
/// parity: NET-014
#[test]
fn another_window_loads_the_saved_credential() {
    let (store, _keyring) = store_with_keyring();
    persist(&window_credentials(&store), "smb://nas/a", &sam());

    let other_window = window_credentials(&store);

    assert_eq!(load(&other_window, "smb://nas/b"), Some(sam()));
}

/// Another `OpenXplorer` process has its own store on the same keyring.
///
/// parity: NET-014
#[test]
fn another_process_on_the_same_keyring_loads_the_saved_credential() {
    let (credentials, keyring) = window_with_keyring();
    persist(&credentials, "smb://nas/a", &sam());

    let other_process = SessionCredentials::new(Arc::new(CredentialStore::new(keyring)));

    assert_eq!(load(&other_process, "smb://nas/b"), Some(sam()));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_other_host_no_load`
///
/// parity: NET-014
#[test]
fn another_server_does_not_load_the_credential() {
    let (credentials, _keyring) = window_with_keyring();

    persist(&credentials, "smb://nas/a", &sam());

    assert_eq!(load(&credentials, "smb://other/a"), None);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_session_overrides_older_account`
///
/// parity: NET-011, NET-014
#[test]
fn a_session_entry_wins_over_an_older_permanent_account() {
    let (credentials, _keyring) = window_with_keyring();
    let old_account = Credential {
        username: "old".into(),
        ..remembered_sam()
    };
    persist(&credentials, "smb://nas/a", &old_account);

    persist(&credentials, "smb://nas/b", &sam());

    let loaded = load(&credentials, "smb://nas/a").expect("a saved credential");
    assert_eq!(loaded.username, "sam");
}

/// A permanent save removes the server's stale session entry
/// (`_persist_current` in `session_credentials.py`).
///
/// parity: NET-015
#[test]
fn a_permanent_save_removes_the_stale_session_entry() {
    let (store, keyring) = store_with_keyring();
    let credentials = window_credentials(&store);
    persist(&credentials, "smb://nas/a", &sam());
    let remembered = Credential {
        username: "new".into(),
        ..remembered_sam()
    };

    persist(&credentials, "smb://nas/a", &remembered);

    assert_eq!(keyring.texts(), [remembered.to_keyring_text()]);
    let fresh_window = window_credentials(&store);
    assert_eq!(load(&fresh_window, "smb://nas/a"), Some(remembered));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_forget_clears_only_matching_host`
///
/// parity: NET-020, NET-021
#[test]
fn forgetting_clears_only_the_matching_server() {
    let (credentials, _keyring) = window_with_keyring();
    persist(&credentials, "smb://nas/a", &sam());
    persist(&credentials, "smb://other/a", &sam());

    let removed = credentials.forget("smb://nas/b", ForgetScope::AllScopes);

    assert!(matches!(removed, Ok(true)), "{removed:?}");
    assert_eq!(load(&credentials, "smb://nas/a"), None);
    assert!(load(&credentials, "smb://other/a").is_some());
}

/// Sign out without "Forget saved credentials" removes session entries
/// only (`forget(uri, permanent=False)`).
///
/// parity: NET-021
#[test]
fn forgetting_session_entries_keeps_the_permanent_account() {
    let (credentials, _keyring) = window_with_keyring();
    persist(&credentials, "smb://nas/a", &remembered_sam());

    let removed = credentials.forget("smb://nas/a", ForgetScope::SessionOnly);

    assert!(matches!(removed, Ok(false)), "{removed:?}");
    assert_eq!(load(&credentials, "smb://nas/a"), Some(remembered_sam()));
}

/// Forgetting session entries deletes them, on every share of the server,
/// and the permanent account then serves the server again.
///
/// parity: NET-021
#[test]
fn forgetting_session_entries_deletes_them_but_not_the_permanent_account() {
    let (credentials, keyring) = window_with_keyring();
    persist(&credentials, "smb://nas/a", &remembered_sam());
    persist(&credentials, "smb://nas/b", &sam());

    let removed = credentials.forget("smb://nas/c", ForgetScope::SessionOnly);

    assert!(matches!(removed, Ok(true)), "{removed:?}");
    assert!(!keyring.contains(&nas_entry(CredentialScope::Session)));
    assert_eq!(load(&credentials, "smb://nas/c"), Some(remembered_sam()));
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_no_plaintext_fallback`
///
/// parity: NET-011, SAFE-011
#[test]
fn without_a_keyring_nothing_is_saved_and_memory_still_works() {
    let credentials = window_credentials(&Arc::new(CredentialStore::memory_only()));
    credentials.accept_memory("smb://nas/a", &sam());

    let saved = credentials.persist("smb://nas/a", &sam(), credentials.generation("smb://nas/a"));

    assert!(matches!(saved, Err(KeyringError::Unavailable)), "{saved:?}");
    assert_eq!(credentials.peek("smb://nas/b"), Some(sam()));
}

/// A saved credential leaves memory, so the next sign-in reads the
/// keyring and sees a Sign out made by another process.
#[test]
fn a_saved_credential_leaves_memory() {
    let (credentials, _keyring) = window_with_keyring();
    credentials.accept_memory("smb://nas/a", &sam());

    persist(&credentials, "smb://nas/a", &sam());

    assert_eq!(credentials.peek("smb://nas/a"), None);
}

/// Each window keeps its own memory, as each Python window had its own
/// `SessionCredentials`: another window reuses an account only once it is
/// saved in the keyring.
///
/// parity: SAFE-011
#[test]
fn a_credential_in_one_windows_memory_is_not_seen_by_another() {
    let store = Arc::new(CredentialStore::memory_only());
    let first_window = window_credentials(&store);
    let second_window = window_credentials(&store);

    first_window.accept_memory("smb://nas/a", &sam());

    assert_eq!(second_window.peek("smb://nas/a"), None);
    assert_eq!(first_window.peek("smb://nas/b"), Some(sam()));
}

/// Closing a window wipes its memory only (`close` in `auth_bridge.py`).
///
/// parity: SAFE-011, TAB-050
#[test]
fn clearing_a_windows_memory_leaves_the_other_windows_alone() {
    let store = Arc::new(CredentialStore::memory_only());
    let closing_window = window_credentials(&store);
    let open_window = window_credentials(&store);
    closing_window.accept_memory("smb://nas/a", &sam());
    open_window.accept_memory("smb://nas/a", &sam());

    closing_window.clear_memory();

    assert_eq!(closing_window.peek("smb://nas/a"), None);
    assert_eq!(open_window.peek("smb://nas/a"), Some(sam()));
}

/// Sign out clears the in-memory credentials in every window
/// (`forget_memory` on every controller in `winspace.py`).
///
/// parity: NET-021, SAFE-012
#[test]
fn signing_out_in_one_window_wipes_the_server_from_every_windows_memory() {
    let store = Arc::new(CredentialStore::memory_only());
    let signing_out_window = window_credentials(&store);
    let other_window = window_credentials(&store);
    other_window.accept_memory("smb://nas/a", &sam());
    other_window.accept_memory("smb://other/a", &sam());

    signing_out_window.forget_memory("smb://NAS/b");

    assert_eq!(other_window.peek("smb://nas/a"), None);
    assert_eq!(other_window.load("smb://nas/a").ok().flatten(), None);
    assert_eq!(other_window.peek("smb://other/a"), Some(sam()));
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_stale_credential_write_after_signout_discarded`
///
/// parity: NET-020, NET-021, SAFE-012
#[test]
fn a_save_started_before_sign_out_is_discarded() {
    let (credentials, keyring) = window_with_keyring();
    let uri = "smb://security-test-nas/Projects";
    let generation = credentials.generation(uri);
    credentials.accept_memory(uri, &sam());

    credentials.forget_memory(uri);
    credentials
        .forget(uri, ForgetScope::AllScopes)
        .expect("the memory keyring clears");
    credentials
        .persist(uri, &sam(), generation)
        .expect("a stale save is skipped");

    assert!(keyring.is_empty());
    assert_eq!(credentials.peek(uri), None);
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_forget_waits_for_inflight_keyring_save`
///
/// The save is held inside the keyring until the clearer has started and
/// had time to reach the keyring; only the server lock keeps it out.
///
/// parity: NET-020, NET-021, SAFE-012
#[test]
fn forgetting_waits_for_a_save_in_flight() {
    let (credentials, keyring) = window_with_keyring();
    let uri = "smb://security-test-nas/Projects";
    let held = HeldSaves::on(&keyring);
    let generation = credentials.generation(uri);

    let writer = thread::spawn({
        let credentials = Arc::clone(&credentials);
        move || credentials.persist(uri, &sam(), generation)
    });
    held.started
        .recv_timeout(Duration::from_secs(1))
        .expect("the save starts");
    let clearer = thread::spawn({
        let credentials = Arc::clone(&credentials);
        move || credentials.forget(uri, ForgetScope::AllScopes)
    });
    // `forget` advances the generation before it takes the server lock.
    wait_until("the clear starts", || credentials.generation(uri) != generation);
    // Time for a clear that ignored the lock to reach the keyring.
    thread::sleep(Duration::from_millis(50));
    assert!(!clearer.is_finished(), "the clear waits for the save");
    held.release.send(()).expect("the save waits for release");

    let saved = writer.join().expect("the writer finishes");
    let forgotten = clearer.join().expect("the clearer finishes");
    assert!(matches!(saved, Ok(())), "{saved:?}");
    assert!(matches!(forgotten, Ok(true)), "{forgotten:?}");
    assert_eq!(keyring.changes(), [KeyringChange::Stored, KeyringChange::Cleared]);
    assert!(keyring.is_empty());
}

/// A keyring lookup that raced Sign out does not bring the credential
/// back (`generation != self.generation(uri)` in `load`).
///
/// parity: SAFE-012
#[test]
fn a_lookup_that_raced_sign_out_loads_nothing() {
    let (credentials, keyring) = window_with_keyring();
    let uri = "smb://nas/a";
    persist(&credentials, uri, &sam());
    let signing_out = Arc::downgrade(&credentials);
    keyring.set_lookup_hook(move || {
        if let Some(credentials) = signing_out.upgrade() {
            credentials.forget_memory(uri);
        }
    });

    assert_eq!(load(&credentials, uri), None);
    assert_eq!(credentials.peek(uri), None);
}

/// parity: SAFE-011
#[test]
fn debug_output_names_servers_but_never_passwords() {
    let (credentials, _keyring) = window_with_keyring();
    credentials.accept_memory("smb://nas/a", &sam());

    let debug = format!("{credentials:?}");

    assert!(debug.contains("nas:445"), "{debug}");
    assert!(!debug.contains("not-a-real-password"), "{debug}");
}
