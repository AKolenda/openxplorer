// SPDX-License-Identifier: AGPL-3.0-only
//! Network shares in the app: what every window shares, each window's
//! sign-in and server discovery.
//!
//! Ports the network half of `v2.0.0:desktop/winspace.py` (the application's
//! `visited_network`, `signing_out_hosts` and keyring, each window's
//! `MountPrompts`) and of `v2.0.0:desktop/ui/app.js` (`receiveAuth`,
//! `discoverNetwork`). The work itself is ox-core's
//! [`network`](ox_core::network) service; this module keeps its state for
//! the app and shows its questions. The window's own commands (Map network
//! location, Sign out, the Network page) are in `crate::window`.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `services` | The keyring, sign-outs, visited servers and kernel SMB mounts every window shares | `winspace.py` |
//! | `sign_in_queue` | One sign-in dialog at a time for a window's challenges | `receiveAuth`, `dismissAuth` in `app.js` |
//! | `discovery` | Discover servers: three passes, merged, with Stop | `discoverNetwork` in `app.js` |
//! | `window_network` | One window's prompts, sign-in queue and discovery | `MountPrompts` in `auth_bridge.py` |

mod discovery;
mod services;
mod sign_in_queue;
mod window_network;

#[cfg(test)]
pub(crate) use discovery::Discoverer;
pub(crate) use discovery::DiscoveryState;
pub(crate) use services::{read_stable_mounts, user_recent_servers, NetworkServices};
pub(crate) use window_network::WindowNetwork;
