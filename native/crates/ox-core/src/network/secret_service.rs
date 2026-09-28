// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop keyring through the freedesktop Secret Service.
//!
//! Replaces the libsecret calls of `desktop/session_credentials.py` and
//! `desktop/winspace.py` with the pure-Rust `oo7` client: GNOME Keyring,
//! `KWallet` and `oo7-daemon` all implement the Secret Service D-Bus API.
//! The session is encrypted where the service supports it, as libsecret's
//! is, so secrets do not cross the session bus in the clear.
//!
//! Like libsecret, lookups prefer unlocked items and unlock a locked one
//! (the service may show its unlock prompt), stores unlock the collection
//! first, and clears delete every matching item in every collection.

use std::future::Future;

use oo7::dbus::{Collection, Item, Service};

use super::keyring::{Keyring, KeyringCollection, KeyringError, NewSecret, SecretAttributes};

/// The session bus's Secret Service. Each call opens its own connection
/// and blocks until the service answers (at most 30 seconds per D-Bus
/// call), so call it off the main thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct SecretService;

impl Keyring for SecretService {
    fn lookup(&self, query: &SecretAttributes) -> Result<Option<String>, KeyringError> {
        block_on(async {
            let service = connect().await?;
            let items = matching_items(&service, query).await?;
            let Some(item) = first_unlocked_or_locked(items).await? else {
                return Ok(None);
            };
            if item.is_locked().await? {
                item.unlock(None).await?;
            }
            let secret = item.secret().await?;
            let text = String::from_utf8(secret.as_bytes().to_vec());
            // A secret that is not text is not a credential of this app.
            Ok(text.ok())
        })
    }

    fn store(&self, secret: &NewSecret<'_>) -> Result<(), KeyringError> {
        block_on(async {
            let service = connect().await?;
            let collection = match secret.collection {
                KeyringCollection::Default => service.default_collection().await?,
                KeyringCollection::Session => service.session_collection().await?,
            };
            if collection.is_locked().await? {
                collection.unlock(None).await?;
            }
            let attributes = attribute_pairs(secret.attributes);
            let text = oo7::Secret::text(secret.text);
            let replace_existing = true;
            collection
                .create_item(secret.label, &attributes, text, replace_existing, None)
                .await?;
            Ok(())
        })
    }

    fn clear(&self, query: &SecretAttributes) -> Result<bool, KeyringError> {
        block_on(async {
            let service = connect().await?;
            let items = matching_items(&service, query).await?;
            let deleted_any = !items.is_empty();
            for item in items {
                if item.is_locked().await? {
                    item.unlock(None).await?;
                }
                item.delete(None).await?;
            }
            Ok(deleted_any)
        })
    }
}

impl From<oo7::dbus::Error> for KeyringError {
    /// A service error, such as a dismissed unlock prompt, in the
    /// service's own words.
    fn from(error: oo7::dbus::Error) -> Self {
        KeyringError::Failed(error.to_string())
    }
}

/// Runs `future` to completion on this worker thread. `oo7` runs its
/// D-Bus I/O on its own threads, so any executor can drive it.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    glib::MainContext::new().block_on(future)
}

/// Opens an encrypted (or, where unsupported, plain) session with the
/// Secret Service.
async fn connect() -> Result<Service, KeyringError> {
    // No session bus, or no keyring on it that can be started.
    Service::new().await.map_err(|_| KeyringError::Unavailable)
}

/// Every item matching `query`, in every collection.
async fn matching_items(service: &Service, query: &SecretAttributes) -> Result<Vec<Item>, KeyringError> {
    let attributes = attribute_pairs(query);
    let collections: Vec<Collection> = service.collections().await?;
    let mut items = Vec::new();
    for collection in collections {
        let found = collection.search_items(&attributes).await?;
        items.extend(found);
    }
    Ok(items)
}

/// The first unlocked item, else the first locked one.
async fn first_unlocked_or_locked(items: Vec<Item>) -> Result<Option<Item>, KeyringError> {
    let mut first_locked = None;
    for item in items {
        if !item.is_locked().await? {
            return Ok(Some(item));
        }
        first_locked.get_or_insert(item);
    }
    Ok(first_locked)
}

/// The attributes in the form `oo7` sends them.
fn attribute_pairs(attributes: &SecretAttributes) -> Vec<(&str, &str)> {
    attributes.iter().collect()
}
