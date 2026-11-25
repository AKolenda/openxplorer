// SPDX-License-Identifier: AGPL-3.0-only
//! Servers advertising on the local network, for the Network page's
//! "Discover servers".
//!
//! Ports `discover_network` in `v2.0.0:desktop/winspace.py` and `discover_servers`
//! in `v2.0.0:desktop/gio_backend.py`. `GVfs`'s `network:///` combines DNS-SD,
//! WS-Discovery and the SMB browser; this module only reads it. There is
//! no port scan, no password prompt and no listing of any server's shares.
//!
//! `GVfs` lists a WebDAV server as `dav+sd://Name._webdav._tcp.local`, a
//! DNS-SD service rather than a host. Its host, port and folder are asked
//! of Avahi, the DNS-SD service `GVfs` reads them from, so the server joins
//! the other services of its host ([`computers_among`]); a service Avahi
//! cannot resolve stays listed as `GVfs` gives it.
//!
//! [`discovery_sources`] says whether discovery can find anything at all:
//! whether `GVfs` has its network browser and a DNS-SD (Avahi) or
//! WS-Discovery (wsdd) source is there, so Network can say discovery is
//! off, as Windows does, instead of listing nothing.

use std::time::Duration;

use gio::prelude::*;

use super::computers::{computers_among, DiscoveredComputer, ServiceProtocol};
use super::error::NetworkError;
use crate::location::decode_uri_component;
use crate::location::{normalise, split_location};

/// Shown under the discovered servers: discovery cannot promise to find
/// every host.
pub const DISCOVERY_NOTE: &str = "Finds servers advertising on this network. Firewalls, VLANs, disabled \
     discovery or missing GVfs services can hide devices. Enter a server address manually when needed.";

/// `GVfs`'s combined network browser.
const NETWORK_ROOT: &str = "network:///";
/// How long mounting and reading the browser may take together.
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(15);
/// The most browser entries read, whether or not they are file servers.
const MAX_ENTRIES: usize = 500;
/// How many entries one read asks GIO for.
const ENTRIES_PER_READ: i32 = 100;
/// The attributes read for each entry.
const ENTRY_ATTRIBUTES: &str = "standard::target-uri,standard::display-name,standard::name";
/// What `GVfs`'s DNS-SD browser starts the name of each service with.
const DNS_SD_PREFIX: &str = "dnssd-server-";
/// How long Avahi may take to resolve one service.
const RESOLVE_TIMEOUT_MS: i32 = 2000;
/// The most DNS-SD services resolved in one pass.
const MAX_RESOLVED: usize = 32;

/// One SMB, SFTP, FTP, WebDAV or NFS server found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// The server's location, `smb://host/` or `sftp://host/` (with the port if it
    /// advertises one).
    pub uri: String,
    /// The name `GVfs` shows, which may end in the kind of service, such
    /// as "Harbor Files (Remote Login)".
    pub label: String,
    /// The name the service advertises ("Harbor Files"), else the label.
    pub name: String,
    /// The lower-case host name; empty for a DNS-SD service whose host is
    /// not known.
    pub host: String,
}

impl DiscoveredServer {
    /// The protocol the service is reached over.
    pub fn protocol(&self) -> Option<ServiceProtocol> {
        let scheme = self.uri.split_once(':').map_or("", |(scheme, _)| scheme);
        ServiceProtocol::from_scheme(&scheme.to_ascii_lowercase())
    }
}

/// The result of one discovery run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Servers, each once, sorted by label ignoring case.
    pub servers: Vec<DiscoveredServer>,
    /// Why reading the browser stopped early, in GIO's words; the servers
    /// found until then are kept.
    pub warnings: Vec<String>,
}

impl Discovery {
    /// The computers the servers belong to, one per device.
    pub fn computers(&self) -> Vec<DiscoveredComputer> {
        computers_among(&self.servers)
    }
}

/// What network discovery can use on this desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoverySources {
    /// `GVfs` has its network browser (`network:///`).
    pub browser: bool,
    /// `GVfs` can browse DNS-SD and Avahi answers on the system bus.
    pub dns_sd: bool,
    /// `GVfs` can browse WS-Discovery and the `wsdd` program it runs is
    /// installed.
    pub ws_discovery: bool,
}

impl DiscoverySources {
    /// Every source there.
    pub const ALL: Self = Self {
        browser: true,
        dns_sd: true,
        ws_discovery: true,
    };

    /// Whether discovery can find any computer: the browser and at least
    /// one source it reads. (`GVfs`'s SMB browser also reads `NetBIOS`, but
    /// SMB1 browsing is off on current Windows and Samba.)
    pub fn can_discover(self) -> bool {
        self.browser && (self.dns_sd || self.ws_discovery)
    }
}

/// Which discovery sources this desktop has. Reads the schemes GIO
/// supports, looks for `wsdd` on the `PATH` and asks Avahi its state on
/// the system bus, giving up on it after two seconds. Nothing is started
/// or changed.
pub async fn discovery_sources() -> DiscoverySources {
    let schemes: Vec<String> = gio::Vfs::default()
        .supported_uri_schemes()
        .iter()
        .map(ToString::to_string)
        .collect();
    let has = |scheme: &str| schemes.iter().any(|known| known == scheme);
    let browser = has("network");
    let ws_discovery = has("wsdd") && glib::find_program_in_path("wsdd").is_some();
    let dns_sd = (has("dns-sd") || has("dnssd")) && avahi_is_running().await;
    DiscoverySources {
        browser,
        dns_sd,
        ws_discovery,
    }
}

/// Whether Avahi answers on the system bus.
async fn avahi_is_running() -> bool {
    let Ok(bus) = gio::bus_get_future(gio::BusType::System).await else {
        return false;
    };
    bus.call_future(
        Some(AVAHI_NAME),
        "/",
        AVAHI_SERVER,
        "GetState",
        None,
        Some(glib::VariantTy::new("(i)").expect("valid type")),
        gio::DBusCallFlags::NO_AUTO_START,
        RESOLVE_TIMEOUT_MS,
    )
    .await
    .is_ok()
}

/// Avahi's bus name.
const AVAHI_NAME: &str = "org.freedesktop.Avahi";
/// Avahi's server interface.
const AVAHI_SERVER: &str = "org.freedesktop.Avahi.Server";

/// One entry of the network browser.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AdvertisedEntry {
    /// Where the entry leads, for example `smb://nas/`.
    target_uri: Option<String>,
    /// The name the service advertises.
    display_name: String,
    /// The browser's own name of the entry, which for DNS-SD services is
    /// `dnssd-server-` and the escaped service instance and type.
    name: String,
}

/// Lists the SMB, SFTP, FTP, WebDAV and NFS servers advertising on the
/// local network, giving up after 15 seconds.
///
/// Discovery may start `GVfs` services, but it never asks for a password:
/// a server that wants one is skipped.
///
/// # Errors
///
/// [`NetworkError::Cancelled`] when the deadline passes, or GIO's error
/// when the network browser cannot be mounted or nothing of it can be
/// read.
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
    let warnings = match failure {
        // Nothing could be read: the browser cannot be used at all.
        Some(error) if entries.is_empty() => return Err(error.into()),
        failure => failure
            .map(|error| crate::system_error::message_of(&error))
            .into_iter()
            .collect(),
    };
    let mut servers = servers_among(entries.iter().cloned());
    let mut resolved = 0;
    for entry in &entries {
        let Some(service) = dns_sd_service(entry) else {
            continue;
        };
        if resolved == MAX_RESOLVED {
            break;
        }
        resolved += 1;
        let found = match resolve_service(&service).await {
            Some(answer) => service.server(&answer),
            None => Some(service.unresolved()),
        };
        if let Some(found) = found {
            if !servers.iter().any(|known| known.uri == found.uri) {
                servers.push(found);
            }
        }
    }
    servers.sort_by_cached_key(|server| glib::casefold(&server.label));
    Ok(Discovery { servers, warnings })
}

/// A DNS-SD WebDAV service, which `GVfs` lists by its service name
/// (`dav+sd://`) instead of its host.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DnsSdService {
    /// The service instance, such as "Harbor Files".
    instance: String,
    /// The service type, such as `_webdav._tcp`.
    service_type: String,
    /// The domain, normally `local`.
    domain: String,
    /// The location `GVfs` gave, kept when Avahi cannot resolve it.
    uri: String,
    /// The name `GVfs` shows.
    label: String,
}

/// What Avahi answered for a service.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedService {
    /// The host name, such as `harbornas.local`.
    host: String,
    /// The port.
    port: u16,
    /// The service's TXT strings.
    txt: Vec<String>,
}

impl DnsSdService {
    /// The WebDAV server Avahi resolved this service to:
    /// `dav://host:port/path`, the path from the TXT record's `path`. A
    /// user name in the TXT record (`u=`) is not used (NET-024).
    fn server(&self, answer: &ResolvedService) -> Option<DiscoveredServer> {
        let secure = self.service_type.starts_with("_webdavs.");
        let (scheme, default_port) = if secure { ("davs", 443) } else { ("dav", 80) };
        let path = answer
            .txt
            .iter()
            .find_map(|entry| entry.strip_prefix("path="))
            .filter(|path| path.starts_with('/'))
            .unwrap_or("/");
        let authority = if answer.port == default_port {
            answer.host.clone()
        } else {
            format!("{}:{}", answer.host, answer.port)
        };
        let uri = normalise(&format!("{scheme}://{authority}{path}")).ok()?;
        let parts = split_location(&uri).ok()?;
        if parts.has_credentials() {
            return None;
        }
        Some(DiscoveredServer {
            uri,
            label: self.label.clone(),
            name: self.instance.clone(),
            host: parts.hostname()?,
        })
    }

    /// The service as `GVfs` listed it, a device of its own, when Avahi
    /// could not resolve it.
    fn unresolved(&self) -> DiscoveredServer {
        DiscoveredServer {
            uri: self.uri.clone(),
            label: self.label.clone(),
            name: self.instance.clone(),
            host: String::new(),
        }
    }
}

/// The DNS-SD WebDAV service `entry` names, if it is one.
fn dns_sd_service(entry: &AdvertisedEntry) -> Option<DnsSdService> {
    let target = entry.target_uri.as_deref()?;
    let (scheme, rest) = target.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "dav+sd" | "davs+sd") {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    let service = decode_uri_component(authority)?;
    let (instance, service_type, domain) = split_service_name(&service)?;
    let label = if entry.display_name.is_empty() {
        instance.clone()
    } else {
        entry.display_name.clone()
    };
    Some(DnsSdService {
        instance,
        service_type,
        domain,
        uri: target.to_owned(),
        label,
    })
}

/// Splits `Harbor Files._webdav._tcp.local` into the instance, the type
/// and the domain. The instance may hold dots, so the type is found from
/// the end.
fn split_service_name(service: &str) -> Option<(String, String, String)> {
    for service_type in ["._webdav._tcp.", "._webdavs._tcp."] {
        if let Some(at) = service.rfind(service_type) {
            let instance = &service[..at];
            let domain = service[at + service_type.len()..].trim_end_matches('.');
            if instance.is_empty() || domain.is_empty() {
                return None;
            }
            return Some((
                instance.to_owned(),
                service_type.trim_matches('.').to_owned(),
                domain.to_owned(),
            ));
        }
    }
    None
}

/// Asks Avahi for the host, port and TXT record of `service`.
async fn resolve_service(service: &DnsSdService) -> Option<ResolvedService> {
    let bus = gio::bus_get_future(gio::BusType::System).await.ok()?;
    // Any interface and protocol; no flags.
    let parameters = (
        -1_i32,
        -1_i32,
        service.instance.as_str(),
        service.service_type.as_str(),
        service.domain.as_str(),
        -1_i32,
        0_u32,
    )
        .to_variant();
    let reply = bus
        .call_future(
            Some(AVAHI_NAME),
            "/",
            AVAHI_SERVER,
            "ResolveService",
            Some(&parameters),
            Some(glib::VariantTy::new("(iissssisqaayu)").expect("valid type")),
            gio::DBusCallFlags::NO_AUTO_START,
            RESOLVE_TIMEOUT_MS,
        )
        .await
        .ok()?;
    resolved_reply(&reply)
}

/// Avahi's `ResolveService` reply, read.
fn resolved_reply(reply: &glib::Variant) -> Option<ResolvedService> {
    let host: String = reply.try_child_value(5)?.get()?;
    let port: u16 = reply.try_child_value(8)?.get()?;
    let txt: Vec<Vec<u8>> = reply.try_child_value(9)?.get()?;
    let txt = txt
        .into_iter()
        .filter_map(|entry| String::from_utf8(entry).ok())
        .collect();
    (!host.is_empty()).then_some(ResolvedService { host, port, txt })
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
        name: info.name().to_string_lossy().into_owned(),
    }
}

/// The service instance `GVfs`'s DNS-SD browser gives as an entry's
/// name (`dnssd-server-Harbor%20Files._sftp-ssh._tcp`), or `None` for an
/// entry of another browser.
fn advertised_instance(name: &str) -> Option<String> {
    let service = decode_uri_component(name.strip_prefix(DNS_SD_PREFIX)?)?;
    // The instance ends where the service type (`._name._tcp`) starts.
    let at = service.rfind("._")?;
    let before = service[..at].rfind("._").unwrap_or(at);
    let instance = &service[..before];
    (!instance.is_empty()).then(|| instance.to_owned())
}

/// The servers among `entries`: each server once (the latest entry wins),
/// sorted by label ignoring case.
fn servers_among(entries: impl IntoIterator<Item = AdvertisedEntry>) -> Vec<DiscoveredServer> {
    let mut servers: Vec<DiscoveredServer> = Vec::new();
    for server in entries.into_iter().filter_map(advertised_server) {
        match servers.iter_mut().find(|known| known.uri == server.uri) {
            Some(known) => *known = server,
            None => servers.push(server),
        }
    }
    servers.sort_by_cached_key(|server| glib::casefold(&server.label));
    servers
}

/// The SMB, SFTP, FTP, WebDAV or NFS server `entry` leads to (NET-025),
/// or `None` for another kind of service, such as a printer, or an
/// address that is not a valid location.
fn advertised_server(entry: AdvertisedEntry) -> Option<DiscoveredServer> {
    let target = normalise(entry.target_uri.as_deref()?).ok()?;
    let parts = split_location(&target).ok()?;
    // Privacy rule (NET-024): an advertised account is not used.
    if !(parts.is_smb() || parts.is_remote()) || parts.has_credentials() {
        return None;
    }
    let host = parts.hostname()?;
    let label = if entry.display_name.is_empty() {
        host.clone()
    } else {
        entry.display_name
    };
    let name = advertised_instance(&entry.name).unwrap_or_else(|| label.clone());
    Some(DiscoveredServer {
        uri: format!("{}://{}/", parts.scheme, parts.authority),
        label,
        name,
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
            name: String::new(),
        }
    }

    fn server(uri: &str, label: &str, host: &str) -> DiscoveredServer {
        DiscoveredServer {
            uri: uri.into(),
            label: label.into(),
            name: label.into(),
            host: host.into(),
        }
    }

    /// A WebDAV server advertised over DNS-SD (`dav+sd`, which `GVfs` lists
    /// by service name) becomes `dav://host:port/path` once Avahi resolves
    /// it, or stays as `GVfs` listed it, a device of its own; an advertised
    /// user name is never used.
    ///
    /// parity: NET-024, NET-025
    #[test]
    fn a_dns_sd_webdav_service_resolves_to_its_host() {
        let mut advertised = entry(
            "dav+sd://Harbor%20Files._webdav._tcp.local",
            "Harbor Files (dav+sd)",
        );
        advertised.name = "dnssd-server-Harbor%20Files._webdav._tcp".into();
        let service = dns_sd_service(&advertised).expect("a DNS-SD WebDAV service");
        assert_eq!(
            (
                service.instance.as_str(),
                service.service_type.as_str(),
                service.domain.as_str()
            ),
            ("Harbor Files", "_webdav._tcp", "local")
        );
        assert!(
            advertised_server(advertised).is_none(),
            "dav+sd is resolved, not listed raw"
        );

        let answer = ResolvedService {
            host: "harbornas.local".into(),
            port: 8080,
            txt: vec!["u=nasuser".into(), "path=/dav".into()],
        };
        let resolved = service.server(&answer).expect("a server");
        assert_eq!(resolved.uri, "dav://harbornas.local:8080/dav");
        assert_eq!(
            (resolved.name.as_str(), resolved.host.as_str()),
            ("Harbor Files", "harbornas.local")
        );

        let secure =
            dns_sd_service(&entry("davs+sd://Vault._webdavs._tcp.local", "Vault")).expect("a secure service");
        let at_default = ResolvedService {
            host: "vault.local".into(),
            port: 443,
            txt: Vec::new(),
        };
        assert_eq!(
            secure.server(&at_default).map(|server| server.uri).as_deref(),
            Some("davs://vault.local/")
        );

        let unresolved = service.unresolved();
        assert_eq!(unresolved.uri, "dav+sd://Harbor%20Files._webdav._tcp.local");
        assert!(unresolved.host.is_empty());
        let computers = super::super::computers::computers_among(&[unresolved]);
        assert_eq!(computers[0].name, "Harbor Files");
    }

    /// parity: NET-024, NET-025
    #[test]
    fn file_servers_are_listed_once_and_sorted_by_label() {
        let entries = [
            entry("smb://studio-nas/", "Studio NAS"),
            entry("sftp://build-host/home", "Build host"),
            entry("smb://archive-nas/Projects", "archive-nas"),
            entry("smb://STUDIO-NAS/", "Studio NAS (renamed)"),
            entry("https://printer.invalid/", "Printer"),
            entry("davs://cloud:8443/", "Cloud"),
            entry("ftp://anna@mirror/", "Mirror"),
        ];

        let servers = servers_among(entries);

        let expected = [
            server("smb://archive-nas/", "archive-nas", "archive-nas"),
            server("sftp://build-host/", "Build host", "build-host"),
            server("davs://cloud:8443/", "Cloud", "cloud"),
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
            name: String::new(),
        };
        let with_credentials = entry("smb://user@nas/", "NAS");

        assert!(servers_among([without_target, with_credentials]).is_empty());
    }
}
