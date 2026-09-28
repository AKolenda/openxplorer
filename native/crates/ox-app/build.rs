// SPDX-License-Identifier: AGPL-3.0-only
//! Compiles the vendored icons into `icons.gresource`, which
//! `src/icons.rs` includes in the binary with
//! `gio::resources_register_include!`.
//!
//! `glib-compile-resources` must be installed; it comes with the `GLib`
//! development files that building against GTK needs anyway. Cargo builds
//! the bundle again whenever the manifest or one of the icons it lists
//! changes.

fn main() {
    glib_build_tools::compile_resources(
        &["resources/icons/hicolor"],
        "resources/icons/icons.gresource.xml",
        "icons.gresource",
    );
}
