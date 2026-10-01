// SPDX-License-Identifier: AGPL-3.0-only
//! Finding a running instance whose files were replaced by an upgrade, and
//! asking it to quit. Ports `Session.status`, `Session.stop` and
//! `require_current` in `v2.0.0:desktop/runtime_guard.py`.
//!
//! Safety rules of the whole module: no process-name matching, no
//! signals, no root actions and no shell. Requests go only to the exact
//! unique bus name that owns the application ID, and a running instance
//! may refuse to quit (while a file operation writes); it is then left
//! running.

use std::thread;
use std::time::{Duration, Instant};

use super::{InstanceError, RuntimeIdentity};

/// The D-Bus calls the guard makes: [`SessionBus`](super::SessionBus) in
/// the app, a scripted double in tests.
pub trait InstanceBus {
    /// The unique bus name (such as `:1.55`) that owns the application ID,
    /// or `None` if no instance runs.
    ///
    /// # Errors
    ///
    /// [`InstanceError::Bus`] if the bus could not answer.
    fn owner(&self) -> Result<Option<String>, InstanceError>;

    /// What the instance at `owner` reports through its `runtime-info`
    /// action; `None` for releases without it, or an unreadable report.
    fn reported_identity(&self, owner: &str) -> Option<RuntimeIdentity>;

    /// Activates the `quit` action of the instance at `owner`, which quits
    /// only if no file operation is writing.
    ///
    /// # Errors
    ///
    /// [`InstanceError::Bus`] if the call failed.
    fn request_quit(&self, owner: &str) -> Result<(), InstanceError>;
}

/// How long [`InstanceGuard`] waits, and how often it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopTiming {
    /// How long the instance has to quit: 6 seconds.
    pub timeout: Duration,
    /// How long a starting instance has to publish its digest: 1.5
    /// seconds. Reading it takes well under that; an instance that could
    /// not read its executable never publishes one, and a launch must not
    /// stall on it.
    pub settle_timeout: Duration,
    /// How often the bus is asked: every 80 ms.
    pub poll_interval: Duration,
}

impl Default for StopTiming {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(6),
            settle_timeout: Duration::from_millis(1500),
            poll_interval: Duration::from_millis(80),
        }
    }
}

/// Whether the launch was `openxplorer --restart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// A normal launch: only an outdated instance is replaced, and only
    /// with the user's consent.
    Normal,
    /// `--restart`: the running instance is asked to quit even when it is
    /// current.
    Restart,
}

/// What runs, compared with what is installed. `--diagnose` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceStatus {
    /// The installed build.
    pub installed: RuntimeIdentity,
    /// The unique bus name of the running instance.
    pub owner: Option<String>,
    /// What the running instance reports; `None` if nothing runs or it
    /// predates `runtime-info`.
    pub running: Option<RuntimeIdentity>,
}

impl InstanceStatus {
    /// Whether the running instance is the installed build; `None` if
    /// nothing runs. Ports `same_build`: version, protocol and build must
    /// all be equal.
    pub fn matches(&self) -> Option<bool> {
        self.owner.as_ref()?;
        Some(self.running.as_ref() == Some(&self.installed))
    }

    /// Whether an instance runs that reports no identity: a release from
    /// before `runtime-info`.
    pub fn is_legacy_process(&self) -> bool {
        self.owner.is_some() && self.running.is_none()
    }
}

/// The running-instance guard on a bus. Ports `Session` in
/// `v2.0.0:desktop/runtime_guard.py`.
#[derive(Debug)]
pub struct InstanceGuard<B> {
    bus: B,
    timing: StopTiming,
}

impl<B: InstanceBus> InstanceGuard<B> {
    /// A guard that waits [`StopTiming::default`] for an instance to quit.
    pub fn new(bus: B) -> Self {
        Self::with_timing(bus, StopTiming::default())
    }

    /// A guard with its own timing, for tests.
    pub fn with_timing(bus: B, timing: StopTiming) -> Self {
        Self { bus, timing }
    }

    /// The bus this guard asks.
    pub fn bus(&self) -> &B {
        &self.bus
    }

    /// What runs, compared with `installed`.
    ///
    /// # Errors
    ///
    /// [`InstanceError::Bus`] if the owner could not be asked for.
    pub fn status(&self, installed: &RuntimeIdentity) -> Result<InstanceStatus, InstanceError> {
        let owner = self.bus.owner()?;
        let running = owner
            .as_deref()
            .and_then(|owner| self.bus.reported_identity(owner));
        Ok(InstanceStatus {
            installed: installed.clone(),
            owner,
            running,
        })
    }

    /// What runs, once the running instance has read its own identity.
    ///
    /// An instance publishes its identity without a digest until it has
    /// read its executable, which takes a moment after it starts; a launch
    /// in that moment would otherwise take it for an outdated build and
    /// ask to restart it. The guard waits up to
    /// [`StopTiming::settle_timeout`]
    /// for the digest.
    fn settled_status(&self, installed: &RuntimeIdentity) -> Result<InstanceStatus, InstanceError> {
        let deadline = Instant::now() + self.timing.settle_timeout;
        loop {
            let status = self.status(installed)?;
            let is_pending = status
                .running
                .as_ref()
                .is_some_and(|running| running.build.is_empty());
            if !is_pending || Instant::now() >= deadline {
                return Ok(status);
            }
            thread::sleep(self.timing.poll_interval);
        }
    }

    /// Asks the instance at `owner` to quit and waits for it to release
    /// the application name. It is never forced.
    ///
    /// # Errors
    ///
    /// [`InstanceError::StillRunning`] if it still runs after the timeout,
    /// [`InstanceError::AnotherInstance`] if another instance took the
    /// name, and [`InstanceError::Bus`] if the quit request failed while
    /// the instance still owns the name.
    pub fn stop(&self, owner: &str) -> Result<(), InstanceError> {
        if let Err(error) = self.bus.request_quit(owner) {
            // A failed call from an instance that already quit is fine.
            if self.bus.owner()?.as_deref() == Some(owner) {
                return Err(error);
            }
        }
        let deadline = Instant::now() + self.timing.timeout;
        loop {
            let Some(present) = self.bus.owner()? else {
                return Ok(());
            };
            if present != owner {
                return Err(InstanceError::AnotherInstance);
            }
            if Instant::now() >= deadline {
                return Err(InstanceError::StillRunning);
            }
            thread::sleep(self.timing.poll_interval);
        }
    }

    /// Returns only when a launch may go on: nothing runs, or the current
    /// build runs and this is a normal launch, or the running instance
    /// agreed to quit.
    ///
    /// `confirm` asks the user whether to restart an outdated instance. It
    /// is supplied by the interface; `None` in service mode, which never
    /// asks at login.
    ///
    /// # Errors
    ///
    /// [`InstanceError::OutdatedInstance`] if the user declined or could
    /// not be asked, and everything [`InstanceGuard::stop`] returns.
    pub fn require_current(
        &self,
        installed: &RuntimeIdentity,
        mode: LaunchMode,
        confirm: Option<&dyn Fn(&InstanceStatus) -> bool>,
    ) -> Result<InstanceStatus, InstanceError> {
        let status = self.settled_status(installed)?;
        let Some(owner) = status.owner.as_deref() else {
            return Ok(status);
        };
        if mode == LaunchMode::Normal {
            if status.matches() == Some(true) {
                return Ok(status);
            }
            let agreed = confirm.is_some_and(|confirm| confirm(&status));
            if !agreed {
                return Err(InstanceError::OutdatedInstance);
            }
        }
        self.stop(owner)?;
        Ok(status)
    }
}
