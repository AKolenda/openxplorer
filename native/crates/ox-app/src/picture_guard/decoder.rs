// SPDX-License-Identifier: AGPL-3.0-only
//! Decoding a picture in a child process, so that no picture can take the
//! app down with it.
//!
//! gdk-pixbuf's loaders are C libraries (libpng, libjpeg, libtiff,
//! librsvg, and on newer systems glycin's glue), and a malformed file can
//! make one of them abort, crash or loop. [`super::decode_to_fit`]
//! therefore never runs them in the app: it starts the app again with
//! [`DECODER_FLAG`], streams the picture's bytes to that child's standard
//! input, and reads the decoded pixels back from its standard output. The
//! child runs the same guarded decoding as before
//! ([`super::decode_here_to_fit`]), limits its own processor time, memory
//! and file size first, and, where bubblewrap works, runs in the
//! thumbnailers' sandbox without the network, the session bus or any of
//! the user's files ([`crate::thumbnails::decoder_sandbox`]).
//!
//! The app waits at most the picture's time limit and a little more, and
//! kills the child when the caller cancels (the selection moved on). A
//! child that crashed, hung, was killed or wrote nonsense is a
//! [`Refusal`], and every caller shows the file's icon or "No preview
//! available" for one.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use rustix::process::{kill_process_group, Pid, Signal};

use super::{Decoded, Limits, Refusal, READ_CHUNK};

/// The first argument that starts the app as a picture decoder:
/// `<app> --ox-decode-picture <width> <height> <max pixels> <max JPEG
/// pixels> <milliseconds>`, the picture on standard input.
pub(crate) const DECODER_FLAG: &str = "--ox-decode-picture";

/// What every answer starts with.
const MAGIC: &[u8; 8] = b"\0OXPIX1\0";

/// How much longer than the picture's own time limit the app waits for
/// the child, which needs a moment to start.
const GRACE: Duration = Duration::from_secs(3);

/// How often the app looks whether the child finished, was cancelled or
/// ran out of time.
const POLL: Duration = Duration::from_millis(5);

/// The processor time the child may use beyond the picture's time limit.
const CPU_GRACE_SECONDS: u64 = 5;

/// The memory the child may allocate (`RLIMIT_DATA`): a progressive JPEG
/// at the JPEG limit needs its coefficients in memory, and anything more
/// is a picture made to exhaust memory.
const MEMORY_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// The most file descriptors the child may open: gdk-pixbuf's loaders
/// read their modules, and glycin starts loaders of its own.
const OPEN_FILES: u64 = 256;

/// The pixbuf options the child passes back: a cached thumbnail's keys
/// (`thumbnails::writer`).
const OPTIONS: [&str; 4] = [
    "tEXt::Thumb::URI",
    "tEXt::Thumb::MTime",
    "tEXt::Thumb::Size",
    "tEXt::Thumb::Mimetype",
];

/// The longest option value passed back.
const OPTION_LIMIT: usize = 64 * 1024;

/// What a refusal is called on the wire, and back.
const REFUSALS: [(u8, Refusal); 5] = [
    (1, Refusal::TooLarge { width: 0, height: 0 }),
    (2, Refusal::Unreadable),
    (3, Refusal::Damaged),
    (4, Refusal::TimedOut),
    (5, Refusal::Cancelled),
];

/// Where the picture's bytes come from.
pub(super) enum Source {
    /// A file GIO reads, on this computer or a share.
    File(gio::File),
    /// A file the caller opened and checked itself.
    Reader(Box<dyn Read + Send>),
}

/// How the child is started on this computer, found once per run.
#[derive(Debug, Clone)]
enum Runner {
    /// The words before the child's own arguments: the app itself, in
    /// bubblewrap where that works here.
    Child(Vec<OsString>),
    /// No child can be started here (the app's own program cannot be run
    /// again): pictures are decoded in the app, as before.
    InProcess,
}

static RUNNER: OnceLock<Runner> = OnceLock::new();

/// The picture `source` gives, decoded to fit `width` × `height` within
/// `limits` in a child process. Blocks.
pub(super) fn decode(
    source: Source,
    width: i32,
    height: i32,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    if cancellable.is_some_and(CancellableExt::is_cancelled) {
        return Err(Refusal::Cancelled);
    }
    match RUNNER.get_or_init(find_runner) {
        Runner::Child(words) => run(words, source, width, height, limits, cancellable),
        Runner::InProcess => match source {
            Source::File(file) => super::decode_here_to_fit(&file, width, height, limits, cancellable),
            Source::Reader(mut reader) => {
                super::decode_reader_here_to_fit(&mut reader, width, height, limits, cancellable)
            }
        },
    }
}

/// How this computer runs the child: sandboxed where bubblewrap works
/// (with a `/proc` of its own, then without, as for the thumbnailers),
/// else on its own; each way is tried on a picture of one pixel.
fn find_runner() -> Runner {
    let Some(program) = program() else {
        glib::g_warning!(
            ox_core::LOG_DOMAIN,
            "The picture decoder cannot be started: pictures are decoded in the app"
        );
        return Runner::InProcess;
    };
    let mut candidates: Vec<Vec<OsString>> = Vec::new();
    if !cfg!(test) {
        for proc in [true, false] {
            candidates.extend(crate::thumbnails::decoder_sandbox(&program.0, proc));
        }
    }
    let mut plain = vec![program.0.clone().into_os_string()];
    plain.extend(program.1.iter().cloned());
    candidates.push(plain);
    if let Some(words) = candidates.into_iter().find(|words| works(words)) {
        return Runner::Child(words);
    }
    glib::g_warning!(
        ox_core::LOG_DOMAIN,
        "The picture decoder does not run here: pictures are decoded in the app"
    );
    Runner::InProcess
}

/// Whether the child started with `words` decodes a picture of one pixel.
fn works(words: &[OsString]) -> bool {
    let probe = Source::Reader(Box::new(std::io::Cursor::new(super::ONE_PIXEL_PNG)));
    run(words, probe, 1, 1, Limits::DISPLAY, None).is_ok_and(|decoded| decoded.width == 1)
}

/// The child's program and the arguments before its own: the running app
/// itself, by the path of its file (bubblewrap needs a path it can show
/// inside the sandbox; `/proc/self/exe` when the file was replaced by an
/// update while the app runs). In a test, the test program, made to run
/// [`tests::picture_decoder_child`] alone.
fn program() -> Option<(std::path::PathBuf, Vec<OsString>)> {
    let running = std::fs::read_link("/proc/self/exe")
        .ok()
        .filter(|path| path.is_file())
        .or_else(|| {
            let proc = std::path::Path::new("/proc/self/exe");
            proc.exists().then(|| proc.to_path_buf())
        })
        .or_else(|| std::env::current_exe().ok())?;
    let before: Vec<OsString> = if cfg!(test) {
        [
            "--exact",
            "picture_guard::decoder::tests::picture_decoder_child",
            "--nocapture",
            "--quiet",
            "--test-threads=1",
        ]
        .map(OsString::from)
        .to_vec()
    } else {
        Vec::new()
    };
    Some((running, before))
}

/// Starts the child with `words`, feeds it `source` and reads its answer.
fn run(
    words: &[OsString],
    source: Source,
    width: i32,
    height: i32,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    let (program, before) = words.split_first().ok_or(Refusal::Unreadable)?;
    let milliseconds = u64::try_from(limits.time.as_millis()).unwrap_or(u64::MAX);
    let parameters = format!(
        "{width} {height} {} {} {milliseconds}",
        limits.max_pixels, limits.max_jpeg_pixels
    );
    let mut command = Command::new(program);
    command
        .args(before)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // The child shows nothing and talks to nobody: it is handed none
        // of the app's environment beyond what finds gdk-pixbuf's loaders
        // and the user's language.
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| name.to_str().is_some_and(passed_on)))
        // A group of its own, so stopping it stops whatever it started.
        .process_group(0);
    if cfg!(test) {
        command.env(TEST_PARAMETERS, &parameters);
    } else {
        command.arg(DECODER_FLAG).args(parameters.split(' '));
    }
    let mut child = command.spawn().map_err(|_| Refusal::Unreadable)?;
    let stop_feeding = Arc::new(AtomicBool::new(false));
    if let Some(stdin) = child.stdin.take() {
        feed(source, stdin, Arc::clone(&stop_feeding), cancellable.cloned());
    }
    let answer = child.stdout.take().map(|stdout| {
        let limit = answer_limit(width, height, limits);
        thread::spawn(move || {
            let mut answer = Vec::new();
            let read = stdout.take(limit).read_to_end(&mut answer);
            read.ok().map(|_| answer)
        })
    });
    let deadline = Instant::now() + limits.time.saturating_add(GRACE);
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(_) => break Err(Refusal::Unreadable),
        }
        if cancellable.is_some_and(CancellableExt::is_cancelled) {
            break Err(Refusal::Cancelled);
        }
        if Instant::now() >= deadline {
            break Err(Refusal::TimedOut);
        }
        thread::sleep(POLL);
    };
    stop_feeding.store(true, Ordering::Relaxed);
    let status = match ended {
        Ok(status) => status,
        Err(refusal) => {
            stop(&mut child);
            return Err(refusal);
        }
    };
    // Whatever it started is stopped too, so its output ends.
    stop(&mut child);
    let answer = answer.and_then(|reader| reader.join().ok()).flatten();
    if !status.success() {
        glib::g_debug!(
            ox_core::LOG_DOMAIN,
            "A picture's decoder ended without an answer ({status}): the picture is not shown"
        );
        return Err(Refusal::Damaged);
    }
    parse_answer(&answer.ok_or(Refusal::Unreadable)?)
}

/// Whether the child is handed the environment variable `name`.
fn passed_on(name: &str) -> bool {
    matches!(
        name,
        "PATH"
            | "LANG"
            | "LANGUAGE"
            | "XDG_DATA_DIRS"
            | "GDK_PIXBUF_MODULE_FILE"
            | "GDK_PIXBUF_MODULEDIR"
            | "LD_LIBRARY_PATH"
    ) || name.starts_with("LC_")
}

/// Reads the next chunk of a picture into the buffer: how much it read,
/// `None` on an error.
type ReadChunk = dyn FnMut(&mut [u8]) -> Option<usize>;

/// Streams `source` to the child's `stdin` on a thread of its own, so a
/// child that stops reading (it decoded a GIF's first frame, refused the
/// header, or hangs) never holds the caller. Reading stops when `stop`
/// is set or `cancellable` is cancelled; a read that blocks (a share that
/// stopped answering) leaves only this thread waiting.
fn feed(
    source: Source,
    mut stdin: std::process::ChildStdin,
    stop: Arc<AtomicBool>,
    cancellable: Option<gio::Cancellable>,
) {
    thread::spawn(move || {
        let mut buffer = vec![0; READ_CHUNK];
        let mut next: Box<ReadChunk> = match source {
            Source::File(file) => {
                let Ok(stream) = file.read(cancellable.as_ref()) else {
                    return;
                };
                let cancellable = cancellable.clone();
                Box::new(move |buffer| stream.read(buffer, cancellable.as_ref()).ok())
            }
            Source::Reader(mut reader) => Box::new(move |buffer| reader.read(buffer).ok()),
        };
        while !stop.load(Ordering::Relaxed) && !cancellable.as_ref().is_some_and(CancellableExt::is_cancelled)
        {
            let Some(read) = next(&mut buffer) else {
                return;
            };
            let Some(bytes) = buffer.get(..read).filter(|bytes| !bytes.is_empty()) else {
                return;
            };
            if stdin.write_all(bytes).is_err() {
                return;
            }
        }
    });
}

/// The most the app reads of the child's answer: the pixels of the
/// largest picture `limits` let it decode to fit `width` × `height`, at
/// four bytes a pixel and padded rows, and room for the options.
fn answer_limit(width: i32, height: i32, limits: Limits) -> u64 {
    let side = |side: i32| u64::try_from(side.max(1)).unwrap_or(1);
    let fit = side(width).saturating_mul(side(height));
    let largest = limits.max_pixels.max(limits.max_jpeg_pixels);
    // A vector picture is drawn at the size asked for, however large.
    let pixels = fit.min(largest.max(super::MAX_PIXELS));
    pixels
        .saturating_mul(4)
        .saturating_add(side(height).saturating_mul(8))
        .saturating_add(1024 * 1024)
}

/// Kills the process group `child` leads, and reaps the child.
fn stop(child: &mut Child) {
    let group = i32::try_from(child.id())
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|group| kill_process_group(group, Signal::KILL).is_ok());
    if !group {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Reads an answer in `bytes`: [`MAGIC`], then a status byte, then for a
/// decoded picture its own size, the decoded size, row stride, alpha,
/// options and pixels; for a refusal its size. Nothing the child wrote is
/// trusted: every length and size is checked before it is used.
fn parse_answer(bytes: &[u8]) -> Result<Decoded, Refusal> {
    let start = find(bytes, MAGIC).ok_or(Refusal::Unreadable)?;
    let mut reader = Wire(bytes.get(start + MAGIC.len()..).ok_or(Refusal::Unreadable)?);
    let bad = Refusal::Unreadable;
    let status = reader.byte().ok_or(bad)?;
    if status != 0 {
        let (_, refusal) = REFUSALS
            .iter()
            .find(|(code, _)| *code == status)
            .copied()
            .ok_or(bad)?;
        if let Refusal::TooLarge { .. } = refusal {
            let width = reader.int().ok_or(bad)?;
            let height = reader.int().ok_or(bad)?;
            return Err(Refusal::TooLarge { width, height });
        }
        return Err(refusal);
    }
    let own_width = reader.int().ok_or(bad)?;
    let own_height = reader.int().ok_or(bad)?;
    let width = reader.int().ok_or(bad)?;
    let height = reader.int().ok_or(bad)?;
    let rowstride = reader.int().ok_or(bad)?;
    let has_alpha = match reader.byte().ok_or(bad)? {
        0 => false,
        1 => true,
        _ => return Err(bad),
    };
    let channels: usize = if has_alpha { 4 } else { 3 };
    let mut options = Vec::new();
    let count = reader.length().ok_or(bad)?;
    if count > OPTIONS.len() {
        return Err(bad);
    }
    for _ in 0..count {
        let key = reader.text().ok_or(bad)?;
        let value = reader.text().ok_or(bad)?;
        if OPTIONS.contains(&key.as_str()) {
            options.push((key, value));
        }
    }
    let pixels = reader.rest();
    let (Ok(columns), Ok(rows), Ok(stride)) = (
        usize::try_from(width),
        usize::try_from(height),
        usize::try_from(rowstride),
    ) else {
        return Err(bad);
    };
    let row = columns.checked_mul(channels).ok_or(bad)?;
    // The last row may be unpadded, as gdk-pixbuf keeps it.
    let needed = stride
        .checked_mul(rows.checked_sub(1).ok_or(bad)?)
        .and_then(|before| before.checked_add(row))
        .ok_or(bad)?;
    let whole = stride.checked_mul(rows).ok_or(bad)?;
    let fits = columns > 0 && rows > 0 && stride >= row && stride - row < 64 && pixels.len() == needed;
    if !fits || own_width <= 0 || own_height <= 0 {
        return Err(bad);
    }
    let mut padded = Vec::with_capacity(whole);
    padded.extend_from_slice(pixels);
    padded.resize(whole, 0);
    let pixbuf = Pixbuf::from_bytes(
        &glib::Bytes::from_owned(padded),
        Colorspace::Rgb,
        has_alpha,
        8,
        width,
        height,
        rowstride,
    );
    for (key, value) in &options {
        pixbuf.set_option(key, value);
    }
    Ok(Decoded {
        pixbuf,
        width: own_width,
        height: own_height,
    })
}

/// Where `needle` first starts in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Reads the answer's fields in order.
struct Wire<'a>(&'a [u8]);

impl Wire<'_> {
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let (taken, rest) = (self.0.get(..count)?, self.0.get(count..)?);
        self.0 = rest;
        Some(taken)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn int(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn length(&mut self) -> Option<usize> {
        usize::try_from(u32::from_le_bytes(self.take(4)?.try_into().ok()?)).ok()
    }

    fn text(&mut self) -> Option<String> {
        let length = self.length().filter(|length| *length <= OPTION_LIMIT)?;
        String::from_utf8(self.take(length)?.to_vec()).ok()
    }

    fn rest(&self) -> &[u8] {
        self.0
    }
}

/// The answer the child writes for `result`.
fn answer(result: &Result<Decoded, Refusal>) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    let int = |bytes: &mut Vec<u8>, value: i32| bytes.extend_from_slice(&value.to_le_bytes());
    let length = |bytes: &mut Vec<u8>, value: usize| {
        bytes.extend_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
    };
    match result {
        Ok(decoded) => {
            let pixbuf = &decoded.pixbuf;
            // Anything but 8-bit RGB is not passed on.
            if pixbuf.bits_per_sample() != 8 || pixbuf.n_channels() != if pixbuf.has_alpha() { 4 } else { 3 }
            {
                bytes.push(3);
                return bytes;
            }
            bytes.push(0);
            for value in [
                decoded.width,
                decoded.height,
                pixbuf.width(),
                pixbuf.height(),
                pixbuf.rowstride(),
            ] {
                int(&mut bytes, value);
            }
            bytes.push(u8::from(pixbuf.has_alpha()));
            let options: Vec<(&str, glib::GString)> = OPTIONS
                .iter()
                .filter_map(|key| Some((*key, pixbuf.option(key)?)))
                .filter(|(_, value)| value.len() <= OPTION_LIMIT)
                .collect();
            length(&mut bytes, options.len());
            for (key, value) in options {
                length(&mut bytes, key.len());
                bytes.extend_from_slice(key.as_bytes());
                length(&mut bytes, value.len());
                bytes.extend_from_slice(value.as_bytes());
            }
            bytes.extend_from_slice(&pixbuf.read_pixel_bytes());
        }
        Err(refusal) => {
            let code = REFUSALS
                .iter()
                .find(|(_, known)| std::mem::discriminant(known) == std::mem::discriminant(refusal))
                .map_or(2, |(code, _)| *code);
            bytes.push(code);
            if let Refusal::TooLarge { width, height } = refusal {
                int(&mut bytes, *width);
                int(&mut bytes, *height);
            }
        }
    }
    bytes
}

/// The parameters of a decode: `<width> <height> <max pixels> <max JPEG
/// pixels> <milliseconds>`.
fn parse_parameters<'a>(mut words: impl Iterator<Item = &'a str>) -> Option<(i32, i32, Limits)> {
    let width = words.next()?.parse().ok()?;
    let height = words.next()?.parse().ok()?;
    let max_pixels = words.next()?.parse().ok()?;
    let max_jpeg_pixels = words.next()?.parse().ok()?;
    let time = Duration::from_millis(words.next()?.parse().ok()?);
    let limits = Limits {
        max_pixels,
        max_jpeg_pixels,
        time,
    };
    Some((width, height, limits))
}

/// Decodes the picture on standard input with the parameters given, and
/// writes the answer to standard output: what the child does.
fn decode_standard_input(width: i32, height: i32, limits: Limits) -> bool {
    // A picture that keeps the child busy, or makes it allocate without
    // end, is stopped by the kernel too; it writes no files and leaves no
    // core.
    let cpu = limits.time.as_secs().saturating_add(CPU_GRACE_SECONDS);
    for (resource, limit) in [
        (rustix::process::Resource::Cpu, cpu),
        (rustix::process::Resource::Data, MEMORY_LIMIT),
        (rustix::process::Resource::Fsize, 0),
        (rustix::process::Resource::Core, 0),
        (rustix::process::Resource::Nofile, OPEN_FILES),
    ] {
        let inherited = rustix::process::getrlimit(resource).maximum;
        let limit = inherited.map_or(limit, |hard| hard.min(limit));
        let _ = rustix::process::setrlimit(
            resource,
            rustix::process::Rlimit {
                current: Some(limit),
                maximum: Some(limit),
            },
        );
    }
    let mut input = std::io::stdin().lock();
    let result = super::decode_reader_here_to_fit(&mut input, width, height, limits, None);
    let mut output = std::io::stdout().lock();
    output.write_all(&answer(&result)).is_ok() && output.flush().is_ok()
}

/// Runs the app as a picture decoder when `arguments` ask for it. Returns
/// the exit status then, `None` for every other command line.
pub(crate) fn run_as_decoder(arguments: &[String]) -> Option<glib::ExitCode> {
    if arguments.get(1).map(String::as_str) != Some(DECODER_FLAG) {
        return None;
    }
    let Some((width, height, limits)) = parse_parameters(arguments.iter().skip(2).map(String::as_str)) else {
        return Some(glib::ExitCode::FAILURE);
    };
    Some(if decode_standard_input(width, height, limits) {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    })
}

/// The environment variable that carries a test child's parameters.
const TEST_PARAMETERS: &str = "OX_TEST_PICTURE_DECODER";

#[cfg(test)]
pub(crate) mod tests;
