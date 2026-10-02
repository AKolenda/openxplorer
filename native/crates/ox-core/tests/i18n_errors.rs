// SPDX-License-Identifier: AGPL-3.0-only
//! A real installed catalogue translates backend errors without changing their data.

use std::process::Command;

use ox_core::archive::ArchiveError;
use ox_core::network::NetworkError;
use ox_core::transfer::TransferError;
use ox_core::update::UpdateError;

/// parity: INT-031
#[test]
fn backend_errors_use_the_desktop_catalogue_and_preserve_inserted_paths() {
    if std::env::var_os("OX_I18N_ERROR_CHILD").is_some() {
        assert!(ox_core::i18n::install(), "the requested catalogue is installed");
        assert_eq!(NetworkError::Cancelled.to_string(), "Test cancellation");
        assert_eq!(
            UpdateError::CheckRefused { status: 403 }.to_string(),
            "Test status 403: update unavailable"
        );
        let error = ArchiveError::StagingLeftBehind {
            cause: Box::new(ArchiveError::Cancelled),
            staging_uri: "file:///tmp/{cleanup}.part".to_owned(),
            cleanup: TransferError::failed("raw backend detail"),
        };
        assert_eq!(
            error.to_string(),
            "Test path file:///tmp/{cleanup}.part: Test cancellation; raw backend detail"
        );
        return;
    }
    assert!(
        std::env::var_os("OX_ISOLATED_SESSION").is_some(),
        "run this test through native/tools/check.py's isolated environment"
    );
    let temporary = tempfile::tempdir().expect("temporary catalogue data");
    let po = temporary.path().join("fr.po");
    std::fs::write(
        &po,
        r#"msgid ""
msgstr "Content-Type: text/plain; charset=UTF-8\n"

msgid "Operation cancelled."
msgstr "Test cancellation"

msgid "GitHub could not check for updates (HTTP {status}). Try again later."
msgstr "Test status {status}: update unavailable"

msgid "{cause}\nIncomplete extraction remains at {staging_uri}. Inspect it before removing it. {cleanup}"
msgstr "Test path {staging_uri}: {cause}; {cleanup}"
"#,
    )
    .expect("test-only PO catalogue");
    let mo = temporary.path().join("locale/fr/LC_MESSAGES/openxplorer.mo");
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/i18n.py");
    let compiled = Command::new("python3")
        .args([
            compiler.as_os_str(),
            "compile".as_ref(),
            po.as_os_str(),
            mo.as_os_str(),
        ])
        .output()
        .expect("catalogue compiler");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let child = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "backend_errors_use_the_desktop_catalogue_and_preserve_inserted_paths",
            "--test-threads=1",
        ])
        .env("OX_I18N_ERROR_CHILD", "1")
        .env("XDG_DATA_HOME", temporary.path())
        .env("XDG_DATA_DIRS", temporary.path())
        .env("LANGUAGE", "fr")
        .output()
        .expect("test with a fresh locale cache");
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}
