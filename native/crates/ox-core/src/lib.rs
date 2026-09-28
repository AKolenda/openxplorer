// SPDX-License-Identifier: AGPL-3.0-only
//! Toolkit-independent core of the native Rust + GTK4 app.
//!
//! Nothing in this crate depends on GTK. Filesystem access goes through GIO,
//! so local folders, SMB shares, phones (MTP) and the Trash behave the same
//! way they do in the Python application. The Python modules under
//! `desktop/` are the behavioural specification; each module names the
//! files it ports:
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | [`location`] | Parse, validate and display locations | `core.py`, `ui/app.js` |
//! | [`settings`] | The settings file shared with the Python app | `core.py`, `private_storage.py` |
//! | [`entry`] | Folder listings, single items and Quick access pins | `entry_model.py`, `gio_backend.py` |
//! | [`places`] | The Quick access and Network sidebar sections | `winspace.py`, `network_locations.py` |
//! | [`clipboard`] | File clipboard formats shared with GNOME and KDE | `file_clipboard.py` |
//! | [`format`](mod@format) | Size and date text | `ui/app.js` |
//! | [`transfer`] | Copy, move, Trash and permanent delete | `operations.py` |
//! | [`gio_node`] | The transfer engine's GIO and GVfs adapter | `gio_backend.py` |

pub mod clipboard;
pub mod entry;
pub mod format;
pub mod gio_node;
pub mod location;
pub mod ops;
pub mod places;
pub mod settings;
pub mod transfer;

mod private_storage;
#[cfg(test)]
mod test_support;
