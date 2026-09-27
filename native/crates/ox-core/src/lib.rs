// SPDX-License-Identifier: AGPL-3.0-only
//! Toolkit-independent core of the native OpenXplorer.
//!
//! Nothing in this crate depends on GTK. Filesystem access goes through GIO,
//! so local folders, SMB shares, phones (MTP) and the Trash behave the same
//! way they do in the Python application. The Python modules under
//! `desktop/` are the behavioural specification; each module below names the
//! file it ports.

pub mod clipboard;
pub mod entry;
pub mod format;
pub mod gio_node;
pub mod location;
pub mod places;
pub mod settings;
pub mod transfer;
