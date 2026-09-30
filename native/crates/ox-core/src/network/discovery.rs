// SPDX-License-Identifier: AGPL-3.0-only
//! Servers advertising on the local network, for the Network page's
//! "Discover servers".
//!
//! Ports `discover_network` in `desktop/winspace.py` and `discover_servers`
//! in `desktop/gio_backend.py`. `GVfs`'s `network:///` combines DNS-SD,
//! WS-Discovery and the SMB browser; this module only reads it. There is
//! no port scan, no password prompt and no listing of any server's shares.

use std::time::Duration;

use gio::prelude::*;

use super::error::NetworkError;
use crate::location::{normalise, split_location};

/// Shown under the discovered servers: discovery cannot promise to find
/// every host.
pub const DISCOVERY_NOTE: &str = "Finds servers advertising on this network. Firewalls, VLANs, disabled \
     discovery or missing GVfs services can hide devices. Enter a server address manually when needed.";

/// `GVfs`'s combined network browser.
const NETWORK_ROOT: &str = "network:///";
/// How long mounting and reading the browser may take together.
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(15);
/// The most browser entries read, whether or not they are SMB servers.
const MAX_ENTRIES: usize = 500;
/// How many entries one read asks GIO for.
const ENTRIES_PER_READ: i32 = 100;
/// The attributes read for each entry.
const ENTRY_ATTRIBUTES: &str = "standard::target-uri,standard::display-name";

/// One SMB server found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// The server's location, `smb://host/` (with the port if it
    /// advertises one).
    pub uri: String,
    /// The advertised name, else the host name.
    pub label: String,
    /// The lower-case host name.
    pub host: String,
}

/// The result of one discovery run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    /// SMB servers, each once, sorted by label ignoring case.
    pub servers: Vec<DiscoveredServer>,
    /// Why reading the browser stopped early, in GIO's words; the servers
    /// found until then are kept.
    pub warnings: Vec<String>,
}

/// One entry of the network browser.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AdvertisedEntry {
    /// Where the entry leads, for example `smb://nas/`.
    target_uri: Option<String>,
    /// The name the service advertises.
    display_name: String,
}

/// Lists the SMB servers advertising on the local network, giving up after
/// 15 seconds.
///
/// Discovery may start `GVfs` services, but it never asks for a password:
/// a server that wants one is skipped.
///
/// # Errors
///
/// [`NetworkError::Cancelled`] when the deadline passes, or GIO's error
/// when the network browser cannot be mounted.
pub async fn discover_servers() -> Result<Discovery, NetworkError> {
    let discovery = glib::future_with_timeout(DISCOVERY_DEADLINE, discover()).await;
    discovery.unwrap_or(Err(NetworkError::Cancelled))
}

async fn discover() -> Result<Discovery, NetworkError> {
    let root = gio::File::for_uri(NETWORK_ROOT);
    let operation = silent_operation();
    let mounted = root
        .mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(&operation))
        .await;
    if let Err(error) = mounted {
        if !error.matches(gio::IOErrorEnum::AlreadyMounted) {
            return Err(error.into());
        }
    }
    let (entries, failure) = read_entries(&root).await;
    let warnings = failure
        .map(|error| error.message().to_owned())
        .into_iter()
        .collect();
    Ok(Discovery {
        servers: servers_among(entries),
        warnings,
    })
}

/// A mount operation that aborts every password request.
///
/// Privacy rule (NET-024): discovery may never ask for credentials.
fn silent_operation() -> gio::MountOperation {
    let operation = gio::MountOperation::new();
    operation.connect_ask_password(|operation, _message, _username, _domain, _flags| {
        // GIO's default handler would reply "unhandled" instead.
        operation.stop_signal_emission_by_name("ask-password");
        operation.reply(gio::MountOperationResult::Aborted);
    });
    operation
}

/// Reads up to [`MAX_ENTRIES`] entries of `root`. A failure part-way keeps
/// the entries read before it.
async fn read_entries(root: &gio::File) -> (Vec<AdvertisedEntry>, Option<glib::Error>) {
    let enumerator = root
        .enumerate_children_future(
            ENTRY_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await;
    let enumerator = match enumerator {
        Ok(enumerator) => enumerator,
        Err(error) => return (Vec::new(), Some(error)),
    };
    let mut entries = Vec::new();
    while entries.len() < MAX_ENTRIES {
        let batch = match enumerator
            .next_files_future(ENTRIES_PER_READ, glib::Priority::DEFAULT)
            .await
        {
            Ok(batch) if batch.is_empty() => break,
            Ok(batch) => batch,
            Err(error) => return (entries, Some(error)),
        };
        entries.extend(batch.iter().map(advertised_entry));
    }
    entries.truncate(MAX_ENTRIES);
    (entries, None)
}

fn advertised_entry(info: &gio::FileInfo) -> AdvertisedEntry {
    AdvertisedEntry {
        target_uri: info.attribute_string("standard::target-uri").map(Into::into),
        display_name: info.display_name().into(),
    }
}

/// The SMB servers among `entries`: each server once (the latest entry
/// wins), sorted by label ignoring case.
fn servers_among(entries: impl IntoIterator<Item = AdvertisedEntry>) -> Vec<DiscoveredServer> {
    let mut servers: Vec<DiscoveredServer> = Vec::new();
    for server in entries.into_iter().filter_map(smb_server) {
        match servers.iter_mut().find(|known| known.uri == server.uri) {
            Some(known) => *known = server,
            None => servers.push(server),
        }
    }
    servers.sort_by_cached_key(|server| glib::casefold(&server.label));
    servers
}

/// The SMB server `entry` leads to, or `None` for another kind of service
/// or an address that is not a valid location.
fn smb_server(entry: AdvertisedEntry) -> Option<DiscoveredServer> {
    let target = normalise(entry.target_uri.as_deref()?).ok()?;
    let parts = split_location(&target).ok()?;
    if !parts.is_smb() {
        return None;
    }
    let host = parts.hostname()?;
    let label = if entry.display_name.is_empty() {
        host.clone()
    } else {
        entry.display_name
    };
    Some(DiscoveredServer {
        uri: format!("smb://{}/", parts.authority),
        label,
        host,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    fn entry(target_uri: &str, display_name: &str) -> AdvertisedEntry {
        AdvertisedEntry {
            target_uri: Some(target_uri.into()),
            display_name: display_name.into(),
        }
    }

    fn server(uri: &str, label: &str, host: &str) -> DiscoveredServer {
        DiscoveredServer {
            uri: uri.into(),
            label: label.into(),
            host: host.into(),
        }
    }

    /// parity: NET-024
    #[test]
    fn only_smb_servers_are_listed_once_and_sorted_by_label() {
        let entries = [
            entry("smb://studio-nas/", "Studio NAS"),
            entry("sftp://build-host/", "Build host"),
            entry("smb://archive-nas/Projects", "archive-nas"),
            entry("smb://STUDIO-NAS/", "Studio NAS (renamed)"),
            entry("https://printer.invalid/", "Printer"),
        ];

        let servers = servers_among(entries);

        let expected = [
            server("smb://archive-nas/", "archive-nas", "archive-nas"),
            server("smb://studio-nas/", "Studio NAS (renamed)", "studio-nas"),
        ];
        assert_eq!(servers, expected);
    }

    /// parity: NET-024
    #[test]
    fn a_server_without_a_name_is_labelled_by_its_host_and_keeps_its_port() {
        let servers = servers_among([entry("smb://nas:1445/", "")]);

        assert_eq!(servers, [server("smb://nas:1445/", "nas", "nas")]);
    }

    /// Discovery never asks for a password: `GVfs`'s request is aborted
    /// and no password is set on the operation.
    ///
    /// parity: NET-024, SAFE-014
    #[test]
    fn discovery_aborts_every_password_request() {
        let operation = silent_operation();
        let replies = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&replies);
        operation.connect_reply(move |_, result| recorded.borrow_mut().push(result));

        let flags = gio::AskPasswordFlags::NEED_USERNAME | gio::AskPasswordFlags::NEED_PASSWORD;
        operation.emit_by_name::<()>("ask-password", &[&"Sign in to nas", &"user", &"", &flags]);

        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
        assert_eq!(operation.password(), None);
    }

    #[test]
    fn entries_without_a_valid_target_are_skipped() {
        let without_target = AdvertisedEntry {
            target_uri: None,
            display_name: "Workgroup".into(),
        };
        let with_credentials = entry("smb://user@nas/", "NAS");

        assert!(servers_among([without_target, with_credentials]).is_empty());
    }
}
