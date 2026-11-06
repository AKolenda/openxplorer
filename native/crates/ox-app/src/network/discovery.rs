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
//! Discovery never asks for a password and never lists a server's shares
//! (NET-024); both rules are enforced in ox-core.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use ox_core::network::{discover_servers, DiscoveredServer, Discovery, NetworkError};

/// How many times one listing of Network reads the network browser.
const PASSES: usize = 3;
/// The pause between two passes.
const PAUSE_BETWEEN_PASSES: Duration = Duration::from_millis(2500);

/// One reading of the network browser.
type DiscoveryPass = Pin<Box<dyn Future<Output = Result<Discovery, NetworkError>>>>;

/// Where discovery looks: `GVfs`'s network browser, or, in tests, a fixed
/// answer, so no test sends discovery traffic onto a network.
#[derive(Clone)]
pub(crate) struct Discoverer {
    /// Starts one pass.
    pass: Rc<dyn Fn() -> DiscoveryPass>,
    /// The pause between two passes.
    pause: Duration,
}

impl Discoverer {
    /// Reads `GVfs`'s network browser.
    fn network_browser() -> Self {
        Self {
            pass: Rc::new(|| Box::pin(discover_servers())),
            pause: PAUSE_BETWEEN_PASSES,
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
        }
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

/// Reads the network browser through `discoverer` up to [`PASSES`] times,
/// calling `on_found` with the servers each pass finds that no earlier
/// pass did, in the order found.
///
/// # Errors
///
/// Why discovery cannot run, in GIO's words, when the first pass fails:
/// the network browser cannot be mounted or read. A pass that gives up
/// at its deadline, or a later pass that fails, ends discovery with what
/// it found. Warnings about a browser read part-way are only logged.
pub(crate) async fn discover_computers(
    discoverer: &Discoverer,
    on_found: impl Fn(Vec<DiscoveredServer>),
) -> Result<(), String> {
    let mut listed: Vec<String> = Vec::new();
    for pass in 1..=PASSES {
        let discovery = match (discoverer.pass)().await {
            Ok(discovery) => discovery,
            Err(error) if pass == 1 && !error.is_cancelled() => return Err(error.to_string()),
            Err(_) => return Ok(()),
        };
        for warning in &discovery.warnings {
            glib::g_debug!(ox_core::LOG_DOMAIN, "Network discovery: {warning}");
        }
        let found: Vec<DiscoveredServer> = discovery
            .servers
            .into_iter()
            .filter(|server| !listed.contains(&server.uri))
            .collect();
        listed.extend(found.iter().map(|server| server.uri.clone()));
        if !found.is_empty() {
            on_found(found);
        }
        if pass < PASSES {
            glib::timeout_future(discoverer.pause).await;
        }
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
            host: label.to_lowercase(),
        }
    }

    /// Runs discovery through `discoverer` to its end: the batches it
    /// listed, and its outcome.
    fn run(discoverer: &Discoverer) -> (Vec<Vec<String>>, Result<(), String>) {
        let batches = RefCell::new(Vec::new());
        let outcome = glib::MainContext::default().block_on(discover_computers(discoverer, |found| {
            let labels = found.into_iter().map(|server| server.label).collect();
            batches.borrow_mut().push(labels);
        }));
        (batches.into_inner(), outcome)
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
