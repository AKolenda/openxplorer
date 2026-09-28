// SPDX-License-Identifier: AGPL-3.0-only
//! Discover servers, on the Network page.
//!
//! Ports `discoverNetwork` and `cancelDiscovery` in `desktop/ui/app.js`.
//! ox-core's [`discover_servers`] reads `GVfs`'s network browser once;
//! advertisements arrive over a few seconds, so the page reads it up to
//! three times, 2.5 seconds apart, and merges the servers by address. The
//! results belong to the window and last for its session. Stop drops the
//! passes still to come, and results that arrive after it are ignored.
//!
//! Discovery never asks for a password and never lists a server's shares
//! (NET-024); both rules are enforced in ox-core.

use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use ox_core::network::{discover_servers, DiscoveredServer, Discovery, NetworkError};

/// How many times one Discover servers reads the network browser.
const PASSES: usize = 3;
/// The pause between two passes.
const PAUSE_BETWEEN_PASSES: Duration = Duration::from_millis(2500);

/// The banner's hint while discovery runs.
const LISTENING: &str = "Listening for advertised SMB servers…";
/// The banner's hint otherwise.
const IDLE_HINT: &str = "Discover devices without scanning their files.";
/// The notice while discovery runs and has found nothing yet.
const STILL_LOOKING: &str = "Discovery can take a few seconds.";
/// The notice when discovery found nothing.
const NOTHING_FOUND: &str = "No advertised SMB servers found yet. Discover again or enter an address above.";

/// One reading of the network browser.
type DiscoveryPass = Pin<Box<dyn Future<Output = Result<Discovery, NetworkError>>>>;

/// Where Discover servers looks: `GVfs`'s network browser, or, in tests, a
/// fixed answer, so no test sends discovery traffic onto a network.
#[derive(Clone)]
pub(crate) struct Discoverer(Rc<dyn Fn() -> DiscoveryPass>);

impl Discoverer {
    /// Reads `GVfs`'s network browser.
    fn network_browser() -> Self {
        Self(Rc::new(|| Box::pin(discover_servers())))
    }

    /// Answers every pass with `discovery`.
    pub(crate) fn finding(discovery: Discovery) -> Self {
        Self(Rc::new(move || {
            let discovery = discovery.clone();
            Box::pin(async move { Ok(discovery) })
        }))
    }

    /// Starts one pass.
    fn pass(&self) -> DiscoveryPass {
        (self.0)()
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

/// Where discovery stands, as the Network page shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DiscoveryState {
    /// Discovery ran in this window; the first visit of the Network page
    /// starts it.
    pub has_started: bool,
    /// A pass runs, or another follows.
    pub is_busy: bool,
    /// The servers found, each once, in the order first found.
    pub servers: Vec<DiscoveredServer>,
    /// The last pass's warnings, one per line, or why discovery failed;
    /// empty when there were none.
    pub problem: String,
}

impl DiscoveryState {
    /// The line under the banner's title.
    pub(crate) fn banner_hint(&self) -> &'static str {
        if self.is_busy {
            LISTENING
        } else {
            IDLE_HINT
        }
    }

    /// The notice shown while no server is listed.
    pub(crate) fn empty_notice(&self) -> &str {
        if self.is_busy {
            STILL_LOOKING
        } else if self.problem.is_empty() {
            NOTHING_FOUND
        } else {
            &self.problem
        }
    }

    /// A new run starts with nothing found.
    fn begin(&mut self) {
        self.has_started = true;
        self.is_busy = true;
        self.servers.clear();
        self.problem.clear();
    }

    /// Adds the servers of one pass: a server found again replaces the
    /// earlier row in its place (`new Map(...)` in app.js).
    fn merge(&mut self, discovery: Discovery) {
        for server in discovery.servers {
            let known = self.servers.iter().position(|known| known.uri == server.uri);
            match known {
                Some(index) => self.servers[index] = server,
                None => self.servers.push(server),
            }
        }
        self.problem = discovery.warnings.join("\n");
    }

    /// Records why a pass failed; a cancelled or timed-out pass is not a
    /// problem to show.
    fn fail(&mut self, error: &NetworkError) {
        if !error.is_cancelled() {
            self.problem = error.to_string();
        }
    }
}

/// A window's server discovery: what it found and the passes running.
#[derive(Debug, Default)]
pub(crate) struct ServerDiscovery {
    /// What the Network page shows.
    state: RefCell<DiscoveryState>,
    /// The passes of the current run; aborting it is Stop.
    running: RefCell<Option<glib::JoinHandle<()>>>,
}

impl ServerDiscovery {
    /// What the Network page shows now.
    pub(crate) fn state(&self) -> DiscoveryState {
        self.state.borrow().clone()
    }

    /// Discover servers: runs the passes through `discoverer` unless they
    /// run already, calling `on_change` whenever the page changes.
    pub(crate) fn start(self: &Rc<Self>, discoverer: Discoverer, on_change: impl Fn() + 'static) {
        if self.state.borrow().is_busy {
            return;
        }
        self.state.borrow_mut().begin();
        let on_change: Rc<dyn Fn()> = Rc::new(on_change);
        let passes = run_passes(Rc::downgrade(self), discoverer, on_change);
        let running = glib::spawn_future_local(passes);
        self.running.replace(Some(running));
    }

    /// Stop: drops the passes still to come; what they would find is
    /// ignored.
    pub(crate) fn stop(&self) {
        if let Some(running) = self.running.take() {
            running.abort();
        }
        self.state.borrow_mut().is_busy = false;
    }
}

impl Drop for ServerDiscovery {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Reads the network browser [`PASSES`] times, merging each pass into
/// `discovery` while it lives, and ends the run.
async fn run_passes(discovery: Weak<ServerDiscovery>, discoverer: Discoverer, on_change: Rc<dyn Fn()>) {
    for pass in 1..=PASSES {
        let found = discoverer.pass().await;
        let Some(owner) = discovery.upgrade() else {
            return;
        };
        let failed = found.is_err();
        match found {
            Ok(found) => owner.state.borrow_mut().merge(found),
            Err(error) => owner.state.borrow_mut().fail(&error),
        }
        drop(owner);
        if failed {
            break;
        }
        on_change();
        if pass < PASSES {
            glib::timeout_future(PAUSE_BETWEEN_PASSES).await;
        }
    }
    let Some(owner) = discovery.upgrade() else {
        return;
    };
    owner.state.borrow_mut().is_busy = false;
    // The run is over; its handle has nothing left to abort.
    owner.running.take();
    on_change();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::test_support::harness::wait_until;

    fn server(uri: &str, label: &str) -> DiscoveredServer {
        DiscoveredServer {
            uri: uri.into(),
            label: label.into(),
            host: label.to_lowercase(),
        }
    }

    /// A server found again replaces its row in place; the warnings of
    /// the latest pass replace the earlier ones.
    ///
    /// parity: HOME-007
    #[test]
    fn passes_merge_servers_by_address_in_the_order_first_found() {
        let mut state = DiscoveryState::default();
        state.begin();

        state.merge(Discovery {
            servers: vec![server("smb://nas/", "NAS"), server("smb://studio/", "Studio")],
            warnings: vec!["Browsing stopped early.".into()],
        });
        state.merge(Discovery {
            servers: vec![
                server("smb://printer/", "Printer"),
                server("smb://nas/", "NAS (renamed)"),
            ],
            warnings: Vec::new(),
        });

        let labels: Vec<&str> = state.servers.iter().map(|found| found.label.as_str()).collect();
        assert_eq!(labels, ["NAS (renamed)", "Studio", "Printer"]);
        assert_eq!(state.problem, "");
    }

    /// parity: HOME-006, HOME-008
    #[test]
    fn the_page_says_what_discovery_is_doing() {
        let mut state = DiscoveryState::default();
        assert_eq!(
            state.banner_hint(),
            "Discover devices without scanning their files."
        );
        assert_eq!(state.empty_notice(), NOTHING_FOUND);

        state.begin();
        assert_eq!(state.banner_hint(), "Listening for advertised SMB servers…");
        assert_eq!(state.empty_notice(), "Discovery can take a few seconds.");

        state.fail(&NetworkError::NotAFolder);
        state.is_busy = false;
        assert_eq!(state.empty_notice(), "This location is not a folder.");
    }

    #[test]
    fn a_cancelled_pass_shows_no_problem() {
        let mut state = DiscoveryState::default();
        state.fail(&NetworkError::Cancelled);
        assert_eq!(state.problem, "");
    }

    /// The first pass shows its servers at once; Stop ends the run, and
    /// the passes still to come find nothing more.
    ///
    /// parity: HOME-006, HOME-007
    #[gtk::test]
    fn stop_ends_the_passes_and_keeps_what_was_found() {
        let discovery = Rc::new(ServerDiscovery::default());
        let changes = Rc::new(Cell::new(0));
        let counted = Rc::clone(&changes);
        let found = Discovery {
            servers: vec![server("smb://nas/", "NAS")],
            warnings: Vec::new(),
        };

        discovery.start(Discoverer::finding(found), move || counted.set(counted.get() + 1));
        wait_until("the first pass", || changes.get() == 1);
        assert!(discovery.state().is_busy, "two more passes follow");
        discovery.stop();

        let state = discovery.state();
        assert!(!state.is_busy);
        assert!(state.has_started);
        assert_eq!(state.servers, [server("smb://nas/", "NAS")]);
        assert!(discovery.running.borrow().is_none());
    }
}
