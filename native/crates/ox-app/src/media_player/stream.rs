// SPDX-License-Identifier: AGPL-3.0-only
//! The app's side of a player process: a [`gtk::MediaStream`] that
//! `gtk::Video` and `gtk::MediaControls` show and control as they would
//! a `gtk::MediaFile`, but whose decoding happens in the player process.
//! A player that dies, hangs up or says something it should not puts the
//! stream in error; the app stays as it was.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use super::protocol::{frame_length, Command, Event, HELLO, MAX_LINE};
use super::Kind;

/// How long a player told to quit may take before it is killed.
const QUIT_GRACE: Duration = Duration::from_secs(1);

/// What the reading thread hands the main loop.
enum Message {
    Event(Event),
    Frame {
        width: u32,
        height: u32,
        stride: u32,
        pixels: Vec<u8>,
    },
    /// The player closed its end, or broke the protocol.
    Closed,
}

mod imp {
    use std::cell::{Cell, RefCell};
    use std::io::Write;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, gio, glib};

    use super::Command;

    #[derive(Default)]
    pub(crate) struct PlayerStream {
        /// The file played, as a local `file:` URI.
        pub(super) uri: RefCell<String>,
        /// The file as the app knows it and changes it: a share's own
        /// URI (`smb:`) when it plays through a local path.
        pub(super) source: RefCell<String>,
        pub(super) process: RefCell<Option<gio::Subprocess>>,
        /// The player's standard input, which never blocks the app.
        pub(super) commands: RefCell<Option<std::io::PipeWriter>>,
        pub(super) frame: RefCell<Option<gdk::Texture>>,
        /// Stopped on purpose: whatever the player does next is no error.
        pub(super) stopped: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlayerStream {
        const NAME: &'static str = "OxPlayerStream";
        type Type = super::PlayerStream;
        type ParentType = gtk::MediaStream;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for PlayerStream {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> = std::sync::OnceLock::new();
            // Stopped because the app is about to change its file.
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("released").build()])
        }

        fn dispose(&self) {
            self.obj().end();
        }
    }

    impl MediaStreamImpl for PlayerStream {
        fn play(&self) -> bool {
            self.send(Command::Play)
        }

        fn pause(&self) {
            self.send(Command::Pause);
        }

        fn seek(&self, timestamp: i64) {
            if !self.send(Command::Seek(timestamp.max(0))) {
                self.obj().seek_failed();
            }
        }

        fn update_audio(&self, muted: bool, volume: f64) {
            self.send(Command::Volume {
                volume: volume.clamp(0.0, 1.0),
                muted,
            });
        }
    }

    impl PaintableImpl for PlayerStream {
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(frame) = self.frame.borrow().as_ref() {
                frame.snapshot(snapshot, width, height);
            }
        }

        fn current_image(&self) -> gdk::Paintable {
            match self.frame.borrow().as_ref() {
                Some(frame) => frame.clone().upcast(),
                None => gdk::Paintable::new_empty(0, 0),
            }
        }

        fn intrinsic_width(&self) -> i32 {
            self.frame.borrow().as_ref().map_or(0, gdk::Texture::width)
        }

        fn intrinsic_height(&self) -> i32 {
            self.frame.borrow().as_ref().map_or(0, gdk::Texture::height)
        }
    }

    impl PlayerStream {
        /// Writes `command` for the player; false when it is stopped or
        /// gone. A player that does not read is never waited for.
        pub(super) fn send(&self, command: Command) -> bool {
            if self.stopped.get() && command != Command::Quit {
                return false;
            }
            let mut commands = self.commands.borrow_mut();
            let Some(input) = commands.as_mut() else {
                return false;
            };
            input.write_all(command.line().as_bytes()).is_ok()
        }
    }
}

glib::wrapper! {
    /// A recording played by a player process of its own.
    pub(crate) struct PlayerStream(ObjectSubclass<imp::PlayerStream>)
        @extends gtk::MediaStream,
        @implements gdk::Paintable;
}

thread_local! {
    /// Every player stream of the app, to stop before a write.
    static PLAYERS: RefCell<Vec<glib::WeakRef<PlayerStream>>> = const { RefCell::new(Vec::new()) };
    /// The locations the app is changing, each with its hold's number: no
    /// player of a file at or inside one starts.
    static HOLDS: RefCell<Vec<(u64, Vec<gio::File>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_HOLD: Cell<u64> = const { Cell::new(0) };
}

/// Keeps players from starting for the files at or inside some locations
/// while the app changes them; dropped once the change is done.
#[derive(Debug)]
#[must_use = "players may start again as soon as the hold is dropped"]
pub(crate) struct PlayerHold(u64);

impl Drop for PlayerHold {
    fn drop(&mut self) {
        HOLDS.with(|holds| holds.borrow_mut().retain(|(number, _)| *number != self.0));
    }
}

/// Whether `file` is at or inside `location`.
fn is_at_or_inside(file: &gio::File, location: &gio::File) -> bool {
    file.equal(location) || file.has_prefix(location)
}

/// Whether the app holds the file at `uri` or `source`.
fn is_held(uri: &str, source: &str) -> bool {
    let files = [gio::File::for_uri(uri), gio::File::for_uri(source)];
    HOLDS.with(|holds| {
        holds.borrow().iter().any(|(_, locations)| {
            locations
                .iter()
                .any(|location| files.iter().any(|file| is_at_or_inside(file, location)))
        })
    })
}

impl PlayerStream {
    /// Starts a player process for the recording at `path`, which the
    /// app knows as `source` (the URI its file operations use: a share's
    /// own URI when `path` is the share's local path). Its video, if
    /// `kind` has one, is scaled to fit `max_width` by `max_height`
    /// pixels. It prepares at once and plays when asked. A player that
    /// could not start, or whose file the app holds while it changes it
    /// ([`hold_players_within`]), leaves the stream in error.
    pub(crate) fn new(path: &Path, source: &str, kind: Kind, max_width: u32, max_height: u32) -> Self {
        let stream: Self = glib::Object::new();
        let uri = gio::File::for_path(path).uri().to_string();
        stream.imp().uri.replace(uri.clone());
        stream.imp().source.replace(source.to_owned());
        if is_held(&uri, source) {
            stream.imp().stopped.set(true);
            stream.set_error(glib::Error::new(
                gio::IOErrorEnum::Busy,
                "The file is being changed",
            ));
            return stream;
        }
        PLAYERS.with(|players| {
            let mut players = players.borrow_mut();
            players.retain(|player| player.upgrade().is_some());
            players.push(stream.downgrade());
        });
        if let Err(error) = stream.start(kind, max_width, max_height, &uri) {
            stream.set_error(error);
        }
        stream
    }

    fn start(&self, kind: Kind, max_width: u32, max_height: u32, uri: &str) -> Result<(), glib::Error> {
        let failed = |error: std::io::Error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string());
        let (command_reader, command_writer) = std::io::pipe().map_err(failed)?;
        let (event_reader, event_writer) = std::io::pipe().map_err(failed)?;
        let flags = rustix::fs::fcntl_getfl(&command_writer).map_err(|error| failed(error.into()))?;
        rustix::fs::fcntl_setfl(&command_writer, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|error| failed(error.into()))?;
        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
        // The player shows nothing itself, so it never connects to the display.
        launcher.unsetenv("DISPLAY");
        launcher.unsetenv("WAYLAND_DISPLAY");
        launcher.take_stdin_fd(Some(command_reader.into()));
        launcher.take_stdout_fd(Some(event_writer.into()));
        let (arguments, environment) = super::player_command(kind, max_width, max_height, uri)
            .ok_or_else(|| glib::Error::new(gio::IOErrorEnum::NotFound, "no player program"))?;
        for (name, value) in environment {
            launcher.setenv(name, value, true);
        }
        let arguments: Vec<&std::ffi::OsStr> = arguments.iter().map(std::ffi::OsString::as_os_str).collect();
        let process = launcher.spawn(&arguments)?;
        // The launcher holds the child's ends of the pipes until dropped:
        // only then does a player that dies close them for the app.
        drop(launcher);
        self.imp().process.replace(Some(process));
        self.imp().commands.replace(Some(command_writer));
        let (sender, receiver) = async_channel::bounded(2);
        std::thread::Builder::new()
            .name("ox-player-events".into())
            .spawn(move || read_events(event_reader, &sender))
            .map_err(failed)?;
        let stream = self.downgrade();
        glib::spawn_future_local(async move {
            while let Ok(message) = receiver.recv().await {
                let Some(stream) = stream.upgrade() else {
                    return;
                };
                if !stream.receive(message) {
                    return;
                }
            }
        });
        Ok(())
    }

    /// Handles what the player said; false once nothing more is read.
    fn receive(&self, message: Message) -> bool {
        let imp = self.imp();
        if imp.stopped.get() {
            return false;
        }
        match message {
            Message::Event(Event::Ready {
                duration,
                seekable,
                has_audio,
                has_video,
            }) => self.stream_prepared(has_audio, has_video, seekable, duration),
            Message::Event(Event::Position(micros)) => self.update(micros),
            Message::Event(Event::Seeked) => self.seek_success(),
            Message::Event(Event::SeekFailed) => self.seek_failed(),
            Message::Event(Event::Ended) => self.stream_ended(),
            Message::Event(Event::Failed(text)) => {
                self.fail(&text);
                return false;
            }
            Message::Event(Event::Frame { .. }) => {}
            Message::Frame {
                width,
                height,
                stride,
                pixels,
            } => self.show_frame(width, height, stride, pixels),
            Message::Closed => {
                self.fail("The player stopped");
                return false;
            }
        }
        true
    }

    fn show_frame(&self, width: u32, height: u32, stride: u32, pixels: Vec<u8>) {
        let (Ok(width), Ok(height), Ok(stride)) = (
            i32::try_from(width),
            i32::try_from(height),
            usize::try_from(stride),
        ) else {
            return;
        };
        let texture = gdk::MemoryTexture::new(
            width,
            height,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(pixels),
            stride,
        );
        let resized = self
            .imp()
            .frame
            .replace(Some(texture.upcast()))
            .is_none_or(|old| old.width() != width || old.height() != height);
        if resized {
            self.invalidate_size();
        }
        self.invalidate_contents();
    }

    /// Puts the stream in error after its player failed, and ends it.
    fn fail(&self, text: &str) {
        if let Some(process) = self.stop() {
            process.force_exit();
        }
        self.set_error(glib::Error::new(gio::IOErrorEnum::Failed, text));
    }

    /// Stops the stream: paused, then the player told to stop its
    /// pipeline (to NULL) and exit, and its input closed. Returns the
    /// player process, which may still be exiting, the first time.
    pub(crate) fn stop(&self) -> Option<gio::Subprocess> {
        let imp = self.imp();
        if imp.stopped.get() {
            return None;
        }
        if self.is_playing() {
            self.pause();
        }
        imp.stopped.set(true);
        imp.send(Command::Quit);
        imp.commands.take();
        imp.process.take()
    }

    /// Stops the stream as [`Self::stop`] does, and kills its player
    /// unless it exits on its own within [`QUIT_GRACE`].
    pub(crate) fn end(&self) {
        if let Some(process) = self.stop() {
            kill_after_grace(process);
        }
    }

    /// The player process, while it runs, for tests.
    #[cfg(test)]
    pub(crate) fn process(&self) -> Option<gio::Subprocess> {
        self.imp().process.borrow().clone()
    }

    /// Whether the stream plays the file at `uri` or one inside it.
    fn is_within(&self, location: &gio::File) -> bool {
        let imp = self.imp();
        [&imp.uri, &imp.source]
            .into_iter()
            .any(|uri| is_at_or_inside(&gio::File::for_uri(&uri.borrow()), location))
    }
}

/// Kills `process` unless it exits on its own within [`QUIT_GRACE`].
fn kill_after_grace(process: gio::Subprocess) {
    glib::spawn_future_local(async move {
        if glib::future_with_timeout(QUIT_GRACE, process.wait_future())
            .await
            .is_err()
        {
            process.force_exit();
        }
    });
}

/// Holds the files at or inside `locations` (URIs) while the app renames,
/// moves, deletes or overwrites them: stops every player of one (each
/// stream says `released`), waits for each to exit, and keeps any new
/// one from starting until the hold is dropped. A player that does not
/// exit within [`QUIT_GRACE`] is killed, and waited for until it is gone.
pub(crate) async fn hold_players_within(locations: &[String]) -> PlayerHold {
    let locations: Vec<gio::File> = locations.iter().map(|uri| gio::File::for_uri(uri)).collect();
    let number = NEXT_HOLD.with(|next| next.replace(next.get().wrapping_add(1)));
    let players: Vec<PlayerStream> = PLAYERS.with(|players| {
        players
            .borrow()
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .filter(|player| !player.imp().stopped.get())
            .filter(|player| locations.iter().any(|location| player.is_within(location)))
            .collect()
    });
    HOLDS.with(|holds| holds.borrow_mut().push((number, locations)));
    let hold = PlayerHold(number);
    let mut exiting = Vec::new();
    for player in players {
        if let Some(process) = player.stop() {
            exiting.push(process);
        }
        player.emit_by_name::<()>("released", &[]);
    }
    for process in exiting {
        if glib::future_with_timeout(QUIT_GRACE, process.wait_future())
            .await
            .is_err()
        {
            // Killed, it is gone once the kernel has taken it down, which
            // the wait sees; the write starts only then.
            process.force_exit();
            let _ = process.wait_future().await;
        }
    }
    hold
}

/// Reads the player's events until it closes its end or breaks the
/// protocol, and hands them to the main loop, which takes at most two
/// waiting messages: a player that sends faster waits.
fn read_events(events: std::io::PipeReader, sender: &async_channel::Sender<Message>) {
    let mut events = BufReader::new(events);
    // Lines before the player's first are not its own.
    loop {
        let mut line = String::new();
        match (&mut events).take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => {
                let _ = sender.send_blocking(Message::Closed);
                return;
            }
            // A test harness may have begun the line.
            Ok(_) if line.ends_with(HELLO) => break,
            Ok(_) => {}
        }
    }
    loop {
        let mut line = String::new();
        let message = match (&mut events).take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => Message::Closed,
            Ok(_) => match Event::parse(&line) {
                Some(Event::Frame {
                    width,
                    height,
                    stride,
                }) => {
                    match frame_length(width, height, stride).and_then(|length| {
                        let mut pixels = vec![0; length];
                        events.read_exact(&mut pixels).ok().map(|()| pixels)
                    }) {
                        Some(pixels) => Message::Frame {
                            width,
                            height,
                            stride,
                            pixels,
                        },
                        None => Message::Closed,
                    }
                }
                Some(event) => Message::Event(event),
                None => Message::Closed,
            },
        };
        let closed = matches!(message, Message::Closed);
        if sender.send_blocking(message).is_err() || closed {
            return;
        }
    }
}
