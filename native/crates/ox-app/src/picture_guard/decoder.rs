// SPDX-License-Identifier: AGPL-3.0-only
//! Decoding a picture in a child process, so that no picture can take the
//! app down with it.
//!
//! gdk-pixbuf's loaders are C libraries (libpng, libjpeg, libtiff,
//! librsvg, and on newer systems glycin's glue), and a malformed file can
//! make one of them abort, crash or loop. [`super::decode_to_fit`]
//! therefore never runs them in the app: it keeps a small pool of
//! decoder processes, the app started again with [`DECODER_FLAG`], each
//! serving one picture at a time for many pictures in turn ([`Pool`]). A
//! picture's bytes are streamed to a decoder's standard input and its
//! pixels read back from its standard output. The decoder runs the same
//! guarded decoding as before ([`super::decode_here_to_fit`]), limits its
//! own processor time, memory and file size first, and, where bubblewrap
//! works, runs in the thumbnailers' sandbox without the network, the
//! session bus or any of the user's files
//! ([`crate::thumbnails::decoder_sandbox`]).
//!
//! The app waits at most the picture's time limit and a little more, and
//! kills the decoder when the caller cancels (the selection moved on). A
//! decoder that crashed, hung, was killed or wrote nonsense is a
//! [`Refusal`], and every caller shows the file's icon or "No preview
//! available" for one; the next picture starts a new decoder.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use rustix::process::{kill_process_group, Pid, Signal};

use super::{Decoded, Limits, Refusal, READ_CHUNK};

/// The first argument that starts the app as a picture decoder, which
/// serves requests ([`REQUEST`]) on standard input until it ends.
pub(crate) const DECODER_FLAG: &str = "--ox-decode-picture";

/// What every answer starts with.
const MAGIC: &[u8; 8] = b"\0OXPIX1\0";

/// How much longer than the picture's own time limit the app waits for
/// the child, which needs a moment to start.
const GRACE: Duration = Duration::from_secs(3);

/// How often the app looks whether the child finished, was cancelled or
/// ran out of time.
const POLL: Duration = Duration::from_millis(5);

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

/// How the decoders are started on this computer, found once per run.
#[derive(Debug)]
enum Runner {
    /// A pool of decoder processes started with these words: the app
    /// itself, in bubblewrap where that works here.
    Pool(Pool),
    /// No child can be started here (the app's own program cannot be run
    /// again): pictures are decoded in the app, as before.
    InProcess,
}

static RUNNER: OnceLock<Runner> = OnceLock::new();

/// The picture `source` gives, decoded to fit `width` × `height` within
/// `limits` by a decoder process. Blocks.
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
        Runner::Pool(pool) => pool.decode(source, width, height, limits, cancellable),
        Runner::InProcess => match source {
            Source::File(file) => super::decode_here_to_fit(&file, width, height, limits, cancellable),
            Source::Reader(mut reader) => {
                super::decode_reader_here_to_fit(&mut reader, width, height, limits, cancellable)
            }
        },
    }
}

/// How this computer runs the decoders: sandboxed where bubblewrap works
/// (with a `/proc` of its own, then without, as for the thumbnailers),
/// else on their own; each way is tried on a picture of one pixel.
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
        return Runner::Pool(Pool::new(words));
    }
    glib::g_warning!(
        ox_core::LOG_DOMAIN,
        "The picture decoder does not run here: pictures are decoded in the app"
    );
    Runner::InProcess
}

/// Whether the decoder started with `words` decodes a picture of one
/// pixel.
fn works(words: &[OsString]) -> bool {
    let probe = Source::Reader(Box::new(std::io::Cursor::new(super::ONE_PIXEL_PNG)));
    run(words, probe, 1, 1, Limits::DISPLAY, None).is_ok_and(|decoded| decoded.width == 1)
}

/// The decoder's program and the arguments before its own: the running
/// app itself, by the path of its file (bubblewrap needs a path it can
/// show inside the sandbox; `/proc/self/exe` when the file was replaced
/// by an update while the app runs). In a test, the test program, made to
/// run [`tests::picture_decoder_child`] alone.
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

/// How many idle decoders are kept waiting for the next picture: the
/// thumbnail slots decode a few at a time, and each decoder serves one
/// picture at a time.
const POOL_SIZE: usize = 4;

/// How many pictures one decoder serves before it is replaced, so that
/// nothing a picture left in it (a leak, a fragmented heap) builds up.
const PICTURES_PER_DECODER: u32 = 500;

/// The processor time one decoder may use over its whole life, in
/// seconds; each picture is held to its own time limit by the app.
const DECODER_CPU_SECONDS: u64 = 600;

/// How long a decoder may take to take the rest of a picture back once
/// it answered, before it is replaced instead.
const RETURN_WAIT: Duration = Duration::from_secs(2);

/// The decoders kept idle, and how to start more.
#[derive(Debug)]
struct Pool {
    words: Vec<OsString>,
    idle: Mutex<Vec<Decoder>>,
}

impl Pool {
    fn new(words: Vec<OsString>) -> Self {
        Self {
            words,
            idle: Mutex::new(Vec::new()),
        }
    }

    /// Decodes with an idle decoder, or a new one; a decoder that served
    /// well goes back to the pool, any other is stopped.
    fn decode(
        &self,
        source: Source,
        width: i32,
        height: i32,
        limits: Limits,
        cancellable: Option<&gio::Cancellable>,
    ) -> Result<Decoded, Refusal> {
        let idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner).pop();
        let decoder = match idle {
            Some(decoder) => decoder,
            None => Decoder::start(&self.words)?,
        };
        let (result, kept) = decoder.decode(source, width, height, limits, cancellable);
        if let Some(decoder) = kept {
            let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
            if idle.len() < POOL_SIZE && decoder.served < PICTURES_PER_DECODER {
                idle.push(decoder);
            }
        }
        result
    }
}

/// Starts a decoder with `words`, decodes `source` with it and stops it.
fn run(
    words: &[OsString],
    source: Source,
    width: i32,
    height: i32,
    limits: Limits,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Decoded, Refusal> {
    Decoder::start(words)?
        .decode(source, width, height, limits, cancellable)
        .0
}

/// One decoder process, waiting for pictures.
#[derive(Debug)]
struct Decoder {
    child: Process,
    stdin: std::process::ChildStdin,
    /// Its answers, read off its standard output by a thread of their own:
    /// each one whole, or nothing more once it ended or lied.
    answers: mpsc::Receiver<Vec<u8>>,
    /// The most the next answer may be, which the reading thread holds to.
    limit: Arc<std::sync::atomic::AtomicU64>,
    /// How many pictures it decoded.
    served: u32,
}

impl Decoder {
    /// Starts a decoder with `words`.
    fn start(words: &[OsString]) -> Result<Self, Refusal> {
        let (program, before) = words.split_first().ok_or(Refusal::Unreadable)?;
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
            command.env(TEST_DECODER, "1");
        } else {
            command.arg(DECODER_FLAG);
        }
        let mut child = Process(command.spawn().map_err(|_| Refusal::Unreadable)?);
        let (Some(stdin), Some(stdout)) = (child.0.stdin.take(), child.0.stdout.take()) else {
            return Err(Refusal::Unreadable);
        };
        let (sender, answers) = mpsc::channel();
        let limit = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let reading = Arc::clone(&limit);
        thread::Builder::new()
            .name("ox-picture-answers".to_owned())
            .spawn(move || {
                let mut stdout = std::io::BufReader::new(stdout);
                while let Some(answer) = read_frame(&mut stdout, &reading) {
                    if sender.send(answer).is_err() {
                        return;
                    }
                }
            })
            .map_err(|_| Refusal::Unreadable)?;
        Ok(Self {
            child,
            stdin,
            answers,
            limit,
            served: 0,
        })
    }

    /// Decodes `source`: the result, and the decoder again when it can
    /// serve another picture (it answered, and took the rest of this one
    /// back in time). A decoder that crashed, hung past the time limit,
    /// lied or was no longer wanted is stopped.
    fn decode(
        self,
        source: Source,
        width: i32,
        height: i32,
        limits: Limits,
        cancellable: Option<&gio::Cancellable>,
    ) -> (Result<Decoded, Refusal>, Option<Self>) {
        self.limit
            .store(answer_limit(width, height, limits), Ordering::Relaxed);
        // Drop any answer left from an earlier picture: there is none, as
        // each is taken before the decoder is reused.
        while self.answers.try_recv().is_ok() {}
        let mut request = REQUEST.to_vec();
        request.extend_from_slice(&width.to_le_bytes());
        request.extend_from_slice(&height.to_le_bytes());
        request.extend_from_slice(&limits.max_pixels.to_le_bytes());
        request.extend_from_slice(&limits.max_jpeg_pixels.to_le_bytes());
        let milliseconds = u64::try_from(limits.time.as_millis()).unwrap_or(u64::MAX);
        request.extend_from_slice(&milliseconds.to_le_bytes());
        let answered = Arc::new(AtomicBool::new(false));
        let (give_back, returned) = mpsc::channel();
        let stdin = self.stdin;
        feed(
            source,
            stdin,
            request,
            Arc::clone(&answered),
            cancellable.cloned(),
            give_back,
        );
        let deadline = Instant::now() + limits.time.saturating_add(GRACE);
        let outcome = loop {
            match self.answers.recv_timeout(POLL) {
                Ok(answer) => break Ok(answer),
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(Refusal::Damaged),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if cancellable.is_some_and(CancellableExt::is_cancelled) {
                break Err(Refusal::Cancelled);
            }
            if Instant::now() >= deadline {
                break Err(Refusal::TimedOut);
            }
        };
        answered.store(true, Ordering::Relaxed);
        let child = self.child;
        let answer = match outcome {
            Ok(answer) => answer,
            Err(refusal) => {
                if refusal == Refusal::Damaged {
                    glib::g_debug!(
                        ox_core::LOG_DOMAIN,
                        "A picture's decoder ended without an answer: the picture is not shown"
                    );
                }
                drop(child);
                return (Err(refusal), None);
            }
        };
        let result = parse_answer(&answer);
        // A decoder that lowered its limits for this picture ends after it.
        if answer.first() == Some(&ENDING_MARK) {
            drop(child);
            return (result, None);
        }
        // The decoder serves the next picture once it has taken the rest of
        // this one, which the feeding thread ends and hands back.
        // A decoder that did not take the rest in time is dropped, which
        // stops it.
        let kept = returned
            .recv_timeout(RETURN_WAIT)
            .ok()
            .flatten()
            .map(|stdin| Self {
                child,
                stdin,
                answers: self.answers,
                limit: self.limit,
                served: self.served.saturating_add(1),
            });
        (result, kept)
    }
}

/// A decoder's process, stopped with everything it started when dropped.
#[derive(Debug)]
struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        stop(&mut self.0);
    }
}

/// What starts every request: then the width, height, pixel limits and
/// time limit, little-endian, then the picture in chunks, each its
/// length (`u32`) and its bytes, ended by a chunk of no bytes.
const REQUEST: &[u8; 8] = b"\0OXREQ1\0";

/// What starts every answer on the wire, before its length (`u64`).
const FRAME: &[u8; 8] = b"\0OXFRM1\0";

/// The next answer `stdout` gives, of at most `limit` bytes, the limit of
/// the request it answers, read once its length has come (the request is
/// made after this starts waiting): `None` when the decoder ended, or
/// wrote one larger than that or nonsense. Bytes before the frame (a test
/// harness's own lines) are skipped.
fn read_frame(stdout: &mut impl Read, limit: &std::sync::atomic::AtomicU64) -> Option<Vec<u8>> {
    let mut window = [0_u8; 8];
    let mut seen = 0_usize;
    let mut byte = [0_u8; 1];
    // Only the first answer may follow other output; a few kilobytes of
    // it at most.
    for _ in 0..64 * 1024 {
        stdout.read_exact(&mut byte).ok()?;
        window.rotate_left(1);
        window[7] = byte[0];
        seen += 1;
        if seen >= 8 && &window == FRAME {
            let mut length = [0_u8; 8];
            stdout.read_exact(&mut length).ok()?;
            let length = u64::from_le_bytes(length);
            if length > limit.load(Ordering::Relaxed) {
                return None;
            }
            let mut answer = vec![0; usize::try_from(length).ok()?];
            stdout.read_exact(&mut answer).ok()?;
            return Some(answer);
        }
    }
    None
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

/// Sends `request`, then `source` in chunks, then the end, to a decoder's
/// `stdin`, on a thread of its own, so a decoder that stops reading (it
/// decoded a GIF's first frame, refused the header, or hangs) never holds
/// the caller. The picture's bytes stop once `answered` is set or
/// `cancellable` is cancelled, and the end is sent then; `stdin` is
/// handed back through `give_back` once the request is complete, or
/// `None` when it could not be. A read that blocks (a share that stopped
/// answering) leaves only this thread waiting, and the decoder is stopped
/// instead of handed back.
fn feed(
    source: Source,
    mut stdin: std::process::ChildStdin,
    request: Vec<u8>,
    answered: Arc<AtomicBool>,
    cancellable: Option<gio::Cancellable>,
    give_back: mpsc::Sender<Option<std::process::ChildStdin>>,
) {
    let spawned = thread::Builder::new()
        .name("ox-picture-feed".to_owned())
        .spawn(move || {
            let sent = (|| -> Option<()> {
                stdin.write_all(&request).ok()?;
                let mut buffer = vec![0; READ_CHUNK];
                let mut next: Option<Box<ReadChunk>> = match source {
                    Source::File(file) => file.read(cancellable.as_ref()).ok().map(|stream| {
                        let cancellable = cancellable.clone();
                        Box::new(move |buffer: &mut [u8]| stream.read(buffer, cancellable.as_ref()).ok())
                            as Box<ReadChunk>
                    }),
                    Source::Reader(mut reader) => {
                        Some(Box::new(move |buffer: &mut [u8]| reader.read(buffer).ok()) as Box<ReadChunk>)
                    }
                };
                while let Some(read_chunk) = next.as_mut() {
                    let stopped = answered.load(Ordering::Relaxed)
                        || cancellable.as_ref().is_some_and(CancellableExt::is_cancelled);
                    if stopped {
                        break;
                    }
                    let Some(read) = read_chunk(&mut buffer).filter(|read| *read > 0) else {
                        break;
                    };
                    let bytes = buffer.get(..read)?;
                    stdin.write_all(&u32::try_from(read).ok()?.to_le_bytes()).ok()?;
                    stdin.write_all(bytes).ok()?;
                }
                next.take();
                stdin.write_all(&0_u32.to_le_bytes()).ok()?;
                stdin.flush().ok()
            })();
            let _ = give_back.send(sent.map(|()| stdin));
        });
    // Without a thread the request is never sent, and the caller's wait
    // ends at the time limit.
    let _ = spawned;
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

/// The picture's chunks as the decoder reads them: each its length and
/// its bytes, until a chunk of no bytes ends it.
struct Chunks<'a, R: Read> {
    input: &'a mut R,
    /// What is left of the chunk being read.
    left: usize,
    /// The chunk of no bytes was read.
    ended: bool,
}

impl<R: Read> Read for Chunks<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        while self.left == 0 {
            if self.ended {
                return Ok(0);
            }
            let mut length = [0_u8; 4];
            self.input.read_exact(&mut length)?;
            self.left = usize::try_from(u32::from_le_bytes(length)).unwrap_or(0);
            self.ended = self.left == 0;
        }
        let wanted = buffer.len().min(self.left);
        let read = self.input.read(&mut buffer[..wanted])?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= read;
        Ok(read)
    }
}

impl<R: Read> Chunks<'_, R> {
    /// Reads what is left of the picture, to the chunk that ends it.
    fn drain(&mut self) -> bool {
        let mut buffer = [0_u8; 8192];
        loop {
            match self.read(&mut buffer) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    }
}

/// The parameters after a request's [`REQUEST`]: `None` at the end of the
/// input or for anything else.
fn read_request(input: &mut impl Read) -> Option<(i32, i32, Limits)> {
    let mut start = [0_u8; 8];
    input.read_exact(&mut start).ok()?;
    if &start != REQUEST {
        return None;
    }
    let mut fields = [0_u8; 32];
    input.read_exact(&mut fields).ok()?;
    let int = |at: usize| Some(i32::from_le_bytes(fields.get(at..at + 4)?.try_into().ok()?));
    let long = |at: usize| Some(u64::from_le_bytes(fields.get(at..at + 8)?.try_into().ok()?));
    let limits = Limits {
        max_pixels: long(8)?,
        max_jpeg_pixels: long(16)?,
        time: Duration::from_millis(long(24)?),
    };
    Some((int(0)?, int(4)?, limits))
}

/// Set in a decoder process, which then draws a vector picture itself.
static IN_CHILD: AtomicBool = AtomicBool::new(false);

/// Set once a picture lowered this decoder's limits: it ends after
/// answering for that picture, and the app does not keep it.
static ENDING: AtomicBool = AtomicBool::new(false);

/// What comes before the [`MAGIC`] of the answer of a decoder that ends
/// after it, so the app drops that decoder rather than handing it another
/// picture. [`parse_answer`] reads the answer from its magic on.
const ENDING_MARK: u8 = b'E';

/// True in a picture decoder process.
pub(super) fn in_child() -> bool {
    IN_CHILD.load(Ordering::Relaxed)
}

/// Makes this decoder end after it answers for the picture it decodes.
pub(super) fn end_after_this_picture() {
    ENDING.store(true, Ordering::Relaxed);
}

/// Serves requests from `input` until it ends: decodes each picture and
/// writes its answer to `output` as soon as it is known, then takes the
/// rest of the picture. What a decoder process runs.
fn serve(input: &mut impl Read, output: &mut impl Write) -> bool {
    while let Some((width, height, limits)) = read_request(input) {
        let mut chunks = Chunks {
            input: &mut *input,
            left: 0,
            ended: false,
        };
        #[cfg(test)]
        if width == tests::CRASH_WIDTH {
            std::process::abort();
        }
        let result = super::decode_reader_here_to_fit(&mut chunks, width, height, limits, None);
        let ending = ENDING.load(Ordering::Relaxed);
        let mut answer = answer(&result);
        if ending {
            answer.insert(0, ENDING_MARK);
        }
        let written = output
            .write_all(FRAME)
            .and_then(|()| output.write_all(&u64::try_from(answer.len()).unwrap_or(u64::MAX).to_le_bytes()))
            .and_then(|()| output.write_all(&answer))
            .and_then(|()| output.flush());
        if ending {
            return written.is_ok();
        }
        if written.is_err() || !chunks.drain() {
            return false;
        }
    }
    true
}

/// Limits the decoder process's own resources: a picture that keeps it
/// busy, or makes it allocate without end, is stopped by the kernel too;
/// it writes no files and leaves no core.
fn limit_resources() {
    for (resource, limit) in [
        (rustix::process::Resource::Cpu, DECODER_CPU_SECONDS),
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
}

/// Serves the requests on standard input: what the decoder process does.
fn serve_standard_input() -> bool {
    IN_CHILD.store(true, Ordering::Relaxed);
    limit_resources();
    let mut input = std::io::BufReader::new(std::io::stdin().lock());
    let mut output = std::io::stdout().lock();
    serve(&mut input, &mut output)
}

/// Runs the app as a picture decoder when `arguments` ask for it. Returns
/// the exit status then, `None` for every other command line.
pub(crate) fn run_as_decoder(arguments: &[String]) -> Option<glib::ExitCode> {
    if arguments.get(1).map(String::as_str) != Some(DECODER_FLAG) {
        return None;
    }
    Some(if serve_standard_input() {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    })
}

/// The environment variable that makes a test program a decoder.
const TEST_DECODER: &str = "OX_TEST_PICTURE_DECODER";

#[cfg(test)]
pub(crate) mod tests;
