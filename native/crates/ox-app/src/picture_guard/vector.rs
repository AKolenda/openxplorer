// SPDX-License-Identifier: AGPL-3.0-only
//! Drawing vector pictures (SVG) in a child process.
//!
//! A vector picture is a document that gdk-pixbuf reads whole and draws
//! only once its loader is closed, and drawing one can take any amount of
//! processor time and memory however small its file is (a single line of
//! very long text, for one). Neither can be stopped from outside once the
//! drawing has started. So the app draws it in a child process: the app
//! itself, started again with [`DRAWER_FLAG`], which limits its own
//! processor time ([`DRAWER_CPU_SECONDS`]) and memory
//! ([`DRAWER_MEMORY_BYTES`]), draws the picture to fit and writes it back
//! as a PNG. The app waits no longer than the picture's time limit and
//! stops the child then, or when the caller cancels. A child that ran out
//! of time or memory, crashed or wrote nonsense only leaves the picture
//! refused, so the caller shows the file's type icon.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use super::{decode_loaded, Decoded, Limits, Refusal};

/// The argument that makes the app draw one vector picture and exit.
pub(crate) const DRAWER_FLAG: &str = "--ox-draw-vector-picture";

/// The processor time the drawing process may use.
const DRAWER_CPU_SECONDS: u64 = 10;

/// The memory the drawing process may write to (`RLIMIT_DATA`, not address
/// space: library threads reserve far more than they use).
const DRAWER_MEMORY_BYTES: u64 = 1 << 30;

/// The largest vector picture file drawn.
const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;

/// The most the drawing process may write back: a PNG of a picture within
/// [`super::MAX_PIXELS`], uncompressed at worst.
const MAX_OUTPUT_BYTES: usize = 300 * 1024 * 1024;

/// How many first bytes are looked at to tell a vector picture, as many as
/// gdk-pixbuf looks at to tell a picture's format.
pub(super) const SNIFF_BYTES: usize = 4096;

/// What the drawing process writes before its answer.
const ANSWER_MARK: &[u8] = b"OX-VECTOR-PICTURE-1\n";

/// How often the app looks whether the drawing process has finished.
const POLL: Duration = Duration::from_millis(10);

/// Whether `first`, the first bytes of a picture, may be a vector picture:
/// markup (after a byte order mark and white space), UTF-16 text, or an
/// SVG signature anywhere in them, as gdk-pixbuf looks for one.
pub(super) fn may_be_vector(first: &[u8]) -> bool {
    let head = &first[..first.len().min(SNIFF_BYTES)];
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    if text.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<') {
        return true;
    }
    let lowered = head.to_ascii_lowercase();
    lowered.windows(4).any(|window| window == b"<svg")
}

/// The whole vector picture: `first` and what `next_chunk` reads after
/// it, within the time limit and [`MAX_DOCUMENT_BYTES`].
pub(super) fn read_document(
    first: Vec<u8>,
    mut next_chunk: impl FnMut(&mut [u8]) -> Result<usize, ()>,
    started: Instant,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Vec<u8>, Refusal> {
    let mut document = first;
    let mut buffer = vec![0; super::READ_CHUNK];
    loop {
        if cancellable.is_some_and(CancellableExt::is_cancelled) {
            return Err(Refusal::Cancelled);
        }
        if started.elapsed() >= limits.time {
            return Err(Refusal::TimedOut);
        }
        let read = next_chunk(&mut buffer).map_err(|()| super::refusal_for(cancellable))?;
        if read == 0 {
            return Ok(document);
        }
        if document.len() + read > MAX_DOCUMENT_BYTES {
            return Err(Refusal::TooLarge { width: 0, height: 0 });
        }
        document.extend_from_slice(&buffer[..read]);
    }
}

/// `document` drawn to fit `width` × `height` in a drawing process,
/// within `limits` from `started`. Blocks.
pub(super) fn draw(
    document: &[u8],
    width: i32,
    height: i32,
    limits: Limits,
    started: Instant,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    let mut command = drawer_command(width.max(1), height.max(1)).ok_or(Refusal::Unreadable)?;
    // The drawing process shows nothing, so it never connects to the
    // display.
    command
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| Refusal::Unreadable)?;
    let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Refusal::Unreadable);
    };
    let answer = std::thread::scope(|scope| {
        // The document goes in and the answer comes out on threads of
        // their own, so a full pipe never holds the wait below.
        scope.spawn(move || {
            let _ = input.write_all(document);
        });
        let reader = scope.spawn(move || {
            let mut answer = Vec::new();
            let limit = u64::try_from(MAX_OUTPUT_BYTES + ANSWER_MARK.len() + 1024).unwrap_or(u64::MAX);
            output.take(limit).read_to_end(&mut answer).ok().map(|_| answer)
        });
        let finished = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.success()),
                Ok(None) => {}
                Err(_) => break Err(Refusal::Unreadable),
            }
            if cancellable.is_some_and(CancellableExt::is_cancelled) {
                break Err(Refusal::Cancelled);
            }
            if started.elapsed() >= limits.time {
                break Err(Refusal::TimedOut);
            }
            std::thread::sleep(POLL);
        };
        if finished.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        let answer = reader.join().ok().flatten();
        finished.map(|success| answer.filter(|_| success))
    })?;
    // A drawing process that ran out of time or memory, or crashed, ends
    // unsuccessfully.
    let answer = answer.ok_or(Refusal::Damaged)?;
    read_answer(&answer)
}

/// The command that starts the drawing process for a picture to fit
/// `width` × `height`.
#[cfg(not(test))]
fn drawer_command(width: i32, height: i32) -> Option<Command> {
    let program = crate::media_details::reader_program()?;
    let mut command = Command::new(program);
    command
        .arg(DRAWER_FLAG)
        .arg(width.to_string())
        .arg(height.to_string());
    Some(command)
}

/// Tests are not the app: the drawing process is the test program, made to
/// run only [`tests::drawing_process`], which draws when
/// [`tests::DRAWING_PROCESS`] names the size.
#[cfg(test)]
fn drawer_command(width: i32, height: i32) -> Option<Command> {
    let mut command = Command::new(std::env::current_exe().ok()?);
    command
        .args([
            "--exact",
            "picture_guard::vector::tests::drawing_process",
            "--nocapture",
            "--test-threads=1",
            "--quiet",
        ])
        .env(tests::DRAWING_PROCESS, format!("{width} {height}"));
    Some(command)
}

/// What the drawing process answered: the picture's own size and the
/// picture drawn, or why it was refused.
fn read_answer(answer: &[u8]) -> Result<Decoded, Refusal> {
    // Anything written before the mark (a test program's own lines) is
    // passed over.
    let at = answer
        .windows(ANSWER_MARK.len())
        .position(|window| window == ANSWER_MARK)
        .ok_or(Refusal::Damaged)?;
    let answer = &answer[at + ANSWER_MARK.len()..];
    let (&status, rest) = answer.split_first().ok_or(Refusal::Damaged)?;
    let number = |at: usize| {
        rest.get(at..at + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(i32::from_le_bytes)
            .ok_or(Refusal::Damaged)
    };
    let (width, height) = (number(0)?, number(4)?);
    match status {
        STATUS_DRAWN => {
            let mut picture = rest.get(8..).ok_or(Refusal::Damaged)?;
            // The drawing process writes PNG; nothing else goes to a loader.
            if !picture.starts_with(super::PNG_SIGNATURE) {
                return Err(Refusal::Damaged);
            }
            let read = |buffer: &mut [u8]| picture.read(buffer).map_err(|_| ());
            // The PNG the drawing process wrote is checked as any picture.
            let drawn = decode_loaded(read, Instant::now(), i32::MAX, i32::MAX, Limits::DISPLAY, None)
                .map_err(|_| Refusal::Damaged)?;
            Ok(Decoded {
                pixbuf: drawn.pixbuf,
                width,
                height,
            })
        }
        STATUS_TOO_LARGE => Err(Refusal::TooLarge { width, height }),
        STATUS_UNREADABLE => Err(Refusal::Unreadable),
        STATUS_TIMED_OUT => Err(Refusal::TimedOut),
        _ => Err(Refusal::Damaged),
    }
}

/// Reads `bytes` as the drawing process's answer, for the fuzz targets.
#[cfg(feature = "fuzzing")]
pub(crate) fn fuzz_vector_answer(bytes: &[u8]) {
    let _ = may_be_vector(bytes);
    let _ = read_answer(bytes);
}

/// The picture was drawn.
const STATUS_DRAWN: u8 = 0;
/// [`Refusal::TooLarge`].
const STATUS_TOO_LARGE: u8 = 1;
/// [`Refusal::Unreadable`].
const STATUS_UNREADABLE: u8 = 2;
/// [`Refusal::Damaged`].
const STATUS_DAMAGED: u8 = 3;
/// [`Refusal::TimedOut`].
const STATUS_TIMED_OUT: u8 = 4;

/// Runs the app as the drawing process when `arguments` ask for it
/// ([`DRAWER_FLAG`], then the width and height to fit): draws the picture
/// it reads from its input and writes the answer to its output.
pub(crate) fn run_as_drawer(arguments: &[String]) -> Option<glib::ExitCode> {
    if arguments.get(1).map(String::as_str) != Some(DRAWER_FLAG) {
        return None;
    }
    let size = |at: usize| arguments.get(at).and_then(|value| value.parse::<i32>().ok());
    let (Some(width), Some(height)) = (size(2), size(3)) else {
        return Some(glib::ExitCode::FAILURE);
    };
    Some(draw_here(width, height))
}

/// Draws the picture on this process's input to fit `width` × `height`,
/// within its own limits, and writes the answer to its output.
/// Limits this process to `cpu_seconds` of processor time in all and
/// [`DRAWER_MEMORY_BYTES`] of memory, never raising a limit already lower:
/// a picture that keeps the drawing busy, or makes it allocate without
/// end, is stopped by the kernel too, and a refused allocation aborts the
/// process. The drawing process sets them first thing.
fn limit_drawing(cpu_seconds: u64) {
    for (resource, limit) in [
        (rustix::process::Resource::Cpu, cpu_seconds),
        (rustix::process::Resource::Data, DRAWER_MEMORY_BYTES),
    ] {
        let inherited = rustix::process::getrlimit(resource).maximum;
        let limit = inherited.map_or(limit, |hard| hard.min(limit));
        let limit = rustix::process::Rlimit {
            current: Some(limit),
            maximum: Some(limit),
        };
        let _ = rustix::process::setrlimit(resource, limit);
    }
}

/// The drawing limits of a vector picture for a process that has already
/// run (a picture decoder, which ends after this picture): processor time
/// for [`DRAWER_CPU_SECONDS`] more on top of what it used so far, and
/// [`DRAWER_MEMORY_BYTES`] of memory. The picture's time limit is kept by
/// the app, which stops a decoder that has not answered by then.
pub(super) fn limit_drawing_after_use() {
    let used = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    let used_seconds = u64::try_from(used.tv_sec).unwrap_or(0).saturating_add(1);
    limit_drawing(used_seconds.saturating_add(DRAWER_CPU_SECONDS));
}

fn draw_here(width: i32, height: i32) -> glib::ExitCode {
    limit_drawing(DRAWER_CPU_SECONDS);
    let mut input = std::io::stdin()
        .lock()
        .take(u64::try_from(MAX_DOCUMENT_BYTES).unwrap_or(u64::MAX));
    let read = |buffer: &mut [u8]| input.read(buffer).map_err(|_| ());
    let drawn = decode_loaded(read, Instant::now(), width, height, Limits::DISPLAY, None);
    let mut answer = ANSWER_MARK.to_vec();
    let (status, own_size) = match &drawn {
        Ok(decoded) => (STATUS_DRAWN, (decoded.width, decoded.height)),
        Err(Refusal::TooLarge { width, height }) => (STATUS_TOO_LARGE, (*width, *height)),
        Err(Refusal::TimedOut) => (STATUS_TIMED_OUT, (0, 0)),
        Err(Refusal::Unreadable) => (STATUS_UNREADABLE, (0, 0)),
        Err(Refusal::Damaged | Refusal::Cancelled) => (STATUS_DAMAGED, (0, 0)),
    };
    answer.push(status);
    answer.extend_from_slice(&own_size.0.to_le_bytes());
    answer.extend_from_slice(&own_size.1.to_le_bytes());
    if let Ok(decoded) = drawn {
        match decoded.pixbuf.save_to_bufferv("png", &[("compression", "1")]) {
            Ok(png) => answer.extend_from_slice(&png),
            Err(_) => return glib::ExitCode::FAILURE,
        }
    }
    let mut output = std::io::stdout().lock();
    if output.write_all(&answer).and_then(|()| output.flush()).is_err() {
        return glib::ExitCode::FAILURE;
    }
    glib::ExitCode::SUCCESS
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Names the size to fit when the test program runs as the drawing
    /// process.
    pub(crate) const DRAWING_PROCESS: &str = "OX_TEST_DRAWING_PROCESS";

    /// The drawing process of the tests: draws and exits when
    /// [`DRAWING_PROCESS`] is set, and does nothing in an ordinary run.
    #[test]
    fn drawing_process() {
        let Ok(size) = std::env::var(DRAWING_PROCESS) else {
            return;
        };
        let mut size = size.split(' ').filter_map(|side| side.parse::<i32>().ok());
        let (Some(width), Some(height)) = (size.next(), size.next()) else {
            std::process::exit(1);
        };
        let status = draw_here(width, height);
        std::process::exit(i32::from(status != glib::ExitCode::SUCCESS));
    }

    /// Markup and text that names an SVG may be a vector picture; the
    /// raster formats' own signatures are not.
    #[test]
    fn vector_pictures_are_told_from_their_first_bytes() {
        assert!(may_be_vector(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(may_be_vector(b"\xEF\xBB\xBF\n  <?xml version=\"1.0\"?><svg/>"));
        assert!(may_be_vector(b"<!-- Logo --><svg/>"));
        assert!(may_be_vector(b"junk before <SVG width=\"1\"/>"));
        assert!(may_be_vector(b"\xFF\xFE<\0s\0v\0g\0"));
        assert!(!may_be_vector(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(!may_be_vector(b"\xFF\xD8\xFF\xE0\0\x10JFIF"));
        assert!(!may_be_vector(b"GIF89a"));
        assert!(!may_be_vector(b""));
    }

    /// An answer that is not the drawing process's is refused.
    #[test]
    fn a_broken_answer_is_refused() {
        assert_eq!(read_answer(b"").map(|_| ()), Err(Refusal::Damaged));
        let mut cut = ANSWER_MARK.to_vec();
        cut.extend_from_slice(&[STATUS_DRAWN, 1, 0]);
        assert_eq!(read_answer(&cut).map(|_| ()), Err(Refusal::Damaged));
        let mut not_png = ANSWER_MARK.to_vec();
        not_png.push(STATUS_DRAWN);
        not_png.extend_from_slice(&10_i32.to_le_bytes());
        not_png.extend_from_slice(&10_i32.to_le_bytes());
        not_png.extend_from_slice(b"not a picture");
        assert_eq!(read_answer(&not_png).map(|_| ()), Err(Refusal::Damaged));
        // The drawing process writes PNG: a picture of another format is
        // not handed to that format's loader (found by fuzzing).
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 2, 2)
            .expect("a small picture");
        let bmp = pixbuf.save_to_bufferv("bmp", &[]).expect("a BMP");
        let mut drawn_bmp = ANSWER_MARK.to_vec();
        drawn_bmp.push(STATUS_DRAWN);
        drawn_bmp.extend_from_slice(&2_i32.to_le_bytes());
        drawn_bmp.extend_from_slice(&2_i32.to_le_bytes());
        drawn_bmp.extend_from_slice(&bmp);
        assert_eq!(read_answer(&drawn_bmp).map(|_| ()), Err(Refusal::Damaged));
        let png = pixbuf.save_to_bufferv("png", &[]).expect("a PNG");
        let mut drawn_png = drawn_bmp[..drawn_bmp.len() - bmp.len()].to_vec();
        drawn_png.extend_from_slice(&png);
        assert!(read_answer(&drawn_png).is_ok(), "a drawn PNG is read");
        let mut too_large = b"running 1 test\n".to_vec();
        too_large.extend_from_slice(ANSWER_MARK);
        too_large.push(STATUS_TOO_LARGE);
        too_large.extend_from_slice(&7_i32.to_le_bytes());
        too_large.extend_from_slice(&9_i32.to_le_bytes());
        assert_eq!(
            read_answer(&too_large).map(|_| ()),
            Err(Refusal::TooLarge { width: 7, height: 9 })
        );
    }
}
