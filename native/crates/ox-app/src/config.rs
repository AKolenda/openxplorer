// SPDX-License-Identifier: AGPL-3.0-only
//! The application's identity on the session bus.
//!
//! Ports `APP_ID` in `desktop/runtime_guard.py`.

/// Application ID during the preview. It differs from the Python app's
/// `io.winspace.Development` so both can run side by side; the native app
/// takes over that ID (a compatibility contract) when it becomes the default.
pub(crate) const APP_ID: &str = "io.winspace.Development.Native";
