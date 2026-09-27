// SPDX-License-Identifier: AGPL-3.0-only
//! Identity and paths.

/// Application ID during the preview. It differs from the Python app's
/// `io.winspace.Development` so both can run side by side; the native app
/// takes over that ID (a compatibility contract) when it becomes the default.
pub const APP_ID: &str = "io.winspace.Development.Native";
