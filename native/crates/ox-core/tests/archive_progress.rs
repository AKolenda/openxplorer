// SPDX-License-Identifier: AGPL-3.0-only
//! Compressing and extracting report their progress the way a copy does:
//! bytes processed against the whole job, items done out of all of them,
//! and the item's name, so the progress window can show the bar, the
//! speed, the time left and the items remaining (OPS-021). A ZIP is
//! written under its final name, as Explorer writes it, and removed again
//! when the compression stops (ARC-023).

mod archive_support;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ox_core::archive::{
    ArchiveError, CompressionRequest, ExtractionRequest, GioExtractionOutput, ZipCompressor, ZipExtractor,
};
use ox_core::transfer::{Cancellation, Progress};

use archive_support::{file_uri, gio_factory, incompressible_bytes, opener, ExtractionFixture, TestMember};

/// The single large file of these tests, fictional.
const FILE_NAME: &str = "harbour-survey.iso";
/// Large enough for many 64 KiB blocks.
const FILE_SIZE: usize = 3 * 1024 * 1024;

/// Every report a job sends.
type Reports = Arc<Mutex<Vec<Progress>>>;

/// A folder with one large file that does not compress.
fn large_file(root: &Path) -> String {
    let path = root.join(FILE_NAME);
    fs::write(&path, incompressible_bytes(FILE_SIZE)).unwrap();
    file_uri(&path)
}

fn compression(root: &Path, uris: Vec<String>, name: &str) -> CompressionRequest {
    CompressionRequest {
        uris,
        destination_uri: file_uri(root),
        archive_name: name.to_owned(),
    }
}

fn names_in(root: &Path) -> Vec<String> {
    let items = fs::read_dir(root).unwrap().filter_map(Result::ok);
    items
        .map(|item| item.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Asserts that `reports` move a progress window smoothly through one
/// large file: many byte reports, rising, against the whole job's size,
/// naming the file and counting it as the one item.
fn assert_smooth_byte_progress(reports: &[Progress]) {
    let total = u64::try_from(FILE_SIZE).unwrap();
    let byte_reports: Vec<&Progress> = reports.iter().filter(|report| report.bytes.is_some()).collect();
    assert!(
        byte_reports.len() >= 10,
        "a large file is reported block by block, not once: {} byte reports",
        byte_reports.len()
    );
    let mut previous = 0;
    let mut shares = Vec::new();
    for report in &byte_reports {
        let bytes = report.bytes.unwrap();
        assert_eq!(bytes.batch_size, Some(total), "the job's size, for the time left");
        assert!(bytes.batch_written >= previous, "bytes never go back");
        previous = bytes.batch_written;
        assert_eq!(bytes.file_size, total);
        assert_eq!(
            report.name.as_deref(),
            Some(FILE_NAME),
            "the progress window's Name"
        );
        let items = report.items.expect("the progress window's Items remaining");
        assert_eq!(items.total, 1);
        assert!(items.done <= 1);
        #[expect(clippy::cast_precision_loss, reason = "a share for a progress bar")]
        shares.push(bytes.batch_written as f64 / total as f64);
    }
    assert_eq!(previous, total, "every byte is counted once");
    let between: Vec<f64> = shares
        .iter()
        .copied()
        .filter(|share| *share > 0.0 && *share < 1.0)
        .collect();
    assert!(
        between.len() >= 5 && between.windows(2).all(|pair| pair[0] < pair[1]),
        "the bar rises between 0 and 100%, not in one jump: {shares:?}"
    );
}

/// parity: ARC-023, OPS-021
#[test]
fn compressing_one_large_file_reports_rising_bytes_name_and_items() {
    let root = tempfile::tempdir().unwrap();
    let uris = vec![large_file(root.path())];
    let reports: Reports = Arc::default();
    let collected = Arc::clone(&reports);
    let mut compressor =
        ZipCompressor::new().with_progress(move |progress| collected.lock().unwrap().push(progress));

    compressor
        .compress(
            &compression(root.path(), uris, "harbour-survey.zip"),
            &Cancellation::new(),
        )
        .unwrap();

    let reports = reports.lock().unwrap();
    assert_smooth_byte_progress(&reports);
    let last = reports.last().unwrap();
    assert!((last.fraction - 1.0).abs() < f64::EPSILON);
    assert_eq!(
        last.items.map(|items| items.done),
        Some(1),
        "no item remains at the end"
    );
}

/// parity: ARC-013, OPS-021
#[test]
fn extracting_one_large_file_reports_rising_bytes_name_and_items() {
    let root = tempfile::tempdir().unwrap();
    let uris = vec![large_file(root.path())];
    ZipCompressor::new()
        .compress(
            &compression(root.path(), uris, "harbour-survey.zip"),
            &Cancellation::new(),
        )
        .unwrap();
    let destination = root.path().join("unpacked");
    fs::create_dir(&destination).unwrap();
    let reports: Reports = Arc::default();
    let collected = Arc::clone(&reports);
    let mut extractor = ZipExtractor::new(opener(), gio_factory(), Arc::new(GioExtractionOutput))
        .with_progress(move |progress| collected.lock().unwrap().push(progress));

    extractor
        .extract(
            &ExtractionRequest {
                archive_uri: file_uri(&root.path().join("harbour-survey.zip")),
                destination_uri: file_uri(&destination),
                folder_name: "harbour-survey".to_owned(),
            },
            &Cancellation::new(),
        )
        .unwrap();

    let reports = reports.lock().unwrap();
    assert_smooth_byte_progress(&reports);
    let last = reports.last().unwrap();
    assert!((last.fraction - 1.0).abs() < f64::EPSILON);
    assert_eq!(
        last.items.map(|items| items.done),
        Some(1),
        "no item remains at the end"
    );
}

/// Explorer creates the ZIP under its final name at once and it grows
/// there; a cancelled compression removes it, and never anything another
/// program put under that name.
///
/// parity: ARC-023
#[test]
fn the_zip_grows_under_its_final_name_and_a_cancel_removes_it() {
    let root = tempfile::tempdir().unwrap();
    let uris = vec![large_file(root.path())];
    let cancel = Cancellation::new();
    let seen: Arc<Mutex<Option<Vec<String>>>> = Arc::default();
    let (watched, observed, cancelling) = (root.path().to_owned(), Arc::clone(&seen), cancel.clone());
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        let halfway = progress
            .bytes
            .is_some_and(|bytes| bytes.file_written >= u64::try_from(FILE_SIZE / 2).unwrap());
        if halfway && observed.lock().unwrap().is_none() {
            *observed.lock().unwrap() = Some(names_in(&watched));
            cancelling.cancel();
        }
    });

    let stopped = compressor.compress(&compression(root.path(), uris, "harbour-survey.zip"), &cancel);

    let mut while_writing = seen
        .lock()
        .unwrap()
        .clone()
        .expect("reports while the file was written");
    while_writing.sort();
    assert_eq!(
        while_writing,
        ["harbour-survey.iso", "harbour-survey.zip"],
        "only the final name, no hidden temporary file"
    );
    assert_eq!(stopped, Err(ArchiveError::Cancelled));
    assert_eq!(names_in(root.path()), [FILE_NAME], "the partial ZIP is removed");
}

/// A name another program takes after the check is never replaced, and a
/// ZIP another program puts in place of the partial one is left alone
/// when the compression then stops.
///
/// parity: ARC-023
#[test]
fn a_compression_never_replaces_or_removes_another_programs_file() {
    let root = tempfile::tempdir().unwrap();
    let uris = vec![large_file(root.path())];
    let taken = root.path().join("harbour-survey.zip");
    let claimed = taken.clone();
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        if progress.label.starts_with("Preparing") {
            fs::write(&claimed, b"another program's file").unwrap();
        }
    });

    let refused = compressor.compress(
        &compression(root.path(), uris.clone(), "harbour-survey.zip"),
        &Cancellation::new(),
    );

    assert_eq!(refused, Err(ArchiveError::ArchiveExists));
    assert_eq!(fs::read(&taken).unwrap(), b"another program's file");

    fs::remove_file(&taken).unwrap();
    let cancel = Cancellation::new();
    let (replaced, cancelling) = (taken.clone(), cancel.clone());
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        if progress.bytes.is_some_and(|bytes| bytes.file_written > 0) && !cancelling.is_cancelled() {
            if replaced.exists() {
                fs::remove_file(&replaced).unwrap();
            }
            fs::write(&replaced, b"a newer file").unwrap();
            cancelling.cancel();
        }
    });

    let stopped = compressor.compress(&compression(root.path(), uris, "harbour-survey.zip"), &cancel);

    assert!(stopped.is_err());
    assert_eq!(fs::read(&taken).unwrap(), b"a newer file");
}

/// A ZIP whose archive is moved away and replaced by another program's
/// file while it is written is not reported as created: the item under
/// the name is not the archive. Neither file is removed.
///
/// parity: ARC-023
#[test]
fn a_zip_moved_and_replaced_while_written_is_not_reported_as_created() {
    let root = tempfile::tempdir().unwrap();
    let uris = vec![large_file(root.path())];
    let (folder, moved) = (root.path().to_owned(), Arc::new(Mutex::new(false)));
    let once = Arc::clone(&moved);
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        let halfway = progress
            .bytes
            .is_some_and(|bytes| bytes.file_written >= u64::try_from(FILE_SIZE / 2).unwrap());
        if halfway && !std::mem::replace(&mut *once.lock().unwrap(), true) {
            fs::rename(folder.join("harbour-survey.zip"), folder.join("moved-away.zip")).unwrap();
            fs::write(folder.join("harbour-survey.zip"), b"another program's file").unwrap();
        }
    });

    let result = compressor.compress(
        &compression(root.path(), uris, "harbour-survey.zip"),
        &Cancellation::new(),
    );

    assert!(
        *moved.lock().unwrap(),
        "the archive was moved while it was written"
    );
    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        fs::read(root.path().join("harbour-survey.zip")).unwrap(),
        b"another program's file"
    );
    assert!(root.path().join("moved-away.zip").exists());
}

/// When a stopped compression cannot even look up its partial ZIP, it
/// says where the ZIP may remain instead of claiming nothing was left.
/// Skipped as root, which reads any folder.
///
/// parity: ARC-023
#[test]
fn a_partial_zip_that_cannot_be_looked_up_is_reported_where_it_is() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sources");
    fs::create_dir(&source).unwrap();
    let uris = vec![large_file(&source)];
    let destination = root.path().join("Archives");
    fs::create_dir(&destination).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o000)).unwrap();
    let as_root = fs::read_dir(&destination).is_ok();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o755)).unwrap();
    if as_root {
        return;
    }
    let cancel = Cancellation::new();
    let (locked, cancelling) = (destination.clone(), cancel.clone());
    let mut compressor = ZipCompressor::new().with_progress(move |progress| {
        if progress.bytes.is_some_and(|bytes| bytes.file_written > 0) && !cancelling.is_cancelled() {
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            cancelling.cancel();
        }
    });

    let stopped = compressor
        .compress(
            &CompressionRequest {
                uris,
                destination_uri: file_uri(&destination),
                archive_name: "harbour-survey.zip".to_owned(),
            },
            &cancel,
        )
        .unwrap_err();

    fs::set_permissions(&destination, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(stopped, ArchiveError::ArchiveLeftBehind { .. }),
        "{stopped:?}"
    );
    assert!(stopped.to_string().contains("harbour-survey.zip"), "{stopped}");
    assert!(destination.join("harbour-survey.zip").exists());
}

/// A ZIP of empty files moves the progress window file by file rather
/// than from "Checking" straight to the end.
///
/// parity: ARC-013, OPS-021
#[test]
fn extracting_empty_files_reports_each_file() {
    let fixture = ExtractionFixture::new();
    let members: Vec<TestMember> = (1..=3)
        .map(|number| TestMember::file(&format!("empty-{number}.txt"), b""))
        .collect();
    fixture.write_zip(&members);

    fixture.extract("Unpacked").expect("the empty files extract");

    let reports = fixture.events();
    let names: Vec<Option<String>> = reports
        .iter()
        .filter(|report| report.bytes.is_some())
        .map(|report| report.name.clone())
        .collect();
    assert_eq!(
        names,
        ["empty-1.txt", "empty-2.txt", "empty-3.txt"].map(|name| Some(name.to_owned())),
        "one report per file, each naming it"
    );
    let done: Vec<u64> = reports
        .iter()
        .filter_map(|report| report.items.map(|items| items.done))
        .collect();
    assert_eq!(done, [0, 1, 2, 3], "the items remaining count down");
}
