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
//! | [`network`] | SMB sign-in, credentials, mounts, sign-out and discovery | `session_credentials.py`, `auth_bridge.py`, `mount_support.py`, `winspace.py` |
//! | [`clipboard`] | File clipboard formats shared with GNOME and KDE | `file_clipboard.py` |
//! | [`format`](mod@format) | Size and date text | `ui/app.js` |
//! | [`transfer`] | Copy, move, Trash and permanent delete | `operations.py` |
//! | [`gio_node`] | The transfer engine's GIO and GVfs adapter | `gio_backend.py` |
//! | [`search`] | The metadata-only filename search cache and its index service | `search_index.py`, `index_service.py`, `local_watch.py` |
//! | [`archive`] | ZIP browsing, opening a member as a private copy, and extraction | `archives.py`, `zip_extraction.py`, `native_opening.py`, `winspace.py` |
//! | [`versions`] | Previous versions and the read-only rule for snapshots | `previous_versions.py`, `file_services.py`, `ui/snapshot-meta.js` |
//! | [`sizes`] | On-demand folder sizes | `folder_sizes.py`, `mount_support.py` |
//! | [`integration`] | Default apps, Show in folder, Brave's download folder, opening files, Open in Terminal | `desktop_integration.py`, `reveal_integration.py`, `filemanager_bus.py`, `brave_integration.py`, `activation.py`, `native_opening.py`, `terminal_integration.py`, `app_catalog.py` |

pub mod archive;
pub mod clipboard;
pub mod entry;
pub mod format;
pub mod gio_node;
pub mod integration;
pub mod location;
pub mod network;
pub mod places;
pub mod search;
pub mod settings;
pub mod sizes;
pub mod transfer;
pub mod versions;

mod private_storage;
#[cfg(test)]
mod test_support;
