// SPDX-License-Identifier: AGPL-3.0-only
//! The keyring the SMB credential store reads and writes through.
//!
//! Ports the libsecret calls of `desktop/session_credentials.py`
//! (`password_lookup_sync`, `password_store_sync`, `password_clear_sync`)
//! and the GNOME `NetworkPassword` clear of `sign_out` in
//! `desktop/winspace.py`. [`Keyring`] is the seam: production code uses
//! [`SecretService`](super::SecretService), tests an in-memory keyring.
//!
//! libsecret names an item's schema in the `xdg:schema` attribute. Items
//! written by the Python app carry it, so the native app adds it too
//! ([`SecretAttributes::for_schema`]); both apps then find each other's
//! credentials.

use std::collections::BTreeMap;
use std::fmt;

/// The attribute libsecret stores an item's schema name in.
pub const SCHEMA_ATTRIBUTE: &str = "xdg:schema";

/// Attributes of keyring items. Stored with an item, they identify it; as
/// a query, they match every item that has all of them, as the Secret
/// Service's `SearchItems` does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretAttributes(BTreeMap<String, String>);

impl SecretAttributes {
    /// Attributes of an item of `schema`, as libsecret writes and matches
    /// them for a schema without `SECRET_SCHEMA_DONT_MATCH_NAME`.
    pub fn for_schema(schema: &str) -> Self {
        Self::default().with(SCHEMA_ATTRIBUTE, schema)
    }

    /// Attributes without a schema name, which match items of any schema,
    /// as libsecret does for a schema with `SECRET_SCHEMA_DONT_MATCH_NAME`.
    pub fn any_schema() -> Self {
        Self::default()
    }

    /// These attributes and `name` = `value`.
    #[must_use]
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.0.insert(name.to_owned(), value.to_owned());
        self
    }

    /// The value of `name`, if set.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Every attribute, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// True when an item with `item` attributes matches this query: it has
    /// every attribute of the query with the same value.
    pub fn matches(&self, item: &SecretAttributes) -> bool {
        self.iter().all(|(name, value)| item.get(name) == Some(value))
    }
}

/// Where a new secret is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyringCollection {
    /// The default (login) keyring: kept across logins
    /// (libsecret's `SECRET_COLLECTION_DEFAULT`).
    Default,
    /// The session keyring: kept until the user logs out, never written to
    /// disk (libsecret's `SECRET_COLLECTION_SESSION`).
    Session,
}

/// A text secret to store.
///
/// `Debug` never shows the secret text, so a stored credential cannot leak
/// into a log message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NewSecret<'a> {
    /// The collection to store it in.
    pub collection: KeyringCollection,
    /// The label a keyring manager such as Seahorse shows.
    pub label: &'a str,
    /// The attributes that find it again. An existing item with the same
    /// attributes is replaced.
    pub attributes: &'a SecretAttributes,
    /// The secret, stored as `text/plain` so libsecret reads it back.
    pub text: &'a str,
}

impl fmt::Debug for NewSecret<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewSecret")
            .field("collection", &self.collection)
            .field("label", &self.label)
            .field("attributes", &self.attributes)
            .finish_non_exhaustive()
    }
}

/// Why the keyring could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyringError {
    /// No Secret Service answers on the session bus. Credentials still work
    /// from memory; there is deliberately no plaintext fallback.
    #[error(
        "The system keyring is unavailable. Credentials are reused in this window only; they cannot \
         survive closing it."
    )]
    Unavailable,
    /// The keyring did not answer in time, for example because its unlock
    /// prompt was left open.
    #[error("The system keyring did not answer in time.")]
    TimedOut,
    /// The Secret Service reported an error, for example a dismissed unlock
    /// prompt; the message is the service's.
    #[error("{0}")]
    Failed(String),
}

/// A keyring holding text secrets, such as the Secret Service.
///
/// Calls may block while the keyring is unlocked or a prompt is shown, so
/// never call them on the GTK main thread.
pub trait Keyring: Send + Sync {
    /// The text of the first item matching `query`, unlocking it if needed.
    ///
    /// # Errors
    ///
    /// A [`KeyringError`] when the keyring cannot be searched or read.
    fn lookup(&self, query: &SecretAttributes) -> Result<Option<String>, KeyringError>;

    /// Stores `secret`, replacing an item with the same attributes.
    ///
    /// # Errors
    ///
    /// A [`KeyringError`] when the secret was not stored.
    fn store(&self, secret: &NewSecret<'_>) -> Result<(), KeyringError>;

    /// Deletes every item matching `query`, in every collection. Returns
    /// whether anything was deleted.
    ///
    /// # Errors
    ///
    /// A [`KeyringError`] when the keyring cannot be searched or an item
    /// cannot be deleted.
    fn clear(&self, query: &SecretAttributes) -> Result<bool, KeyringError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_matches_items_holding_all_of_its_attributes() {
        let item = SecretAttributes::for_schema("io.winspace.SmbCredentials")
            .with("server", "nas")
            .with("port", "445")
            .with("scope", "session");
        let same_server = SecretAttributes::for_schema("io.winspace.SmbCredentials")
            .with("server", "nas")
            .with("port", "445");
        let other_scope = same_server.clone().with("scope", "permanent");
        let other_schema = SecretAttributes::for_schema("org.gnome.keyring.NetworkPassword");

        assert!(same_server.matches(&item));
        assert!(!other_scope.matches(&item));
        assert!(!other_schema.matches(&item));
        assert!(SecretAttributes::any_schema()
            .with("server", "nas")
            .matches(&item));
    }

    /// parity: SAFE-011
    #[test]
    fn secrets_are_left_out_of_debug_output() {
        let attributes = SecretAttributes::for_schema("schema");
        let secret = NewSecret {
            collection: KeyringCollection::Session,
            label: "OpenXplorer SMB: nas",
            attributes: &attributes,
            text: "unique-secret-value",
        };
        let debug = format!("{secret:?}");
        assert!(!debug.contains("unique-secret-value"), "{debug}");
        assert!(debug.contains("OpenXplorer SMB: nas"), "{debug}");
    }
}
