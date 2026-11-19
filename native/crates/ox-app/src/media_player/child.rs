// SPDX-License-Identifier: AGPL-3.0-only
//! The player process: plays one recording with `GStreamer`'s classic
//! `playbin` (`decodebin`, never `decodebin3`), sends the app scaled RGBA
//! frames and where it is, and plays the sound itself. It reads the app's
//! commands from its standard input, writes its events to its standard
//! output, kept for them alone, and exits when told to, when its input closes,
//! or when the app dies. Whatever a broken file makes `GStreamer` do, an
//! assertion's abort included, ends only this process.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;

use super::protocol::{Command, Event, HELLO, MAX_LINE};
use super::Kind;

/// The memory the player may allocate, in bytes (`RLIMIT_DATA`): decoders'
/// frame pools and `GStreamer`'s threads with room to spare, and a stop for
/// a file that asks for more.
const MEMORY_BYTES: u64 = 2 << 30;
/// How often the player looks at its commands, its bus and its position.
const TICK: Duration = Duration::from_millis(40);
/// How many ticks after its preroll a recording whose length is unknown
/// waits before it is ready all the same.
const READY_TICKS: u32 = 25;

/// The events the player writes, shared by its threads.
type Events = Arc<Mutex<File>>;

/// Plays the recording at `uri` until told to quit; the exit status.
pub(super) fn run(kind: Kind, max_width: u32, max_height: u32, uri: &str) -> glib::ExitCode {
    // The app's death ends the player too, as does its input closing.
    let _ = rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL));
    let limit = rustix::process::Rlimit {
        current: Some(MEMORY_BYTES),
        maximum: Some(MEMORY_BYTES),
    };
    let _ = rustix::process::setrlimit(rustix::process::Resource::Data, limit);
    // The events keep the output the app reads; whatever else prints to
    // standard output, a plugin's message, goes to standard error.
    let Ok(output) = rustix::io::dup(std::io::stdout()) else {
        return glib::ExitCode::FAILURE;
    };
    if rustix::stdio::dup2_stdout(std::io::stderr()).is_err() {
        return glib::ExitCode::FAILURE;
    }
    let events: Events = Arc::new(Mutex::new(File::from(output)));
    if !write_line(&events, HELLO) {
        return glib::ExitCode::FAILURE;
    }
    if let Err(error) = gst::init() {
        send(&events, &Event::Failed(error.to_string()));
        return glib::ExitCode::FAILURE;
    }
    let mailbox = Arc::new(Mailbox::default());
    let Some(playbin) = build(kind, max_width, max_height, uri, &mailbox) else {
        send(&events, &Event::Failed("GStreamer has no playbin".to_owned()));
        return glib::ExitCode::FAILURE;
    };
    let frames = {
        let events = Arc::clone(&events);
        let mailbox = Arc::clone(&mailbox);
        thread::spawn(move || write_frames(&events, &mailbox))
    };
    let commands = read_commands();
    let status = play(&playbin, kind, &commands, &events);
    // Stopped as the pane stops a stream: paused, then to NULL, then let go.
    let _ = playbin.set_state(gst::State::Paused);
    let _ = playbin.set_state(gst::State::Null);
    drop(playbin);
    mailbox.close();
    let _ = frames.join();
    status
}

/// The playbin for `uri`: for a video, with an `appsink` that takes RGBA
/// frames scaled to fit `max_width` by `max_height`; for audio, with no
/// video at all, so a cover picture in the file is not decoded.
fn build(
    kind: Kind,
    max_width: u32,
    max_height: u32,
    uri: &str,
    mailbox: &Arc<Mailbox>,
) -> Option<gst::Element> {
    // The classic playbin, whatever the GStreamer default: its decodebin
    // has none of decodebin3's stream-collection assertions.
    let playbin = gst::ElementFactory::make("playbin")
        .property("uri", uri)
        .build()
        .ok()?;
    match kind {
        Kind::Audio => playbin.set_property_from_str("flags", "audio+soft-volume"),
        Kind::Video => {
            playbin.set_property_from_str("flags", "video+audio+soft-volume");
            let description = format!(
                "videoconvert ! videoscale ! \
                 video/x-raw,format=RGBA,pixel-aspect-ratio=1/1,width=[1,{max_width}],height=[1,{max_height}] ! \
                 appsink name=frames max-buffers=1 drop=true emit-signals=true"
            );
            let sink = gst::parse::bin_from_description(&description, true).ok()?;
            let appsink = sink.by_name("frames")?;
            for (signal, pull) in [("new-sample", "pull-sample"), ("new-preroll", "pull-preroll")] {
                let mailbox = Arc::clone(mailbox);
                appsink.connect(signal, false, move |values| {
                    if let Some(sink) = values.first().and_then(|value| value.get::<gst::Element>().ok()) {
                        if let Some(frame) = sink
                            .emit_by_name::<Option<gst::Sample>>(pull, &[])
                            .as_ref()
                            .and_then(frame_of)
                        {
                            mailbox.put(frame);
                        }
                    }
                    Some(gst::FlowReturn::Ok.to_value())
                });
            }
            playbin.set_property("video-sink", &sink);
        }
    }
    Some(playbin)
}

/// Runs the player until the app says quit or goes away; the exit status.
fn play(
    playbin: &gst::Element,
    kind: Kind,
    commands: &mpsc::Receiver<Command>,
    events: &Events,
) -> glib::ExitCode {
    let Some(bus) = playbin.bus() else {
        return glib::ExitCode::FAILURE;
    };
    if playbin.set_state(gst::State::Paused).is_err() {
        send(
            events,
            &Event::Failed("The recording cannot be opened".to_owned()),
        );
    }
    let mut prepared = false;
    // Ticks since the first preroll, while the length is not yet known.
    let mut prerolled: Option<u32> = None;
    let mut playing = false;
    let mut seeking = false;
    let mut position = -1;
    loop {
        match commands.recv_timeout(TICK) {
            Ok(Command::Play) => {
                #[cfg(test)]
                if std::env::var_os(super::TEST_ABORT_VARIABLE).is_some() {
                    // As GStreamer's g_assert does.
                    std::process::abort();
                }
                playing = playbin.set_state(gst::State::Playing).is_ok();
            }
            Ok(Command::Pause) => {
                playing = false;
                let _ = playbin.set_state(gst::State::Paused);
            }
            Ok(Command::Seek(micros)) => {
                let target = gst::ClockTime::from_useconds(micros.unsigned_abs());
                seeking = playbin
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, target)
                    .is_ok();
                if !seeking && !send(events, &Event::SeekFailed) {
                    return glib::ExitCode::SUCCESS;
                }
            }
            Ok(Command::Volume { volume, muted }) => {
                playbin.set_property("volume", volume);
                playbin.set_property("mute", muted);
            }
            Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => return glib::ExitCode::SUCCESS,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while let Some(message) = bus.pop() {
            let event = match message.view() {
                gst::MessageView::Eos(_) => {
                    playing = false;
                    Some(Event::Ended)
                }
                gst::MessageView::Error(error) => Some(Event::Failed(error.error().to_string())),
                gst::MessageView::AsyncDone(_) if message.src() == Some(playbin.upcast_ref()) => {
                    prerolled.get_or_insert(0);
                    std::mem::take(&mut seeking).then_some(Event::Seeked)
                }
                _ => None,
            };
            let failed = matches!(event, Some(Event::Failed(_)));
            if event.is_some_and(|event| !send(events, &event)) || failed {
                return glib::ExitCode::SUCCESS;
            }
        }
        // Ready once the length is known, or a second after the preroll
        // without one: a stream, or a file whose end is never found.
        if let Some(ticks) = prerolled.as_mut().filter(|_| !prepared) {
            *ticks += 1;
            let duration = playbin
                .query_duration::<gst::ClockTime>()
                .and_then(|time| i64::try_from(time.useconds()).ok())
                .filter(|micros| *micros > 0);
            if duration.is_some() || *ticks >= READY_TICKS {
                prepared = true;
                if !send(events, &ready(playbin, kind, duration.unwrap_or(0))) {
                    return glib::ExitCode::SUCCESS;
                }
            }
        }
        if playing && prepared {
            let now = playbin
                .query_position::<gst::ClockTime>()
                .map_or(position, |time| {
                    i64::try_from(time.useconds()).unwrap_or(i64::MAX)
                });
            if now != position {
                position = now;
                if !send(events, &Event::Position(now)) {
                    return glib::ExitCode::SUCCESS;
                }
            }
        }
    }
}

/// What the app needs to know once the recording is ready.
fn ready(playbin: &gst::Element, kind: Kind, duration: i64) -> Event {
    let mut seeking = gst::query::Seeking::new(gst::Format::Time);
    let seekable = playbin.query(&mut seeking) && seeking.result().0;
    let has = |property| playbin.property::<i32>(property) > 0;
    Event::Ready {
        duration,
        seekable,
        has_audio: has("n-audio"),
        has_video: kind == Kind::Video && has("n-video"),
    }
}

/// Writes `event` for the app; false once it is gone.
fn send(events: &Events, event: &Event) -> bool {
    write_line(events, &event.line())
}

fn write_line(events: &Events, line: &str) -> bool {
    let mut events = events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    events.write_all(line.as_bytes()).is_ok()
}

/// The app's commands, read on a thread of their own; the channel closes
/// with the app's end of the input.
fn read_commands() -> mpsc::Receiver<Command> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut input = BufReader::new(std::io::stdin().lock());
        loop {
            let mut line = String::new();
            match (&mut input).take(MAX_LINE).read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let Some(command) = Command::parse(&line) else {
                        continue;
                    };
                    if sender.send(command).is_err() || command == Command::Quit {
                        return;
                    }
                }
            }
        }
    });
    receiver
}

/// A decoded frame: RGBA rows of `stride` bytes.
struct Frame {
    width: u32,
    height: u32,
    stride: u32,
    pixels: Vec<u8>,
}

fn frame_of(sample: &gst::Sample) -> Option<Frame> {
    let structure = sample.caps()?.structure(0)?.to_owned();
    let width = u32::try_from(structure.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(structure.get::<i32>("height").ok()?).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let rows = usize::try_from(height).ok()?;
    let stride = map.size().checked_div(rows)?;
    let length = super::protocol::frame_length(width, height, u32::try_from(stride).ok()?)?;
    Some(Frame {
        width,
        height,
        stride: u32::try_from(stride).ok()?,
        pixels: map.as_slice().get(..length)?.to_vec(),
    })
}

/// The newest frame not yet written: a slow app sees fewer frames, and
/// `GStreamer`'s streaming threads never wait for it.
#[derive(Default)]
struct Mailbox {
    slot: Mutex<(Option<Frame>, bool)>,
    filled: Condvar,
}

impl Mailbox {
    fn put(&self, frame: Frame) {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.0 = Some(frame);
        self.filled.notify_one();
    }

    fn close(&self) {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.1 = true;
        self.filled.notify_one();
    }

    /// The next frame, or `None` once closed.
    fn take(&self) -> Option<Frame> {
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if slot.1 {
                return None;
            }
            if let Some(frame) = slot.0.take() {
                return Some(frame);
            }
            slot = self
                .filled
                .wait(slot)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Writes each frame the mailbox gets, with its line, until it closes or
/// the app is gone.
fn write_frames(events: &Events, mailbox: &Mailbox) {
    while let Some(frame) = mailbox.take() {
        let line = Event::Frame {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
        }
        .line();
        let mut events = events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if events.write_all(line.as_bytes()).is_err() || events.write_all(&frame.pixels).is_err() {
            return;
        }
    }
}
