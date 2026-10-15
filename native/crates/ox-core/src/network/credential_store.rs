// SPDX-License-Identifier: AGPL-3.0-only
//! The part of the SMB credentials that every window shares: the keyring,
//! each server's sign-out generation and the lock that serialises its
//! keyring writes.
//!
//! Ports the class attributes of `SessionCredentials` in
//! `v2.0.0:desktop/session_credentials.py` (`_generations`, `_io_locks`) and its
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
pub struct CredentialGeneration {
    server: u64,
    host: u64,
}

/// Per-endpoint invalidation and host-wide Sign out share one snapshot.
#[derive(Default)]
struct Generations {
    servers: HashMap<ServerKey, u64>,
    hosts: HashMap<String, u64>,
}

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
    generations: Mutex<Generations>,
    /// Serialises writes and clears across every port of a host, so a
    /// host-wide Sign out waits for all saves that are in flight.
    host_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
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
            host_locks: Mutex::default(),
        }
    }

    /// The keyring credentials are saved in, if any.
    pub(crate) fn keyring(&self) -> Option<&dyn Keyring> {
        self.keyring.as_deref()
    }

    /// The current sign-out generation of the server `key`.
    pub(super) fn generation_of(&self, key: &ServerKey) -> CredentialGeneration {
        let generations = locked(&self.generations);
        CredentialGeneration {
            server: generations.servers.get(key).copied().unwrap_or_default(),
            host: generations.hosts.get(key.host()).copied().unwrap_or_default(),
        }
    }

    /// Makes every credential kept in memory, and every keyring read or
    /// write in flight, for the server `key` stale.
    pub(super) fn advance_generation(&self, key: &ServerKey) {
        let mut generations = locked(&self.generations);
        *generations.servers.entry(key.clone()).or_default() += 1;
    }

    /// Invalidates every port, including an endpoint whose first save
    /// has not started yet.
    pub(super) fn advance_host_generation(&self, host: &str) {
        *locked(&self.generations)
            .hosts
            .entry(host.to_owned())
            .or_default() += 1;
    }

    /// Lock around every save or clear on any port of `host`.
    pub(super) fn host_lock(&self, host: &str) -> Arc<Mutex<()>> {
        Arc::clone(locked(&self.host_locks).entry(host.to_owned()).or_default())
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

    /// Deletes the selected scopes on every port of `host`. The caller
    /// holds the host's lock, also used by endpoint-specific saves.
    pub(super) fn clear_host(&self, host: &str, scope: ForgetScope) -> Result<bool, KeyringError> {
        let Some(keyring) = self.keyring.as_deref() else {
            return Ok(false);
        };
        let attributes = host_attributes(host);
        let attributes = match scope {
            ForgetScope::AllScopes => attributes,
            ForgetScope::SessionOnly => attributes.with("scope", CredentialScope::Session.as_str()),
        };
        keyring.clear(&attributes)
    }
}

impl fmt::Debug for CredentialStore {
    /// Shows whether there is a keyring and how many servers were signed
    /// out of, never a credential.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let generations = locked(&self.generations);
        formatter
            .debug_struct("CredentialStore")
            .field("has_keyring", &self.keyring.is_some())
            .field("signed_out_servers", &generations.servers.len())
            .field("signed_out_hosts", &generations.hosts.len())
            .finish_non_exhaustive()
    }
}

/// The keyring attributes of the credentials of `key`: those of one scope,
/// or of every scope for `None`.
fn credential_attributes(key: &ServerKey, scope: Option<CredentialScope>) -> SecretAttributes {
    let attributes = host_attributes(key.host()).with("port", &key.port().to_string());
    match scope {
        Some(scope) => attributes.with("scope", scope.as_str()),
        None => attributes,
    }
}

fn host_attributes(host: &str) -> SecretAttributes {
    SecretAttributes::for_schema(SMB_CREDENTIAL_SCHEMA).with("server", host)
}

/// Locks `mutex`, recovering the data if a thread panicked while holding
/// it: every guarded map stays consistent between statements, so the data
/// is still valid.
pub(super) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
