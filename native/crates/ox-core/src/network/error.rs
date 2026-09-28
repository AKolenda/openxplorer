// SPDX-License-Identifier: AGPL-3.0-only
//! Why a network operation failed: mounting, connecting, unmounting,
//! ejecting, signing out or discovering servers.
//!
//! The messages are the Python app's (`desktop/winspace.py` and
//! `desktop/gio_backend.py`); GIO's own errors keep GIO's message.

use super::keyring::KeyringError;
use crate::location::LocationError;

/// Why a network operation failed. The message is shown to the user.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// An address that is not a supported location, or not a share.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// GIO or `GVfs` refused, for example a wrong password or an offline
    /// server; the message is GIO's.
    #[error(transparent)]
    Gio(#[from] glib::Error),
    /// The operation was cancelled or timed out; not an error to show.
    #[error("Operation cancelled.")]
    Cancelled,
    /// A connected share turned out not to be a folder.
    #[error("This location is not a folder.")]
    NotAFolder,
    /// The volume to mount was removed meanwhile.
    #[error("This volume is no longer available.")]
    VolumeUnavailable,
    /// The volume mounted, but GIO reports no mount for it.
    #[error("The system did not return a mount for this volume.")]
    NoMountReturned,
    /// Disconnect was chosen while this window writes files.
    #[error("Finish the active file operation first.")]
    WriteInProgress,
    /// Disconnect was chosen for a location outside any user mount.
    #[error("This location has no active user-session mount.")]
    NoUserMount,
    /// The mount cannot be unmounted by the user.
    #[error("The system does not permit unmounting this location.")]
    UnmountNotPermitted,
    /// Eject was chosen for a drive whose medium cannot be ejected.
    #[error("This device cannot be ejected.")]
    CannotEject,
    /// Safely remove was chosen for a drive that cannot be powered off.
    #[error("This drive cannot be safely removed.")]
    CannotSafelyRemove,
    /// Sign out was chosen while a window writes files.
    #[error("Finish active file operations in every OpenXplorer window before signing out.")]
    SignOutDuringWrites,
    /// Sign out was chosen for a location that is not on an SMB server.
    #[error("Select an SMB location to sign out.")]
    NotAnSmbLocation,
    /// Sign out was chosen twice for one server.
    #[error("Sign-out is already in progress for this server.")]
    SignOutAlreadyRunning,
    /// A server being signed out was opened.
    #[error("This server is being signed out. Reopen it after sign-out finishes.")]
    ServerSigningOut,
    /// A share on a server being signed out was connected.
    #[error("Sign-out is in progress. Reconnect after it finishes.")]
    ReconnectAfterSignOut,
    /// One of the server's mounts cannot be unmounted.
    #[error(
        "The system cannot disconnect one of this server’s mounts. Close other applications using it and \
         try again."
    )]
    MountCannotBeDisconnected,
    /// The server was disconnected, but its saved credentials remain.
    #[error("Disconnected, but saved credentials could not be removed. {}", removal_advice(.0))]
    CredentialsNotRemoved(#[source] KeyringError),
}

/// What to tell the user when the keyring could not delete a server's
/// credentials. The Python app asked to install its libsecret binding; the
/// native app needs a running Secret Service instead.
fn removal_advice(error: &KeyringError) -> String {
    match error {
        KeyringError::Unavailable => {
            "Make sure the system keyring is running and try Sign out again.".to_owned()
        }
        KeyringError::TimedOut | KeyringError::UnlockDismissed | KeyringError::Failed { .. } => {
            error.to_string()
        }
    }
}

impl NetworkError {
    /// True when the user or a deadline cancelled the operation, so no
    /// error needs to be shown.
    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::Cancelled => true,
            Self::Gio(error) => error.matches(gio::IOErrorEnum::Cancelled),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_gio_calls_count_as_cancelled() {
        let cancelled = NetworkError::from(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "Operation was cancelled",
        ));
        let refused = NetworkError::from(glib::Error::new(
            gio::IOErrorEnum::PermissionDenied,
            "Access denied",
        ));

        assert!(cancelled.is_cancelled());
        assert!(NetworkError::Cancelled.is_cancelled());
        assert!(!refused.is_cancelled());
        assert_eq!(refused.to_string(), "Access denied");
    }

    /// parity: NET-021
    #[test]
    fn a_failed_credential_removal_says_the_server_was_disconnected() {
        let refused = NetworkError::CredentialsNotRemoved(KeyringError::UnlockDismissed);
        let missing = NetworkError::CredentialsNotRemoved(KeyringError::Unavailable);

        assert_eq!(
            refused.to_string(),
            "Disconnected, but saved credentials could not be removed. The keyring unlock was cancelled."
        );
        assert_eq!(
            missing.to_string(),
            "Disconnected, but saved credentials could not be removed. Make sure the system keyring is running \
             and try Sign out again."
        );
    }
}
