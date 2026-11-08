// SPDX-License-Identifier: AGPL-3.0-only
//! "Install updates without asking for a password", the setting on
//! Settings › About, on by default: whether this build has it, and turning it on or off
//! through `pkexec` and the update helper
//! ([`ox_core::update::PasswordlessUpdates`]). New in the native app.

use std::path::Path;

use gtk::gio;
use ox_core::update::{Installation, PasswordlessUpdates};

use super::service::this_executable;

/// The exit status `pkexec` returns when the administrator prompt was
/// dismissed or refused.
const PROMPT_DISMISSED: i32 = 126;

/// The exit status `pkexec` returns when authentication failed or polkit
/// did not allow the action.
const NOT_AUTHORIZED: i32 = 127;

/// How changing the setting ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasswordlessChange {
    /// The helper changed it.
    Changed,
    /// The administrator prompt was cancelled or refused; nothing changed.
    Cancelled,
    /// The helper or `pkexec` could not change it.
    Failed,
}

/// The setting of the running build, if it has one: only the stable
/// Debian package, which installs the update helper and the rule
/// template, does. RPM, Arch, Flatpak, the preview and source builds have
/// none, so Settings does not show it.
pub(crate) fn passwordless_for_this_build() -> Option<PasswordlessUpdates> {
    let root = Path::new("/");
    let installation = Installation::detect_for_executable(&this_executable(), root);
    PasswordlessUpdates::detect(installation, root)
}

/// Turns `setting` `on` or off: runs `pkexec <helper>
/// --enable-passwordless` (or `--disable-passwordless`), which asks for an
/// administrator's password, and waits for it without blocking the
/// interface.
pub(crate) async fn change_passwordless(setting: &PasswordlessUpdates, on: bool) -> PasswordlessChange {
    let argv = setting.command(on);
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsString::as_os_str).collect();
    let Ok(process) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDERR_SILENCE) else {
        return PasswordlessChange::Failed;
    };
    if process.wait_future().await.is_err() || !process.has_exited() {
        return PasswordlessChange::Failed;
    }
    match process.exit_status() {
        0 => PasswordlessChange::Changed,
        PROMPT_DISMISSED | NOT_AUTHORIZED => PasswordlessChange::Cancelled,
        _ => PasswordlessChange::Failed,
    }
}
