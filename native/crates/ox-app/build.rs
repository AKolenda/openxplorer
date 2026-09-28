// SPDX-License-Identifier: AGPL-3.0-only
//! Compiles the vendored icons into `icons.gresource`, which
//! `src/icons.rs` includes in the binary with
//! `gio::resources_register_include!`, and chooses the application ID the
//! binary registers on the session bus.
//!
//! `glib-compile-resources` must be installed; it comes with the `GLib`
//! development files that building against GTK needs anyway. Cargo builds
//! the bundle again whenever the manifest or one of the icons it lists
//! changes.

use std::env;

/// The variable a package build sets to choose the application ID.
const APP_ID_VARIABLE: &str = "OX_APP_ID";

/// The preview's ID, which lets it run beside the Python app.
const PREVIEW_APP_ID: &str = "io.winspace.Development.Native";

/// The Python app's ID, which the native app takes over when it replaces
/// it (a compatibility contract, see AGENTS.md).
const STABLE_APP_ID: &str = "io.winspace.Development";

fn main() {
    glib_build_tools::compile_resources(
        &["resources/icons/hicolor"],
        "resources/icons/icons.gresource.xml",
        "icons.gresource",
    );
    export_app_id();
}

/// Passes the application ID to the crate as the compile-time variable
/// `OX_APP_ID` (read by `src/config.rs`): the preview's, unless the build
/// sets `OX_APP_ID` to the stable ID, as the packages of the release that
/// replaces the Python app do (native/packaging/README.md).
///
/// # Panics
///
/// If `OX_APP_ID` names another ID. GTK would only refuse it at startup,
/// and the desktop entry, D-Bus name and Flatpak ID must all agree with it.
fn export_app_id() {
    println!("cargo::rerun-if-env-changed={APP_ID_VARIABLE}");
    let app_id = env::var(APP_ID_VARIABLE).unwrap_or_else(|_| PREVIEW_APP_ID.to_owned());
    assert!(
        [PREVIEW_APP_ID, STABLE_APP_ID].contains(&app_id.as_str()),
        "{APP_ID_VARIABLE} must be {PREVIEW_APP_ID} or {STABLE_APP_ID}, not {app_id:?}"
    );
    println!("cargo::rustc-env={APP_ID_VARIABLE}={app_id}");
}
