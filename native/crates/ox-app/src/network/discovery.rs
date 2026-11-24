// SPDX-License-Identifier: AGPL-3.0-only
//! Finding the computers that Network lists.
//!
//! Ports `discoverNetwork` and `cancelDiscovery` in `v2.0.0:desktop/ui/app.js`.
//! ox-core's [`discover_servers`] reads `GVfs`'s network browser once;
//! advertisements arrive over a few seconds, so every listing of Network
//! reads it up to three times, 2.5 seconds apart, and lists each server
//! the first time a pass finds it. Network is listed like a folder
//! (`crate::window`): its listing runs these passes, so going there,
//! Refresh (F5) and Stop discover, as in Windows Explorer, and results
//! that arrive after Stop are dropped with the listing.
//!
//! Network lists devices, as Windows 11 does: the services each pass
//! finds are merged into one computer per host
//! ([`computers_among`](ox_core::network::computers_among)), and a
//! computer is listed the first time a pass finds any of its services.
//! Before the passes, the discovery sources are checked: without `GVfs`'s
//! network browser, or with neither Avahi nor wsdd, Network says that
//! discovery is off, as Windows' info bar does, rather than listing
//! nothing.
//!
//! Discovery never asks for a password and never lists a server's shares
//! (NET-024); both rules are enforced in ox-core.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use ox_core::network::{
    computers_among, discover_servers, discovery_sources, DiscoveredComputer, DiscoveredServer, Discovery,
    DiscoverySources, NetworkError,
};

/// How many times one listing of Network reads the network browser.
const PASSES: usize = 3;
/// The pause between two passes.
const PAUSE_BETWEEN_PASSES: Duration = Duration::from_millis(2500);

/// One reading of the network browser.
type DiscoveryPass = Pin<Box<dyn Future<Output = Result<Discovery, NetworkError>>>>;
/// One check of the discovery sources.
type SourcesCheck = Pin<Box<dyn Future<Output = DiscoverySources>>>;

/// Where discovery looks: `GVfs`'s network browser, or, in tests, a fixed
/// answer, so no test sends discovery traffic onto a network.
#[derive(Clone)]
pub(crate) struct Discoverer {
    /// Starts one pass.
    pass: Rc<dyn Fn() -> DiscoveryPass>,
    /// The pause between two passes.
    pause: Duration,
    /// Checks which discovery sources there are.
    sources: Rc<dyn Fn() -> SourcesCheck>,
}

/// Every source there, at once, as tests find them unless they say
/// otherwise.
fn every_source() -> Rc<dyn Fn() -> SourcesCheck> {
    Rc::new(|| Box::pin(async { DiscoverySources::ALL }))
}

impl Discoverer {
    /// Reads `GVfs`'s network browser.
    fn network_browser() -> Self {
        Self {
            pass: Rc::new(|| Box::pin(discover_servers())),
            pause: PAUSE_BETWEEN_PASSES,
            sources: Rc::new(|| Box::pin(discovery_sources())),
        }
    }

    /// Answers every pass with `discovery`, at once.
    pub(crate) fn finding(discovery: Discovery) -> Self {
        Self {
            pass: Rc::new(move || {
                let discovery = discovery.clone();
                Box::pin(async move { Ok(discovery) })
            }),
            pause: Duration::ZERO,
            sources: every_source(),
        }
    }

    /// The same discoverer, finding `sources` when it checks them, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn with_sources(mut self, sources: DiscoverySources) -> Self {
        self.sources = Rc::new(move || Box::pin(async move { sources }));
        self
    }

    /// Answers every pass with `discovery` after `delay`, for tests.
    #[cfg(test)]
    pub(crate) fn finding_after(delay: Duration, discovery: Discovery) -> Self {
        Self {
            pass: Rc::new(move || {
                let discovery = discovery.clone();
                Box::pin(async move {
                    glib::timeout_future(delay).await;
                    Ok(discovery)
                })
            }),
            pause: Duration::ZERO,
            sources: every_source(),
        }
    }

    /// Answers every pass with what `answer` returns, at once, for tests.
    #[cfg(test)]
    pub(crate) fn answering(answer: impl Fn() -> Result<Discovery, NetworkError> + 'static) -> Self {
        let answer = Rc::new(answer);
        Self {
            pass: Rc::new(move || {
                let answer = Rc::clone(&answer);
                Box::pin(async move { answer() })
            }),
            pause: Duration::ZERO,
            sources: every_source(),
        }
    }
}

impl Default for Discoverer {
    /// The network browser. Test safety: tests find an empty network
    /// unless they choose what it holds.
    fn default() -> Self {
        if cfg!(test) {
            Self::finding(Discovery::default())
        } else {
            Self::network_browser()
        }
    }
}

impl fmt::Debug for Discoverer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Discoverer")
    }
}

/// Why Network could not discover computers: the info bar's tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryProblem {
    /// The network browser could not be read, in GIO's words.
    Browser(String),
    /// Discovery has nothing to read: no DNS-SD (Avahi) and no
    /// WS-Discovery (wsdd).
    Off(DiscoverySources),
}

impl DiscoveryProblem {
    /// What the info bar's tooltip says.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Browser(reason) => reason.clone(),
            Self::Off(sources) if !sources.browser => {
                ox_core::i18n::gettext("The GVfs network browser is not installed.")
            }
            Self::Off(_) => ox_core::i18n::gettext(
                "Avahi (DNS-SD) is not running and wsdd (WS-Discovery) is not installed.",
            ),
        }
    }
}

/// Checks the discovery sources, then reads the network browser through
/// `discoverer` up to [`PASSES`] times, calling `on_found` with the
/// computers each pass finds that no earlier pass did, in the order found,
/// and `on_pass` with every computer found so far after each pass.
///
/// # Errors
///
/// Why discovery cannot run: the network browser is missing, or the first
/// pass fails (it cannot be mounted or read), in GIO's words; or no pass
/// found anything while neither DNS-SD nor WS-Discovery is there. A pass
/// that gives up at its deadline, or a later pass that fails, ends
/// discovery with what it found. Warnings about a browser read part-way
/// are only logged.
pub(crate) async fn discover_computers(
    discoverer: &Discoverer,
    on_found: impl Fn(Vec<DiscoveredComputer>),
    on_pass: impl Fn(&[DiscoveredComputer]),
) -> Result<(), DiscoveryProblem> {
    let sources = (discoverer.sources)().await;
    if !sources.browser {
        return Err(DiscoveryProblem::Off(sources));
    }
    let mut servers: Vec<DiscoveredServer> = Vec::new();
    let mut listed: Vec<String> = Vec::new();
    for pass in 1..=PASSES {
        let discovery = match (discoverer.pass)().await {
            Ok(discovery) => discovery,
            Err(error) if pass == 1 && !error.is_cancelled() => {
                return Err(DiscoveryProblem::Browser(error.to_string()))
            }
            Err(_) => break,
        };
        for warning in &discovery.warnings {
            glib::g_debug!(ox_core::LOG_DOMAIN, "Network discovery: {warning}");
        }
        for server in discovery.servers {
            if !servers.iter().any(|known| known.uri == server.uri) {
                servers.push(server);
            }
        }
        let computers = computers_among(&servers);
        let found: Vec<DiscoveredComputer> = computers
            .iter()
            .filter(|computer| !listed.contains(&computer.key))
            .cloned()
            .collect();
        listed.extend(found.iter().map(|computer| computer.key.clone()));
        if !found.is_empty() {
            on_found(found);
        }
        on_pass(&computers);
        if pass < PASSES {
            glib::timeout_future(discoverer.pause).await;
        }
    }
    if listed.is_empty() && !sources.can_discover() {
        return Err(DiscoveryProblem::Off(sources));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    fn server(uri: &str, label: &str) -> DiscoveredServer {
        DiscoveredServer {
            uri: uri.into(),
            label: label.into(),
            name: label.into(),
            host: label.to_lowercase(),
        }
    }

    /// Runs discovery through `discoverer` to its end: the batches it
    /// listed, and its outcome as the info bar's tooltip.
    fn run(discoverer: &Discoverer) -> (Vec<Vec<String>>, Result<(), String>) {
        let batches = RefCell::new(Vec::new());
        let outcome = glib::MainContext::default().block_on(discover_computers(
            discoverer,
            |found| {
                let names = found.into_iter().map(|computer| computer.name).collect();
                batches.borrow_mut().push(names);
            },
            |_| {},
        ));
        (batches.into_inner(), outcome.map_err(|problem| problem.reason()))
    }

    /// Services of one host found in different passes make one computer,
    /// listed when the first of them is found.
    ///
    /// parity: HOME-007, NET-025
    #[gtk::test]
    fn the_services_of_one_host_are_one_computer_across_passes() {
        let passes = Cell::new(0);
        let discoverer = Discoverer::answering(move || {
            passes.set(passes.get() + 1);
            let mut found = vec![DiscoveredServer {
                uri: "sftp://harbornas.local:2222/".into(),
                label: "Harbor Files (Remote Login)".into(),
                name: "Harbor Files".into(),
                host: "harbornas.local".into(),
            }];
            if passes.get() > 1 {
                found.push(DiscoveredServer {
                    uri: "smb://harbornas.local:445/".into(),
                    label: "HARBORNAS".into(),
                    name: "HARBORNAS".into(),
                    host: "harbornas.local".into(),
                });
            }
            Ok(Discovery {
                servers: found,
                warnings: Vec::new(),
            })
        });
        let last = RefCell::new(Vec::new());

        let batches = RefCell::new(Vec::new());
        let outcome = glib::MainContext::default().block_on(discover_computers(
            &discoverer,
            |found| batches.borrow_mut().push(found.len()),
            |computers| {
                last.replace(computers.to_vec());
            },
        ));

        assert_eq!(outcome, Ok(()));
        assert_eq!(*batches.borrow(), [1]);
        let last = last.borrow();
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].name, "HARBORNAS");
        assert!(last[0].has_smb);
    }

    /// Each pass lists only the servers no earlier pass found, so Network
    /// lists every server once, in the order first found.
    ///
    /// parity: HOME-007
    #[gtk::test]
    fn passes_list_each_server_once_in_the_order_first_found() {
        let passes = Cell::new(0);
        let discoverer = Discoverer::answering(move || {
            passes.set(passes.get() + 1);
            let servers = match passes.get() {
                1 => vec![server("smb://nas/", "NAS"), server("smb://studio/", "Studio")],
                2 => vec![server("smb://printer/", "Printer"), server("smb://nas/", "NAS")],
                _ => vec![server("smb://studio/", "Studio")],
            };
            Ok(Discovery {
                servers,
                warnings: vec!["Browsing stopped early.".into()],
            })
        });

        let (batches, outcome) = run(&discoverer);

        assert_eq!(batches, [vec!["NAS", "Studio"], vec!["Printer"]]);
        assert_eq!(outcome, Ok(()));
    }

    /// A first pass that fails says why discovery cannot run; one that
    /// gives up at its deadline does not, and neither does a later pass
    /// that fails after servers were found.
    ///
    /// parity: HOME-007
    #[gtk::test]
    fn only_a_first_pass_that_fails_says_discovery_cannot_run() {
        let unsupported = Discoverer::answering(|| Err(NetworkError::NotAFolder));
        assert_eq!(
            run(&unsupported),
            (Vec::new(), Err("This location is not a folder.".into()))
        );

        let timed_out = Discoverer::answering(|| Err(NetworkError::Cancelled));
        assert_eq!(run(&timed_out), (Vec::new(), Ok(())));

        let passes = Cell::new(0);
        let failing_later = Discoverer::answering(move || {
            passes.set(passes.get() + 1);
            if passes.get() == 1 {
                Ok(Discovery {
                    servers: vec![server("smb://nas/", "NAS")],
                    warnings: Vec::new(),
                })
            } else {
                Err(NetworkError::NotAFolder)
            }
        });
        assert_eq!(run(&failing_later), (vec![vec!["NAS".to_owned()]], Ok(())));
    }
}
