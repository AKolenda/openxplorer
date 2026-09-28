// SPDX-License-Identifier: AGPL-3.0-only
//! Which SMB server a location belongs to.
//!
//! Ports `server_key` in `desktop/session_credentials.py`. Credentials are
//! kept per server, never per share, so every share on a server reuses one
//! sign-in. Host names compare case-insensitively, but ports and aliases
//! stay distinct: `nas.local` and `10.0.0.1` are different servers even if
//! they resolve to the same machine.

use std::fmt;

use crate::location::{normalise, split_location};

/// SMB's port, used when a location names none (or port 0, as Python's
/// `u.port or 445` does).
pub const DEFAULT_SMB_PORT: u16 = 445;

/// One SMB server: a lower-case host name and a port.
///
/// Safety rule (NET-014): the key is never derived from DNS, search
/// results or a share redirect, so credentials are never forwarded to
/// another host name or port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerKey {
    host: String,
    port: u16,
}

impl ServerKey {
    /// The server of an SMB location, or `None` for an address that is not
    /// a valid SMB location with a host name.
    pub fn for_location(uri: &str) -> Option<Self> {
        let canonical = normalise(uri).ok()?;
        let parts = split_location(&canonical).ok()?;
        if parts.scheme != "smb" {
            return None;
        }
        let host = parts.hostname()?;
        let port = parts.port().ok()?.filter(|port| *port != 0);
        Some(Self {
            host,
            port: port.unwrap_or(DEFAULT_SMB_PORT),
        })
    }

    /// The lower-case host name, without brackets or port.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port; [`DEFAULT_SMB_PORT`] when the location names none.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for ServerKey {
    /// `host:port`, for messages and keyring item labels.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(uri: &str) -> Option<ServerKey> {
        ServerKey::for_location(uri)
    }

    /// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_host_key_not_share_key`
    ///
    /// parity: NET-011, NET-014
    #[test]
    fn shares_on_one_server_share_a_key_whatever_the_host_case() {
        assert_eq!(key("smb://NAS/a"), key("smb://nas/b"));
        assert!(key("smb://nas/b").is_some());
    }

    /// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_distinct_ports`
    ///
    /// parity: NET-014
    #[test]
    fn a_custom_port_is_a_different_server() {
        assert_ne!(key("smb://nas:1445/a"), key("smb://nas/a"));
        assert_eq!(key("smb://nas:445/a"), key("smb://nas/a"));
    }

    /// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_no_alias_sharing`
    ///
    /// parity: NET-014
    #[test]
    fn host_aliases_are_never_merged() {
        assert_ne!(key("smb://nas.local/a"), key("smb://10.0.0.1/a"));
    }

    #[test]
    fn port_zero_means_the_default_port() {
        let server = key("smb://nas:0/a").expect("an SMB location");
        assert_eq!(server.port(), DEFAULT_SMB_PORT);
        assert_eq!(server.to_string(), "nas:445");
    }

    #[test]
    fn locations_that_are_not_smb_servers_have_no_key() {
        assert_eq!(key("file:///home/demo"), None);
        assert_eq!(key("mtp://[usb:001,010]/"), None);
        assert_eq!(key("smb://user@nas/share"), None, "credentials are refused");
        assert_eq!(key(""), None);
    }
}
