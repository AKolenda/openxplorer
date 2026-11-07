// SPDX-License-Identifier: AGPL-3.0-only
//! A real pointer on the tests' private X display, for tests that must go
//! through GTK's own event handling: which gesture claims a press, and in
//! what order, is decided there and nowhere a test could call directly.
//!
//! [`Pointer`] moves the pointer, presses buttons and taps keys with the X
//! server's XTEST extension, so GTK receives the same device events a mouse
//! and a keyboard make.
//! It speaks the few X11 requests it needs over the display's socket
//! itself, so the tests need no X library or crate beyond GTK. It refuses
//! to run outside `native/tools/check.py`'s isolated session, so a test can
//! never move the pointer of a live desktop.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;

use super::harness::wait_for;

/// How long the window gets to handle each step, after the X server has.
const STEP_SETTLE: Duration = Duration::from_millis(30);

/// The X11 opcodes and constants used here (X11 protocol and XTEST 2.2).
const QUERY_EXTENSION: u8 = 98;
const GET_INPUT_FOCUS: u8 = 43;
const XTEST_FAKE_INPUT: u8 = 2;
const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION_NOTIFY: u8 = 6;
const COOKIE_AUTHORIZATION: &[u8] = b"MIT-MAGIC-COOKIE-1";

/// The private display's screen (`XVFB_SERVER_ARGUMENTS` in check.py), and
/// how far apart the pointer looks for a window on it.
const SCREEN_WIDTH: i32 = 1440;
const SCREEN_HEIGHT: i32 = 1000;
const SEARCH_STEP: usize = 80;

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Primary = 1,
    Middle = 2,
    Secondary = 3,
}

/// The tests' pointer, on the private X display. Dropping it releases
/// the buttons it holds, so a test that fails mid-drag leaves none pressed
/// for the next one.
#[derive(Debug)]
pub(crate) struct Pointer {
    connection: UnixStream,
    root: u32,
    xtest: u8,
    /// The buttons pressed and not released yet.
    held: Vec<Button>,
}

impl Pointer {
    /// Connects to the private display of `native/tools/check.py`, after
    /// closing what earlier tests left on its screen (see
    /// [`close_leftovers`]), so the pointer reaches the window under test.
    ///
    /// # Panics
    ///
    /// Outside that isolated session, or when the display cannot be
    /// reached or lacks XTEST.
    pub(crate) fn connect() -> Self {
        assert_eq!(
            std::env::var("OX_ISOLATED_SESSION").as_deref(),
            Ok("1"),
            "a real pointer moves only on check.py's private display"
        );
        let display = std::env::var("DISPLAY").expect("the private display is named in DISPLAY");
        let number = display_number(&display);
        let mut connection = UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))
            .expect("the private display takes local connections");
        connection
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("a read timeout");
        let root = set_up(&mut connection, &number);
        let mut pointer = Self {
            connection,
            root,
            xtest: 0,
            held: Vec::new(),
        };
        pointer.xtest = pointer.extension_opcode(b"XTEST");
        close_leftovers();
        pointer
    }

    /// Moves the pointer to (`x`, `y`) of the screen.
    pub(crate) fn move_to(&mut self, x: i32, y: i32) {
        self.fake(MOTION_NOTIFY, 0, x, y);
    }

    /// Presses `button` where the pointer is.
    pub(crate) fn press(&mut self, button: Button) {
        self.held.push(button);
        self.fake(BUTTON_PRESS, button as u8, 0, 0);
    }

    /// Releases `button`.
    pub(crate) fn release(&mut self, button: Button) {
        self.held.retain(|held| *held != button);
        self.fake(BUTTON_RELEASE, button as u8, 0, 0);
    }

    /// Presses and releases `key` on the private display's keyboard.
    pub(crate) fn tap_key(&mut self, key: gdk::Key) {
        let keycode = keycode_of(key);
        self.fake(KEY_PRESS, keycode, 0, 0);
        self.fake(KEY_RELEASE, keycode, 0, 0);
    }

    /// Holds `key` down on the private display's keyboard until
    /// [`Self::let_go_key`].
    pub(crate) fn hold_key(&mut self, key: gdk::Key) {
        let keycode = keycode_of(key);
        self.fake(KEY_PRESS, keycode, 0, 0);
    }

    /// Lets go of `key`, held with [`Self::hold_key`].
    pub(crate) fn let_go_key(&mut self, key: gdk::Key) {
        let keycode = keycode_of(key);
        self.fake(KEY_RELEASE, keycode, 0, 0);
    }

    /// Moves to (`x`, `y`) in `steps` even steps from (`from_x`, `from_y`),
    /// the window handling each one, as a hand moving a mouse.
    pub(crate) fn glide(&mut self, (from_x, from_y): (i32, i32), (x, y): (i32, i32), steps: i32) {
        for step in 1..=steps {
            self.move_to(
                from_x + (x - from_x) * step / steps,
                from_y + (y - from_y) * step / steps,
            );
        }
    }

    /// Presses `button` where the pointer is and moves it through `path` as
    /// one flick of the hand: the X server has every step before the window
    /// handles the first, so the window handles them one after another, and
    /// nothing it times from the press, such as GTK's 100 milliseconds
    /// before a drag and drop may start, comes between them however busy
    /// its main loop is.
    pub(crate) fn press_and_flick(&mut self, button: Button, path: &[(i32, i32)]) {
        self.held.push(button);
        let mut requests = self.fake_request(BUTTON_PRESS, button as u8, 0, 0);
        for &(x, y) in path {
            requests.extend(self.fake_request(MOTION_NOTIFY, 0, x, y));
        }
        self.send(&requests);
        self.sync();
        wait_for(STEP_SETTLE);
    }

    /// One fake device event, then waits until the X server handled it and
    /// the window had time to.
    fn fake(&mut self, kind: u8, detail: u8, x: i32, y: i32) {
        let request = self.fake_request(kind, detail, x, y);
        self.send(&request);
        self.sync();
        wait_for(STEP_SETTLE);
    }

    /// The XTEST request for one fake device event, now.
    fn fake_request(&self, kind: u8, detail: u8, x: i32, y: i32) -> Vec<u8> {
        let mut request = Vec::with_capacity(36);
        request.extend_from_slice(&[self.xtest, XTEST_FAKE_INPUT]);
        request.extend_from_slice(&9_u16.to_le_bytes());
        request.extend_from_slice(&[kind, detail, 0, 0]);
        request.extend_from_slice(&0_u32.to_le_bytes()); // now
        request.extend_from_slice(&self.root.to_le_bytes());
        request.extend_from_slice(&[0; 8]);
        request.extend_from_slice(&coordinate(x).to_le_bytes());
        request.extend_from_slice(&coordinate(y).to_le_bytes());
        request.extend_from_slice(&[0; 8]); // padding, then device 0
        request
    }

    /// The major opcode of the extension `name`.
    fn extension_opcode(&mut self, name: &[u8]) -> u8 {
        let padded = padded(name);
        let mut request = vec![QUERY_EXTENSION, 0];
        request.extend_from_slice(&request_length(8 + padded.len()).to_le_bytes());
        request.extend_from_slice(&length16(name.len()).to_le_bytes());
        request.extend_from_slice(&[0, 0]);
        request.extend_from_slice(&padded);
        self.send(&request);
        let reply = self.reply();
        assert_eq!(reply[8], 1, "the private display has the XTEST extension");
        reply[9]
    }

    /// Waits until the X server has handled every request sent so far.
    fn sync(&mut self) {
        self.send(&[GET_INPUT_FOCUS, 0, 1, 0]);
        self.reply();
    }

    fn send(&mut self, request: &[u8]) {
        self.connection
            .write_all(request)
            .expect("the private display takes requests");
    }

    /// The next reply, skipping events; an X error fails the test.
    fn reply(&mut self) -> Vec<u8> {
        loop {
            let mut packet = vec![0; 32];
            self.connection
                .read_exact(&mut packet)
                .expect("the private display answers");
            match packet[0] {
                0 => panic!("the private display refused a request: error {}", packet[1]),
                1 => {
                    let extra = u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]);
                    let mut rest = vec![0; usize::try_from(extra).expect("a small reply") * 4];
                    self.connection
                        .read_exact(&mut rest)
                        .expect("the private display answers");
                    packet.extend(rest);
                    return packet;
                }
                _ => {}
            }
        }
    }
}

impl Drop for Pointer {
    fn drop(&mut self) {
        // Never panics, as a drop may run during a failed test's
        // unwinding: a display that went away holds no buttons either.
        for button in std::mem::take(&mut self.held) {
            let request = self.fake_request(BUTTON_RELEASE, button as u8, 0, 0);
            if self.connection.write_all(&request).is_err() {
                return;
            }
        }
        let _ = self.connection.write_all(&[GET_INPUT_FOCUS, 0, 1, 0]);
        let mut reply = [0; 32];
        let _ = self.connection.read_exact(&mut reply);
        super::harness::settle();
    }
}

/// Takes off the screen what earlier tests of the process left on it
/// (see [`super::harness::put_away`]): a window or dialog still shown
/// over no window, with any menu open in it and the pointer and keyboard
/// grab such a menu holds, which go with its window. Those tests' own
/// events never went through the X server, so nothing they left showed
/// there; a real pointer's press would land on it instead of on the window
/// under test. The test's own windows stay, as a test may connect after
/// opening them: browser windows, which a test window's teardown always
/// destroys, and what shows over a window still shown.
/// Says on standard error what it put away, for a failure's log.
fn close_leftovers() {
    // Hiding a window leaves the dialogs over it shown, now over no shown
    // window: the next pass puts them away. A dialog chain is never this
    // deep.
    const PASSES: usize = 8;
    for _ in 0..PASSES {
        let shown: Vec<gtk::Window> = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .filter(|window| {
                let over_a_shown_window = window.transient_for().is_some_and(|parent| parent.is_visible());
                window.is_visible() && !window.is::<crate::window::BrowserWindow>() && !over_a_shown_window
            })
            .collect();
        if shown.is_empty() {
            return;
        }
        for window in &shown {
            eprintln!(
                "putting away a {} left shown by an earlier test: {:?}",
                window.type_().name(),
                window.title().unwrap_or_default()
            );
            super::harness::put_away(window);
        }
        super::harness::settle();
    }
}

/// The screen coordinates of (`x`, `y`) in `widget`, found by placing the
/// pointer at `probe` on the screen, over `widget`'s window, and asking GDK
/// where that is in the window.
pub(crate) fn screen_point(
    pointer: &mut Pointer,
    widget: &impl IsA<gtk::Widget>,
    (x, y): (f64, f64),
    probe: (i32, i32),
) -> (i32, i32) {
    let (left, top) = window_origin(pointer, widget, probe);
    let native = widget.native().expect("a shown widget is in a window");
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let in_native = widget
        .compute_point(&native, &gtk::graphene::Point::new(x as f32, y as f32))
        .expect("a shown widget is laid out in its window");
    let (shadow_x, shadow_y) = native.surface_transform();
    // GDK measures in logical pixels, the X server in device pixels.
    let scale = f64::from(native.scale_factor());
    (
        left + round((f64::from(in_native.x()) + shadow_x) * scale),
        top + round((f64::from(in_native.y()) + shadow_y) * scale),
    )
}

/// Where the top left corner of `widget`'s window surface is on the
/// screen, found as [`screen_point`] does: it changes when the window
/// moves. GDK answers only while the pointer is over the window, so when
/// `probe` is not, the pointer looks for the window across the screen.
pub(crate) fn window_origin(
    pointer: &mut Pointer,
    widget: &impl IsA<gtk::Widget>,
    probe: (i32, i32),
) -> (i32, i32) {
    let native = widget.native().expect("a shown widget is in a window");
    let surface = native.surface().expect("a shown window has its surface");
    let device = gdk::Display::default()
        .and_then(|display| display.default_seat())
        .and_then(|seat| seat.pointer())
        .expect("the private display has a pointer");
    let screen = (0..SCREEN_HEIGHT)
        .step_by(SEARCH_STEP)
        .flat_map(|y| (0..SCREEN_WIDTH).step_by(SEARCH_STEP).map(move |x| (x, y)));
    std::iter::once(probe)
        .chain(screen)
        .find_map(|(x, y)| {
            pointer.move_to(x, y);
            let (in_x, in_y, _) = surface.device_position(&device)?;
            let scale = f64::from(surface.scale_factor());
            Some((x - round(in_x * scale), y - round(in_y * scale)))
        })
        .expect("the window is on the screen")
}

/// The X11 keycode of `key` on the private display's keyboard.
fn keycode_of(key: gdk::Key) -> u8 {
    let keycode = gdk::Display::default()
        .and_then(|display| display.map_keyval(key))
        .and_then(|keys| keys.first().map(gdk::KeymapKey::keycode))
        .expect("the private display's keyboard has the key");
    u8::try_from(keycode).expect("an X11 keycode")
}

#[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
fn round(value: f64) -> i32 {
    value.round() as i32
}

fn coordinate(value: i32) -> i16 {
    i16::try_from(value).expect("a point on the screen")
}

fn length16(value: usize) -> u16 {
    u16::try_from(value).expect("a short X11 field")
}

/// A request's length in four-byte units.
fn request_length(bytes: usize) -> u16 {
    length16(bytes / 4)
}

/// `bytes` padded with zeros to a multiple of four.
fn padded(bytes: &[u8]) -> Vec<u8> {
    let mut padded = bytes.to_vec();
    padded.resize(bytes.len().div_ceil(4) * 4, 0);
    padded
}

/// The display number of `display`, such as `99` of `:99.0`.
fn display_number(display: &str) -> String {
    let number = display
        .rsplit(':')
        .next()
        .and_then(|rest| rest.split('.').next())
        .unwrap_or_default();
    assert!(
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
        "the private display is local: {display}"
    );
    number.to_owned()
}

/// The private display's cookie from the X authority file, if it has one.
fn cookie(number: &str) -> Option<Vec<u8>> {
    let path = std::env::var_os("XAUTHORITY")?;
    let bytes = std::fs::read(path).ok()?;
    let mut rest = bytes.as_slice();
    let field = |rest: &mut &[u8]| -> Option<Vec<u8>> {
        let (length, tail) = rest.split_first_chunk::<2>()?;
        let length = usize::from(u16::from_be_bytes(*length));
        let value = tail.get(..length)?.to_vec();
        *rest = &tail[length..];
        Some(value)
    };
    while rest.len() >= 2 {
        rest = &rest[2..]; // the address family
        let _address = field(&mut rest)?;
        let entry_number = field(&mut rest)?;
        let name = field(&mut rest)?;
        let data = field(&mut rest)?;
        if entry_number == number.as_bytes() && name == COOKIE_AUTHORIZATION {
            return Some(data);
        }
    }
    None
}

/// Opens the connection, with the display's cookie when it has one; the
/// first screen's root window.
fn set_up(connection: &mut UnixStream, number: &str) -> u32 {
    let cookie = cookie(number);
    let (name, data) = match &cookie {
        Some(data) => (COOKIE_AUTHORIZATION, data.as_slice()),
        None => (&b""[..], &b""[..]),
    };
    let mut request = vec![b'l', 0];
    request.extend_from_slice(&11_u16.to_le_bytes());
    request.extend_from_slice(&0_u16.to_le_bytes());
    request.extend_from_slice(&length16(name.len()).to_le_bytes());
    request.extend_from_slice(&length16(data.len()).to_le_bytes());
    request.extend_from_slice(&[0, 0]);
    request.extend_from_slice(&padded(name));
    request.extend_from_slice(&padded(data));
    connection
        .write_all(&request)
        .expect("the private display takes the connection");
    let mut header = [0; 8];
    connection
        .read_exact(&mut header)
        .expect("the private display answers the connection");
    let extra = usize::from(u16::from_le_bytes([header[6], header[7]])) * 4;
    let mut setup = vec![0; extra];
    connection
        .read_exact(&mut setup)
        .expect("the private display describes itself");
    assert_eq!(header[0], 1, "the private display accepts the connection");
    let vendor = usize::from(u16::from_le_bytes([setup[16], setup[17]]));
    let formats = usize::from(setup[21]);
    let screen = 32 + vendor.div_ceil(4) * 4 + formats * 8;
    u32::from_le_bytes([
        setup[screen],
        setup[screen + 1],
        setup[screen + 2],
        setup[screen + 3],
    ])
}
