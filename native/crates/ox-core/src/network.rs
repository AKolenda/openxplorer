// SPDX-License-Identifier: AGPL-3.0-only
//! Network shares, sign-in and mounts: the network service of the window.
//!
//! Ports `desktop/session_credentials.py`, `desktop/auth_bridge.py`,
//! `desktop/mount_support.py`, the file rules of `desktop/mount_share.py`,
//! and the network operations of `desktop/winspace.py` (`mount`,
//! `connect`, `mountVolume`, `unmount`, `sign_out`, `discover_network`,
//! `remember_network`), `verify_folder` and `discover_servers` of
//! `desktop/gio_backend.py` and `local_path` of `desktop/native_opening.py`.
//! The Network sidebar list (`desktop/network_locations.py`) is composed in
//! [`places`](crate::places); the volume rows (`desktop/volume_locations.py`)
//! are read in the app.
//!
//! Nothing here depends on GTK. Keyring calls block, so they run on worker
//! threads ([`gio::spawn_blocking`]); mounting, unmounting and discovery use
//! GIO's asynchronous calls on the main loop. Dropping the future of an
//! operation cancels it and aborts any sign-in dialog it opened.
//!
//! The app creates one [`CredentialStore`] and one [`SignOutRegistry`];
//! each window gets its own [`SessionCredentials`] over that store and its
//! own [`MountPrompts`].
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `server` | Which SMB server a location belongs to | `session_credentials.py` |
//! | `credential` | One SMB account and how long it is remembered | `session_credentials.py`, `auth_bridge.py` |
//! | `keyring`, `secret_service` | The desktop keyring (Secret Service) | `session_credentials.py` |
//! | `credential_store` | The keyring, sign-out generations and keyring locks every window shares | `session_credentials.py` |
//! | `session_credentials` | One window's credentials per server, in memory and in the keyring | `session_credentials.py` |
//! | `prompts` | `GVfs`'s sign-in questions, answered by the window's dialog | `auth_bridge.py` |
//! | `mounting` | Mount on demand and Map network location | `winspace.py`, `gio_backend.py` |
//! | `volumes` | Connect drives and phones, Disconnect, Eject and Safely remove | `winspace.py`, `volume_locations.py` |
//! | `sign_out` | Sign out of a server | `winspace.py` |
//! | `discovery` | Servers advertising on the local network | `winspace.py`, `gio_backend.py` |
//! | `visited` | Servers and shares browsed this session | `winspace.py` |
//! | `mount_table`, `local_path` | Local paths of SMB locations | `mount_support.py`, `native_opening.py` |
//! | `mount_plan`, `mount_helper` | The persistent mount assistant | `mount_support.py`, `mount_share.py` |
//!
//! The privacy rules of the Python modules hold here too, each enforced
//! and documented where it applies:
//!
//! - Passwords go only to the mount operation and the keyring. They never
//!   reach a [`Challenge`], a log message or the settings, and there is no
//!   plaintext fallback without a keyring (SAFE-011).
//! - A window's in-memory credentials are its own and are wiped when it
//!   closes (SAFE-011, TAB-050); Sign out wipes them in every window
//!   (NET-021).
//! - Credentials are kept per server ([`ServerKey`]), never guessed from
//!   host aliases, DNS or redirects (NET-014).
//! - Credentials are saved only after a successful mount (NET-015), and a
//!   save racing a Sign out is discarded (SAFE-012).

mod credential;
mod credential_store;
mod discovery;
mod error;
mod keyring;
mod local_path;
// The helper program that uses these file rules is ported in the "Network
// and devices" milestone of ROADMAP.md; until then only the tests call them.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the openxplorer-mount-share helper, not ported yet"
    )
)]
mod mount_helper;
mod mount_plan;
mod mount_table;
mod mounting;
mod prompts;
mod secret_service;
mod server;
mod session_credentials;
mod sign_out;
#[cfg(test)]
mod test_support;
mod visited;
mod volumes;

pub use credential::{Credential, CredentialScope, Password};
pub use credential_store::{CredentialGeneration, CredentialStore, ForgetScope, SMB_CREDENTIAL_SCHEMA};
pub use discovery::{discover_servers, DiscoveredServer, Discovery, DISCOVERY_NOTE};
pub use error::NetworkError;
pub use keyring::{Keyring, KeyringCollection, KeyringError, NewSecret, SecretAttributes};
pub use local_path::{fuse_export_path, local_path};
pub use mount_plan::{mount_plan, DesktopUser, MountPlan, MountPlanError};
pub use mount_table::{
    mount_for_path, parse_mount_table, read_mount_table, read_stable_smb_mounts, resolve_smb_path, MountEntry,
};
pub use mounting::{
    connect_share, mount_location, read_mounting_once, ConnectedShare, MountedReadError, NeedsMount,
    WriteActivity,
};
pub use prompts::{
    split_identity, Answer, Challenge, ChallengeId, ChallengeKind, Identity, MountOutcome, MountPrompts,
    PasswordChallenge, QuestionChallenge, SignIn, SignInError, SignInPrompter, KEYRING_SAVE_NOTICE,
};
pub use secret_service::SecretService;
pub use server::{ServerKey, DEFAULT_SMB_PORT};
pub use session_credentials::SessionCredentials;
pub use sign_out::{
    begin_sign_out, finish_sign_out, SignOutRegistry, SignOutReport, SignOutRequest, SigningOut,
};
pub use visited::{session_network_root, VisitedNetwork};
pub use volumes::{
    eject_location, mount_volume, safely_remove_location, unmount_location, volume_id, volume_id_from,
};
