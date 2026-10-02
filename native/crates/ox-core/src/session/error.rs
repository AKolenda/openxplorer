// SPDX-License-Identifier: AGPL-3.0-only
//! Why a tab's state or a `FileManager1` request was refused. The messages
//! are those of `v2.0.0:desktop/window_state.py`, word for word.

use crate::location::LocationError;

/// Why window state was refused. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowStateError {
    /// The tab state is not a JSON object.
    #[error("{}", crate::i18n::gettext("Invalid tab state."))]
    InvalidTab,
    /// The tab's location or a history entry is not text.
    #[error("{}", crate::i18n::gettext("Location must be a string."))]
    LocationNotText,
    /// The history is not a list of 1 to 200 locations.
    #[error("{}", crate::i18n::gettext("Tab history must contain 1–200 locations."))]
    HistoryLength,
    /// The history position is not an integer inside the history.
    #[error("{}", crate::i18n::gettext("Invalid history position."))]
    HistoryPosition,
    /// The selection is not a list of at most 10,000 locations.
    #[error("{}", crate::i18n::gettext("Invalid selection."))]
    Selection,
    /// The scroll position is not a finite number.
    #[error("{}", crate::i18n::gettext("Invalid scroll position."))]
    ScrollPosition,
    /// A `FileManager1` method other than the three it implements.
    #[error("{}", crate::i18n::gettext("Unsupported method."))]
    UnsupportedMethod,
    /// A `FileManager1` request without locations or with more than 100.
    #[error("{}", crate::i18n::gettext("Expected 1–100 file locations."))]
    RequestLength,
    /// A location the location rules refuse.
    #[error(transparent)]
    Location(#[from] LocationError),
}
