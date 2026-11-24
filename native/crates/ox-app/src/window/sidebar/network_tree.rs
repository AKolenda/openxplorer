// SPDX-License-Identifier: AGPL-3.0-only
//! What the navigation pane lists under Network and under This PC's
//! network locations, as Windows 11's navigation pane does.
//!
//! Windows expands Network to the computers on the network, each with a
//! chevron of its own that expands it to its shares; the drives mapped to
//! a letter and the network locations the user added are under This PC.
//! [`network_tree`] composes that without GTK, so it is tested on its own:
//!
//! - Under This PC: every saved network location and every share mapped
//!   to a letter for this session ("Projects (Z:)"), as before under
//!   Network.
//! - Under Network: the computers discovery found, then the servers of the
//!   other network locations (mounted or browsed this session) that no
//!   discovered computer stands for, each once, by host.
//! - Inside an expanded computer: its shares as its listing gave them (no
//!   `$` share unless hidden items are shown), its other services, and the
//!   mounted or browsed locations on it that the listing did not give,
//!   which keep their own menu (Disconnect, Sign out).

use gtk::glib;
use ox_core::location::{is_smb_server, location_kind, same_location, split_location, LocationKind};
use ox_core::network::{computer_key_of, DiscoveredComputer};
use ox_core::places::NetworkLocation;

use crate::icons::Art;

/// One share or service a computer's listing gave, as the navigation pane
/// lists it inside the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct ListedShare {
    /// Its name.
    pub name: String,
    /// The location it opens.
    pub uri: String,
    /// A share Windows hides: its name ends in `$`.
    pub is_hidden: bool,
    /// A service of the computer ("Harbor Files (SFTP)"), not a share.
    pub is_service: bool,
}

/// A computer under Network in the navigation pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct ComputerNode {
    /// The computer's name ("HARBORNAS").
    pub name: String,
    /// What it opens: `\\host`, or for a server known only by another
    /// protocol, that server.
    pub uri: String,
    /// The host it is grouped by.
    pub key: String,
    /// Whether its chevron shows its shares.
    pub expanded: bool,
    /// What it lists while expanded.
    pub children: Vec<ComputerChild>,
}

/// A row inside an expanded computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct ComputerChild {
    /// The visible name.
    pub label: String,
    /// The location it opens.
    pub uri: String,
    /// Its icon.
    pub icon: Art,
    /// The network location it is, for its menu (Disconnect, Sign out);
    /// `None` for a share only the listing gave.
    pub location: Option<NetworkLocation>,
}

/// What [`network_tree`] reads.
#[derive(Debug, Clone, Copy)]
pub(in crate::window) struct NetworkTreeSources<'a> {
    /// The computers discovered this session.
    pub computers: &'a [DiscoveredComputer],
    /// Every network location, as places composes them.
    pub locations: &'a [NetworkLocation],
    /// The keys of the computers expanded.
    pub expanded: &'a [String],
    /// Each expanded computer's listing, by key, once it arrived.
    pub listed: &'a [(String, Vec<ListedShare>)],
    /// Whether hidden items (`$` shares) are shown.
    pub show_hidden: bool,
}

/// Whether `location` is listed under This PC: a saved network location,
/// or a share mapped to a drive letter, as Windows lists mapped drives.
pub(in crate::window) fn is_this_pc_location(location: &NetworkLocation) -> bool {
    location.is_saved || names_a_drive_letter(&location.label)
}

/// Whether `label` ends in a drive letter in brackets, such as "(Z:)".
fn names_a_drive_letter(label: &str) -> bool {
    let Some(inside) = label.trim_end().strip_suffix(":)") else {
        return false;
    };
    let mut rest = inside.chars().rev();
    matches!(
        (rest.next(), rest.next()),
        (Some(letter), Some('(')) if letter.is_ascii_alphabetic()
    )
}

/// The computers the navigation pane lists under Network; see the module
/// documentation.
pub(in crate::window) fn network_tree(sources: NetworkTreeSources<'_>) -> Vec<ComputerNode> {
    let mut nodes: Vec<ComputerNode> = sources
        .computers
        .iter()
        .map(|computer| ComputerNode {
            name: computer.name.clone(),
            uri: computer.uri.clone(),
            key: computer.key.clone(),
            expanded: false,
            children: Vec::new(),
        })
        .collect();
    let others: Vec<&NetworkLocation> = sources
        .locations
        .iter()
        .filter(|location| !is_this_pc_location(location))
        .collect();
    for location in &others {
        let Some(key) = computer_key_of(&location.uri) else {
            continue;
        };
        if nodes.iter().any(|node| node.key == key) {
            continue;
        }
        nodes.push(ComputerNode {
            name: host_name(&location.uri),
            uri: server_of(&location.uri),
            key,
            expanded: false,
            children: Vec::new(),
        });
    }
    for node in &mut nodes {
        node.expanded = sources.expanded.contains(&node.key);
        if node.expanded {
            node.children = children_of(node, &others, sources);
        }
    }
    nodes
}

/// The rows inside the expanded computer `node`.
fn children_of(
    node: &ComputerNode,
    others: &[&NetworkLocation],
    sources: NetworkTreeSources<'_>,
) -> Vec<ComputerChild> {
    let listed = sources
        .listed
        .iter()
        .find(|(key, _)| *key == node.key)
        .map(|(_, shares)| shares.as_slice())
        .unwrap_or_default();
    // Shares first, by name, then the computer's other services.
    let mut listed: Vec<&ListedShare> = listed
        .iter()
        .filter(|share| sources.show_hidden || !share.is_hidden)
        .collect();
    listed.sort_by_cached_key(|share| (share.is_service, glib::casefold(&share.name)));
    let mut children: Vec<ComputerChild> = listed
        .into_iter()
        .map(|share| {
            let location = others
                .iter()
                .find(|location| is_same_place(&location.uri, &share.uri))
                .map(|location| (*location).clone());
            let icon = match (&location, share.is_service) {
                (Some(location), _) => Art::for_network_row(location),
                (None, true) => Art::SERVER,
                (None, false) => Art::SHARE,
            };
            ComputerChild {
                label: share.name.clone(),
                uri: share.uri.clone(),
                icon,
                location,
            }
        })
        .collect();
    // The locations on it the listing did not give: a share mounted or
    // browsed before the listing arrived, or an SFTP folder.
    for location in others {
        let on_node = computer_key_of(&location.uri).as_deref() == Some(node.key.as_str());
        let listed_already = children
            .iter()
            .any(|child| is_same_place(&child.uri, &location.uri));
        if !on_node
            || listed_already
            || is_smb_server(&location.uri)
            || same_location(&location.uri, &node.uri)
        {
            continue;
        }
        children.push(ComputerChild {
            label: location.label.clone(),
            uri: location.uri.clone(),
            icon: Art::for_network_row(location),
            location: Some((*location).clone()),
        });
    }
    children
}

/// Whether `a` and `b` are one place, as SMB compares names, ignoring
/// case: a share listed as "Projects" and mounted as `projects` is one.
fn is_same_place(a: &str, b: &str) -> bool {
    match (ox_core::places::network_key(a), ox_core::places::network_key(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => same_location(a, b),
    }
}

/// The host of `uri` without an account or `.local`, as a computer is
/// named when discovery did not find it.
fn host_name(uri: &str) -> String {
    let Some(host) = split_location(uri).ok().and_then(|parts| parts.hostname()) else {
        return uri.to_owned();
    };
    host.strip_suffix(".local").unwrap_or(&host).to_owned()
}

/// The server `uri` is on: `\\host` for SMB, else the root of the other
/// protocol's server, which is what a computer known only by it opens.
fn server_of(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return uri.to_owned();
    };
    match location_kind(uri) {
        LocationKind::Smb => format!("smb://{}/", parts.authority),
        _ => format!("{}://{}/", parts.scheme, parts.authority),
    }
}
