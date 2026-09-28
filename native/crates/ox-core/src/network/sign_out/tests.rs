// SPDX-License-Identifier: AGPL-3.0-only
//! Sign out of a server, against an in-memory keyring and without mounts:
//! the isolated test session has no SMB server to disconnect, so which
//! mounts are disconnected is tested on their root locations.

use std::sync::mpsc;

use super::*;
use crate::network::credential::{Credential, CredentialScope, Password};
use crate::network::credential_store::{locked, SMB_CREDENTIAL_SCHEMA};
use crate::network::keyring::{Keyring, KeyringCollection, NewSecret};
use crate::network::mounting::connect_share;
use crate::network::test_support::{with_memory_only_prompts, with_prompts, MemoryKeyring, PromptsFixture};

/// The schema `GVfs` stores remembered network passwords under.
const GNOME_NETWORK_PASSWORD_SCHEMA: &str = "org.gnome.keyring.NetworkPassword";

fn sam(scope: CredentialScope) -> Credential {
    Credential {
        username: "sam".into(),
        domain: "WORKGROUP".into(),
        password: Password::from("not-a-real-password"),
        scope,
    }
}

/// Saves `credential` for the server of `uri` as a mount would.
fn save(fixture: &PromptsFixture, uri: &str, credential: &Credential) {
    let generation = fixture.credentials.generation(uri);
    fixture
        .credentials
        .persist(uri, credential, generation)
        .expect("the memory keyring saves");
}

/// The password `GVfs` remembers for `host`, as GNOME's keyring stores it.
fn gnome_password(host: &str) -> SecretAttributes {
    SecretAttributes::for_schema(GNOME_NETWORK_PASSWORD_SCHEMA)
        .with("server", host)
        .with("protocol", "smb")
        .with("user", "sam")
}

fn remember_gnome_password(keyring: &MemoryKeyring, host: &str) {
    let attributes = gnome_password(host);
    let secret = NewSecret {
        collection: KeyringCollection::Default,
        label: "sam@nas",
        attributes: &attributes,
        text: "not-a-real-password",
    };
    keyring.store(&secret).expect("the memory keyring saves");
}

/// `OpenXplorer`'s saved credential of `host` in `scope`.
fn own_entry(host: &str, scope: CredentialScope) -> SecretAttributes {
    SecretAttributes::for_schema(SMB_CREDENTIAL_SCHEMA)
        .with("server", host)
        .with("port", "445")
        .with("scope", scope.as_str())
}

fn request(uri: &str, forget: ForgetScope) -> SignOutRequest<'_> {
    SignOutRequest {
        uri,
        forget,
        writes: WriteActivity::Idle,
    }
}

/// Begins and finishes a sign-out, as a window without mounts does.
fn sign_out(
    fixture: &PromptsFixture,
    registry: &SignOutRegistry,
    request: SignOutRequest<'_>,
) -> Result<SignOutReport, NetworkError> {
    let signing_out = begin_sign_out(&fixture.prompts, registry, request)?;
    fixture.block_on(finish_sign_out(&signing_out, &[]))
}

/// parity: NET-020, NET-021, SAFE-012
#[test]
fn signing_out_forgets_every_saved_credential_of_that_server_only() {
    with_prompts(|fixture| {
        save(fixture, "smb://nas/a", &sam(CredentialScope::Permanent));
        save(fixture, "smb://nas/b", &sam(CredentialScope::Session));
        save(fixture, "smb://other/a", &sam(CredentialScope::Session));
        remember_gnome_password(&fixture.keyring, "nas");
        remember_gnome_password(&fixture.keyring, "other");
        fixture
            .credentials
            .accept_memory("smb://nas/a", &sam(CredentialScope::Session));
        let registry = SignOutRegistry::default();

        let signed_out = sign_out(
            fixture,
            &registry,
            request("smb://NAS/Projects", ForgetScope::AllScopes),
        );

        let expected = SignOutReport {
            host: "nas".into(),
            disconnected: 0,
            credentials_removed: true,
            forget: ForgetScope::AllScopes,
        };
        assert_eq!(signed_out.expect("sign out succeeds"), expected);
        assert_eq!(fixture.credentials.peek("smb://nas/a"), None);
        assert!(!fixture
            .keyring
            .contains(&own_entry("nas", CredentialScope::Permanent)));
        assert!(!fixture
            .keyring
            .contains(&own_entry("nas", CredentialScope::Session)));
        assert!(!fixture.keyring.contains(&gnome_password("nas")));
        assert!(fixture
            .keyring
            .contains(&own_entry("other", CredentialScope::Session)));
        assert!(fixture.keyring.contains(&gnome_password("other")));
        assert!(
            !registry.is_signing_out("smb://nas/Projects"),
            "the sign-out has ended"
        );
    });
}

/// parity: NET-021
#[test]
fn keeping_saved_credentials_removes_only_session_entries() {
    with_prompts(|fixture| {
        save(fixture, "smb://nas/a", &sam(CredentialScope::Permanent));
        save(fixture, "smb://nas/b", &sam(CredentialScope::Session));
        remember_gnome_password(&fixture.keyring, "nas");
        let registry = SignOutRegistry::default();

        let signed_out = sign_out(
            fixture,
            &registry,
            request("smb://nas/a", ForgetScope::SessionOnly),
        );

        let report = signed_out.expect("sign out succeeds");
        assert!(report.credentials_removed, "the session entry was deleted");
        assert!(!fixture
            .keyring
            .contains(&own_entry("nas", CredentialScope::Session)));
        assert!(fixture
            .keyring
            .contains(&own_entry("nas", CredentialScope::Permanent)));
        assert!(fixture.keyring.contains(&gnome_password("nas")));
    });
}

/// Without session entries, keeping the saved credentials deletes
/// nothing, and the report says so.
///
/// parity: NET-021
#[test]
fn keeping_saved_credentials_without_session_entries_removes_nothing() {
    with_prompts(|fixture| {
        save(fixture, "smb://nas/a", &sam(CredentialScope::Permanent));
        remember_gnome_password(&fixture.keyring, "nas");
        let registry = SignOutRegistry::default();

        let signed_out = sign_out(
            fixture,
            &registry,
            request("smb://nas/a", ForgetScope::SessionOnly),
        );

        let report = signed_out.expect("sign out succeeds");
        assert!(!report.credentials_removed);
        assert!(fixture
            .keyring
            .contains(&own_entry("nas", CredentialScope::Permanent)));
        assert!(fixture.keyring.contains(&gnome_password("nas")));
    });
}

/// parity: NET-021
#[test]
fn without_a_keyring_the_server_is_disconnected_but_the_user_is_told() {
    with_memory_only_prompts(|fixture| {
        let registry = SignOutRegistry::default();

        let signed_out = sign_out(fixture, &registry, request("smb://nas/a", ForgetScope::AllScopes));

        let error = signed_out.expect_err("the credentials cannot be removed");
        assert!(matches!(
            error,
            NetworkError::CredentialsNotRemoved(KeyringError::Unavailable)
        ));
        assert!(
            error
                .to_string()
                .starts_with("Disconnected, but saved credentials could not be removed."),
            "{error}"
        );
        assert!(
            !registry.is_signing_out("smb://nas/a"),
            "a failed sign-out ends too"
        );
    });
}

/// A keyring that does not answer within the deadline, for example
/// because its unlock prompt was left open, ends the sign-out with an
/// error instead of keeping the server marked.
///
/// parity: NET-021
#[test]
fn a_keyring_that_does_not_answer_in_time_is_reported() {
    with_prompts(|fixture| {
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        fixture.keyring.set_clear_hook(move || {
            let _released = locked(&released).recv_timeout(Duration::from_secs(5));
        });
        let registry = SignOutRegistry::default();
        let signing_out = begin_sign_out(
            &fixture.prompts,
            &registry,
            request("smb://nas/a", ForgetScope::AllScopes),
        )
        .expect("the sign-out starts");

        let finished = fixture.block_on(finish_within(&signing_out, &[], Duration::from_millis(20)));

        let error = finished.expect_err("the keyring did not answer");
        assert!(
            matches!(error, NetworkError::CredentialsNotRemoved(KeyringError::TimedOut)),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Disconnected, but saved credentials could not be removed. The system keyring did not answer in \
             time."
        );
        drop(signing_out);
        assert!(!registry.is_signing_out("smb://nas/a"));
        release.send(()).expect("the keyring worker waits for release");
    });
}

struct RefusedSignOut {
    uri: &'static str,
    writes: WriteActivity,
    expected: &'static str,
}

/// Every precondition is checked before anything changes: the saved
/// entry, the window's memory and the registry stay as they were.
///
/// parity: NET-023
#[test]
fn sign_out_is_refused_during_writes_and_outside_smb_servers() {
    let cases = [
        RefusedSignOut {
            uri: "smb://nas/a",
            writes: WriteActivity::Writing,
            expected: "Finish active file operations in every OpenXplorer window before signing out.",
        },
        RefusedSignOut {
            uri: "file:///home/demo",
            writes: WriteActivity::Idle,
            expected: "Select an SMB location to sign out.",
        },
    ];
    with_prompts(|fixture| {
        save(fixture, "smb://nas/a", &sam(CredentialScope::Session));
        fixture
            .credentials
            .accept_memory("smb://nas/a", &sam(CredentialScope::Session));
        let registry = SignOutRegistry::default();
        for case in &cases {
            let request = SignOutRequest {
                uri: case.uri,
                forget: ForgetScope::AllScopes,
                writes: case.writes,
            };

            let refused = begin_sign_out(&fixture.prompts, &registry, request);

            let message = refused.expect_err("sign out is refused").to_string();
            assert_eq!(message, case.expected, "{}", case.uri);
            assert!(!registry.is_signing_out("smb://nas/a"), "{}", case.uri);
        }
        assert!(
            fixture
                .keyring
                .contains(&own_entry("nas", CredentialScope::Session)),
            "nothing was forgotten"
        );
        assert!(fixture.credentials.peek("smb://nas/a").is_some());
    });
}

/// parity: NET-023
#[test]
fn a_server_is_signed_out_once_at_a_time() {
    with_prompts(|fixture| {
        let registry = SignOutRegistry::default();
        let _running = registry.begin("nas").expect("the first sign-out starts");

        let refused = sign_out(fixture, &registry, request("smb://NAS/b", ForgetScope::AllScopes));

        let message = refused.expect_err("a second sign-out is refused").to_string();
        assert_eq!(message, "Sign-out is already in progress for this server.");
    });
}

/// The server stays marked while the window does its part and after the
/// credentials are gone, until the window drops the sign-out, so it can
/// clear the search cache before anything reconnects.
///
/// parity: NET-023
#[test]
fn the_server_stays_marked_until_the_sign_out_is_dropped() {
    with_prompts(|fixture| {
        let registry = SignOutRegistry::default();

        let signing_out = begin_sign_out(
            &fixture.prompts,
            &registry,
            request("smb://NAS/Projects", ForgetScope::AllScopes),
        )
        .expect("the sign-out starts");
        assert_eq!(signing_out.host(), "nas");
        assert!(registry.is_signing_out("smb://nas/other"));
        fixture
            .block_on(finish_sign_out(&signing_out, &[]))
            .expect("the sign-out finishes");
        assert!(registry.is_signing_out("smb://nas/other"));

        drop(signing_out);
        assert!(!registry.is_signing_out("smb://nas/other"));
    });
}

/// parity: NET-023
#[test]
fn a_server_being_signed_out_can_be_neither_listed_nor_connected() {
    with_prompts(|fixture| {
        let registry = SignOutRegistry::default();
        let running = registry.begin("nas").expect("the sign-out starts");

        let listing = registry.check_listing("smb://nas/Projects/Film");
        let connecting = fixture.block_on(connect_share(&fixture.prompts, &registry, r"\\nas\Projects", ""));

        let listing_error = listing.expect_err("listing is refused").to_string();
        assert_eq!(
            listing_error,
            "This server is being signed out. Reopen it after sign-out finishes."
        );
        let connecting_error = connecting.expect_err("connecting is refused").to_string();
        assert_eq!(
            connecting_error,
            "Sign-out is in progress. Reconnect after it finishes."
        );
        assert!(registry.check_listing("smb://other/Projects").is_ok());
        drop(running);
        assert!(registry.check_listing("smb://nas/Projects").is_ok());
    });
}

struct MountRootCase {
    root_uri: &'static str,
    is_disconnected: bool,
}

/// Sign out disconnects the SMB mounts of the server on every port, and
/// no other mount (`startswith('smb:')` and `hostname == host` in
/// `winspace.py`).
///
/// parity: NET-020
#[test]
fn only_the_servers_smb_mounts_are_disconnected() {
    let cases = [
        MountRootCase {
            root_uri: "smb://NAS/share/",
            is_disconnected: true,
        },
        MountRootCase {
            root_uri: "smb://nas:1445/x/",
            is_disconnected: true,
        },
        MountRootCase {
            root_uri: "smb://other/",
            is_disconnected: false,
        },
        MountRootCase {
            root_uri: "smb://nas.local/share/",
            is_disconnected: false,
        },
        MountRootCase {
            root_uri: "file:///mnt/nas",
            is_disconnected: false,
        },
        MountRootCase {
            root_uri: "sftp://nas/",
            is_disconnected: false,
        },
        MountRootCase {
            root_uri: "mtp://nas/",
            is_disconnected: false,
        },
    ];
    for case in &cases {
        assert_eq!(
            is_on_host(case.root_uri, "nas"),
            case.is_disconnected,
            "{}",
            case.root_uri
        );
    }
}
