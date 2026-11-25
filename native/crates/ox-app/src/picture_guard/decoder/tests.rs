// SPDX-License-Identifier: AGPL-3.0-only
//! The picture decoder process: broken, oversized and hanging pictures are
//! refused, and the app (here, the test process) keeps running.

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::gio;

use super::*;
use crate::picture_guard::{decode_file_to_fit, decode_to_fit};

/// Not a test of its own: the decoder the tests start, which serves the
/// requests on its standard input when [`TEST_DECODER`] is set, as the
/// app does with [`DECODER_FLAG`], and exits before the test harness
/// writes anything after its answers. [`TEST_ABORT`] and [`TEST_HANG`]
/// make it crash or hang at once instead, as a broken C loader would; a
/// request [`CRASH_WIDTH`] wide crashes it mid-stream.
#[test]
fn picture_decoder_child() {
    if std::env::var_os(TEST_DECODER).is_none() {
        return;
    }
    if std::env::var_os(TEST_ABORT).is_some() {
        std::process::abort();
    }
    if std::env::var_os(TEST_HANG).is_some() {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    // As in bubblewrap's sandbox (`--die-with-parent`), the decoder is
    // killed when the thread that started it ends: the kernel counts that
    // thread as its parent, not the app.
    let _ = rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL));
    let ok = serve_standard_input();
    std::process::exit(i32::from(!ok));
}

const TEST_ABORT: &str = "OX_TEST_PICTURE_DECODER_ABORT";
const TEST_HANG: &str = "OX_TEST_PICTURE_DECODER_HANG";
/// A request this wide crashes the test decoder mid-stream.
pub(super) const CRASH_WIDTH: i32 = 31_337;

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

/// Every answer a broken child could write is refused: cut short, lying about its size or row stride, or nonsense.
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

/// One decoder serves many pictures in turn, through the pool; a decoder
/// that crashes on one is replaced, and the next picture is decoded.
#[test]
fn the_pool_reuses_its_decoders_and_replaces_one_that_crashed() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let path = saved_png(folder.path(), "Harbor.png", 120, 80);
    let pool = Pool::new(child_words());
    let decode = |width: i32| {
        let file = std::fs::File::open(&path).expect("opened");
        pool.decode(
            Source::Reader(Box::new(file)),
            width,
            width,
            Limits::DISPLAY,
            None,
        )
    };
    let pid = || {
        let idle = pool.idle.lock().expect("the pool");
        idle.first().map(|decoder| (decoder.child.0.id(), decoder.served))
    };
    for _ in 0..30 {
        assert_eq!(decode(60).expect("decoded").pixbuf.width(), 60);
    }
    let (first, served) = pid().expect("an idle decoder");
    assert_eq!(served, 30);
    assert_eq!(decode(CRASH_WIDTH).map(|_| ()), Err(Refusal::Damaged));
    assert!(pid().is_none(), "the crashed decoder is not kept");
    assert_eq!(decode(40).expect("decoded again").pixbuf.width(), 40);
    let (second, served) = pid().expect("a new decoder");
    assert_ne!(first, second);
    assert_eq!(served, 1);
    // A large picture after small ones in the same decoder: its answer is
    // held to its own limit, not the last one's.
    let large = saved_png(folder.path(), "Panorama.png", 2400, 1200);
    let file = std::fs::File::open(&large).expect("opened");
    let decoded = pool
        .decode(Source::Reader(Box::new(file)), 4096, 4096, Limits::DISPLAY, None)
        .expect("decoded at its own size");
    assert_eq!(decoded.pixbuf.width(), 2400);
    assert_eq!(pid().map(|(pid, _)| pid), Some(second));
    // A picture whose answer comes before its end (a refused header) is
    // drained, and the decoder serves the next one.
    let mut huge =
        Vec::from(&b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\x7f\xff\xff\xff\x7f\xff\xff\xff\x08\x06\0\0\0"[..]);
    huge.extend(std::iter::repeat_n(0_u8, 3 * 1024 * 1024));
    let refused = pool.decode(
        Source::Reader(Box::new(Cursor::new(huge))),
        64,
        64,
        Limits::DISPLAY,
        None,
    );
    assert!(refused.is_err());
    assert_eq!(decode(30).expect("decoded after a refusal").pixbuf.width(), 30);
    assert_eq!(pid().map(|(pid, _)| pid), Some(second));
}

/// The answers' frames are found after other output, and one larger than
/// its limit, cut short or missing is no answer.
#[test]
fn an_answers_frame_is_read_with_care() {
    let mut wire = b"running 1 test\n".to_vec();
    wire.extend_from_slice(FRAME);
    wire.extend_from_slice(&3_u64.to_le_bytes());
    wire.extend_from_slice(b"abc");
    wire.extend_from_slice(FRAME);
    wire.extend_from_slice(&2_u64.to_le_bytes());
    wire.extend_from_slice(b"de");
    let mut reader = Cursor::new(wire.clone());
    assert_eq!(
        read_frame(&mut reader, &std::sync::atomic::AtomicU64::new(16)).as_deref(),
        Some(&b"abc"[..])
    );
    assert_eq!(
        read_frame(&mut reader, &std::sync::atomic::AtomicU64::new(16)).as_deref(),
        Some(&b"de"[..])
    );
    assert_eq!(
        read_frame(&mut reader, &std::sync::atomic::AtomicU64::new(16)),
        None
    );
    assert_eq!(
        read_frame(
            &mut Cursor::new(wire.clone()),
            &std::sync::atomic::AtomicU64::new(2)
        ),
        None
    );
    assert_eq!(
        read_frame(
            &mut Cursor::new(&wire[..wire.len() - 9]),
            &std::sync::atomic::AtomicU64::new(16)
        )
        .as_deref(),
        Some(&b"abc"[..])
    );
    let mut cut = Cursor::new(&wire[..27]);
    assert_eq!(read_frame(&mut cut, &std::sync::atomic::AtomicU64::new(16)), None);
    assert_eq!(
        read_frame(
            &mut Cursor::new(vec![0_u8; 100_000]),
            &std::sync::atomic::AtomicU64::new(16)
        ),
        None
    );
}

/// A vector picture is drawn in a decoder within the drawing limits of
/// one, and that decoder then ends, so those limits never reach the next
/// picture: the pool starts a new decoder for it, which decodes it.
#[test]
fn a_decoder_ends_after_a_vector_picture_and_the_next_picture_is_decoded() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let path = saved_png(folder.path(), "Harbor.png", 120, 80);
    let pool = Pool::new(child_words());
    let decode_png = || {
        let file = std::fs::File::open(&path).expect("opened");
        pool.decode(Source::Reader(Box::new(file)), 60, 60, Limits::DISPLAY, None)
    };
    let pid = || {
        let idle = pool.idle.lock().expect("the pool");
        idle.first().map(|decoder| decoder.child.0.id())
    };
    assert_eq!(decode_png().expect("decoded").pixbuf.width(), 60);
    let first = pid().expect("an idle decoder");
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\">\
                <rect width=\"40\" height=\"20\" fill=\"#336699\"/></svg>";
    let drawn = pool
        .decode(
            Source::Reader(Box::new(Cursor::new(&svg[..]))),
            80,
            80,
            Limits::DISPLAY,
            None,
        )
        .expect("drawn");
    assert_eq!((drawn.width, drawn.height), (40, 20));
    assert!(pid().is_none(), "the decoder that drew it is not kept");
    assert_eq!(decode_png().expect("decoded after it").pixbuf.width(), 60);
    let second = pid().expect("a new decoder");
    assert_ne!(first, second);
}

/// A picture whose bytes are these crashes the test decoder as it reads
/// them, as a C loader aborting on one damaged file of many would.
pub(super) const CRASH_PICTURE: &[u8] = b"\0OX-TEST-CRASH\0";

/// Cached thumbnails are decoded many to one decoder in a single batch:
/// every picture comes back, in order, with its keys; a picture that
/// crashes its decoder is refused alone and the pictures after it are
/// decoded by a new decoder; and one decoder served the whole batch.
///
/// parity: VIEW-057
#[test]
fn a_batch_of_thumbnails_is_decoded_by_one_decoder() {
    let pool = Pool::new(child_words());
    let thumbnail = |seed: u32| {
        let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, 128, 96).expect("a picture");
        pixbuf.fill(seed);
        let bytes = pixbuf
            .save_to_bufferv(
                "png",
                &[("tEXt::Thumb::URI", format!("file:///p/{seed}.jpg").as_str())],
            )
            .expect("saved");
        bytes
    };
    let pictures: Vec<Vec<u8>> = (0..12).map(|seed| thumbnail(0x1000_00ff + seed)).collect();
    let mut answers = Vec::new();
    pool.decode_batch(
        &pictures,
        96,
        96,
        Limits::THUMBNAIL_FILE,
        None,
        &mut |index, result| {
            answers.push((index, result));
        },
    );
    assert_eq!(answers.len(), 12);
    for (expected, (index, result)) in answers.iter().enumerate() {
        assert_eq!(*index, expected, "in order");
        let decoded = result.as_ref().expect("decoded");
        assert_eq!((decoded.pixbuf.width(), decoded.pixbuf.height()), (96, 72));
        let seed = 0x1000_00ff + u32::try_from(expected).expect("small");
        assert_eq!(
            decoded.pixbuf.option("tEXt::Thumb::URI").as_deref(),
            Some(format!("file:///p/{seed}.jpg").as_str())
        );
    }
    let idle: Vec<(u32, u32)> = pool
        .idle
        .lock()
        .expect("the pool")
        .iter()
        .map(|decoder| (decoder.child.0.id(), decoder.served))
        .collect();
    assert_eq!(idle.len(), 1, "one decoder served the batch: {idle:?}");
    assert_eq!(idle[0].1, 12);

    // The fifth picture crashes its decoder: it alone is refused.
    let mut with_crash = pictures;
    with_crash[4] = CRASH_PICTURE.to_vec();
    let mut answers = Vec::new();
    pool.decode_batch(
        &with_crash,
        96,
        96,
        Limits::THUMBNAIL_FILE,
        None,
        &mut |index, result| {
            answers.push((index, result.map(|decoded| decoded.pixbuf.width())));
        },
    );
    let indexes: Vec<usize> = answers.iter().map(|(index, _)| *index).collect();
    assert_eq!(indexes, (0..12).collect::<Vec<_>>());
    for (index, result) in &answers {
        if *index == 4 {
            assert_eq!(*result, Err(Refusal::Damaged));
        } else {
            assert_eq!(*result, Ok(96), "picture {index}");
        }
    }

    // A canceled batch answers every picture as canceled at once.
    let cancelled = gio::Cancellable::new();
    cancelled.cancel();
    let mut answers = Vec::new();
    pool.decode_batch(
        &vec![thumbnail(1); 3],
        96,
        96,
        Limits::THUMBNAIL_FILE,
        Some(&cancelled),
        &mut |_, result| {
            answers.push(result.map(|_| ()));
        },
    );
    assert_eq!(answers, vec![Err(Refusal::Cancelled); 3]);
}

/// Whether the child `pid` has ended, reaped or not. Ended means every
/// thread of it has, so it can be waited for: a decoder is the test
/// program, whose test runs on a thread of its own, and when it is killed
/// its first thread shows as a zombie in `/proc` while the others are
/// still ending, too early for the pool to see it ended.
fn has_ended(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return true;
    };
    let ended = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    // Reaped already, it is no longer a child: an error.
    !matches!(waitid(WaitId::Pid(pid), ended), Ok(None))
}

/// A decoder kept idle in the pool serves the next picture although the
/// thread that started it has ended, as GIO's worker threads do once idle
/// a while: bubblewrap's `--die-with-parent` ends a process with the
/// thread that started it, which left the app's idle decoders dead and
/// unreaped overnight.
#[test]
fn a_pooled_decoder_outlives_the_thread_that_started_it() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let path = saved_png(folder.path(), "Harbor.png", 120, 80);
    let pool = Pool::new(child_words());
    // The width alone comes back from the worker: a picture stays on it.
    let decode = || {
        let file = std::fs::File::open(&path).expect("opened");
        pool.decode(Source::Reader(Box::new(file)), 60, 60, Limits::DISPLAY, None)
            .map(|decoded| decoded.pixbuf.width())
    };
    let width = std::thread::scope(|scope| scope.spawn(decode).join().expect("the worker"));
    assert_eq!(width, Ok(60), "decoded on a worker");
    let pid = || {
        let idle = pool.idle.lock().expect("the pool");
        idle.first().map(|decoder| decoder.child.0.id())
    };
    let first = pid().expect("an idle decoder");
    // The worker has ended; a decoder that ends with it does so at once.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !has_ended(first),
        "the idle decoder ended with the worker that started it"
    );
    let again = std::thread::scope(|scope| scope.spawn(decode).join().expect("another worker"));
    assert_eq!(again, Ok(60), "decoded on another worker");
    assert_eq!(pid(), Some(first), "the same decoder served again");
}

/// A decoder that died while idle (killed, or out of processor time) is
/// reaped and replaced when the next picture comes, and that picture is
/// decoded instead of refused as damaged.
#[test]
fn a_decoder_that_died_while_idle_is_replaced_not_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let path = saved_png(folder.path(), "Harbor.png", 120, 80);
    let pool = Pool::new(child_words());
    let decode = || {
        let file = std::fs::File::open(&path).expect("opened");
        pool.decode(Source::Reader(Box::new(file)), 60, 60, Limits::DISPLAY, None)
    };
    assert_eq!(decode().expect("decoded").pixbuf.width(), 60);
    let dead = {
        let idle = pool.idle.lock().expect("the pool");
        idle.first().expect("an idle decoder").child.0.id()
    };
    let group = Pid::from_raw(i32::try_from(dead).expect("a pid")).expect("a pid");
    kill_process_group(group, Signal::KILL).expect("killed");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !has_ended(dead) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(has_ended(dead), "the decoder was killed");
    assert_eq!(decode().expect("decoded, not refused").pixbuf.width(), 60);
    let idle = pool.idle.lock().expect("the pool");
    let kept = idle.first().expect("a new idle decoder").child.0.id();
    assert_ne!(kept, dead);
    assert!(
        std::fs::metadata(format!("/proc/{dead}")).is_err(),
        "the dead decoder was reaped"
    );
}
