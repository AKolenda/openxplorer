// SPDX-License-Identifier: AGPL-3.0-only
//! The window drawn at twice the display scale, through the app's own
//! snapshot hook (`OPENXPLORER_SNAPSHOT`, src/snapshot.rs).
//!
//! Each run starts the built app as a separate process on the display the
//! test runs on (`native/tools/check.py` gives it a private X server and a
//! disposable home), once at `GDK_SCALE=1` and once at `GDK_SCALE=2`.
//! Fractional scales need a Wayland compositor, which the test display is
//! not. Outside that isolation the test does nothing: the app it starts
//! would read the user's settings and could open on the live desktop.

use std::path::Path;
use std::process::Command;

use gtk::gdk;
use gtk::prelude::*;

/// The window's size in logical pixels: near its 670 by 470 minimum, so it
/// fits the test display at twice the scale.
const WIDTH: i32 = 680;
const HEIGHT: i32 = 480;

/// The navigation row and the command bar, where their lines and fills
/// run the whole width (from `the_bars_and_panes_stack_as_in_explorer`).
const BARS: std::ops::Range<i32> = 42..159;

/// How far a channel may stray between the two pictures.
const CHANNEL_TOLERANCE: u8 = 2;

/// Whether the test runs in `native/tools/check.py`'s isolation: temporary
/// files in a private run directory rather than `/tmp`, the home folder and
/// every XDG directory inside it, and a display to draw on.
fn isolated() -> bool {
    let temp = std::env::temp_dir();
    let inside_temp =
        |variable: &str| std::env::var_os(variable).is_some_and(|path| Path::new(&path).starts_with(&temp));
    let user_directories = [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
    ];
    temp != Path::new("/tmp")
        && user_directories.iter().all(|variable| inside_temp(variable))
        && std::env::var_os("DISPLAY").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

/// Saves the app's window at `scale` into `picture`, showing `folder`.
fn snapshot(folder: &Path, scale: u32, picture: &Path) -> gdk::Texture {
    let status = Command::new(env!("CARGO_BIN_EXE_openxplorer-native"))
        .env("GDK_SCALE", scale.to_string())
        .env("OPENXPLORER_SNAPSHOT", picture)
        .env("OPENXPLORER_START", folder)
        .env("OPENXPLORER_THEME", "light")
        .env("OPENXPLORER_SIZE", format!("{WIDTH}x{HEIGHT}"))
        .status()
        .expect("the app starts");
    assert!(status.success(), "the snapshot at scale {scale} failed: {status}");
    gdk::Texture::from_filename(picture).expect("the snapshot is a PNG")
}

/// The pixels of `texture` in GDK's default memory format, and the
/// length of one row.
fn pixels(texture: &gdk::Texture) -> (Vec<u8>, usize) {
    let (bytes, stride) = gdk::TextureDownloader::new(texture).download_bytes();
    (bytes.to_vec(), stride)
}

/// The pixel at (`x`, `y`) of `pixels`.
fn pixel_at((bytes, stride): &(Vec<u8>, usize), x: i32, y: i32) -> [u8; 4] {
    let column = usize::try_from(x).expect("inside the picture");
    let row = usize::try_from(y).expect("inside the picture");
    let start = row * stride + column * 4;
    bytes[start..start + 4].try_into().expect("four channels")
}

/// Whether two pixels are the same colour.
fn same(first: [u8; 4], second: [u8; 4]) -> bool {
    first
        .iter()
        .zip(second)
        .all(|(a, b)| a.abs_diff(b) <= CHANNEL_TOLERANCE)
}

/// At twice the scale the window is drawn with twice the pixels, not
/// enlarged: the layout doubles exactly, and down one column of the
/// navigation row and the command bar each line and fill covers exactly two
/// device pixels of the same colour, where an enlarged picture would blur
/// them into their neighbours.
///
/// parity: LOOK-028
#[test]
fn twice_the_scale_draws_the_bars_on_whole_device_pixels() {
    if !isolated() {
        return;
    }
    gtk::init().expect("the test display");
    let folder = tempfile::tempdir().expect("a folder to show");
    std::fs::create_dir(folder.path().join("Documents")).expect("a subfolder");
    std::fs::write(folder.path().join("Notes.txt"), "notes").expect("a file");
    let pictures = tempfile::tempdir().expect("a folder for the pictures");
    let single = snapshot(folder.path(), 1, &pictures.path().join("scale-1.png"));
    let double = snapshot(folder.path(), 2, &pictures.path().join("scale-2.png"));
    // The picture leaves out the frame GTK draws without a compositor.
    assert!(
        (single.width() - WIDTH).abs() <= 8,
        "{} pixels wide",
        single.width()
    );
    let doubled = (2 * single.width(), 2 * single.height());
    assert_eq!((double.width(), double.height()), doubled, "the layout doubles");

    let column = single.width() - 5;
    let (single, double) = (pixels(&single), pixels(&double));
    for y in BARS {
        let logical = pixel_at(&single, column, y);
        for device_y in [2 * y, 2 * y + 1] {
            let device = pixel_at(&double, 2 * column, device_y);
            assert!(
                same(logical, device),
                "row {y}: {logical:?} at 1x, {device:?} at 2x"
            );
        }
    }
}
