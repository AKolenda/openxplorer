// SPDX-License-Identifier: AGPL-3.0-only
//! Server-scoped SMB credentials in memory and in the keyring.
//!
//! Ports `SessionCredentials` in `desktop/session_credentials.py`. There is
//! no settings, database or plaintext-file fallback: without a keyring,
//! credentials live in memory only. Session credentials survive closing
//! `OpenXplorer` but end at logout; remembered ones go to the default
//! keyring.
//!
//! The Python class kept its sign-out generations and keyring locks at
//! class level, shared by every window of the process. Here one
//! [`SessionCredentials`] is shared by every window instead (behind an
//! `Arc`), so the in-memory credentials are shared too: a window may reuse
//! a credential another window accepted, and the app clears the memory
//! when its last window closes ([`SessionCredentials::clear_memory`]).
//!
//! Every method may block on the keyring; call them off the GTK main
//! thread.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::credential::{Credential, CredentialScope};
use super::keyring::{Keyring, KeyringError, NewSecret, SecretAttributes};
use super::server::ServerKey;

/// The libsecret schema of `OpenXplorer`'s SMB credentials. A compatibility
/// contract with the Python app (`AGENTS.md`): never rename it.
pub const SMB_CREDENTIAL_SCHEMA: &str = "io.winspace.SmbCredentials";

/// How often a server's credentials were forgotten. A keyring read or
/// write that started under an older generation is discarded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CredentialGeneration(u64);

/// Which saved credentials [`SessionCredentials::forget`] deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgetScope {
    /// Session and permanent entries: Sign out with "Forget saved
    /// credentials for this server" checked.
    AllScopes,
    /// Session entries only: Sign out keeping the saved credentials.
    SessionOnly,
}

/// SMB credentials by server, shared by every window of the app.
pub struct SessionCredentials {
    /// The keyring, or `None` to keep credentials in memory only.
    keyring: Option<Arc<dyn Keyring>>,
    /// Credentials accepted by a mount and not yet saved, or not savable.
    memory: Mutex<HashMap<ServerKey, Credential>>,
    /// Sign-out generations by server.
    generations: Mutex<HashMap<ServerKey, CredentialGeneration>>,
    /// Serialises keyring writes and clears per server, so Sign out waits
    /// for a save that is in flight.
    server_locks: Mutex<HashMap<ServerKey, Arc<Mutex<()>>>>,
}

impl SessionCredentials {
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
            memory: Mutex::default(),
            generations: Mutex::default(),
            server_locks: Mutex::default(),
        }
    }

    /// The current sign-out generation of the server of `uri`.
    pub fn generation(&self, uri: &str) -> CredentialGeneration {
        ServerKey::for_location(uri)
            .map_or_else(CredentialGeneration::default, |key| self.generation_of(&key))
    }

    /// The credential held in memory for the server of `uri`, without
    /// asking the keyring.
    pub fn peek(&self, uri: &str) -> Option<Credential> {
        let key = ServerKey::for_location(uri)?;
        locked(&self.memory).get(&key).cloned()
    }

    /// The credential for the server of `uri`: from memory, else from the
    /// keyring, where a session entry wins over an older permanent one.
    /// A credential read from the keyring is kept in memory.
    ///
    /// # Errors
    ///
    /// A [`KeyringError`] when the keyring cannot be searched.
    pub fn load(&self, uri: &str) -> Result<Option<Credential>, KeyringError> {
        let Some(key) = ServerKey::for_location(uri) else {
            return Ok(None);
        };
        let generation = self.generation_of(&key);
        if let Some(credential) = locked(&self.memory).get(&key) {
            return Ok(Some(credential.clone()));
        }
        let Some(keyring) = self.keyring.as_deref() else {
            return Ok(None);
        };
        for scope in [CredentialScope::Session, CredentialScope::Permanent] {
            let Some(text) = keyring.lookup(&credential_attributes(&key, Some(scope)))? else {
                continue;
            };
            // Safety rule (NET-021): a lookup that raced Sign out never
            // brings the forgotten credential back.
            if self.generation_of(&key) != generation {
                continue;
            }
            let Some(credential) = Credential::from_keyring_text(&text, scope) else {
                continue;
            };
            locked(&self.memory).insert(key, credential.clone());
            return Ok(Some(credential));
        }
        Ok(None)
    }

    /// Keeps `credential` in memory for the server of `uri`, for example
    /// after a mount accepted it.
    pub fn accept_memory(&self, uri: &str, credential: &Credential) {
        if let Some(key) = ServerKey::for_location(uri) {
            locked(&self.memory).insert(key, credential.clone());
        }
    }

    /// Saves `credential` in the keyring collection its scope names,
    /// unless the server's credentials were forgotten since `generation`.
    ///
    /// # Errors
    ///
    /// [`KeyringError::Unavailable`] without a keyring, or the keyring's
    /// error. The credential then stays in memory.
    pub fn persist(
        &self,
        uri: &str,
        credential: &Credential,
        generation: CredentialGeneration,
    ) -> Result<(), KeyringError> {
        let Some(key) = ServerKey::for_location(uri) else {
            return Ok(());
        };
        let server_lock = self.server_lock(&key);
        let _serialised = locked(&server_lock);
        // Safety rule (NET-021): a save that started before Sign out is
        // discarded, so signing out cannot be undone by a late write.
        if self.generation_of(&key) != generation {
            return Ok(());
        }
        self.persist_current(&key, credential)
    }

    /// Forgets the in-memory credential of the server of `uri` and makes
    /// every keyring read or write in flight for it stale.
    pub fn forget_memory(&self, uri: &str) {
        let Some(key) = ServerKey::for_location(uri) else {
            return;
        };
        let mut generations = locked(&self.generations);
        let generation = generations.entry(key.clone()).or_default();
        generation.0 += 1;
        drop(generations);
        locked(&self.memory).remove(&key);
    }

    /// Forgets the credential of the server of `uri` in memory and deletes
    /// its saved entries in `scope`, after any save in flight for it.
    /// Returns whether the keyring deleted anything.
    ///
    /// # Errors
    ///
    /// The keyring's error when its entries cannot be deleted.
    pub fn forget(&self, uri: &str, scope: ForgetScope) -> Result<bool, KeyringError> {
        self.forget_memory(uri);
        let (Some(key), Some(keyring)) = (ServerKey::for_location(uri), self.keyring.as_deref()) else {
            return Ok(false);
        };
        let server_lock = self.server_lock(&key);
        let _serialised = locked(&server_lock);
        let scope = match scope {
            ForgetScope::AllScopes => None,
            ForgetScope::SessionOnly => Some(CredentialScope::Session),
        };
        keyring.clear(&credential_attributes(&key, scope))
    }

    /// Forgets every in-memory credential.
    pub fn clear_memory(&self) {
        locked(&self.memory).clear();
    }

    /// The keyring credentials are saved in, if any.
    pub(crate) fn keyring(&self) -> Option<&dyn Keyring> {
        self.keyring.as_deref()
    }

    fn generation_of(&self, key: &ServerKey) -> CredentialGeneration {
        locked(&self.generations).get(key).copied().unwrap_or_default()
    }

    fn server_lock(&self, key: &ServerKey) -> Arc<Mutex<()>> {
        let mut server_locks = locked(&self.server_locks);
        Arc::clone(server_locks.entry(key.clone()).or_default())
    }

    /// Saves `credential` for `key`; the caller holds the server's lock.
    fn persist_current(&self, key: &ServerKey, credential: &Credential) -> Result<(), KeyringError> {
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
        })?;
        // The next challenge reads the keyring again, so Sign out in
        // another process also invalidates this credential here. Memory
        // keeps it only while saving is in flight or impossible.
        let mut memory = locked(&self.memory);
        if memory.get(key) == Some(credential) {
            memory.remove(key);
        }
        drop(memory);
        // An explicitly remembered account replaces a stale session entry.
        if scope == CredentialScope::Permanent {
            keyring.clear(&credential_attributes(key, Some(CredentialScope::Session)))?;
        }
        Ok(())
    }
}

impl fmt::Debug for SessionCredentials {
    /// Shows which servers have credentials in memory, never the
    /// credentials.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let servers: Vec<String> = locked(&self.memory).keys().map(ToString::to_string).collect();
        formatter
            .debug_struct("SessionCredentials")
            .field("has_keyring", &self.keyring.is_some())
            .field("servers_in_memory", &servers)
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
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
