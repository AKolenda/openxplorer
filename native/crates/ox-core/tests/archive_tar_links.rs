// SPDX-License-Identifier: AGPL-3.0-only
//! Symbolic links in TAR archives (ARC-016, ARC-024): a relative link
//! whose target stays inside the extracted folder is extracted as a link,
//! as `tar` extracts it; an absolute link, a link that leaves the folder
//! and every other special file still stop the extraction before anything
//! is written. Compress to › TAR File writes a plain TAR that the archive
//! browser reads back (ARC-023).

mod archive_support;

use std::fs;
use std::path::Path;
use std::process::Command;

use ox_core::archive::{ArchiveBrowser, ArchiveError, CompressionRequest, ZipCompressor};
use ox_core::transfer::Cancellation;

use archive_support::{file_uri, opener, ExtractionFixture};

/// Writes a `.tar.gz` at `path` from `entries`, each `kind:name[:value]`:
/// `f` a file holding `value`, `d` a folder, `l` a symbolic link to
/// `value`, `h` a hard link to `value`, `p` a FIFO.
fn write_tar(path: &Path, entries: &[&str]) {
    let script = "import io, sys, tarfile\n\
                  kinds = {'f': tarfile.REGTYPE, 'd': tarfile.DIRTYPE, 'l': tarfile.SYMTYPE,\n\
                  \x20        'h': tarfile.LNKTYPE, 'p': tarfile.FIFOTYPE}\n\
                  with tarfile.open(sys.argv[1], 'w:gz', format=tarfile.PAX_FORMAT) as archive:\n\
                  \x20   for entry in sys.argv[2:]:\n\
                  \x20       kind, name, value = (entry.split(':', 2) + [''])[:3]\n\
                  \x20       member = tarfile.TarInfo(name); member.type = kinds[kind]\n\
                  \x20       if kind == 'f':\n\
                  \x20           data = value.encode(); member.size = len(data)\n\
                  \x20           archive.addfile(member, io.BytesIO(data))\n\
                  \x20       else:\n\
                  \x20           member.linkname = value; archive.addfile(member)\n";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(path)
        .args(entries)
        .output()
        .expect("Python 3 is required to write reference archives");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The fixture's archive written from `entries`.
fn fixture_with(entries: &[&str]) -> ExtractionFixture {
    let fixture = ExtractionFixture::new();
    write_tar(&fixture.archive, entries);
    fixture
}

/// A library and its relative `libx.so -> libx.so.1` link extract as a
/// file and a link to it, as `tar` extracts them.
///
/// parity: ARC-016, ARC-024
#[test]
fn a_relative_link_inside_the_folder_extracts_as_a_link() {
    let fixture = fixture_with(&["f:libx.so.1:library", "l:libx.so:libx.so.1"]);

    let extracted = fixture.extract("Links").expect("the TAR extracts");

    let link = fixture.destination.join("Links/libx.so");
    let metadata = fs::symlink_metadata(&link).expect("the link exists");
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).expect("a link"), Path::new("libx.so.1"));
    assert_eq!(fs::read(&link).expect("it leads to the file"), b"library");
    assert_eq!(extracted.summary.file_count, 2, "the link counts as an item");
}

/// Links that climb out of their own folder but stay inside the extracted
/// folder, links to folders and long targets extract too.
///
/// parity: ARC-016, ARC-024
#[test]
fn links_that_stay_inside_through_parent_folders_extract() {
    let long_folder = "a".repeat(120);
    let long_file = format!("f:{long_folder}/data.txt:far");
    let long_link = format!("l:lib/far:../{long_folder}/data.txt");
    let fixture = fixture_with(&[
        "d:lib",
        "f:lib/libx.so.1:library",
        "d:bin",
        "l:bin/tool:../lib/libx.so.1",
        "l:docs:lib",
        &long_file,
        &long_link,
    ]);

    fixture.extract("Unpacked").expect("the TAR extracts");

    let root = fixture.destination.join("Unpacked");
    assert_eq!(fs::read(root.join("bin/tool")).expect("followed"), b"library");
    assert_eq!(
        fs::read_link(root.join("docs")).expect("a link"),
        Path::new("lib")
    );
    assert_eq!(fs::read(root.join("lib/far")).expect("followed"), b"far");
}

/// A link leaving the extracted folder, by `..` or by an absolute path,
/// stops the extraction before anything is written, with a message that
/// names it.
///
/// parity: ARC-016, ARC-024
#[test]
fn links_that_leave_the_folder_are_refused_by_name() {
    let cases: [&[&str]; 5] = [
        &["f:a.txt:a", "l:outside:../outside"],
        &["f:a.txt:a", "l:passwd:/etc/passwd"],
        &["d:lib", "l:lib/up:../../x"],
        // `..` after a link: `d/s` leads to the top, so `d/s/..` would
        // be its parent.
        &["d:d", "l:d/s:..", "l:t:d/s/../x"],
        &["f:a.txt:a", "l:empty:"],
    ];
    let names = ["outside", "passwd", "lib/up", "t", "empty"];
    for (entries, name) in cases.into_iter().zip(names) {
        let fixture = fixture_with(entries);

        let refused = fixture.extract("Unpacked").unwrap_err();

        assert_eq!(refused, ArchiveError::UnsafeLink(name.to_owned()), "{entries:?}");
        assert!(refused.to_string().contains(name), "{refused}");
        assert!(!fixture.destination.join("Unpacked").exists(), "{entries:?}");
        fixture.assert_no_output();
    }
}

/// FIFOs and hard links are still special files, and nothing is ever
/// written through a link: a path below a link is refused.
///
/// parity: ARC-015, ARC-016
#[test]
fn special_files_and_paths_through_a_link_are_still_refused() {
    let fifo = fixture_with(&["f:a.txt:a", "p:pipe"]);
    let hard = fixture_with(&["f:a.txt:a", "h:b.txt:a.txt"]);
    let through = fixture_with(&["d:real", "l:alias:real", "f:alias/evil.txt:x"]);

    assert_eq!(
        fifo.extract("Unpacked").unwrap_err(),
        ArchiveError::LinkOrSpecialFile
    );
    assert_eq!(
        hard.extract("Unpacked").unwrap_err(),
        ArchiveError::LinkOrSpecialFile
    );
    assert_eq!(
        through.extract("Unpacked").unwrap_err(),
        ArchiveError::AmbiguousPaths
    );
    for fixture in [fifo, hard, through] {
        fixture.assert_no_output();
    }
}

/// The archive browser still hides links: only files and folders are
/// listed, and links are counted as hidden.
///
/// parity: ARC-004
#[test]
fn the_browser_still_hides_links() {
    let temporary = tempfile::tempdir().expect("a temporary folder");
    let archive = temporary.path().join("Links.tar.gz");
    write_tar(&archive, &["f:libx.so.1:library", "l:libx.so:libx.so.1"]);
    let browser = ArchiveBrowser::new(opener(), temporary.path().join("previews"));

    let listing = browser
        .list(&file_uri(&archive), "", &Cancellation::new())
        .expect("lists");

    let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["libx.so.1"]);
    assert_eq!(listing.hidden_unsafe_count, 1);
}

/// Compressing into a name ending `.tar` writes a plain TAR, which the
/// browser lists and the extractor unpacks.
///
/// parity: ARC-023, ARC-024
#[test]
fn a_tar_file_is_written_plain_and_reads_back() {
    let temporary = tempfile::tempdir().expect("a temporary folder");
    let sources = temporary.path().join("sources");
    fs::create_dir_all(sources.join("Docs")).expect("a folder");
    fs::write(sources.join("Docs/a.txt"), b"first").expect("a file");
    fs::write(sources.join("notes.txt"), b"notes").expect("a file");
    let output = temporary.path().join("out");
    fs::create_dir(&output).expect("a folder");
    let request = CompressionRequest {
        uris: vec![
            file_uri(&sources.join("Docs")),
            file_uri(&sources.join("notes.txt")),
        ],
        destination_uri: file_uri(&output),
        archive_name: "Docs.tar".to_owned(),
    };

    ZipCompressor::new()
        .compress(&request, &Cancellation::new())
        .expect("the TAR is written");

    let archive = output.join("Docs.tar");
    let bytes = fs::read(&archive).expect("written");
    assert_eq!(&bytes[257..262], b"ustar", "a plain, uncompressed TAR");
    let browser = ArchiveBrowser::new(opener(), temporary.path().join("previews"));
    let listing = browser
        .list(&file_uri(&archive), "", &Cancellation::new())
        .expect("the TAR is browsed");
    let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["Docs", "notes.txt"]);
}

/// The Create archive dialog's formats: a TAR compressed with gzip or
/// bzip2 by its ending, read back by the archive browser, and the
/// compression level, which makes a smaller archive at the Smaller end.
///
/// parity: ARC-023
#[test]
fn tar_gz_and_tar_bz2_are_written_and_the_level_changes_the_size() {
    let temporary = tempfile::tempdir().expect("a temporary folder");
    let sources = temporary.path().join("sources");
    fs::create_dir_all(&sources).expect("a folder");
    let mut text = String::new();
    for n in 0..20_000 {
        text.push_str("line ");
        text.push_str(&(n % 997).to_string());
        text.push_str(" of the report\n");
    }
    fs::write(sources.join("report.txt"), &text).expect("a file");
    let output = temporary.path().join("out");
    fs::create_dir(&output).expect("a folder");
    let compress = |name: &str, level: Option<u32>| {
        let request = CompressionRequest {
            uris: vec![file_uri(&sources.join("report.txt"))],
            destination_uri: file_uri(&output),
            archive_name: name.to_owned(),
        };
        let mut compressor = ZipCompressor::new();
        if let Some(level) = level {
            compressor = compressor.with_level(level);
        }
        compressor
            .compress(&request, &Cancellation::new())
            .expect("the archive is written");
        fs::read(output.join(name)).expect("written")
    };

    let gzip = compress("report.tar.gz", None);
    let bzip2 = compress("report.tar.bz2", None);
    assert_eq!(&gzip[..2], &[0x1f, 0x8b], "gzip");
    assert_eq!(&bzip2[..3], b"BZh", "bzip2");
    let browser = ArchiveBrowser::new(opener(), temporary.path().join("previews"));
    for name in ["report.tar.gz", "report.tar.bz2"] {
        let listing = browser
            .list(&file_uri(&output.join(name)), "", &Cancellation::new())
            .expect("browsed");
        let names: Vec<&str> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["report.txt"], "{name}");
    }
    let faster = compress("faster.zip", Some(1)).len();
    let smaller = compress("smaller.zip", Some(9)).len();
    assert!(smaller < faster, "{smaller} < {faster}");
}
