// SPDX-License-Identifier: AGPL-3.0-only
//! The part of the SMB credentials that every window shares: the keyring,
//! each server's sign-out generation and the lock that serialises its
//! keyring writes.
//!
//! Ports the class attributes of `SessionCredentials` in
//! `desktop/session_credentials.py` (`_generations`, `_io_locks`) and its
//! libsecret calls. Each window keeps its accepted credentials in its own
//! [`SessionCredentials`](super::SessionCredentials), which orchestrates
//! these calls and holds the server locks around them.
//!
//! Keyring calls may block; make them off the GTK main thread.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::credential::{Credential, CredentialScope};
use super::keyring::{Keyring, KeyringError, NewSecret, SecretAttributes};
use super::server::ServerKey;

/// The libsecret schema of `OpenXplorer`'s SMB credentials. A compatibility
/// contract with the Python app (`AGENTS.md`): never rename it.
pub const SMB_CREDENTIAL_SCHEMA: &str = "io.winspace.SmbCredentials";

/// How often a server's credentials were forgotten. An in-memory
/// credential kept, or a keyring read or write started, under an older
/// generation is stale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CredentialGeneration(u64);

/// Which saved credentials of a server are deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgetScope {
    /// Session and permanent entries: Sign out with "Forget saved
    /// credentials for this server" checked.
    AllScopes,
    /// Session entries only: Sign out keeping the saved credentials.
    SessionOnly,
}

/// The keyring, sign-out generations and keyring locks shared by every
/// window of the app. Create one per process and give each window a
/// [`SessionCredentials`](super::SessionCredentials) over it.
pub struct CredentialStore {
    /// The keyring, or `None` to keep credentials in memory only.
    keyring: Option<Arc<dyn Keyring>>,
    /// Sign-out generations by server.
    generations: Mutex<HashMap<ServerKey, CredentialGeneration>>,
    /// Serialises keyring writes and clears per server, so Sign out waits
    /// for a save that is in flight.
    server_locks: Mutex<HashMap<ServerKey, Arc<Mutex<()>>>>,
}

impl CredentialStore {
    /// A store that saves credentials in `keyring`.
    pub fn new(keyring: Arc<dyn Keyring>) -> Self {
        Self::with_keyring(Some(keyring))
    }

    /// A store without a keyring: credentials are kept in memory only.
    pub fn memory_only() -> Self {
        Self::with_keyring(None)
    }

    fn with_keyring(keyring: Option<Arc<dyn Keyring>>) -> Self {
        Self {
            keyring,
            generations: Mutex::default(),
            server_locks: Mutex::default(),
        }
    }

    /// The keyring credentials are saved in, if any.
    pub(crate) fn keyring(&self) -> Option<&dyn Keyring> {
        self.keyring.as_deref()
    }

    /// The current sign-out generation of the server `key`.
    pub(super) fn generation_of(&self, key: &ServerKey) -> CredentialGeneration {
        locked(&self.generations).get(key).copied().unwrap_or_default()
    }

    /// Makes every credential kept in memory, and every keyring read or
    /// write in flight, for the server `key` stale.
    pub(super) fn advance_generation(&self, key: &ServerKey) {
        let mut generations = locked(&self.generations);
        let generation = generations.entry(key.clone()).or_default();
        generation.0 += 1;
    }

    /// The lock that serialises the keyring writes and clears of the
    /// server `key`. Lock it around every [`save`](Self::save) and
    /// [`clear`](Self::clear).
    pub(super) fn server_lock(&self, key: &ServerKey) -> Arc<Mutex<()>> {
        let mut server_locks = locked(&self.server_locks);
        Arc::clone(server_locks.entry(key.clone()).or_default())
    }

    /// The saved credential of the server `key`, where a session entry
    /// wins over an older permanent one. `None` without a keyring, or when
    /// the server's credentials were forgotten since `generation`.
    ///
    /// # Errors
    ///
    /// A [`KeyringError`] when the keyring cannot be searched.
    pub(super) fn lookup(
        &self,
        key: &ServerKey,
        generation: CredentialGeneration,
    ) -> Result<Option<Credential>, KeyringError> {
        let Some(keyring) = self.keyring.as_deref() else {
            return Ok(None);
        };
        for scope in [CredentialScope::Session, CredentialScope::Permanent] {
            let Some(text) = keyring.lookup(&credential_attributes(key, Some(scope)))? else {
                continue;
            };
            // Safety rule (NET-021): a lookup that raced Sign out never
            // brings the forgotten credential back.
            if self.generation_of(key) != generation {
                continue;
            }
            let Some(credential) = Credential::from_keyring_text(&text, scope) else {
                continue;
            };
            return Ok(Some(credential));
        }
        Ok(None)
    }

    /// Saves `credential` for the server `key` in the keyring collection
    /// its scope names. The caller holds the server's lock.
    ///
    /// # Errors
    ///
    /// [`KeyringError::Unavailable`] without a keyring, or the keyring's
    /// error.
    pub(super) fn save(&self, key: &ServerKey, credential: &Credential) -> Result<(), KeyringError> {
        // Privacy rule (session_credentials.py): no settings, database or
        // plaintext-file fallback when there is no keyring.
        let keyring = self.keyring.as_deref().ok_or(KeyringError::Unavailable)?;
        let scope = credential.scope;
        let attributes = credential_attributes(key, Some(scope));
        let label = format!("OpenXplorer SMB: {}", key.host());
        let text = credential.to_keyring_text();
        keyring.store(&NewSecret {
            collection: scope.keyring_collection(),
            label: &label,
            attributes: &attributes,
            text: &text,
        })
    }

    /// Deletes the saved entries of the server `key` in `scope`; returns
    /// whether the keyring deleted anything. The caller holds the server's
    /// lock.
    ///
    /// # Errors
    ///
    /// The keyring's error when its entries cannot be deleted.
    pub(super) fn clear(&self, key: &ServerKey, scope: ForgetScope) -> Result<bool, KeyringError> {
        let Some(keyring) = self.keyring.as_deref() else {
            return Ok(false);
        };
        let scope = match scope {
            ForgetScope::AllScopes => None,
            ForgetScope::SessionOnly => Some(CredentialScope::Session),
        };
        keyring.clear(&credential_attributes(key, scope))
    }
}

impl fmt::Debug for CredentialStore {
    /// Shows whether there is a keyring and how many servers were signed
    /// out of, never a credential.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialStore")
            .field("has_keyring", &self.keyring.is_some())
            .field("signed_out_servers", &locked(&self.generations).len())
            .finish_non_exhaustive()
    }
}

/// The keyring attributes of the credentials of `key`: those of one scope,
/// or of every scope for `None`.
fn credential_attributes(key: &ServerKey, scope: Option<CredentialScope>) -> SecretAttributes {
    let attributes = SecretAttributes::for_schema(SMB_CREDENTIAL_SCHEMA)
        .with("server", key.host())
        .with("port", &key.port().to_string());
    match scope {
        Some(scope) => attributes.with("scope", scope.as_str()),
        None => attributes,
    }
}

/// Locks `mutex`, recovering the data if a thread panicked while holding
/// it: every guarded map stays consistent between statements, so the data
/// is still valid.
pub(super) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
