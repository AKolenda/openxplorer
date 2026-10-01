// SPDX-License-Identifier: AGPL-3.0-only
//! One window's server-scoped SMB credentials, in memory and in the
//! keyring.
//!
//! Ports `SessionCredentials` in `v2.0.0:desktop/session_credentials.py`. There is
//! no settings, database or plaintext-file fallback: without a keyring,
//! credentials live in memory only. Session credentials survive closing
//! `OpenXplorer` but end at logout; remembered ones go to the default
//! keyring.
//!
//! As in Python, each window keeps the credentials it accepted in its own
//! memory and wipes them when it closes, while the sign-out generations
//! and keyring locks, class attributes in Python, are shared by every
//! window through the [`CredentialStore`]. Sign out in one window advances
//! the server's generation, so every other window's copy becomes stale and
//! is wiped instead of reused.
//!
//! [`load`](SessionCredentials::load), [`persist`](SessionCredentials::persist)
//! and [`forget`](SessionCredentials::forget) may block on the keyring;
//! call them off the GTK main thread.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use super::credential::{Credential, CredentialScope};
use super::credential_store::{locked, CredentialGeneration, CredentialStore, ForgetScope};
use super::keyring::{Keyring, KeyringError};
use super::server::ServerKey;

/// One window's SMB credentials by server: those it accepted, in memory,
/// and those saved in the shared [`CredentialStore`].
pub struct SessionCredentials {
    /// The keyring, generations and locks every window shares.
    store: Arc<CredentialStore>,
    /// Credentials accepted by a mount and not yet saved, or not savable.
    memory: Mutex<HashMap<ServerKey, KeptCredential>>,
}

/// A credential in a window's memory.
struct KeptCredential {
    credential: Credential,
    /// The server's sign-out generation when the credential was kept.
    generation: CredentialGeneration,
}

impl SessionCredentials {
    /// A new window's credentials over the shared `store`, with nothing in
    /// memory yet.
    pub fn new(store: Arc<CredentialStore>) -> Self {
        Self {
            store,
            memory: Mutex::default(),
        }
    }

    /// The current sign-out generation of the server of `uri`.
    pub fn generation(&self, uri: &str) -> CredentialGeneration {
        ServerKey::for_location(uri).map_or_else(CredentialGeneration::default, |key| {
            self.store.generation_of(&key)
        })
    }

    /// The credential this window holds in memory for the server of
    /// `uri`, without asking the keyring.
    pub fn peek(&self, uri: &str) -> Option<Credential> {
        let key = ServerKey::for_location(uri)?;
        self.current_in_memory(&key)
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
        let generation = self.store.generation_of(&key);
        if let Some(credential) = self.current_in_memory(&key) {
            return Ok(Some(credential));
        }
        let Some(credential) = self.store.lookup(&key, generation)? else {
            return Ok(None);
        };
        self.keep(key, &credential, generation);
        Ok(Some(credential))
    }

    /// Keeps `credential` in this window's memory for the server of `uri`,
    /// for example after a mount accepted it.
    pub fn accept_memory(&self, uri: &str, credential: &Credential) {
        if let Some(key) = ServerKey::for_location(uri) {
            let generation = self.store.generation_of(&key);
            self.keep(key, credential, generation);
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
        let server_lock = self.store.server_lock(&key);
        let _serialised = locked(&server_lock);
        // Safety rule (NET-021): a save that started before Sign out is
        // discarded, so signing out cannot be undone by a late write.
        if self.store.generation_of(&key) != generation {
            return Ok(());
        }
        self.persist_current(&key, credential)
    }

    /// Forgets the in-memory credential of the server of `uri`, in this
    /// window and every other, and makes every keyring read or write in
    /// flight for it stale.
    pub fn forget_memory(&self, uri: &str) {
        let Some(key) = ServerKey::for_location(uri) else {
            return;
        };
        // Safety rule (NET-021, SAFE-012): the new generation makes the
        // other windows' copies and the keyring calls in flight stale.
        self.store.advance_generation(&key);
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
        let Some(key) = ServerKey::for_location(uri) else {
            return Ok(false);
        };
        let server_lock = self.store.server_lock(&key);
        // Safety rule (SAFE-012): wait for a save in flight for this
        // server, so the clear below also deletes what it wrote.
        let _serialised = locked(&server_lock);
        self.store.clear(&key, scope)
    }

    /// Forgets every credential in this window's memory, when the window
    /// closes.
    pub fn clear_memory(&self) {
        locked(&self.memory).clear();
    }

    /// The keyring credentials are saved in, if any.
    pub(crate) fn keyring(&self) -> Option<&dyn Keyring> {
        self.store.keyring()
    }

    /// The credential in memory for `key`, unless a Sign out made it stale.
    fn current_in_memory(&self, key: &ServerKey) -> Option<Credential> {
        let current = self.store.generation_of(key);
        let mut memory = locked(&self.memory);
        let kept = memory.get(key)?;
        // Safety rule (NET-021): Sign out in any window advanced the
        // generation; the stale copy is wiped instead of reused.
        if kept.generation != current {
            memory.remove(key);
            return None;
        }
        Some(kept.credential.clone())
    }

    /// Keeps `credential` for `key`, read or accepted under `generation`.
    fn keep(&self, key: ServerKey, credential: &Credential, generation: CredentialGeneration) {
        let kept = KeptCredential {
            credential: credential.clone(),
            generation,
        };
        locked(&self.memory).insert(key, kept);
    }

    /// Saves `credential` for `key`; the caller holds the server's lock.
    fn persist_current(&self, key: &ServerKey, credential: &Credential) -> Result<(), KeyringError> {
        self.store.save(key, credential)?;
        // The next challenge reads the keyring again, so Sign out in
        // another process also invalidates this credential here. Memory
        // keeps it only while saving is in flight or impossible.
        let mut memory = locked(&self.memory);
        if memory.get(key).is_some_and(|kept| kept.credential == *credential) {
            memory.remove(key);
        }
        drop(memory);
        // An explicitly remembered account replaces a stale session entry.
        if credential.scope == CredentialScope::Permanent {
            self.store.clear(key, ForgetScope::SessionOnly)?;
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
            .field("store", &self.store)
            .field("servers_in_memory", &servers)
            .finish()
    }
}

#[cfg(test)]
mod tests;
