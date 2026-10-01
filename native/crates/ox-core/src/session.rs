// SPDX-License-Identifier: AGPL-3.0-only
//! Window state that crosses a trust boundary: a tab's navigation state
//! when it moves to another window or comes back from a saved session, and
//! the arguments of `org.freedesktop.FileManager1` requests from other
//! applications. Ports `v2.0.0:desktop/window_state.py`.
//!
//! Both are validated as untrusted data: only whitelisted fields survive,
//! lists are bounded, and every location passes the location rules of
//! [`crate::location`]. Nothing is ever executed.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `tab` | [`TabSnapshot`]: a tab's navigation state | `tab_snapshot` |
//! | `file_manager` | [`FileManagerRequest`]: `ShowFolders`, `ShowItems`, `ShowItemProperties` | `filemanager_request` |
//! | `error` | [`WindowStateError`] with the app's messages | both |

mod error;
mod file_manager;
mod tab;

pub use error::WindowStateError;
pub use file_manager::{FileManagerMethod, FileManagerRequest, MAX_REQUEST_LOCATIONS};
pub use tab::{
    SettingsSection, SortDirection, SortField, TabSnapshot, MAX_HISTORY_ENTRIES, MAX_SCROLL,
    MAX_SELECTED_ITEMS,
};
