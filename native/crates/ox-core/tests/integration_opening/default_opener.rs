// SPDX-License-Identifier: AGPL-3.0-only
//! Preparing a file for its default application: real temporary files
//! queried through GIO, applications from [`TestApplications`]. Ports
//! `prepare_default` of `desktop/native_opening.py`, which the Python app
//! had no unit tests for.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ox_core::entry::EntryError;
use ox_core::integration::{ApplicationDatabase, DefaultOpener, Launcher, OpenError, OpenTarget, Sandbox};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;
use tempfile::TempDir;

use super::{app, TestApplication, OWN_ID};

/// An application database with a fixed default and list, which records
/// every content type it is asked about.
#[derive(Debug, Clone, Default)]
struct TestApplications {
    default: Option<TestApplication>,
    registered: Vec<TestApplication>,
    asked_types: Arc<Mutex<Vec<String>>>,
}

impl TestApplications {
    fn asked_types(&self) -> Vec<String> {
        self.asked_types
            .lock()
            .expect("no panic while holding the list")
            .clone()
    }
}

impl ApplicationDatabase for TestApplications {
    type Application = TestApplication;

    fn default_for_type(&self, content_type: &str) -> Option<TestApplication> {
        self.asked_types
            .lock()
            .expect("list")
            .push(content_type.to_owned());
        self.default.clone()
    }

    fn all_for_type(&self, content_type: &str) -> Vec<TestApplication> {
        self.asked_types
            .lock()
            .expect("list")
            .push(content_type.to_owned());
        self.registered.clone()
    }
}

/// A temporary folder with one small PDF named `report.pdf`.
fn folder_with_report() -> (TempDir, PathBuf) {
    let folder = tempfile::tempdir().expect("temporary folder");
    let report = folder.path().join("report.pdf");
    fs::write(&report, "%PDF-1.4\n").expect("report");
    (folder, report)
}

/// A local-path lookup that finds nothing, as for an unmounted share.
fn no_local_path(_: &str) -> Option<PathBuf> {
    None
}

/// The local path of a `file://` URI, as the network service reports it.
fn local_file_path(uri: &str) -> Option<PathBuf> {
    gio::prelude::FileExt::path(&gio::File::for_uri(uri))
}

/// parity: OPEN-005, OPEN-006
#[test]
fn a_local_file_opens_by_its_path_in_the_default_application() {
    let (_folder, report) = folder_with_report();
    let applications = TestApplications {
        default: Some(app("evince.desktop", "Document Viewer")),
        ..TestApplications::default()
    };
    let opener = DefaultOpener::with_applications(local_file_path, applications.clone(), Sandbox::Host);

    let prepared = opener
        .prepare(&file_uri(&report), &Cancellation::new())
        .expect("prepared");

    let expected_launcher = Launcher::Application {
        id: "evince.desktop".to_owned(),
        name: "Document Viewer".to_owned(),
    };
    assert_eq!(prepared.launcher, expected_launcher);
    assert_eq!(prepared.target, OpenTarget::LocalPath(report));
    assert_eq!(prepared.entry.name, "report.pdf");
}

/// parity: OPEN-005
#[test]
fn the_application_is_looked_up_by_content_type_never_by_uri_scheme() {
    let (_folder, report) = folder_with_report();
    let applications = TestApplications {
        default: Some(app(OWN_ID, "OpenXplorer")),
        registered: vec![
            app(OWN_ID, "OpenXplorer"),
            app("evince.desktop", "Document Viewer"),
        ],
        ..TestApplications::default()
    };
    let opener = DefaultOpener::with_applications(local_file_path, applications.clone(), Sandbox::Host);

    let prepared = opener
        .prepare(&file_uri(&report), &Cancellation::new())
        .expect("prepared");

    assert!(matches!(&prepared.launcher, Launcher::Application { id, .. } if id == "evince.desktop"));
    let content_type = prepared.entry.content_type.clone().unwrap_or_default();
    let asked = applications.asked_types();
    assert!(!asked.is_empty());
    assert!(asked.iter().all(|asked| *asked == content_type), "{asked:?}");
    assert!(
        asked.iter().all(|asked| !asked.starts_with("x-scheme-handler/")),
        "{asked:?}"
    );
}

/// An executable script opens in the application registered for its
/// content type, a text editor, with its path; nothing ever runs it.
/// parity: OPEN-005, OPEN-007
#[test]
fn an_executable_script_opens_in_its_editor_and_is_never_run() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let script = folder.path().join("run.sh");
    let ran_marker = folder.path().join("ran");
    fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", ran_marker.display())).expect("script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
    let applications = TestApplications {
        default: Some(app("org.gnome.TextEditor.desktop", "Text Editor")),
        ..TestApplications::default()
    };
    let opener = DefaultOpener::with_applications(local_file_path, applications.clone(), Sandbox::Host);

    let prepared = opener
        .prepare(&file_uri(&script), &Cancellation::new())
        .expect("prepared");

    let expected_launcher = Launcher::Application {
        id: "org.gnome.TextEditor.desktop".to_owned(),
        name: "Text Editor".to_owned(),
    };
    assert_eq!(prepared.launcher, expected_launcher);
    assert_eq!(prepared.target, OpenTarget::LocalPath(script));
    let content_type = prepared.entry.content_type.clone().unwrap_or_default();
    // shared-mime-info 2.5.1 and later name shell scripts text/x-shellscript
    // with application/x-shellscript as its alias; older releases name them
    // application/x-shellscript (alias text/x-sh).
    assert!(
        matches!(
            content_type.as_str(),
            "application/x-shellscript" | "text/x-shellscript"
        ),
        "{content_type:?}"
    );
    // The lookup asks for the detected type as it is: GIO resolves either
    // name to the applications registered under both.
    let asked = applications.asked_types();
    assert!(!asked.is_empty());
    assert!(asked.iter().all(|asked| *asked == content_type), "{asked:?}");
    assert!(!ran_marker.exists());
}

/// parity: OPEN-005
#[test]
fn a_folder_is_never_launched() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let opener =
        DefaultOpener::with_applications(local_file_path, TestApplications::default(), Sandbox::Host);

    let refused = opener.prepare(&file_uri(folder.path()), &Cancellation::new());

    assert_eq!(refused, Err(OpenError::IsFolder));
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "This item is a folder."
    );
}

/// parity: OPEN-005
#[test]
fn a_special_file_is_never_launched() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let fifo = folder.path().join("pipe");
    let created = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(created.success());
    let opener =
        DefaultOpener::with_applications(local_file_path, TestApplications::default(), Sandbox::Host);

    let refused = opener.prepare(&file_uri(&fifo), &Cancellation::new());

    assert_eq!(refused, Err(OpenError::SpecialObject));
}

/// parity: OPEN-006
#[test]
fn without_a_local_path_only_an_application_that_reads_uris_opens_the_file() {
    let (_folder, report) = folder_with_report();
    let path_only = TestApplications {
        default: Some(app("evince.desktop", "Document Viewer")),
        ..TestApplications::default()
    };
    let uri_capable = TestApplications {
        default: Some(TestApplication {
            accepts_uris: true,
            ..app("org.gnome.Papers.desktop", "Papers")
        }),
        ..TestApplications::default()
    };
    let uri = file_uri(&report);

    let refused = DefaultOpener::with_applications(no_local_path, path_only, Sandbox::Host)
        .prepare(&uri, &Cancellation::new());
    let prepared = DefaultOpener::with_applications(no_local_path, uri_capable, Sandbox::Host)
        .prepare(&uri, &Cancellation::new())
        .expect("prepared");

    assert_eq!(refused, Err(OpenError::NeedsLocalPath));
    assert!(refused
        .expect_err("refused")
        .to_string()
        .contains("Install gvfs-fuse"));
    assert_eq!(prepared.target, OpenTarget::Uri(uri));
}

/// parity: OPEN-006
#[test]
fn inside_flatpak_a_file_opens_through_the_portal_by_its_path_only() {
    let (_folder, report) = folder_with_report();
    let uri = file_uri(&report);
    let with_path =
        DefaultOpener::with_applications(local_file_path, TestApplications::default(), Sandbox::Flatpak);
    let without_path =
        DefaultOpener::with_applications(no_local_path, TestApplications::default(), Sandbox::Flatpak);

    let prepared = with_path.prepare(&uri, &Cancellation::new()).expect("prepared");
    let refused = without_path.prepare(&uri, &Cancellation::new());

    assert_eq!(prepared.launcher, Launcher::DesktopPortal);
    assert_eq!(prepared.target, OpenTarget::LocalPath(report));
    assert_eq!(refused, Err(OpenError::NeedsLocalPath));
}

/// parity: OPEN-005
#[test]
fn preparing_runs_on_a_worker_thread_and_can_be_cancelled() {
    let (_folder, report) = folder_with_report();
    let applications = TestApplications {
        default: Some(app("evince.desktop", "Document Viewer")),
        ..TestApplications::default()
    };
    let opener = DefaultOpener::with_applications(local_file_path, applications, Sandbox::Host);
    let context = glib::MainContext::new();
    let cancelled = Cancellation::new();
    cancelled.cancel();

    let prepared = context.block_on(opener.prepare_in_background(file_uri(&report), Cancellation::new()));
    let refused = context.block_on(opener.prepare_in_background(file_uri(&report), cancelled));

    assert!(prepared.is_ok(), "{prepared:?}");
    assert_eq!(refused, Err(OpenError::Entry(EntryError::Cancelled)));
}
