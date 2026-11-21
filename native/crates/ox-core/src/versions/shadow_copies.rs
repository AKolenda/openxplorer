// SPDX-License-Identifier: AGPL-3.0-only
//! The previous versions an SMB server keeps of a share: its shadow copies.
//!
//! Windows' Previous Versions tab asks the server for them with
//! `FSCTL_SRV_ENUMERATE_SNAPSHOTS` (MS-SMB2 2.2.32.2), which answers with
//! one `@GMT-YYYY.MM.DD-HH.MM.SS` name per snapshot, in UTC. Samba (with
//! `shadow_copy2`, or `shadow_copy_zfs` on `TrueNAS`) and Windows servers open
//! such a name as a path component, so the lookup reads each version
//! through GIO at `smb://server/share/@GMT-…/path` like any other item.
//! A [`ShadowCopyLister`] only asks for the names; the lookup checks the
//! item in each one.

use std::fmt;

use crate::transfer::Cancellation;

/// The length of a shadow copy's name: `@GMT-2026.11.20-18.30.00`.
const TOKEN_LENGTH: usize = 24;

/// The shadow copies a server named for one item.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShadowCopies {
    /// The valid `@GMT-…` names, newest first and without repeats.
    pub tokens: Vec<String>,
    /// True when the server keeps more shadow copies than it named.
    pub is_truncated: bool,
}

impl ShadowCopies {
    /// The valid shadow-copy names among `names`, as a server returned
    /// them, newest first. A name that is not exactly `@GMT-` and a date
    /// and time is dropped, so a server's answer never becomes a path
    /// with more than one component.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>, is_truncated: bool) -> Self {
        let mut tokens: Vec<String> = names
            .into_iter()
            .filter(|name| is_shadow_copy_token(name))
            .map(str::to_owned)
            .collect();
        // The fixed-width date sorts newest first as text.
        tokens.sort_unstable_by(|first, second| second.cmp(first));
        tokens.dedup();
        Self { tokens, is_truncated }
    }
}

/// Why a server's shadow copies could not be listed. The message is shown
/// in the tab's Availability details.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ShadowCopyError {
    /// Neither `OpenXplorer` nor `GVfs` saved a password for the server.
    #[error(
        "{}",
        crate::i18n::gettext(
            "No saved password was found for this server, so its previous versions could not be \
             listed. Sign in with \"Remember my credentials\" checked to list them."
        )
    )]
    NoCredential,
    /// The server refused the saved password.
    #[error(
        "{}",
        crate::i18n::gettext(
            "The server refused the saved password, so its previous versions could not be listed."
        )
    )]
    SignInRefused,
    /// The server does not keep shadow copies of the share.
    #[error(
        "{}",
        crate::i18n::gettext("The server does not offer previous versions of this share.")
    )]
    Unsupported,
    /// The server could not be reached or did not answer as expected. The
    /// detail is developer text, for logs, and not shown.
    #[error("{}", crate::i18n::gettext("The server did not list its previous versions."))]
    Failed {
        /// What the SMB client reported.
        detail: String,
    },
    /// The lookup was cancelled.
    #[error("{}", crate::i18n::gettext("Operation cancelled."))]
    Cancelled,
}

/// Lists the shadow copies an SMB server keeps of an item.
pub trait ShadowCopyLister: Send + Sync + fmt::Debug {
    /// The shadow copies of the item at the canonical `smb://` location
    /// `uri`. It blocks while the server answers, so call it on a worker
    /// thread.
    ///
    /// # Errors
    ///
    /// A [`ShadowCopyError`] saying why the server could not be asked;
    /// [`ShadowCopyError::Cancelled`] once `cancel` is cancelled.
    fn list(&self, uri: &str, cancel: &Cancellation) -> Result<ShadowCopies, ShadowCopyError>;
}

/// True when `name` is a shadow copy's name: `@GMT-` followed by a date
/// and time in the form `YYYY.MM.DD-HH.MM.SS`.
pub fn is_shadow_copy_token(name: &str) -> bool {
    let Some(stamp) = name.strip_prefix(super::paths::SMB_VERSION_PREFIX) else {
        return false;
    };
    name.len() == TOKEN_LENGTH
        && stamp.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 | 13 | 16 => byte == b'.',
            10 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-032
    #[test]
    fn only_gmt_names_are_shadow_copies_and_the_newest_comes_first() {
        let copies = ShadowCopies::from_names(
            [
                "@GMT-2026.11.20-08.00.00",
                "@GMT-2026.11.21-00.30.00",
                "@GMT-2026.11.20-08.00.00",
                "@GMT-2026.11.20-08.00.00/../x",
                "@GMT-2026.11.2-08.00.000",
                "@GMT-2026-11-20-08.00.00",
                "auto-2026-11-20_08-00",
                "",
            ],
            false,
        );

        assert_eq!(
            copies.tokens,
            ["@GMT-2026.11.21-00.30.00", "@GMT-2026.11.20-08.00.00"]
        );
    }
}
