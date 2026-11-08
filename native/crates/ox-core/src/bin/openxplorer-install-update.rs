// SPDX-License-Identifier: AGPL-3.0-only
//! The `openxplorer-install-update` executable: the root helper that
//! installs the latest official release when the user updates the app.
//!
//! Everything it does lives in [`ox_core::update::install_update_command`],
//! whose module documents the threat model. The app starts it through
//! `pkexec`, under the polkit action the Debian package installs, after
//! the user has confirmed the update; it is not meant to be run by hand.
//! It takes one version and nothing else, downloads and verifies that
//! release itself, and installs it only if it is the latest release and
//! newer than the installed package.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(ox_core::update::install_update_command(
        std::env::args_os().skip(1),
    ))
}
