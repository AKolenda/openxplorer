// SPDX-License-Identifier: AGPL-3.0-only
//! One SMB account and how long it is remembered.
//!
//! Ports the credential dictionaries of `v2.0.0:desktop/session_credentials.py`
//! and `v2.0.0:desktop/auth_bridge.py` (`{'username', 'domain', 'password',
//! 'remember'}`), including the JSON text stored in the keyring, which the
//! Python app reads too.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::keyring::KeyringCollection;

/// Longest keyring text accepted as a stored credential, in characters
/// (`len(encoded) > 24000` in `session_credentials.py`).
const MAX_STORED_CHARS: usize = 24_000;

/// A password. `Debug` never shows it, so a password cannot reach a log
/// message.
///
/// Privacy rule (`v2.0.0:desktop/auth_bridge.py`): password values are never
/// emitted back to the interface, settings or log messages.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    /// The password text, for the mount operation and the keyring only.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Password {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&str> for Password {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Password(<redacted>)")
    }
}

/// How long a credential is remembered: the "Remember my credentials"
/// check box of the sign-in dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialScope {
    /// Unchecked: kept in the session keyring until the user logs out, so
    /// it survives closing `OpenXplorer` but not logging out.
    Session,
    /// Checked (the default): kept in the default keyring.
    Permanent,
}

impl CredentialScope {
    /// The value of the keyring's `scope` attribute.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Permanent => "permanent",
        }
    }

    /// The keyring collection a credential of this scope is saved in.
    pub(crate) fn keyring_collection(self) -> KeyringCollection {
        match self {
            Self::Session => KeyringCollection::Session,
            Self::Permanent => KeyringCollection::Default,
        }
    }

    /// How `GVfs` is asked to keep the password it was given.
    pub fn password_save(self) -> gio::PasswordSave {
        match self {
            Self::Session => gio::PasswordSave::ForSession,
            Self::Permanent => gio::PasswordSave::Permanently,
        }
    }
}

/// An SMB account for one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The user name, without the domain.
    pub username: String,
    /// The Windows domain or workgroup; empty for the server's default.
    pub domain: String,
    /// The password.
    pub password: Password,
    /// How long the account is remembered.
    pub scope: CredentialScope,
}

/// The JSON text of a credential in the keyring, as the Python app writes
/// and reads it: `{"username", "domain", "password", "remember"}`.
#[derive(Serialize, Deserialize)]
struct StoredCredential {
    username: String,
    domain: String,
    password: String,
    /// Written for the Python app, never read: the item's `scope`
    /// attribute decides instead, as in `SessionCredentials.load`, so any
    /// value here leaves the item readable.
    #[serde(default, skip_deserializing)]
    remember: bool,
}

impl Credential {
    /// The keyring text of this credential.
    pub(crate) fn to_keyring_text(&self) -> String {
        let stored = StoredCredential {
            username: self.username.clone(),
            domain: self.domain.clone(),
            password: self.password.as_str().to_owned(),
            remember: self.scope == CredentialScope::Permanent,
        };
        serde_json::to_string(&stored).expect("a struct of strings and a bool always serialises")
    }

    /// The credential stored as `text` in the keyring under `scope`, or
    /// `None` for text that is too long or not a credential: such items are
    /// skipped, never shown or repaired.
    pub(crate) fn from_keyring_text(text: &str, scope: CredentialScope) -> Option<Self> {
        if text.chars().count() > MAX_STORED_CHARS {
            return None;
        }
        let stored: StoredCredential = serde_json::from_str(text).ok()?;
        Some(Self {
            username: stored.username,
            domain: stored.domain,
            password: Password(stored.password),
            scope,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(scope: CredentialScope) -> Credential {
        Credential {
            username: "sam".into(),
            domain: "WORKGROUP".into(),
            password: Password::from("not-a-real-password"),
            scope,
        }
    }

    /// parity: INT-029
    #[test]
    fn keyring_text_round_trips_under_its_scope() {
        let permanent = credential(CredentialScope::Permanent);

        let text = permanent.to_keyring_text();

        assert!(text.contains(r#""remember":true"#), "{text}");
        assert_eq!(
            Credential::from_keyring_text(&text, CredentialScope::Permanent),
            Some(permanent)
        );
    }

    /// The scope attribute of the keyring item decides how long a
    /// credential is remembered, whatever its `remember` field says, as in
    /// `SessionCredentials.load`.
    ///
    /// parity: INT-029
    #[test]
    fn text_written_by_the_python_app_is_read() {
        let written_by_python = [
            r#"{"username": "sam", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": false}"#,
            r#"{"username": "sam", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": true}"#,
            r#"{"username": "sam", "domain": "WORKGROUP", "password": "not-a-real-password", "remember": "yes"}"#,
        ];

        for text in written_by_python {
            let read = Credential::from_keyring_text(text, CredentialScope::Session);

            assert_eq!(read, Some(credential(CredentialScope::Session)), "{text}");
        }
    }

    #[test]
    fn malformed_or_oversized_keyring_text_is_skipped() {
        let oversized = format!(
            r#"{{"username": "sam", "domain": "", "password": "{}"}}"#,
            "x".repeat(MAX_STORED_CHARS)
        );
        let cases = [
            "{",
            "[]",
            r#"{"username": "sam", "domain": ""}"#,
            r#"{"username": 1, "domain": "", "password": ""}"#,
            oversized.as_str(),
        ];
        for text in cases {
            let parsed = Credential::from_keyring_text(text, CredentialScope::Session);
            assert_eq!(parsed, None, "{}", &text[..text.len().min(60)]);
        }
    }

    /// parity: SAFE-011
    #[test]
    fn passwords_are_left_out_of_debug_output() {
        let debug = format!("{:?}", credential(CredentialScope::Session));
        assert!(!debug.contains("not-a-real-password"), "{debug}");
        assert!(debug.contains("sam"), "{debug}");
    }
}
