// SPDX-License-Identifier: AGPL-3.0-only
//! Moving a tab to another window without ever losing it: the broker half
//! of TAB-036.
//!
//! Ports `TabTransfers` in `v2.0.0:desktop/tab_transfers.py`. A move has two
//! phases: the destination window receives the tab and inserts it
//! tentatively, and only its acknowledgement retires the source tab. Every
//! other ending keeps the source tab, tells the destination to remove its
//! tentative copy, and discards the capability: a cancellation, a refusal,
//! a busy or closed window, a timeout after [`TAB_TRANSFER_LIFETIME`], or
//! an acknowledgement from the wrong window.
//!
//! A move is addressed by a [`TabTransferToken`]: 64 random hexadecimal
//! digits, single use and without any location in it, so a tab drag can
//! offer the token as its only payload. The broker moves the app's tab
//! state as it gets it (`Tab`), so the rest of TAB-036 belongs to that
//! type: keeping only the listed state and dropping passwords and other
//! fields, as `tab_snapshot` in `v2.0.0:desktop/window_state.py` does. The
//! windows' Move tab menu (TAB-030) and the tab drag target (TAB-040) are
//! the interface's.
//!
//! The broker delivers messages while it is borrowed mutably, so a window
//! never answers a message from inside the delivery; see
//! [`TabTransfers::new`].

mod messages;

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

pub use messages::{
    Acceptance, Delivery, KeptReason, TabMessage, TabMoveOutcome, TabTransferError, TabTransferToken,
    WindowId,
};

/// How long a move may wait for its claim, and then for the destination's
/// acknowledgement.
pub const TAB_TRANSFER_LIFETIME: Duration = Duration::from_secs(30);

/// The most moves that can wait at the same time.
pub const MAX_PENDING_TAB_TRANSFERS: usize = 64;

/// The longest tab identifier, in characters.
const MAX_TAB_ID_CHARS: usize = 80;

/// One move that is waiting to be claimed or acknowledged.
struct PendingTransfer<Tab> {
    source: WindowId,
    tab_id: String,
    /// The tab until the destination has received it.
    tab: Option<Tab>,
    expires: Instant,
    destination: Option<WindowId>,
}

/// Decides whether a window can take part in a move now.
type ReadinessCheck = Box<dyn Fn(WindowId) -> bool>;
/// Delivers a message to a window.
type Deliver<Tab> = Box<dyn FnMut(WindowId, TabMessage<Tab>) -> Delivery>;
/// The monotonic clock moves expire by.
type Clock = Box<dyn Fn() -> Instant>;

/// The process's pending tab moves. It lives on the main thread with the
/// windows.
pub struct TabTransfers<Tab> {
    pending: HashMap<TabTransferToken, PendingTransfer<Tab>>,
    is_ready: ReadinessCheck,
    deliver: Deliver<Tab>,
    clock: Clock,
}

impl<Tab> fmt::Debug for TabTransfers<Tab> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TabTransfers")
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl<Tab> TabTransfers<Tab> {
    /// A broker that asks `is_ready` whether a window can take part in a
    /// move and hands messages for windows to `deliver`.
    ///
    /// `deliver` runs while the broker is borrowed mutably, inside
    /// [`TabTransfers::claim`], [`TabTransfers::acknowledge`] and every
    /// rollback, so it must not call back into the broker: it queues the
    /// message for the window (for example with
    /// `glib::idle_add_local_once`), and the window answers from there. A
    /// window that inserted a received tab and acknowledged it from inside
    /// `deliver` would find the broker still borrowed, which panics when
    /// the app keeps it in a `RefCell`. The Python app never met this
    /// because its messages reached the web interface asynchronously.
    pub fn new(
        is_ready: impl Fn(WindowId) -> bool + 'static,
        deliver: impl FnMut(WindowId, TabMessage<Tab>) -> Delivery + 'static,
    ) -> Self {
        Self {
            pending: HashMap::new(),
            is_ready: Box::new(is_ready),
            deliver: Box::new(deliver),
            clock: Box::new(Instant::now),
        }
    }

    /// Replaces the clock moves expire by (tests).
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> Instant + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Offers the tab `tab_id` of window `source`, with its state `tab`,
    /// and returns the capability that addresses the move.
    ///
    /// # Errors
    ///
    /// A source that is not ready, an invalid tab identifier, a tab that is
    /// already moving or too many waiting moves, or no random source.
    pub fn offer(
        &mut self,
        source: WindowId,
        tab_id: &str,
        tab: Tab,
    ) -> Result<TabTransferToken, TabTransferError> {
        self.expire();
        if !(self.is_ready)(source) {
            return Err(TabTransferError::SourceNotReady);
        }
        if tab_id.is_empty() || tab_id.chars().count() > MAX_TAB_ID_CHARS {
            return Err(TabTransferError::InvalidTabId);
        }
        let is_moving = self
            .pending
            .values()
            .any(|transfer| transfer.source == source && transfer.tab_id == tab_id);
        if is_moving || self.pending.len() >= MAX_PENDING_TAB_TRANSFERS {
            return Err(TabTransferError::AlreadyMoving);
        }
        let token = TabTransferToken::generate()?;
        let transfer = PendingTransfer {
            source,
            tab_id: tab_id.to_owned(),
            tab: Some(tab),
            expires: (self.clock)() + TAB_TRANSFER_LIFETIME,
            destination: None,
        };
        self.pending.insert(token.clone(), transfer);
        Ok(token)
    }

    /// Claims the move `token` for window `destination` and sends it the
    /// tab, to insert before `before_tab_id`. The source keeps its tab
    /// until [`TabTransfers::acknowledge`] commits the move.
    ///
    /// # Errors
    ///
    /// An unknown, expired or already claimed token; the source window or
    /// one that is not ready as destination; an invalid position; or a
    /// destination that could not be reached, which rolls the move back.
    pub fn claim(
        &mut self,
        token: &TabTransferToken,
        destination: WindowId,
        before_tab_id: Option<&str>,
    ) -> Result<(), TabTransferError> {
        self.expire();
        let Some(transfer) = self.pending.get_mut(token) else {
            return Err(TabTransferError::NotPending);
        };
        if transfer.destination.is_some() {
            return Err(TabTransferError::NotPending);
        }
        if destination == transfer.source || !(self.is_ready)(destination) {
            return Err(TabTransferError::DestinationNotReady);
        }
        if before_tab_id.is_some_and(|before| before.chars().count() > MAX_TAB_ID_CHARS) {
            return Err(TabTransferError::InvalidPosition);
        }
        let Some(tab) = transfer.tab.take() else {
            return Err(TabTransferError::NotPending);
        };
        transfer.destination = Some(destination);
        transfer.expires = (self.clock)() + TAB_TRANSFER_LIFETIME;
        let receive = TabMessage::Receive {
            token: token.clone(),
            tab,
            before_tab_id: before_tab_id.map(str::to_owned),
        };
        if (self.deliver)(destination, receive) == Delivery::WindowGone {
            self.cancel(token, KeptReason::DestinationUnreachable);
            return Err(TabTransferError::DestinationUnreachable);
        }
        Ok(())
    }

    /// Takes the destination's answer after it tried to insert the tab
    /// (the bridge's `tabTransferReady`, `ready` in
    /// `v2.0.0:desktop/tab_transfers.py`). Only an acceptance from the claiming
    /// window, while both windows are ready, commits the move; a refusal or
    /// a closed window rolls it back. An answer that matches no pending
    /// move changes nothing.
    pub fn acknowledge(
        &mut self,
        token: &TabTransferToken,
        destination: WindowId,
        acceptance: Acceptance,
    ) -> TabMoveOutcome {
        self.expire();
        let Some(transfer) = self.pending.get(token) else {
            return TabMoveOutcome::Kept(KeptReason::NotPending);
        };
        if transfer.destination != Some(destination) {
            return TabMoveOutcome::Kept(KeptReason::NotPending);
        }
        let both_ready = (self.is_ready)(transfer.source) && (self.is_ready)(destination);
        if acceptance == Acceptance::Refused || !both_ready {
            self.cancel(token, KeptReason::DestinationBusy);
            return TabMoveOutcome::Kept(KeptReason::DestinationBusy);
        }
        let Some(transfer) = self.pending.remove(token) else {
            return TabMoveOutcome::Kept(KeptReason::NotPending);
        };
        // The destination has already restored the tab: this commits a
        // finished state, not a promise to restore it later.
        self.notify_end(token, &transfer, TabMoveOutcome::Committed);
        TabMoveOutcome::Committed
    }

    /// Rolls back the move `token`, keeping the source tab for `reason`.
    /// Returns `false` when no such move is pending.
    pub fn cancel(&mut self, token: &TabTransferToken, reason: KeptReason) -> bool {
        let Some(transfer) = self.pending.remove(token) else {
            return false;
        };
        self.notify_end(token, &transfer, TabMoveOutcome::Kept(reason));
        true
    }

    /// Rolls back every move whose time is up.
    pub fn expire(&mut self) {
        let now = (self.clock)();
        let expired: Vec<TabTransferToken> = self
            .pending
            .iter()
            .filter(|(_, transfer)| transfer.expires <= now)
            .map(|(token, _)| token.clone())
            .collect();
        for token in expired {
            self.cancel(&token, KeptReason::TimedOut);
        }
    }

    /// Rolls back every move from or to `window`, which closed.
    pub fn window_closed(&mut self, window: WindowId) {
        let affected: Vec<TabTransferToken> = self
            .pending
            .iter()
            .filter(|(_, transfer)| transfer.involves(window))
            .map(|(token, _)| token.clone())
            .collect();
        for token in affected {
            self.cancel(&token, KeptReason::WindowClosed);
        }
    }

    /// True while a move from or to `window` is pending; such a window
    /// refuses further incoming tabs (TAB-037).
    pub fn is_busy(&self, window: WindowId) -> bool {
        self.pending.values().any(|transfer| transfer.involves(window))
    }

    /// True while any move is pending, which blocks an application update.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Tells the destination, if any, and the source how the move ended.
    fn notify_end(
        &mut self,
        token: &TabTransferToken,
        transfer: &PendingTransfer<Tab>,
        outcome: TabMoveOutcome,
    ) {
        if let Some(destination) = transfer.destination {
            let settled = TabMessage::Settled {
                token: token.clone(),
                outcome,
            };
            (self.deliver)(destination, settled);
        }
        let done = TabMessage::Done {
            token: token.clone(),
            tab_id: transfer.tab_id.clone(),
            outcome,
        };
        (self.deliver)(transfer.source, done);
    }
}

impl<Tab> PendingTransfer<Tab> {
    /// True when `window` is this move's source or destination.
    fn involves(&self, window: WindowId) -> bool {
        self.source == window || self.destination == Some(window)
    }
}
