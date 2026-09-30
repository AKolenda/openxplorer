// SPDX-License-Identifier: AGPL-3.0-only
//! The app's dialogs that are windows of their own.
//!
//! Each is modal and transient for the browser window, so the desktop
//! attaches and dims it as it does every GNOME dialog, and the window's
//! shortcuts do not reach through it. A dialog only asks; the window does
//! the work it asks for.
//!
//! | Module | Dialog | Ports |
//! |---|---|---|
//! | `network_sign_in` | Enter network credentials, and a server's questions | `renderAuth` in `app.js` |
//! | `network_map` | Map network location | `connectDialog` in `app.js` |
//! | `network_protocol` | The protocols Map network location offers | Dolphin and Files |
//! | `network_sign_out` | Sign out of a server | `signOut` in `app.js` |

mod network_map;
mod network_protocol;
mod network_sign_in;
mod network_sign_out;

pub(crate) use network_map::{map_network_dialog, MapRequest, ShareKeeping};
pub(crate) use network_protocol::Protocol;
pub(crate) use network_sign_in::SignInDialog;
pub(crate) use network_sign_out::{sign_out_dialog, SearchCacheChoice, SignOutChoice};
