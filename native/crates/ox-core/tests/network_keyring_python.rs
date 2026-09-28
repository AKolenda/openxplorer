// SPDX-License-Identifier: AGPL-3.0-only
//! The keyring items of `SessionCredentials` in
//! `desktop/session_credentials.py`, written by one app and read by the
//! other through the same keyring double, `FakeSecret` of
//! `desktop/tests/test_v05.py`. `network_secret_service.rs` repeats this
//! against GNOME Keyring and the real libsecret.
//!
//! Each Python script reads its inputs from the JSON file named by
//! `sys.argv[1]` and prints its answers as JSON.

mod python_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use ox_core::network::{
    Credential, CredentialScope, CredentialStore, Keyring, KeyringCollection, KeyringError, NewSecret,
    Password, SecretAttributes, SessionCredentials, SMB_CREDENTIAL_SCHEMA,
};
use python_support::{as_array, python_answers};
use serde_json::{json, Value};

/// `FakeSecret` of `desktop/tests/test_v05.py`: libsecret's calls on a
/// dictionary. Prepended to the credential scripts, so they need none of
/// the modules that test file imports.
const PYTHON_FAKE_SECRET: &str = r"
from types import SimpleNamespace as NS
class FakeSecret:
    Schema = NS(new=lambda *a: object()); SchemaFlags = NS(NONE=0); SchemaAttributeType = NS(STRING=0)
    COLLECTION_SESSION = 'session'; COLLECTION_DEFAULT = 'default'
    def __init__(self): self.rows = {}; self.saved = []
    def password_lookup_sync(self, schema, attrs, c): return self.rows.get(tuple(sorted(attrs.items())))
    def password_store_sync(self, schema, attrs, collection, label, value, c):
        self.rows[tuple(sorted(attrs.items()))] = value; self.saved.append((collection, dict(attrs))); return True
    def password_clear_sync(self, schema, attrs, c):
        for key in list(self.rows):
            if all(dict(key).get(k) == v for k, v in attrs.items()): self.rows.pop(key)
        return True
";

/// Runs one of the credential scripts with [`PYTHON_FAKE_SECRET`] defined.
fn python_credential_answers(script: &str, inputs: &Value) -> Value {
    python_answers(&format!("{PYTHON_FAKE_SECRET}{script}"), inputs)
}

/// A keyring holding items in memory, like `FakeSecret`.
#[derive(Default)]
struct RecordedKeyring {
    items: Mutex<Vec<(SecretAttributes, String)>>,
}

impl RecordedKeyring {
    /// The items as `{attributes, text}` objects, without libsecret's
    /// schema attribute, which `FakeSecret` does not store.
    fn items_as_json(&self) -> Value {
        let items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = items.iter().map(|(attributes, text)| {
            let without_schema: BTreeMap<&str, &str> = attributes
                .iter()
                .filter(|(name, _)| *name != "xdg:schema")
                .collect();
            json!({"attributes": without_schema, "text": text})
        });
        Value::from(rows.collect::<Vec<_>>())
    }
}

impl Keyring for RecordedKeyring {
    fn lookup(&self, query: &SecretAttributes) -> Result<Option<String>, KeyringError> {
        let items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        let found = items.iter().find(|(attributes, _)| query.matches(attributes));
        Ok(found.map(|(_, text)| text.clone()))
    }

    fn store(&self, secret: &NewSecret<'_>) -> Result<(), KeyringError> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        items.retain(|(attributes, _)| attributes != secret.attributes);
        items.push((secret.attributes.clone(), secret.text.to_owned()));
        Ok(())
    }

    fn clear(&self, query: &SecretAttributes) -> Result<bool, KeyringError> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        let before = items.len();
        items.retain(|(attributes, _)| !query.matches(attributes));
        Ok(items.len() < before)
    }
}

/// Loads, with Python's `SessionCredentials`, the account of every server
/// from the keyring items given as input.
const PYTHON_LOADS_CREDENTIALS: &str = r"
import json, sys
from session_credentials import SessionCredentials
inputs = json.load(open(sys.argv[1]))
secret = FakeSecret()
for item in inputs['items']:
    secret.rows[tuple(sorted(item['attributes'].items()))] = item['text']
store = SessionCredentials(secret)
print(json.dumps([store.load(uri) for uri in inputs['uris']]))
";

/// Saves the input accounts with Python's `SessionCredentials` and prints
/// the keyring items it wrote.
const PYTHON_SAVES_CREDENTIALS: &str = r"
import json, sys
from session_credentials import SessionCredentials
secret = FakeSecret()
store = SessionCredentials(secret)
for uri, value in json.load(open(sys.argv[1])):
    store.persist(uri, value)
print(json.dumps([{'attributes': dict(key), 'text': text} for key, text in secret.rows.items()]))
";

fn account(username: &str, scope: CredentialScope) -> Credential {
    Credential {
        username: username.into(),
        domain: "WORKGROUP".into(),
        password: Password::from("not-a-real-password"),
        scope,
    }
}

/// The credentials of a window of a native app whose keyring is `keyring`.
fn window_credentials(keyring: Arc<RecordedKeyring>) -> SessionCredentials {
    SessionCredentials::new(Arc::new(CredentialStore::new(keyring)))
}

/// Credentials saved by the native app are found by the Python app, so
/// signing in once serves both while both exist.
///
/// parity: INT-029, NET-014
#[test]
fn credentials_saved_natively_load_in_the_python_app() {
    let keyring = Arc::new(RecordedKeyring::default());
    let credentials = window_credentials(Arc::clone(&keyring));
    let saves = [
        ("smb://nas/a", account("remembered", CredentialScope::Permanent)),
        ("smb://nas:1445/b", account("session", CredentialScope::Session)),
    ];
    for (uri, credential) in &saves {
        credentials
            .persist(uri, credential, credentials.generation(uri))
            .expect("the recorded keyring saves");
    }
    let inputs = json!({"items": keyring.items_as_json(), "uris": ["smb://NAS/other", "smb://nas:1445/c", "smb://nas:1445/"]});

    let loaded = python_credential_answers(PYTHON_LOADS_CREDENTIALS, &inputs);

    let remembered = json!({"username": "remembered", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": true});
    let session = json!({"username": "session", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": false});
    assert_eq!(loaded, json!([remembered, session, session]));
}

/// Credentials saved by the Python app are found by the native app.
///
/// parity: INT-029, NET-014
#[test]
fn credentials_saved_by_the_python_app_load_natively() {
    let saves = json!([
        ["smb://nas/a", {"username": "remembered", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": true}],
        ["smb://nas:1445/b", {"username": "session", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": false}],
    ]);
    let written = python_credential_answers(PYTHON_SAVES_CREDENTIALS, &saves);
    let keyring = Arc::new(RecordedKeyring::default());
    for item in as_array(&written) {
        let attributes = with_schema(&item["attributes"]);
        let text = item["text"].as_str().expect("the item text is a string");
        let secret = NewSecret {
            collection: KeyringCollection::Default,
            label: "written by Python",
            attributes: &attributes,
            text,
        };
        keyring.store(&secret).expect("the recorded keyring saves");
    }

    let credentials = window_credentials(keyring);

    let remembered = credentials
        .load("smb://NAS/other")
        .expect("the keyring is searched");
    assert_eq!(
        remembered,
        Some(account("remembered", CredentialScope::Permanent))
    );
    let session = credentials
        .load("smb://nas:1445/c")
        .expect("the keyring is searched");
    assert_eq!(session, Some(account("session", CredentialScope::Session)));
}

/// Attributes `FakeSecret` stored, plus the schema name libsecret adds to
/// every item of `io.winspace.SmbCredentials`.
fn with_schema(attributes: &Value) -> SecretAttributes {
    let pairs = attributes.as_object().expect("the attributes are an object");
    let schema = SecretAttributes::for_schema(SMB_CREDENTIAL_SCHEMA);
    pairs.iter().fold(schema, |all, (name, value)| {
        all.with(name, value.as_str().expect("attribute values are strings"))
    })
}
