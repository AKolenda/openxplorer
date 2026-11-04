// SPDX-License-Identifier: AGPL-3.0-only
//! The `openxplorer-admin-delete` executable: the administrator's helper
//! that permanently deletes one folder under `admin://`.
//!
//! Everything it does lives in [`ox_core::admin_delete`]. The app starts it
//! through `pkexec`, under the polkit action its package installs, after
//! the user has confirmed a permanent delete; it is not meant to be run by
//! hand. It takes one absolute path, follows a symbolic link on the way to
//! the item only when nobody but root could have put it there, never
//! follows one inside the item, never leaves the drive the item is on, and
//! reports what it deleted and what it left on its standard output.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(ox_core::admin_delete::admin_delete_command(
        std::env::args_os().skip(1),
    ))
}
