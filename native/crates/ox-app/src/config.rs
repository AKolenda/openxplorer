// SPDX-License-Identifier: AGPL-3.0-only
//! The application's identity on the session bus, and the name of the
//! build.
//!
//! Ports `APP_ID` in `desktop/runtime_guard.py`.

/// The application ID, which `build.rs` chooses. It is the preview's
/// `io.winspace.Development.Native` unless the build sets `OX_APP_ID` to
/// the Python app's `io.winspace.Development`: the preview differs from it
/// so both can run side by side, and the native app takes that ID over (a
/// compatibility contract) when it replaces the Python app
/// (native/packaging/README.md, "Application ID").
pub(crate) const APP_ID: &str = env!("OX_APP_ID");

/// What this build is called in the status bar, About this build and the
/// About settings (`#status-mode` in `desktop/ui/app.js`).
pub(crate) const BUILD_NAME: &str = concat!("OpenXplorer ", env!("CARGO_PKG_VERSION"), " native preview");
