// SPDX-License-Identifier: AGPL-3.0-only
//! What a tab move is addressed by and what it tells the windows: the
//! window ids, the capability token, the messages the app delivers, how a
//! move ends and why it can be refused. Ports the payloads and messages of
//! `desktop/tab_transfers.py`.

use std::fmt;
use std::str::FromStr;

use crate::ops::random::random_hex;

/// Random bytes in a token: 64 hexadecimal digits.
const TOKEN_BYTES: usize = 32;

/// A window of this process, by its GTK window id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u32);

/// The capability that addresses one pending move.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabTransferToken(String);

impl TabTransferToken {
    /// The token as text, for a drag payload.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A new unpredictable token from the kernel's random source.
    pub(super) fn generate() -> Result<Self, TabTransferError> {
        let digits =
            random_hex(TOKEN_BYTES).map_err(|error| TabTransferError::NoCapability(error.to_string()))?;
        Ok(Self(digits))
    }
}

impl FromStr for TabTransferToken {
    type Err = TabTransferError;

    /// Reads a token from a drag payload: exactly 64 lowercase
    /// hexadecimal digits. Anything else cannot address a move.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let is_lower_hex = |digit: u8| matches!(digit, b'0'..=b'9' | b'a'..=b'f');
        if text.len() != TOKEN_BYTES * 2 || !text.bytes().all(is_lower_hex) {
            return Err(TabTransferError::NotPending);
        }
        Ok(Self(text.to_owned()))
    }
}

impl fmt::Display for TabTransferToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why a move ended with the source tab kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptReason {
    /// The user or the source window cancelled the move.
    Cancelled,
    /// The destination window could not be reached with the tab.
    DestinationUnreachable,
    /// The destination refused the tab, or a window stopped being ready.
    DestinationBusy,
    /// Nobody claimed or acknowledged the move in time.
    TimedOut,
    /// The source or destination window closed.
    WindowClosed,
    /// The acknowledgement did not match a pending move to that window.
    NotPending,
}

impl KeptReason {
    /// The message the source window shows, word for word as
    /// `desktop/tab_transfers.py` sends it.
    pub fn message(self) -> &'static str {
        match self {
            KeptReason::Cancelled => "Tab move cancelled. The original tab was kept.",
            KeptReason::DestinationUnreachable => "The destination could not receive the tab.",
            KeptReason::DestinationBusy => "The destination was busy or closed. The original tab was kept.",
            KeptReason::TimedOut => "The tab move timed out. The original tab was kept.",
            KeptReason::WindowClosed => "A window closed before the tab move finished.",
            KeptReason::NotPending => {
                "The tab move has expired or was already accepted. The original tab was kept."
            }
        }
    }
}

/// How a move ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabMoveOutcome {
    /// The destination keeps the tab and the source removes it.
    Committed,
    /// The source keeps the tab and the destination removes its tentative
    /// copy.
    Kept(KeptReason),
}

/// What the destination window says after it tried to insert the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceptance {
    /// The tab is restored and shown.
    Accepted,
    /// The window was busy (a dialog, an operation, another incoming tab).
    Refused,
}

/// A message for one window, which the app delivers to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabMessage<Tab> {
    /// `tabReceive`: insert `tab` tentatively before the tab
    /// `before_tab_id` (at the end without one), then call
    /// [`TabTransfers::ready`].
    Receive {
        /// The move.
        token: TabTransferToken,
        /// The moving tab's state.
        tab: Tab,
        /// The tab to insert it before.
        before_tab_id: Option<String>,
    },
    /// `tabTransferSettled`, to the destination: keep the inserted tab
    /// when committed, remove it otherwise.
    Settled {
        /// The move.
        token: TabTransferToken,
        /// How the move ended.
        outcome: TabMoveOutcome,
    },
    /// `tabTransferDone`, to the source: remove the tab when committed,
    /// otherwise keep it and show the reason.
    Done {
        /// The move.
        token: TabTransferToken,
        /// The source's identifier of the moving tab.
        tab_id: String,
        /// How the move ended.
        outcome: TabMoveOutcome,
    },
}

/// Whether a message reached its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The window received it.
    Delivered,
    /// The window no longer exists.
    WindowGone,
}

/// Why a move could not be offered or claimed. The messages are the
/// Python app's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TabTransferError {
    /// The source window closed or is not ready.
    #[error("The source window is no longer ready.")]
    SourceNotReady,
    /// The tab identifier is empty or longer than 80 characters.
    #[error("Invalid tab identifier.")]
    InvalidTabId,
    /// The tab is already moving, or [`MAX_PENDING_TAB_TRANSFERS`] moves
    /// are waiting.
    #[error("That tab is already moving. Wait for it to finish.")]
    AlreadyMoving,
    /// The token is unknown, expired, already claimed or already used.
    #[error("The tab move has expired or was already accepted. The original tab was kept.")]
    NotPending,
    /// The destination is the source window, or not ready.
    #[error("Choose a different, ready OpenXplorer window.")]
    DestinationNotReady,
    /// The tab to insert before is named by an identifier over 80
    /// characters.
    #[error("Invalid tab position.")]
    InvalidPosition,
    /// The destination window could not be reached; the move was rolled
    /// back.
    #[error("The destination could not receive the tab.")]
    DestinationUnreachable,
    /// No capability could be generated; nothing was offered.
    #[error("Could not create a private tab-move capability. The original tab was kept. {0}")]
    NoCapability(String),
}
