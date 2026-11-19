// SPDX-License-Identifier: AGPL-3.0-only
//! The picture decoder process: broken, hostile and hanging pictures are
//! refused, and the app (here, the test process) keeps running.

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::gio;

use super::*;
use crate::picture_guard::{decode_file_to_fit, decode_to_fit};

/// Not a test of its own: the child the tests start, which decodes the
/// picture on its standard input when [`TEST_PARAMETERS`] is set, as the
/// app does with [`DECODER_FLAG`], and exits before the test harness
/// writes anything after its answer. `abort` and `hang` in the parameters
/// make it crash or hang instead, as a broken C loader would.
#[test]
fn picture_decoder_child() {
    let Ok(parameters) = std::env::var(TEST_PARAMETERS) else {
        return;
    };
    if std::env::var_os(TEST_ABORT).is_some() {
        std::process::abort();
    }
    if std::env::var_os(TEST_HANG).is_some() {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let ok = parse_parameters(parameters.split(' '))
        .is_some_and(|(width, height, limits)| decode_standard_input(width, height, limits));
    std::process::exit(i32::from(!ok));
}

const TEST_ABORT: &str = "OX_TEST_PICTURE_DECODER_ABORT";
const TEST_HANG: &str = "OX_TEST_PICTURE_DECODER_HANG";

/// The test program as the child.
fn child_words() -> Vec<OsString> {
    let (program, before) = program().expect("the test program");
    let mut words = vec![program.into_os_string()];
    words.extend(before);
    words
}

fn saved_png(folder: &Path, name: &str, width: i32, height: i32) -> std::path::PathBuf {
    let pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, width, height).expect("a picture");
    pixbuf.fill(0x3366_99ff);
    let path = folder.join(name);
    pixbuf.savev(&path, "png", &[]).expect("saved");
    path
}

/// A picture decodes in the child and comes back whole: its own size,
/// the size it was decoded to, and its pixels.
#[test]
fn a_picture_is_decoded_in_the_child_and_handed_back() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let path = saved_png(folder.path(), "Harbor.png", 300, 200);
    let decoded =
        decode_to_fit(&gio::File::for_path(&path), 150, 150, Limits::DISPLAY, None).expect("decoded");
    assert_eq!((decoded.width, decoded.height), (300, 200));
    assert_eq!((decoded.pixbuf.width(), decoded.pixbuf.height()), (150, 100));
    assert!(decoded.pixbuf.has_alpha());
    let bytes = decoded.pixbuf.read_pixel_bytes();
    assert_eq!(&bytes[..4], &[0x33, 0x66, 0x99, 0xff]);
    let file = std::fs::File::open(&path).expect("opened");
    let decoded = decode_file_to_fit(file, 64, 64, Limits::DISPLAY, None).expect("decoded");
    assert_eq!(decoded.pixbuf.width(), 64);
}

/// A cached thumbnail's keys come back with its pixels.
#[test]
fn a_thumbnails_keys_come_back_from_the_child() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, 8, 8).expect("a picture");
    let path = folder.path().join("thumbnail.png");
    pixbuf
        .savev(
            &path,
            "png",
            &[
                ("tEXt::Thumb::URI", "file:///home/alex/Pictures/Lake.png"),
                ("tEXt::Thumb::MTime", "42"),
            ],
        )
        .expect("saved");
    let decoded =
        decode_to_fit(&gio::File::for_path(&path), 64, 64, Limits::THUMBNAIL_FILE, None).expect("decoded");
    assert_eq!(
        decoded.pixbuf.option("tEXt::Thumb::URI").as_deref(),
        Some("file:///home/alex/Pictures/Lake.png")
    );
    assert_eq!(decoded.pixbuf.option("tEXt::Thumb::MTime").as_deref(), Some("42"));
}

/// A child that crashes, as a C loader aborting on a malformed file does,
/// is a refusal, and the app goes on.
#[test]
fn a_decoder_that_crashes_is_a_refusal() {
    let mut words = child_words();
    words.insert(0, OsString::from("env"));
    words.insert(1, OsString::from(format!("{TEST_ABORT}=1")));
    let source = Source::Reader(Box::new(Cursor::new(super::super::ONE_PIXEL_PNG)));
    let started = Instant::now();
    let result = run_with_env(&words, source, Limits::DISPLAY);
    assert_eq!(result.map(|_| ()), Err(Refusal::Damaged));
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// A child that hangs is killed at the time limit, and one whose
/// picture is no longer wanted is killed at once.
#[test]
fn a_decoder_that_hangs_is_stopped() {
    let mut words = child_words();
    words.insert(0, OsString::from("env"));
    words.insert(1, OsString::from(format!("{TEST_HANG}=1")));
    let short = Limits {
        time: Duration::from_millis(200),
        ..Limits::DISPLAY
    };
    let started = Instant::now();
    let source = Source::Reader(Box::new(Cursor::new(super::super::ONE_PIXEL_PNG)));
    assert_eq!(
        run_with_env(&words, source, short).map(|_| ()),
        Err(Refusal::TimedOut)
    );
    assert!(started.elapsed() < GRACE + Duration::from_secs(2));

    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        cancel.cancel();
    });
    let started = Instant::now();
    let source = Source::Reader(Box::new(Cursor::new(super::super::ONE_PIXEL_PNG)));
    let result = run(&words, source, 1, 1, Limits::DISPLAY, Some(&cancellable));
    assert_eq!(result.map(|_| ()), Err(Refusal::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(2));
}

/// A child given an endless picture (a pipe that never ends) is stopped
/// at the time limit while the app is still feeding it.
#[test]
fn an_endless_picture_is_stopped_at_the_time_limit() {
    struct Endless;
    impl Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer.fill(0x42);
            Ok(buffer.len())
        }
    }
    let short = Limits {
        time: Duration::from_millis(500),
        ..Limits::DISPLAY
    };
    let started = Instant::now();
    let result = run(
        &child_words(),
        Source::Reader(Box::new(Endless)),
        64,
        64,
        short,
        None,
    );
    assert!(result.is_err());
    assert!(started.elapsed() < GRACE + Duration::from_secs(2));
}

fn run_with_env(words: &[OsString], source: Source, limits: Limits) -> Result<Decoded, Refusal> {
    run(words, source, 1, 1, limits, None)
}

/// Every answer a broken or hostile child could write is refused, never
/// trusted: cut short, lying about its size or row stride, or nonsense.
#[test]
fn a_broken_answer_is_refused() {
    let pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, 4, 3).expect("a picture");
    pixbuf.fill(0x1122_33ff);
    let good = answer(&Ok(Decoded {
        pixbuf,
        width: 40,
        height: 30,
    }));
    let decoded = parse_answer(&good).expect("a good answer");
    assert_eq!((decoded.width, decoded.height), (40, 30));
    assert_eq!((decoded.pixbuf.width(), decoded.pixbuf.height()), (4, 3));
    for cut in 0..good.len() {
        assert!(parse_answer(&good[..cut]).is_err(), "cut at {cut}");
    }
    let mut longer = good.clone();
    longer.push(0);
    assert!(parse_answer(&longer).is_err());
    // The width, height and row stride, each made a lie.
    let header = MAGIC.len() + 1 + 8;
    for (at, value) in [(0, 100_000), (0, -1), (4, i32::MAX), (8, 1), (8, 1 << 30), (4, 0)] {
        let mut lie = good.clone();
        lie[header + at..header + at + 4].copy_from_slice(&i32::to_le_bytes(value));
        assert!(parse_answer(&lie).is_err(), "{at} = {value}");
    }
    // Seeded noise, after the magic and as a whole.
    let mut seed = 0x2545_f491_u32;
    for round in 0..2000 {
        let mut noise = if round % 2 == 0 {
            good.clone()
        } else {
            MAGIC.to_vec()
        };
        let extra = usize::try_from(seed % 64).unwrap_or(0);
        for _ in 0..extra {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let at = usize::try_from(seed).unwrap_or(0) % (noise.len() + 1);
            let byte = seed.to_le_bytes()[1];
            if round % 3 == 0 && at < noise.len() {
                noise[at] = byte;
            } else {
                noise.insert(at, byte);
            }
        }
        let _ = parse_answer(&noise);
    }
    assert_eq!(
        parse_answer(&answer(&Err(Refusal::TooLarge {
            width: 70_000,
            height: 70_000
        })))
        .map(|_| ()),
        Err(Refusal::TooLarge {
            width: 70_000,
            height: 70_000
        })
    );
    assert_eq!(
        parse_answer(&answer(&Err(Refusal::TimedOut))).map(|_| ()),
        Err(Refusal::TimedOut)
    );
}

/// The corpus of broken pictures the Preview pane, the Details pane and
/// the thumbnails meet: each is decoded in the child, refused or drawn,
/// and the app survives every one, within its time limit.
#[test]
fn a_corpus_of_broken_pictures_leaves_the_app_running() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let mut corpus = super::super::tests::broken_pictures(folder.path());
    corpus.extend(super::super::tests::mutated_pictures(folder.path(), 40));
    assert!(corpus.len() > 60);
    for path in &corpus {
        let started = Instant::now();
        let _ = decode_to_fit(&gio::File::for_path(path), 256, 256, Limits::DISPLAY, None);
        assert!(
            started.elapsed() < Limits::DISPLAY.time + GRACE + Duration::from_secs(1),
            "{} took too long",
            path.display()
        );
    }
}

/// Where bubblewrap works, the sandboxed child decodes too.
#[test]
fn the_sandboxed_decoder_decodes_where_bubblewrap_works() {
    let (program, before) = program().expect("the test program");
    let Some(mut words) = crate::thumbnails::decoder_sandbox(&program, false) else {
        return;
    };
    words.extend(before);
    let source = Source::Reader(Box::new(Cursor::new(super::super::ONE_PIXEL_PNG)));
    // A computer whose kernel refuses the namespaces refuses the picture.
    if let Ok(decoded) = run(&words, source, 1, 1, Limits::DISPLAY, None) {
        assert_eq!(decoded.width, 1);
    }
}
