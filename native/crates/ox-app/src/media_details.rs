// SPDX-License-Identifier: AGPL-3.0-only
//! What the Details tab of Properties reads from a picture, a video or an
//! audio file (PROP-034).
//!
//! A file is untrusted input, and reading it may be slow (a share) or
//! large, so it is read in a child process: the app itself, started again
//! with [`READER_FLAG`]. The child limits its own processor time, reads at
//! most the start of a picture ([`picture`]) or lets `GStreamer` read a
//! video or audio file ([`streams`]), and writes what it
//! found as JSON. The app waits at most [`READ_TIMEOUT`] for it and stops
//! it then, or when the Properties window closes; a child that crashed,
//! hung or wrote nonsense only leaves the tab without its media rows.
//! The values are read only: nothing here writes to the file.

mod picture;
mod streams;

use std::ffi::OsStr;
use std::time::Duration;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use serde::{Deserialize, Serialize};

/// The first argument that starts the app as the Details reader.
pub(crate) const READER_FLAG: &str = "--ox-read-media-details";
/// How long the app waits for the reader.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// The processor time the reader may use, in seconds.
const READER_CPU_SECONDS: u64 = 20;
/// The most the app reads from the reader.
const OUTPUT_LIMIT: usize = 256 * 1024;

/// Which of Windows' layouts a file's Details take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MediaKind {
    /// A picture: Description, Origin, Image, Camera, Advanced photo.
    Picture,
    /// A video: Description, Video, Audio, Media, Origin, Content.
    Video,
    /// An audio file: Description, Media, Audio, Origin, Content.
    Audio,
}

impl MediaKind {
    /// The kind of a file of `content_type`, if it has media details.
    pub(crate) fn for_content_type(content_type: &str) -> Option<Self> {
        let mime = gio::content_type_get_mime_type(content_type)
            .map_or_else(|| content_type.to_owned(), String::from);
        if mime.starts_with("image/") {
            Some(Self::Picture)
        } else if mime.starts_with("video/") {
            Some(Self::Video)
        } else if mime.starts_with("audio/") {
            Some(Self::Audio)
        } else {
            None
        }
    }

    const fn argument(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    fn from_argument(argument: &str) -> Option<Self> {
        [Self::Picture, Self::Video, Self::Audio]
            .into_iter()
            .find(|kind| kind.argument() == argument)
    }
}

/// A date and time without a time zone, as EXIF writes it: the camera's
/// local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NaiveDateTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

impl NaiveDateTime {
    /// `2026:05:01 10:30:00`, as EXIF writes a date.
    pub(crate) fn parse_exif(text: &str) -> Option<Self> {
        // Read as bytes: a broken byte the reader turned into a
        // replacement character must never be sliced through.
        let text = text.trim().as_bytes();
        let number = |range: std::ops::Range<usize>| {
            let digits = text.get(range)?;
            digits
                .iter()
                .all(u8::is_ascii_digit)
                .then(|| std::str::from_utf8(digits).ok()?.parse::<i32>().ok())
                .flatten()
        };
        let separators = [(4, b':'), (7, b':'), (10, b' '), (13, b':'), (16, b':')];
        let well_formed = text.len() == 19
            && separators
                .iter()
                .all(|(at, separator)| text.get(*at) == Some(separator));
        if !well_formed {
            return None;
        }
        let date = Self {
            year: number(0..4)?,
            month: number(5..7)?,
            day: number(8..10)?,
            hour: number(11..13)?,
            minute: number(14..16)?,
            second: number(17..19)?,
        };
        let in_range = (1..=12).contains(&date.month)
            && (1..=31).contains(&date.day)
            && (0..24).contains(&date.hour)
            && (0..60).contains(&date.minute)
            && (0..61).contains(&date.second)
            && date.year > 0;
        in_range.then_some(date)
    }

    /// The moment in this computer's time zone, in seconds since the Unix
    /// epoch.
    pub(crate) fn local_unix_seconds(self) -> Option<u64> {
        let time = glib::DateTime::from_local(
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            f64::from(self.second),
        )
        .ok()?;
        u64::try_from(time.to_unix()).ok()
    }
}

/// What the reader found in one file. Every field is optional: Windows'
/// rows without a value here are left out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct MediaDetails {
    // Description
    pub title: Option<String>,
    pub subject: Option<String>,
    /// Stars, 0 to 5.
    pub rating: Option<u8>,
    /// The format keeps a rating, so an unrated file shows empty stars.
    pub rating_readable: bool,
    pub tags: Option<String>,
    pub comments: Option<String>,
    // Origin
    pub authors: Option<String>,
    pub date_taken: Option<NaiveDateTime>,
    pub program_name: Option<String>,
    pub copyright: Option<String>,
    pub publisher: Option<String>,
    pub encoded_by: Option<String>,
    // Image
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Horizontal and vertical dots per inch.
    pub resolution: Option<(f64, f64)>,
    pub bit_depth: Option<u32>,
    /// EXIF's `ColorSpace`: 1 is sRGB, 65535 uncalibrated.
    pub color_space: Option<u16>,
    // Camera
    pub camera_maker: Option<String>,
    pub camera_model: Option<String>,
    pub f_number: Option<f64>,
    /// Seconds, as a fraction.
    pub exposure_time: Option<(u32, u32)>,
    pub iso: Option<u32>,
    pub exposure_bias: Option<f64>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    pub metering_mode: Option<u16>,
    pub flash: Option<u16>,
    pub focal_length_35mm: Option<u32>,
    // Advanced photo
    pub lens_maker: Option<String>,
    pub lens_model: Option<String>,
    pub exposure_program: Option<u16>,
    pub white_balance: Option<u16>,
    pub exif_version: Option<String>,
    // Video
    /// Nanoseconds.
    pub length: Option<u64>,
    pub frame_width: Option<u32>,
    pub frame_height: Option<u32>,
    /// Bits per second.
    pub data_rate: Option<u32>,
    pub total_bitrate: Option<u32>,
    /// Frames per second, as a fraction.
    pub frame_rate: Option<(u32, u32)>,
    // Audio
    pub audio_bitrate: Option<u32>,
    pub channels: Option<u32>,
    /// Hertz.
    pub sample_rate: Option<u32>,
    // Media
    pub artists: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub genre: Option<String>,
    // Content
    pub composers: Option<String>,
    pub conductors: Option<String>,
}

/// Reads the details of the file at `uri`, of `size` bytes, on the calling
/// thread. This is what the reader process runs.
pub(crate) fn read_details(uri: &str, kind: MediaKind, size: Option<u64>) -> MediaDetails {
    let mut details = MediaDetails::default();
    match kind {
        MediaKind::Picture => {
            if let Some(head) = read_head(uri, picture::HEAD_LIMIT) {
                picture::read_picture(&head, &mut details);
            }
        }
        MediaKind::Video | MediaKind::Audio => streams::read_streams(uri, size, &mut details),
    }
    details
}

/// At most `limit` bytes from the start of the file at `uri`.
fn read_head(uri: &str, limit: usize) -> Option<Vec<u8>> {
    let stream = gio::File::for_uri(uri).read(gio::Cancellable::NONE).ok()?;
    let mut head = vec![0; limit];
    let (read, _) = stream.read_all(&mut head, gio::Cancellable::NONE).ok()?;
    head.truncate(read);
    Some(head)
}

/// Runs the app as the Details reader when `arguments` ask for it:
/// `<app> --ox-read-media-details <kind> <uri> [size]`. Returns the exit
/// status then, `None` for every other command line.
pub(crate) fn run_as_reader(arguments: &[String]) -> Option<glib::ExitCode> {
    if arguments.get(1).map(String::as_str) != Some(READER_FLAG) {
        return None;
    }
    let (Some(kind), Some(uri)) = (
        arguments.get(2).and_then(|kind| MediaKind::from_argument(kind)),
        arguments.get(3),
    ) else {
        return Some(glib::ExitCode::FAILURE);
    };
    let size = arguments.get(4).and_then(|size| size.parse().ok());
    // A file that keeps the reader busy is stopped by the kernel too.
    let limit = rustix::process::Rlimit {
        current: Some(READER_CPU_SECONDS),
        maximum: Some(READER_CPU_SECONDS),
    };
    let _ = rustix::process::setrlimit(rustix::process::Resource::Cpu, limit);
    let details = read_details(uri, kind, size);
    match serde_json::to_string(&details) {
        Ok(json) => {
            println!("{json}");
            Some(glib::ExitCode::SUCCESS)
        }
        Err(_) => Some(glib::ExitCode::FAILURE),
    }
}

/// The reader process of one Properties window, which stops it when the
/// window closes.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReaderProcess(std::rc::Rc<std::cell::RefCell<Option<gio::Subprocess>>>);

impl ReaderProcess {
    /// Stops the reader, if it still runs.
    pub(crate) fn stop(&self) {
        if let Some(process) = self.0.take() {
            process.force_exit();
        }
    }
}

/// Reads the details of the file at `uri` in a reader process; `None` when
/// it could not start, failed, or did not finish within [`READ_TIMEOUT`].
pub(crate) async fn read_in_reader(
    uri: &str,
    kind: MediaKind,
    size: Option<u64>,
    process: &ReaderProcess,
) -> Option<MediaDetails> {
    // Tests read in the test process, which is not the app.
    if cfg!(test) {
        let uri = uri.to_owned();
        return gio::spawn_blocking(move || read_details(&uri, kind, size))
            .await
            .ok();
    }
    let program = std::env::current_exe().ok()?;
    let size = size.map(|size| size.to_string()).unwrap_or_default();
    let arguments = [
        program.as_os_str(),
        OsStr::new(READER_FLAG),
        OsStr::new(kind.argument()),
        OsStr::new(uri),
        OsStr::new(&size),
    ];
    let output = run_with_timeout(&arguments, READ_TIMEOUT, process).await?;
    serde_json::from_slice(&output).ok()
}

/// Runs `arguments` without the display, and returns what it wrote if it
/// finished well within `timeout`; it is stopped otherwise.
pub(crate) async fn run_with_timeout(
    arguments: &[&OsStr],
    timeout: Duration,
    process: &ReaderProcess,
) -> Option<Vec<u8>> {
    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    );
    // The reader shows nothing, so it never connects to the display.
    launcher.unsetenv("DISPLAY");
    launcher.unsetenv("WAYLAND_DISPLAY");
    let child = launcher.spawn(arguments).ok()?;
    process.stop();
    process.0.replace(Some(child.clone()));
    let finished = glib::future_with_timeout(timeout, child.communicate_future(None)).await;
    let output = match finished {
        Ok(Ok((Some(output), _))) if child.is_successful() => Some(output.to_vec()),
        _ => None,
    };
    if process.0.borrow().as_ref() == Some(&child) {
        process.stop();
    }
    output.filter(|output| output.len() <= OUTPUT_LIMIT)
}

#[cfg(test)]
pub(crate) mod tests;
