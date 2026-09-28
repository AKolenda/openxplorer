// SPDX-License-Identifier: AGPL-3.0-only
//! Sign out of a server, against an in-memory keyring and without mounts:
//! the isolated test session has no SMB server to disconnect.

use super::*;
use crate::network::credential::{Credential, CredentialScope, Password};
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
    SecretAttributes::for_schema(crate::network::SMB_CREDENTIAL_SCHEMA)
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

        let signed_out = fixture.block_on(sign_out(
            &fixture.prompts,
            &registry,
            &[],
            request("smb://NAS/Projects", ForgetScope::AllScopes),
        ));

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
        remember_gnome_password(&fixture.keyring, "nas");
        let registry = SignOutRegistry::default();

        let signed_out = fixture.block_on(sign_out(
            &fixture.prompts,
            &registry,
            &[],
            request("smb://nas/a", ForgetScope::SessionOnly),
        ));

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

        let signed_out = fixture.block_on(sign_out(
            &fixture.prompts,
            &registry,
            &[],
            request("smb://nas/a", ForgetScope::AllScopes),
        ));

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

struct RefusedSignOut {
    uri: &'static str,
    writes: WriteActivity,
    expected: &'static str,
}

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
        let registry = SignOutRegistry::default();
        for case in &cases {
            let request = SignOutRequest {
                uri: case.uri,
                forget: ForgetScope::AllScopes,
                writes: case.writes,
            };

            let refused = fixture.block_on(sign_out(&fixture.prompts, &registry, &[], request));

            let message = refused.expect_err("sign out is refused").to_string();
            assert_eq!(message, case.expected, "{}", case.uri);
        }
        assert!(
            fixture
                .keyring
                .contains(&own_entry("nas", CredentialScope::Session)),
            "nothing was forgotten"
        );
    });
}

/// parity: NET-023
#[test]
fn a_server_is_signed_out_once_at_a_time() {
    with_prompts(|fixture| {
        let registry = SignOutRegistry::default();
        let _running = registry.begin("nas").expect("the first sign-out starts");

        let refused = fixture.block_on(sign_out(
            &fixture.prompts,
            &registry,
            &[],
            request("smb://NAS/b", ForgetScope::AllScopes),
        ));

        let message = refused.expect_err("a second sign-out is refused").to_string();
        assert_eq!(message, "Sign-out is already in progress for this server.");
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
