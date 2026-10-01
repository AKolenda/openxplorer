// SPDX-License-Identifier: AGPL-3.0-only
//! The Secret Service keyring against a real keyring daemon, and against
//! the Python app's libsecret calls in `v2.0.0:desktop/session_credentials.py`.
//!
//! These tests write to the session keyring. They run only in the
//! isolated session of `native/tools/check.py`, where the private bus
//! starts its own GNOME Keyring with a disposable home folder; anywhere
//! else they return at once, so the user's keyring is never touched.
//! Where no keyring daemon can be started (a machine without GNOME
//! Keyring), they check that the client says so instead, unless
//! `OX_REQUIRE_KEYRING=1` demands the real keyring, as CI does.

mod python_support;

use std::env;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use ox_core::network::{
    Credential, CredentialScope, CredentialStore, ForgetScope, Keyring, KeyringError, Password,
    SecretAttributes, SecretService, SessionCredentials, SMB_CREDENTIAL_SCHEMA,
};
use python_support::{python, python_answers};
use serde_json::{json, Value};

/// The prefix of the temporary folder `native/tools/check.py` puts the
/// test's home folder in.
const ISOLATED_ROOT_PREFIX: &str = "openxplorer-native-test-";

/// Set to `1` where the keyring tests must run against GNOME Keyring and
/// the Python app's libsecret binding: a missing one then fails the test
/// instead of skipping it.
const REQUIRE_KEYRING_VARIABLE: &str = "OX_REQUIRE_KEYRING";

/// True inside the check driver's isolated session: its home folder is
/// `<temporary folder>/home`.
fn in_isolated_session() -> bool {
    let Some(home) = env::var_os("HOME") else {
        return false;
    };
    let root = Path::new(&home).parent().and_then(Path::file_name);
    root.is_some_and(|name| name.to_string_lossy().starts_with(ISOLATED_ROOT_PREFIX))
}

fn is_keyring_required() -> bool {
    env::var_os(REQUIRE_KEYRING_VARIABLE).is_some_and(|value| value == "1")
}

/// Whether the tests can use the keyring daemon of the private bus.
enum KeyringDaemon {
    /// Outside the isolated session: the keyring is the user's.
    NotIsolated,
    /// No Secret Service could be started on the private bus.
    Missing,
    /// A disposable Secret Service answers.
    Running,
}

fn keyring_daemon() -> KeyringDaemon {
    if !in_isolated_session() {
        eprintln!("skipped: the keyring tests run only in native/tools/check.py's isolated session");
        return KeyringDaemon::NotIsolated;
    }
    let probe = SecretAttributes::for_schema(SMB_CREDENTIAL_SCHEMA).with("server", "probe.invalid");
    match SecretService.lookup(&probe) {
        Err(KeyringError::Unavailable) => {
            assert!(
                !is_keyring_required(),
                "{REQUIRE_KEYRING_VARIABLE}=1, but no Secret Service started on the private bus; install \
                 gnome-keyring"
            );
            KeyringDaemon::Missing
        }
        Ok(_) => KeyringDaemon::Running,
        Err(error) => panic!("the test keyring failed: {error}"),
    }
}

fn account(username: &str) -> Credential {
    Credential {
        username: username.into(),
        domain: "WORKGROUP".into(),
        password: Password::from("not-a-real-password"),
        scope: CredentialScope::Session,
    }
}

/// The credentials of a window of a new native process.
fn secret_service_window() -> SessionCredentials {
    SessionCredentials::new(Arc::new(CredentialStore::new(Arc::new(SecretService))))
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_other_window_load`
/// and `test_forget_clears_only_matching_host`, against GNOME Keyring
/// instead of a double.
///
/// parity: NET-014, NET-021, SAFE-011
#[test]
fn a_credential_saved_in_the_session_keyring_serves_another_process_until_forgotten() {
    let uri = "smb://keyring-test-nas/Projects";
    match keyring_daemon() {
        KeyringDaemon::NotIsolated => return,
        KeyringDaemon::Missing => {
            let window = secret_service_window();
            let saved = window.persist(uri, &account("sam"), window.generation(uri));
            assert!(
                matches!(saved, Err(KeyringError::Unavailable)),
                "no plaintext fallback: {saved:?}"
            );
            return;
        }
        KeyringDaemon::Running => {}
    }
    let saving = secret_service_window();
    saving
        .persist(uri, &account("sam"), saving.generation(uri))
        .expect("GNOME Keyring saves in its session collection");

    let other_process = secret_service_window();
    let loaded = other_process
        .load("smb://KEYRING-TEST-NAS/Other")
        .expect("the keyring is searched");
    let forgotten = other_process.forget(uri, ForgetScope::AllScopes);
    let after_forgetting = secret_service_window()
        .load(uri)
        .expect("the keyring is searched");

    assert_eq!(loaded, Some(account("sam")));
    assert!(matches!(forgotten, Ok(true)), "{forgotten:?}");
    assert_eq!(after_forgetting, None);
}

/// Exits with status 0 when the Python app's libsecret binding
/// (`gir1.2-secret-1`) is installed.
const PYTHON_HAS_LIBSECRET: &str = r"
import gi
gi.require_version('Secret', '1')
";

/// Saves a session credential with the Python app's `SessionCredentials`
/// on the real libsecret, or loads one and prints it.
const PYTHON_LIBSECRET: &str = r"
import json, sys
import gi
gi.require_version('Secret', '1')
from gi.repository import Secret
from session_credentials import SessionCredentials
inputs = json.load(open(sys.argv[1]))
store = SessionCredentials(Secret)
if inputs['action'] == 'save':
    store.persist(inputs['uri'], inputs['value'])
    print(json.dumps(None))
else:
    print(json.dumps(store.load(inputs['uri'])))
";

fn python_has_libsecret() -> bool {
    let status = python(PYTHON_HAS_LIBSECRET, &[])
        .stderr(Stdio::null())
        .status()
        .expect("Python 3 is required for the interoperability tests");
    status.success()
}

/// Runs [`PYTHON_LIBSECRET`] with `inputs` and returns what it printed.
fn python_libsecret(inputs: &Value) -> Value {
    python_answers(PYTHON_LIBSECRET, inputs)
}

/// The keyring schema `io.winspace.SmbCredentials` is a compatibility
/// contract: credentials saved by either app are found by the other.
///
/// parity: INT-029, NET-014
#[test]
fn both_apps_find_each_others_credentials_in_the_keyring() {
    let KeyringDaemon::Running = keyring_daemon() else {
        return;
    };
    if !python_has_libsecret() {
        assert!(
            !is_keyring_required(),
            "{REQUIRE_KEYRING_VARIABLE}=1, but the Python app's libsecret binding is missing; install \
             python3-gi and gir1.2-secret-1"
        );
        eprintln!("skipped: the Python app's libsecret binding (gir1.2-secret-1) is not installed");
        return;
    }
    let native_uri = "smb://native-saved-nas/share";
    let python_uri = "smb://python-saved-nas/share";
    let native = secret_service_window();
    native
        .persist(native_uri, &account("native"), native.generation(native_uri))
        .expect("GNOME Keyring saves");
    let python_value = json!({"username": "python", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": false});

    let loaded_by_python = python_libsecret(&json!({"action": "load", "uri": native_uri}));
    python_libsecret(&json!({"action": "save", "uri": python_uri, "value": python_value}));
    let loaded_natively = secret_service_window()
        .load(python_uri)
        .expect("the keyring is searched");

    let native_value = json!({"username": "native", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": false});
    assert_eq!(loaded_by_python, native_value);
    assert_eq!(loaded_natively, Some(account("python")));
}
