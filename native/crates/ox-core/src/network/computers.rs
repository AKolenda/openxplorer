// SPDX-License-Identifier: AGPL-3.0-only
//! The computers Network lists: every service one host advertises, merged
//! into one row per device, as Windows 11's Network folder lists devices
//! rather than services.
//!
//! [`discover_servers`](super::discover_servers) finds services: an SMB
//! server, an SFTP, FTP or WebDAV server, each advertised on its own. One
//! NAS often advertises several of them under different names ("HARBORNAS"
//! for its shares, "Harbor Files" for SFTP, FTP and WebDAV), and the SMB
//! browser and DNS-SD can each find its shares. [`computers_among`] groups
//! them by host:
//!
//! - the host is the service's host name without DNS-SD's `.local`, so
//!   `smb://harbornas/` (the SMB browser's `NetBIOS` name) and
//!   `sftp://harbornas.local:2222/` are one computer;
//! - a host whose SMB service advertises another name (a `NetBIOS` name
//!   that differs from its DNS-SD host name) is also known by that name,
//!   so the SMB browser's row for that name joins it.
//!
//! A computer is named as Windows names it: by the name its SMB server
//! advertises (its `NetBIOS` name), else by its host name. It opens as
//! `\\host` (its SMB address, [`DiscoveredComputer::uri`]), which lists
//! the shares when it has an SMB server and its other services as rows of
//! their own ("Harbor Files (SFTP)"), so nothing that was advertised is
//! lost.

use super::discovery::DiscoveredServer;
use crate::location::{split_location, LocationKind};

/// The DNS-SD domain `GVfs` and Avahi give multicast host names.
const MDNS_DOMAIN: &str = ".local";

/// The longest `NetBIOS` name, which is what an SMB server's advertised name
/// must fit to be taken as another name of its host.
const NETBIOS_NAME_LENGTH: usize = 15;

/// The protocol one service is reached over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceProtocol {
    /// Windows file sharing.
    Smb,
    /// SSH file transfer.
    Sftp,
    /// FTP, plain or over TLS.
    Ftp,
    /// WebDAV, plain or over HTTPS.
    WebDav,
    /// NFS.
    Nfs,
}

impl ServiceProtocol {
    /// The protocol of a location's `scheme`, or `None` for one that is
    /// not a file service.
    pub fn from_scheme(scheme: &str) -> Option<Self> {
        match crate::location::canonical_remote_scheme(scheme) {
            "smb" => Some(Self::Smb),
            "sftp" => Some(Self::Sftp),
            "ftp" | "ftps" => Some(Self::Ftp),
            "dav" | "davs" | "dav+sd" | "davs+sd" => Some(Self::WebDav),
            "nfs" => Some(Self::Nfs),
            _ => None,
        }
    }

    /// The protocol's usual name, which ends a service row's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Smb => "SMB",
            Self::Sftp => "SFTP",
            Self::Ftp => "FTP",
            Self::WebDav => "WebDAV",
            Self::Nfs => "NFS",
        }
    }
}

/// One device on the network and every file service it advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredComputer {
    /// The computer's name: its SMB server's advertised (`NetBIOS`) name,
    /// else its host name without `.local`.
    pub name: String,
    /// What the computer is opened as: its SMB server, `smb://host/` (with
    /// the port when it is not 445), or `smb://host/` for a computer
    /// without one, whose listing then holds only its other services.
    pub uri: String,
    /// The host the computer was grouped by, lower case, without `.local`.
    pub key: String,
    /// Whether it advertises an SMB server, whose shares its listing
    /// shows.
    pub has_smb: bool,
    /// Its services besides SMB, in protocol order, then by name.
    pub services: Vec<DiscoveredServer>,
}

impl DiscoveredComputer {
    /// Whether `uri` is this computer's location, ignoring a trailing `/`
    /// and the case of the host.
    pub fn is_at(&self, uri: &str) -> bool {
        computer_key_of(uri).is_some_and(|key| key == self.key) && crate::location::is_smb_server(uri)
    }
}

/// The name a service row inside a computer has: what the service
/// advertises, then its protocol in brackets, such as "Harbor Files
/// (SFTP)".
pub fn service_row_name(service: &DiscoveredServer) -> String {
    let protocol = service.protocol().map_or("", ServiceProtocol::name);
    let name = if service.name.is_empty() {
        host_without_domain(&service.host)
    } else {
        service.name.clone()
    };
    if protocol.is_empty() {
        name
    } else {
        format!("{name} ({protocol})")
    }
}

/// The host of `uri` as computers are grouped by: lower case, without a
/// trailing dot and without `.local`; `None` for a location without a
/// host.
pub fn computer_key_of(uri: &str) -> Option<String> {
    let parts = split_location(uri).ok()?;
    parts.hostname().map(|host| host_key(&host))
}

/// `host` as computers are grouped by.
fn host_key(host: &str) -> String {
    let host = host.trim_end_matches('.').to_lowercase();
    host.strip_suffix(MDNS_DOMAIN).unwrap_or(&host).to_owned()
}

/// `host` without `.local`, keeping its case.
fn host_without_domain(host: &str) -> String {
    let host = host.trim_end_matches('.');
    let lower = host.to_lowercase();
    if lower.ends_with(MDNS_DOMAIN) {
        host[..host.len() - MDNS_DOMAIN.len()].to_owned()
    } else {
        host.to_owned()
    }
}

/// Whether `name`, an SMB server's advertised name, may be another name
/// of its host: a `NetBIOS` name, short and without spaces or dots.
fn is_netbios_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= NETBIOS_NAME_LENGTH
        && !name.contains(|c: char| c.is_whitespace() || c == '.')
}

/// The computers `servers` belong to, one per host, in the order each
/// host was first found. See the module documentation for how services
/// are grouped and named.
pub fn computers_among(servers: &[DiscoveredServer]) -> Vec<DiscoveredComputer> {
    // Group by host, in the order found.
    let mut groups: Vec<(String, Vec<&DiscoveredServer>)> = Vec::new();
    for server in servers {
        let key = if server.host.is_empty() {
            // A service whose host is not known (an unresolved DNS-SD
            // service) joins the host whose other services advertise the
            // same name, as one NAS names them all; else it is a device of
            // its own.
            servers
                .iter()
                .find(|other| !other.host.is_empty() && other.name.eq_ignore_ascii_case(&server.name))
                .map_or_else(
                    || format!(" {}", server.name.to_lowercase()),
                    |other| host_key(&other.host),
                )
        } else {
            host_key(&server.host)
        };
        match groups.iter_mut().find(|(known, _)| *known == key) {
            Some((_, members)) => members.push(server),
            None => groups.push((key, vec![server])),
        }
    }
    // A host known by its SMB server's NetBIOS name too takes in the group
    // of that name.
    let aliases: Vec<(String, String)> = groups
        .iter()
        .flat_map(|(key, members)| {
            members
                .iter()
                .filter(|server| server.protocol() == Some(ServiceProtocol::Smb))
                .filter(|server| is_netbios_name(&server.name))
                .map(|server| (server.name.to_lowercase(), key.clone()))
                .filter(|(alias, key)| alias != key)
                .collect::<Vec<_>>()
        })
        .collect();
    for (alias, key) in aliases {
        let Some(from) = groups.iter().position(|(known, _)| *known == alias) else {
            continue;
        };
        if !groups.iter().any(|(known, _)| *known == key) {
            continue;
        }
        let (_, members) = groups.remove(from);
        if let Some((_, into)) = groups.iter_mut().find(|(known, _)| *known == key) {
            into.extend(members);
        }
    }
    groups
        .into_iter()
        .map(|(key, members)| computer_of(key, &members))
        .collect()
}

/// The computer of the services `members`, grouped under `key`.
fn computer_of(key: String, members: &[&DiscoveredServer]) -> DiscoveredComputer {
    let smb_servers: Vec<&DiscoveredServer> = members
        .iter()
        .copied()
        .filter(|server| server.protocol() == Some(ServiceProtocol::Smb))
        .collect();
    // The DNS-SD SMB service names the host as DNS resolves it, which
    // opens where a bare NetBIOS name may not; prefer it.
    let smb = smb_servers
        .iter()
        .copied()
        .find(|server| server.host.to_lowercase().ends_with(MDNS_DOMAIN))
        .or_else(|| smb_servers.first().copied());
    let first = members.first().copied();
    let name = smb_servers
        .iter()
        .map(|server| server.name.as_str())
        .find(|name| is_netbios_name(name))
        .map(str::to_owned)
        .or_else(|| {
            first
                .filter(|server| !server.host.is_empty())
                .map(|server| host_without_domain(&server.host))
        })
        .or_else(|| first.map(|server| server.name.clone()))
        .unwrap_or_else(|| key.clone());
    let uri = match smb {
        Some(server) => smb_server_uri(&server.uri),
        None => first.filter(|server| !server.host.is_empty()).map_or_else(
            || first.map(|server| server.uri.clone()).unwrap_or_default(),
            |server| format!("smb://{}/", server.host),
        ),
    };
    let mut services: Vec<DiscoveredServer> = members
        .iter()
        .filter(|server| server.protocol() != Some(ServiceProtocol::Smb))
        .map(|server| (*server).clone())
        .collect();
    services.sort_by_cached_key(|server| {
        (
            server.protocol(),
            glib::casefold(&server.name),
            server.uri.clone(),
        )
    });
    services.dedup_by(|a, b| a.uri == b.uri);
    DiscoveredComputer {
        name,
        uri,
        key,
        has_smb: smb.is_some(),
        services,
    }
}

/// An SMB server's location without the default port, as Windows writes
/// `\\host`.
fn smb_server_uri(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return uri.to_owned();
    };
    if parts.kind() != LocationKind::Smb {
        return uri.to_owned();
    }
    let authority = parts
        .authority
        .strip_suffix(&format!(":{}", super::DEFAULT_SMB_PORT))
        .unwrap_or(&parts.authority);
    format!("smb://{authority}/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(uri: &str, name: &str, host: &str) -> DiscoveredServer {
        DiscoveredServer {
            uri: uri.into(),
            label: name.into(),
            name: name.into(),
            host: host.into(),
        }
    }

    /// One NAS advertising SMB, SFTP, FTP and WebDAV is one computer,
    /// named by its SMB server, and a second host is a second one.
    ///
    /// parity: NET-025, HOME-008
    #[test]
    fn every_service_of_one_host_is_one_computer() {
        let found = [
            service("smb://harbornas.local:445/", "HARBORNAS", "harbornas.local"),
            service("ftp://harbornas.local:2121/", "Harbor Files", "harbornas.local"),
            service("sftp://harbornas.local:2222/", "Harbor Files", "harbornas.local"),
            service(
                "dav://harbornas.local:8080/dav",
                "Harbor Files",
                "harbornas.local",
            ),
            service("smb://willowpc.local:445/", "WILLOWPC", "willowpc.local"),
            service("sftp://willowpc.local:2222/", "Willow Files", "willowpc.local"),
        ];

        let computers = computers_among(&found);

        let names: Vec<&str> = computers.iter().map(|computer| computer.name.as_str()).collect();
        assert_eq!(names, ["HARBORNAS", "WILLOWPC"]);
        let harbor = &computers[0];
        assert_eq!(harbor.uri, "smb://harbornas.local/");
        assert!(harbor.has_smb);
        let rows: Vec<String> = harbor.services.iter().map(service_row_name).collect();
        assert_eq!(
            rows,
            [
                "Harbor Files (SFTP)",
                "Harbor Files (FTP)",
                "Harbor Files (WebDAV)"
            ]
        );
        assert!(harbor.is_at("smb://HARBORNAS.local"));
        assert!(!harbor.is_at("smb://harbornas.local/Projects"));
    }

    /// The SMB browser's `NetBIOS` name and the DNS-SD host name of the same
    /// server join, whichever way the host is spelled.
    ///
    /// parity: NET-025
    #[test]
    fn a_netbios_name_and_its_dns_sd_host_are_one_computer() {
        let found = [
            service("smb://harbornas/", "HARBORNAS", "harbornas"),
            service("sftp://vm.local:2222/", "Harbor Files", "vm.local"),
            service("smb://vm.local/", "HARBORNAS", "vm.local"),
        ];

        let computers = computers_among(&found);

        assert_eq!(computers.len(), 1, "{computers:?}");
        assert_eq!(computers[0].name, "HARBORNAS");
        assert_eq!(computers[0].uri, "smb://vm.local/");
        assert_eq!(computers[0].services.len(), 1);
    }
}
