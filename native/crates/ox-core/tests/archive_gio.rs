// SPDX-License-Identifier: AGPL-3.0-only
//! The archive service over the production GIO adapters: reading archives
//! through seekable GIO streams, extracting with GIO, and the background
//! variants the app awaits. Ports `ArchiveReaderTests` of
//! `desktop/tests/test_rc2.py`, whose fake GIO stream is the
//! [`ShortReadStream`] here.

mod archive_support;

use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use gio::prelude::*;
use ox_core::archive::{
    default_preview_root, ArchiveBrowser, ArchiveError, ArchiveOpener, ArchiveStream, ExtractionRequest,
    GioArchiveReader, GioExtractionOutput, ZipExtractor,
};
use ox_core::transfer::Cancellation;

use archive_support::{
    file_type, file_uri, gio_factory, make_fifo, mode_of, opener, zip_bytes, ExtractionFixture, TestMember,
};
use short_reads::{Seeking, ShortReadStream};
use unsized_file::UnsizedFile;

/// A GIO input stream over bytes in memory that returns at most three
/// bytes per read, like the `Stream` double of `desktop/tests/test_rc2.py`.
mod short_reads {
    use gio::subclass::prelude::*;

    glib::wrapper! {
        /// Bytes in memory, read three at a time.
        pub struct ShortReadStream(ObjectSubclass<imp::ShortReadStream>)
            @extends gio::InputStream,
            @implements gio::Seekable;
    }

    /// Whether a [`ShortReadStream`] can seek.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Seeking {
        Supported,
        Unsupported,
    }

    impl ShortReadStream {
        /// A stream over `data`.
        pub fn new(data: &[u8], seeking: Seeking) -> Self {
            let stream: Self = glib::Object::new();
            stream.imp().data.replace(data.to_vec());
            stream.imp().can_seek.set(seeking == Seeking::Supported);
            stream
        }
    }

    mod imp {
        use std::cell::{Cell, RefCell};

        use gio::subclass::prelude::*;

        /// The most bytes one read returns.
        const CHUNK_BYTES: usize = 3;

        #[derive(Default)]
        pub struct ShortReadStream {
            pub(super) data: RefCell<Vec<u8>>,
            pub(super) position: Cell<usize>,
            pub(super) can_seek: Cell<bool>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for ShortReadStream {
            const NAME: &'static str = "OxArchiveTestShortReadStream";
            type Type = super::ShortReadStream;
            type ParentType = gio::InputStream;
            type Interfaces = (gio::Seekable,);
        }

        impl ObjectImpl for ShortReadStream {}

        impl InputStreamImpl for ShortReadStream {
            fn read(
                &self,
                buffer: &mut [u8],
                _cancellable: Option<&gio::Cancellable>,
            ) -> Result<usize, glib::Error> {
                let data = self.data.borrow();
                let start = self.position.get().min(data.len());
                let count = buffer.len().min(CHUNK_BYTES).min(data.len() - start);
                buffer[..count].copy_from_slice(&data[start..start + count]);
                self.position.set(start + count);
                Ok(count)
            }
        }

        impl SeekableImpl for ShortReadStream {
            fn tell(&self) -> i64 {
                i64::try_from(self.position.get()).expect("test streams are small")
            }

            fn can_seek(&self) -> bool {
                self.can_seek.get()
            }

            fn seek(
                &self,
                offset: i64,
                seek_type: glib::SeekType,
                _cancellable: Option<&gio::Cancellable>,
            ) -> Result<(), glib::Error> {
                assert_eq!(
                    seek_type,
                    glib::SeekType::Set,
                    "the archive reader seeks to absolute positions"
                );
                let position =
                    usize::try_from(offset).expect("the archive reader refuses negative positions");
                self.position.set(position);
                Ok(())
            }

            fn can_truncate(&self) -> bool {
                false
            }

            fn truncate(
                &self,
                _offset: i64,
                _cancellable: Option<&gio::Cancellable>,
            ) -> Result<(), glib::Error> {
                Err(glib::Error::new(
                    gio::IOErrorEnum::NotSupported,
                    "read-only test stream",
                ))
            }
        }
    }
}

/// A GIO file whose data opens but whose size cannot be queried, like the
/// failing `query_info` of `test_constructor_closes_stream_on_failure` in
/// `desktop/tests/test_rc2.py`.
mod unsized_file {
    use std::path::Path;

    use gio::subclass::prelude::*;

    glib::wrapper! {
        /// A file whose `query_info` fails.
        pub struct UnsizedFile(ObjectSubclass<imp::UnsizedFile>)
            @implements gio::File;
    }

    impl UnsizedFile {
        /// A file whose data is the local file at `data_path`.
        pub fn new(data_path: &Path) -> Self {
            let file: Self = glib::Object::new();
            file.imp().data.replace(Some(gio::File::for_path(data_path)));
            file
        }

        /// The stream the last read opened.
        pub fn opened_stream(&self) -> Option<gio::FileInputStream> {
            self.imp().opened.borrow().clone()
        }
    }

    mod imp {
        use std::cell::RefCell;

        use gio::prelude::*;
        use gio::subclass::prelude::*;

        #[derive(Default)]
        pub struct UnsizedFile {
            pub(super) data: RefCell<Option<gio::File>>,
            pub(super) opened: RefCell<Option<gio::FileInputStream>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for UnsizedFile {
            const NAME: &'static str = "OxArchiveTestUnsizedFile";
            type Type = super::UnsizedFile;
            type Interfaces = (gio::File,);
        }

        impl ObjectImpl for UnsizedFile {}

        impl FileImpl for UnsizedFile {
            fn read_fn(
                &self,
                cancellable: Option<&gio::Cancellable>,
            ) -> Result<gio::FileInputStream, glib::Error> {
                let data = self.data.borrow().clone().expect("the test sets the data file");
                let stream = data.read(cancellable)?;
                self.opened.replace(Some(stream.clone()));
                Ok(stream)
            }

            fn query_info(
                &self,
                _attributes: &str,
                _flags: gio::FileQueryInfoFlags,
                _cancellable: Option<&gio::Cancellable>,
            ) -> Result<gio::FileInfo, glib::Error> {
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "query failed",
                ))
            }
        }
    }
}

/// A reader over `data`, read three bytes at a time.
fn short_reader(data: &[u8], cancel: &Cancellation) -> GioArchiveReader {
    let stream = ShortReadStream::new(data, Seeking::Supported);
    let size = data.len() as u64;
    GioArchiveReader::new(stream.upcast(), size, cancel).expect("the stream can seek")
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_read_accumulates_partial_reads`.
///
/// parity: ARC-007
#[test]
fn short_gio_reads_are_completed_by_further_reads() {
    let mut reader = short_reader(b"abcdefghij", &Cancellation::new());

    let mut first = [0u8; 8];
    reader.read_exact(&mut first).expect("eight bytes");
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).expect("the rest");

    assert_eq!(&first, b"abcdefgh");
    assert_eq!(rest, b"ij");
}

/// ARC-007: the reader asks GIO for at most 64 KiB at a time, as the
/// Python reader does.
///
/// parity: ARC-007
#[test]
fn one_read_asks_gio_for_at_most_64_kib() {
    let data = vec![7u8; 200_000];
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&data));
    let mut reader = GioArchiveReader::new(stream.upcast(), 200_000, &Cancellation::new()).expect("seekable");

    let mut buffer = vec![0u8; 200_000];
    let count = reader.read(&mut buffer).expect("a read");

    assert_eq!(count, 64 * 1024);
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_zero_read_and_eof`.
///
/// parity: ARC-007
#[test]
fn an_empty_read_and_the_end_read_nothing() {
    let mut reader = short_reader(b"abc", &Cancellation::new());

    let empty = reader.read(&mut []).expect("an empty read");
    let mut everything = Vec::new();
    reader.read_to_end(&mut everything).expect("read to the end");
    let after_end = reader.read(&mut [0u8; 4]).expect("a read at the end");

    assert_eq!(empty, 0);
    assert_eq!(everything, b"abc");
    assert_eq!(after_end, 0);
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_real_zip_through_short_reads`.
///
/// parity: ARC-007
#[test]
fn a_real_zip_is_read_through_short_reads() {
    let archive = zip_bytes(&[TestMember::file("Project/notes.txt", b"hypothetical sample")]);
    let opener = move |_uri: &str, cancel: &Cancellation| -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        Ok(Box::new(short_reader(&archive, cancel)))
    };
    let opener: Arc<dyn ArchiveOpener> = Arc::new(opener);
    let previews = tempfile::tempdir().expect("create a temporary folder");
    let browser = ArchiveBrowser::new(opener, previews.path().to_path_buf());

    let copy = browser
        .preview_member(
            "smb://nas/share/archive.zip",
            "Project/notes.txt",
            &Cancellation::new(),
        )
        .expect("the member is read through the short reads");

    assert_eq!(
        fs::read_to_string(copy.path).expect("read the copy"),
        "hypothetical sample"
    );
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_seek_validation`.
/// Rust's `SeekFrom` cannot hold an invalid mode, so the refused seek is
/// one before the start.
///
/// parity: ARC-007
#[test]
fn seeks_are_relative_to_the_end_and_never_before_the_start() {
    let mut reader = short_reader(b"abc", &Cancellation::new());

    let position = reader.seek(SeekFrom::End(-1)).expect("a seek to the last byte");
    let mut last = Vec::new();
    reader.read_to_end(&mut last).expect("read the last byte");
    let before_start = reader.seek(SeekFrom::End(-4)).unwrap_err();

    assert_eq!(position, 2);
    assert_eq!(last, b"c");
    assert_eq!(before_start.kind(), ErrorKind::InvalidInput);
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_cancellation_between_reads`.
///
/// parity: ARC-007
#[test]
fn a_cancelled_reader_stops_reading() {
    let cancel = Cancellation::new();
    let mut reader = short_reader(b"abc", &cancel);
    cancel.cancel();

    let result = reader.read(&mut [0u8; 3]);

    assert!(result.is_err());
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_constructor_closes_stream_on_failure`.
///
/// parity: ARC-007
#[test]
fn a_failed_open_closes_the_stream() {
    let folder = tempfile::tempdir().expect("create a temporary folder");
    let data_path = folder.path().join("data");
    fs::write(&data_path, "abc").expect("write the data");
    let file = UnsizedFile::new(&data_path);

    let result = GioArchiveReader::open(file.upcast_ref(), &Cancellation::new());

    assert_eq!(result.unwrap_err().to_string(), "query failed");
    let stream = file.opened_stream().expect("the data was opened");
    assert!(stream.is_closed());
}

/// Ported from `desktop/tests/test_rc2.py::ArchiveReaderTests::test_nonseekable_closes_stream`.
///
/// parity: ARC-007
#[test]
fn a_stream_that_cannot_seek_is_refused_and_closed() {
    let stream = ShortReadStream::new(b"abc", Seeking::Unsupported);

    let result = GioArchiveReader::new(stream.clone().upcast(), 3, &Cancellation::new());

    let error = result.unwrap_err();
    assert_eq!(error, ArchiveError::NotSeekable);
    assert_eq!(
        error.to_string(),
        "This share does not support seekable ZIP reading. Mount it locally or use an archive manager."
    );
    assert!(stream.is_closed());
}

/// ARC-007: the production opener reads a local archive by its path.
///
/// parity: ARC-007
#[test]
fn the_production_opener_reads_local_archives() {
    let folder = tempfile::tempdir().expect("create a temporary folder");
    let archive = folder.path().join("My archive.zip");
    fs::write(&archive, zip_bytes(&[TestMember::file("a.txt", b"data")])).expect("write the archive");
    let browser = ArchiveBrowser::new(opener(), folder.path().join("previews"));

    let listing = browser
        .list(&file_uri(&archive), "", &Cancellation::new())
        .expect("lists");

    assert_eq!(listing.entries[0].name, "a.txt");
    assert_eq!(listing.archive_uri, file_uri(&archive));
}

/// ARC-007: an archive on a share with a local path (here the `GVfs` FUSE
/// export in the private runtime directory) is read from that path, as
/// `archive_stream` in `desktop/native_opening.py` reads it.
///
/// parity: ARC-007
#[test]
fn a_share_archive_with_a_local_path_is_read_from_it() {
    let export = glib::user_runtime_dir().join("gvfs/smb-share:server=archive-nas,share=projects");
    fs::create_dir_all(&export).expect("create the export");
    fs::write(
        export.join("Bundle.zip"),
        zip_bytes(&[TestMember::file("a.txt", b"data")]),
    )
    .expect("write the archive");

    let listed = opener().open("smb://archive-nas/projects/Bundle.zip", &Cancellation::new());
    fs::remove_dir_all(&export).expect("remove the export");

    let mut stream = listed.expect("the archive opens from the export");
    let mut signature = [0; 4];
    stream.read_exact(&mut signature).expect("reads");
    assert_eq!(&signature, b"PK\x03\x04");
}

/// ARC-007: the production opener opens local files without blocking, so
/// a FIFO named like an archive fails at once instead of holding the
/// worker thread until some program writes to it.
///
/// parity: ARC-007
#[test]
fn a_fifo_named_like_an_archive_fails_instead_of_blocking() {
    let folder = tempfile::tempdir().expect("create a temporary folder");
    let fifo = folder.path().join("pipe.zip");
    make_fifo(&fifo);
    let browser = ArchiveBrowser::new(opener(), folder.path().join("previews"));
    let uri = file_uri(&fifo);
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let listing = browser.list(&uri, "", &Cancellation::new());
        // The receiver is gone only after the test has already failed.
        let _ = sender.send(listing);
    });
    let listing = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("listing a FIFO returns instead of blocking");

    assert!(listing.is_err(), "{listing:?}");
}

/// ARC-018: through the production GIO adapters, files are new and
/// owner-only and the new folder is owner-only, whatever the archive
/// records.
///
/// parity: ARC-013, ARC-018
#[test]
fn extraction_through_gio_never_applies_archive_permissions() {
    let fixture = ExtractionFixture::new();
    let script = TestMember::with_unix_mode("bin/run.sh", file_type::REGULAR | 0o4755, b"#!/bin/sh\n");
    fixture.write_zip(&[script, TestMember::file("readme.txt", b"hello")]);
    let mut extractor = ZipExtractor::new(opener(), gio_factory(), Arc::new(GioExtractionOutput));

    let extracted = fixture
        .extract_with(&mut extractor, "Unpacked")
        .expect("the archive extracts");

    let folder = fixture.destination.join("Unpacked");
    assert_eq!(
        fs::read_to_string(folder.join("readme.txt")).expect("extracted"),
        "hello"
    );
    assert_eq!(mode_of(&folder.join("bin/run.sh")), 0o600);
    assert_eq!(mode_of(&folder.join("readme.txt")), 0o600);
    assert_eq!(mode_of(&folder), 0o700);
    assert_eq!(extracted.summary.file_count, 2);
    assert_eq!(fixture.destination_names(), ["Unpacked"]);
}

/// The background variants run on a GIO worker thread and hand the result
/// back to the awaiting main loop.
///
/// parity: ARC-003, ARC-006, ARC-008
#[test]
fn background_variants_return_the_blocking_results() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let browser = ArchiveBrowser::new(opener(), fixture.root.join("previews"));
    let context = glib::MainContext::new();

    let listing = context
        .block_on(browser.list_in_background(fixture.archive_uri(), "Docs/".to_owned(), Cancellation::new()))
        .expect("lists");
    let copy = context
        .block_on(browser.preview_member_in_background(
            fixture.archive_uri(),
            "Docs/Guide.txt".to_owned(),
            Cancellation::new(),
        ))
        .expect("opens");
    let summary = context
        .block_on(
            fixture
                .extractor()
                .inspect_in_background(fixture.archive_uri(), Cancellation::new()),
        )
        .expect("inspects");

    assert_eq!(listing.entries[0].name, "Guide.txt");
    assert_eq!(
        fs::read_to_string(copy.path).expect("read the copy"),
        "hello world"
    );
    assert_eq!(summary.file_count, 2);
}

/// [`ZipExtractor::extract_in_background`] extracts on a worker thread.
///
/// parity: ARC-012
#[test]
fn extraction_in_the_background_publishes_the_folder() {
    let fixture = ExtractionFixture::new();
    fixture.write_sample_zip();
    let request = ExtractionRequest {
        archive_uri: fixture.archive_uri(),
        destination_uri: fixture.destination_uri(),
        folder_name: "Unpacked".to_owned(),
    };
    let extractor = ZipExtractor::new(opener(), gio_factory(), Arc::new(GioExtractionOutput));

    let extracted = glib::MainContext::new()
        .block_on(extractor.extract_in_background(request, Cancellation::new()))
        .expect("the sample extracts");

    assert_eq!(extracted.name, "Unpacked");
    assert!(fixture.destination.join("Unpacked/Docs/Guide.txt").is_file());
}

/// ARC-006: opened members go to a private folder in the user's runtime
/// directory, which the session removes at logout.
///
/// parity: ARC-006
#[test]
fn previews_go_to_the_private_runtime_directory() {
    let expected = glib::user_runtime_dir().join("winspace-archive-previews");

    assert_eq!(default_preview_root(), expected);
}
