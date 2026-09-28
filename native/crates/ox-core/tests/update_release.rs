// SPDX-License-Identifier: AGPL-3.0-only
//! Release metadata, the URL trust boundary and installation detection.
//! Ports `MetadataTests` and `UrlTests` of `desktop/tests/test_updater.py`;
//! `update_python.rs` runs the same functions through both apps.

mod update_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use ox_core::update::{
    parse_release, Installation, ReleaseVersion, TrustedUrl, UpdateError, LATEST_RELEASE_URL,
    MAX_INSTALLER_SIZE, RELEASE_REPOSITORIES, REPOSITORY, TRUSTED_HOSTS,
};
use serde_json::{json, Value};
use update_support::{next_release, release_answer, CURRENT, NEXT};

/// The fixture release with its installer asset changed by `change`.
fn with_asset(change: impl FnOnce(&mut Value)) -> Value {
    let mut release = next_release();
    change(&mut release["assets"][0]);
    release
}

fn version(text: &str) -> ReleaseVersion {
    text.parse().unwrap()
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_stable_versions_are_compared_numerically`
/// parity: UPD-002
#[test]
fn stable_versions_are_compared_numerically() {
    assert_eq!(version("1.10.0"), ReleaseVersion::new(1, 10, 0));
    assert!(version("1.10.0") > version("1.9.9"));

    let newer = parse_release(&release_answer(version("1.10.0"), REPOSITORY), version("1.9.9")).unwrap();
    let same = parse_release(&release_answer(CURRENT, REPOSITORY), CURRENT).unwrap();
    let older = parse_release(&release_answer(CURRENT, REPOSITORY), NEXT).unwrap();

    assert!(newer.is_newer);
    assert!(!same.is_newer);
    assert!(!older.is_newer);
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_versions_reject_prereleases_path_fragments_and_shell_content`
///
/// Python's `None` and `1` cases cannot be written: a version is parsed
/// from text.
/// parity: UPD-002
#[test]
fn versions_reject_prereleases_path_fragments_and_shell_content() {
    let refused = [
        "v1.0.1",
        "01.0.1",
        "1.0",
        "1.0.1-rc1",
        "1.0.1/../../evil",
        "1.0.1;touch /tmp/example",
        "1.0.1\n",
        "",
        " 1.0.1",
        "1.0.1.2",
        "1.+0.1",
        "1.٠.1",
    ];
    for text in refused {
        let parsed = text.parse::<ReleaseVersion>();

        assert!(matches!(parsed, Err(UpdateError::UnsupportedVersion)), "{text:?}");
    }
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_only_stable_public_release_records_are_used`
/// parity: UPD-002
#[test]
fn only_stable_public_release_records_are_used() {
    let not_stable = [
        json!(null),
        json!([]),
        json!({"draft": true}),
        json!({"prerelease": true}),
    ];
    for answer in not_stable {
        let result = parse_release(&answer, CURRENT);

        assert!(matches!(result, Err(UpdateError::NoStableRelease)), "{answer}");
    }
    let bad_tags = [
        json!("1.0.1"),
        json!("v../../evil"),
        json!("v1.0.1-rc1"),
        json!(null),
        json!(123),
    ];
    for tag in bad_tags {
        let mut answer = next_release();
        answer["tag_name"] = tag.clone();

        let result = parse_release(&answer, CURRENT);

        assert!(
            matches!(
                result,
                Err(UpdateError::InvalidTag | UpdateError::UnsupportedVersion)
            ),
            "{tag}"
        );
    }
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_installer_must_have_exact_upstream_name_and_url`
/// parity: UPD-002
#[test]
fn installer_must_have_exact_upstream_name_and_url() {
    for name in ["../../example.deb", "/tmp/example.deb", "other.deb"] {
        let answer = with_asset(|asset| asset["name"] = json!(name));

        let result = parse_release(&answer, CURRENT);

        assert!(matches!(result, Err(UpdateError::MissingInstaller)), "{name}");
    }
    let expected = next_release()["assets"][0]["browser_download_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let bad_urls = [
        "https://example.invalid/package.deb".to_owned(),
        "https://github.com/other/project/releases/download/v1.0.1/openxplorer_1.0.1_all.deb".to_owned(),
        "https://github.com/openxplorer/other/releases/download/v1.0.1/openxplorer_1.0.1_all.deb".to_owned(),
        "https://github.com/AKolenda/openxplorer/releases/download/v1.0.2/openxplorer_1.0.1_all.deb"
            .to_owned(),
        format!("{expected}?path=elsewhere"),
        "file:///tmp/example.deb".to_owned(),
    ];
    for url in bad_urls {
        let answer = with_asset(|asset| asset["browser_download_url"] = json!(url));

        let result = parse_release(&answer, CURRENT);

        assert!(matches!(result, Err(UpdateError::MissingInstaller)), "{url}");
    }
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_installer_size_and_digest_are_required`
/// parity: UPD-002
#[test]
fn installer_size_and_digest_are_required() {
    let package_size = update_support::PACKAGE.len().to_string();
    let bad_sizes = [
        json!(null),
        json!(false),
        json!(true),
        json!(0),
        json!(-1),
        json!(MAX_INSTALLER_SIZE + 1),
        json!(package_size),
        json!(61.0),
    ];
    for size in bad_sizes {
        let answer = with_asset(|asset| asset["size"] = size.clone());

        let result = parse_release(&answer, CURRENT);

        assert!(matches!(result, Err(UpdateError::InvalidInstallerSize)), "{size}");
    }
    let bad_digests = [
        json!(null),
        json!(""),
        json!(123),
        json!({"sha256": "a".repeat(64)}),
        json!("sha256:no"),
        json!(format!("md5:{}", "a".repeat(32))),
        json!(format!("sha256:{}", "A".repeat(64))),
    ];
    for digest in bad_digests {
        let answer = with_asset(|asset| asset["digest"] = digest.clone());

        let result = parse_release(&answer, CURRENT);

        assert!(matches!(result, Err(UpdateError::MissingDigest)), "{digest}");
    }
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_malformed_asset_list_is_rejected_with_actionable_error`
/// parity: UPD-002
#[test]
fn malformed_asset_list_is_rejected_with_actionable_error() {
    for assets in [json!(null), json!({}), json!("invalid"), json!(1)] {
        let mut answer = next_release();
        answer["assets"] = assets.clone();

        let error = parse_release(&answer, CURRENT).unwrap_err();

        assert!(error.to_string().contains("asset list"), "{assets}: {error}");
    }
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_notes_and_release_link_are_bounded_or_derived_locally`
/// parity: UPD-002
#[test]
fn notes_and_release_link_are_bounded_or_derived_locally() {
    let mut answer = next_release();
    answer["body"] = json!("x".repeat(25_000));
    answer["html_url"] = json!("https://example.invalid/untrusted");

    let release = parse_release(&answer, CURRENT).unwrap();

    assert_eq!(release.notes.chars().count(), 20_000);
    assert_eq!(release.release_url, format!("{REPOSITORY}/releases/tag/v{NEXT}"));
}

/// Ported from `desktop/tests/test_updater.py::MetadataTests::test_releases_from_the_organization_repository_are_accepted`
/// parity: UPD-002
#[test]
fn releases_from_the_organization_repository_are_accepted() {
    assert_eq!(
        RELEASE_REPOSITORIES,
        [REPOSITORY, "https://github.com/openxplorer/openxplorer"]
    );
    let answer = release_answer(NEXT, RELEASE_REPOSITORIES[1]);

    let release = parse_release(&answer, CURRENT).unwrap();

    let published = answer["assets"][0]["browser_download_url"].as_str().unwrap();
    assert_eq!(release.installer.url.as_str(), published);
    assert_eq!(
        release.release_url,
        format!("{}/releases/tag/v{NEXT}", RELEASE_REPOSITORIES[1])
    );
}

/// The installer's digest and size are the release's, for the download
/// check.
/// parity: UPD-002
#[test]
fn a_release_carries_its_installer_digest_and_size() {
    let release = parse_release(&next_release(), CURRENT).unwrap();

    let digest =
        glib::compute_checksum_for_data(glib::ChecksumType::Sha256, update_support::PACKAGE).unwrap();
    assert_eq!(release.installer.sha256.as_str(), digest.as_str());
    assert_eq!(release.installer.size, update_support::PACKAGE.len() as u64);
    assert_eq!(release.installer.name, "openxplorer_1.0.1_all.deb");
}

/// Ported from `desktop/tests/test_updater.py::UrlTests::test_only_allowlisted_https_hosts_and_standard_ports_are_accepted`
/// parity: UPD-002
#[test]
fn only_allowlisted_https_hosts_and_standard_ports_are_accepted() {
    for host in TRUSTED_HOSTS {
        let url = format!("https://{host}/fixture");

        assert_eq!(TrustedUrl::parse(&url).unwrap().as_str(), url);
    }
    assert!(TrustedUrl::parse("https://github.com:443/fixture").is_ok());
    let untrusted = [
        "http://github.com/fixture",
        "file:///tmp/example",
        "https://github.com.example.invalid/fixture",
        "https://user@github.com/fixture",
        "https://user:secret@github.com/fixture",
        "https://github.com:444/fixture",
        "https://example.invalid/fixture",
        "not a URL",
    ];
    for url in untrusted {
        let result = TrustedUrl::parse(url);

        assert!(matches!(result, Err(UpdateError::UntrustedLocation)), "{url}");
    }
}

/// Ported from `desktop/tests/test_updater.py::UrlTests::test_redirects_apply_the_same_trust_boundary`
/// parity: UPD-002
#[test]
fn redirects_apply_the_same_trust_boundary() {
    let latest = TrustedUrl::latest_release();
    let target = "https://release-assets.githubusercontent.com/fixture";

    assert_eq!(latest.as_str(), LATEST_RELEASE_URL);
    assert_eq!(latest.redirect(target).unwrap().as_str(), target);
    let refused = latest.redirect("https://example.invalid/fixture");
    assert!(matches!(refused, Err(UpdateError::UntrustedLocation)));
    assert!(latest.redirect("http://github.com/fixture").is_err());
}

/// Makes a fake system in `root` with the programs an installation needs,
/// executable when `mode` allows it.
fn install_tools(root: &Path, mode: u32) {
    let bin = root.join("usr/bin");
    fs::create_dir_all(&bin).unwrap();
    for tool in ["pkexec", "apt-get", "dpkg-deb", "openxplorer"] {
        let path = bin.join(tool);
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    }
}

/// Ported from `desktop/tests/test_updater.py::UrlTests::test_install_capability_requires_exact_system_install_and_executable_tools`
///
/// Python patches `Path.is_file` and `os.access`; here the tools are real
/// files in a fake file-system root.
/// parity: UPD-004
#[test]
fn install_capability_requires_exact_system_install_and_executable_tools() {
    let system = tempfile::tempdir().unwrap();
    let package_root = system.path().join("opt/openxplorer");
    install_tools(system.path(), 0o755);

    assert_eq!(
        Installation::detect(&package_root, system.path()),
        Installation::DebianPackage
    );
    let source_build = Installation::detect(Path::new("/home/demo/openxplorer"), system.path());
    assert_eq!(source_build, Installation::Unpackaged);
    assert!(!source_build.can_install());

    install_tools(system.path(), 0o644);
    let without_tools = Installation::detect(&package_root, system.path());
    assert_eq!(without_tools, Installation::DebianPackageWithoutTools);
    assert!(!without_tools.can_install());
}

/// A Flatpak and other system packages are updated by their own package
/// manager, never by the app.
/// parity: UPD-004
#[test]
fn flatpaks_and_other_packages_update_through_their_package_manager() {
    let system = tempfile::tempdir().unwrap();
    install_tools(system.path(), 0o755);
    let package_root = system.path().join("opt/openxplorer");
    let other_package = system.path().join("usr/lib/openxplorer");

    assert_eq!(
        Installation::detect(&other_package, system.path()),
        Installation::OtherPackage
    );
    fs::write(system.path().join(".flatpak-info"), "[Application]\n").unwrap();
    let flatpak = Installation::detect(&package_root, system.path());

    assert_eq!(flatpak, Installation::Flatpak);
    assert!(!flatpak.can_install());
    assert!(!Installation::OtherPackage.can_install());
}
