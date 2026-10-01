// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop keyring through the freedesktop Secret Service.
//!
//! Replaces the libsecret calls of `v2.0.0:desktop/session_credentials.py` and
//! `v2.0.0:desktop/winspace.py` with the pure-Rust `oo7` client: GNOME Keyring,
//! `KWallet` and `oo7-daemon` all implement the Secret Service D-Bus API.
//! The session is encrypted where the service supports it, as libsecret's
//! is, so secrets do not cross the session bus in the clear.
//!
//! Like libsecret, lookups prefer unlocked items and unlock a locked one
//! (the service may show its unlock prompt), stores unlock the collection
//! first, and clears delete every matching item in every collection.

use std::future::Future;
use std::io;
use std::sync::Arc;

use oo7::dbus::{Collection, Item, Service};
use oo7::zbus;

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
    /// The error in the app's words: a dismissed unlock prompt and a D-Bus
    /// timeout are named, anything else keeps the client's error as its
    /// source.
    fn from(error: oo7::dbus::Error) -> Self {
        match error {
            oo7::dbus::Error::Dismissed => KeyringError::UnlockDismissed,
            error if is_timeout(&error) => KeyringError::TimedOut,
            error => KeyringError::Failed {
                source: Arc::new(error),
            },
        }
    }
}

/// True when a D-Bus call to the service timed out: `zbus`'s own method
/// timeout, or the bus reporting that no reply came in time.
fn is_timeout(error: &oo7::dbus::Error) -> bool {
    let oo7::dbus::Error::ZBus(error) = error else {
        return false;
    };
    match error {
        zbus::Error::InputOutput(io_error) => io_error.kind() == io::ErrorKind::TimedOut,
        zbus::Error::FDO(bus_error) => matches!(
            **bus_error,
            zbus::fdo::Error::TimedOut(_) | zbus::fdo::Error::NoReply(_)
        ),
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;

    struct ServiceErrorCase {
        what: &'static str,
        error: oo7::dbus::Error,
        message: &'static str,
    }

    fn io_error(kind: io::ErrorKind) -> oo7::dbus::Error {
        let error = io::Error::new(kind, "developer text");
        oo7::dbus::Error::ZBus(zbus::Error::InputOutput(Arc::new(error)))
    }

    fn bus_error(error: zbus::fdo::Error) -> oo7::dbus::Error {
        oo7::dbus::Error::ZBus(zbus::Error::FDO(Box::new(error)))
    }

    /// Sign out shows these messages after "Disconnected, but saved
    /// credentials could not be removed.", so they are the app's words,
    /// never the client's.
    #[test]
    fn service_errors_are_reported_in_the_apps_words() {
        let cases = [
            ServiceErrorCase {
                what: "dismissed unlock prompt",
                error: oo7::dbus::Error::Dismissed,
                message: "The keyring unlock was cancelled.",
            },
            ServiceErrorCase {
                what: "zbus method timeout",
                error: io_error(io::ErrorKind::TimedOut),
                message: "The system keyring did not answer in time.",
            },
            ServiceErrorCase {
                what: "no reply from the bus",
                error: bus_error(zbus::fdo::Error::NoReply("developer text".into())),
                message: "The system keyring did not answer in time.",
            },
            ServiceErrorCase {
                what: "broken connection",
                error: io_error(io::ErrorKind::BrokenPipe),
                message: "The system keyring reported an error.",
            },
            ServiceErrorCase {
                what: "deleted item",
                error: oo7::dbus::Error::Deleted,
                message: "The system keyring reported an error.",
            },
        ];
        for case in cases {
            let error = KeyringError::from(case.error);
            assert_eq!(error.to_string(), case.message, "{}", case.what);
        }
    }

    #[test]
    fn an_unexpected_service_error_keeps_the_clients_error_as_its_source() {
        let error = KeyringError::from(io_error(io::ErrorKind::BrokenPipe));

        let source = std::error::Error::source(&error).expect("the client's error");

        assert!(source.to_string().contains("developer text"), "{source}");
    }
}
