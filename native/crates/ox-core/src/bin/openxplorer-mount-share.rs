// SPDX-License-Identifier: AGPL-3.0-only
//! The `openxplorer-mount-share` executable: the administrator's helper
//! that sets up a persistent, on-demand SMB 3.0 mount after review in a
//! terminal.
//!
//! The native counterpart of `v2.0.0:desktop/mount_share.py`; everything it does
//! lives in [`ox_core::network::mount_share_command`]. The app never runs
//! it: the mount assistant prints `sudo /usr/bin/openxplorer-mount-share
//! --share //server/share` for the user to review and run.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(ox_core::network::mount_share_command(std::env::args().skip(1)))
}
