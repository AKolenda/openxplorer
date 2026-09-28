// SPDX-License-Identifier: AGPL-3.0-only
//! The developer snapshot hook, for visual checks of the window.
//!
//! With `OPENXPLORER_SNAPSHOT=<file.png>` set, the app opens one window,
//! waits until its first listing is drawn, saves the window as a PNG and
//! quits. These variables shape the picture, for this run only (nothing is
//! saved to the settings file):
//!
//! - `OPENXPLORER_START`: the first tab's location, a path or URI (the
//!   home folder by default);
//! - `OPENXPLORER_THEME`: `light`, `dark` or `system`;
//! - `OPENXPLORER_VIEW`: `details` or an icon size (`large`, `medium`, ...);
//! - `OPENXPLORER_SIZE`: the window's size, `<width>x<height>`.
//!
//! The picture is the window's title bar and contents without the frame
//! GTK draws around a window on a display without a compositor, so it
//! lines up with the captures of the current app (`#app` in
//! `tools/capture-screenshots.py`), at the display's scale (twice the
//! size with `GDK_SCALE=2`). The app then runs as a separate
//! instance, so it never hands the request to a running preview.
//! The Python app has no such hook.
//!
//! Before quitting, the hook prints when the window drew its first frame
//! and its first listing, on the monotonic clock (`CLOCK_MONOTONIC`, in
//! microseconds), so a benchmark can time start-up against the moment it
//! started the app.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{glib, graphene};

use crate::theme::ThemePreference;
use crate::window::{BrowserWindow, FolderView};

/// The variable naming the PNG to write; its presence turns the hook on.
const SNAPSHOT_VARIABLE: &str = "OPENXPLORER_SNAPSHOT";
const START_VARIABLE: &str = "OPENXPLORER_START";
const THEME_VARIABLE: &str = "OPENXPLORER_THEME";
const VIEW_VARIABLE: &str = "OPENXPLORER_VIEW";
const SIZE_VARIABLE: &str = "OPENXPLORER_SIZE";

/// Frames drawn after the listing, so late layout changes (column widths,
/// scrolled crumbs, icons) are in the picture.
const SETTLE_FRAMES: u32 = 6;

/// How long the first listing may take before the window is saved anyway.
const LISTING_PATIENCE: Duration = Duration::from_secs(20);

/// Why a snapshot could not be taken.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SnapshotError {
    /// A variable holds a value the hook does not know.
    #[error("{variable} cannot be “{value}”: expected {expected}")]
    InvalidValue {
        /// The variable's name.
        variable: &'static str,
        /// What it holds.
        value: String,
        /// What it may hold.
        expected: &'static str,
    },
    /// The window has not been drawn, so there is nothing to save.
    #[error("the window has not been drawn yet")]
    NotDrawn,
    /// The PNG could not be written.
    #[error("could not save the snapshot to {path}: {source}")]
    Write {
        /// The file that could not be written.
        path: PathBuf,
        /// GDK's reason for refusing the write.
        source: glib::BoolError,
    },
}

/// The size of a window's title bar and contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowSize {
    /// Width in logical pixels, as `gtk::Window::set_default_size` takes it.
    pub width: i32,
    /// Height in logical pixels.
    pub height: i32,
}

impl WindowSize {
    /// Parses `<width>x<height>`, such as `1440x900`.
    fn parse(text: &str) -> Option<Self> {
        let (width, height) = text.trim().split_once('x')?;
        let width = width.parse().ok().filter(|width| *width > 0)?;
        let height = height.parse().ok().filter(|height| *height > 0)?;
        Some(Self { width, height })
    }
}

/// What the developer asked the snapshot to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotRequest {
    /// The PNG to write.
    pub png: PathBuf,
    /// The first tab's location, or `None` for the home folder.
    pub start: Option<String>,
    /// The theme to draw, or `None` for the saved one.
    pub theme: Option<ThemePreference>,
    /// The folder view to show, or `None` for the saved one.
    pub view: Option<FolderView>,
    /// The window size, or `None` for the app's default.
    pub size: Option<WindowSize>,
}

impl SnapshotRequest {
    /// The request in the process environment, or `None` when
    /// `OPENXPLORER_SNAPSHOT` is not set.
    ///
    /// # Errors
    ///
    /// [`SnapshotError::InvalidValue`] when a variable holds an unknown
    /// theme, view or size.
    pub(crate) fn from_environment() -> Result<Option<Self>, SnapshotError> {
        Self::from_variables(|name| std::env::var(name).ok())
    }

    /// The request that the variables `lookup` finds describe; separate
    /// from the process environment so it can be tested.
    fn from_variables(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, SnapshotError> {
        let Some(png) = non_empty(&lookup, SNAPSHOT_VARIABLE) else {
            return Ok(None);
        };
        let theme = parse_variable(&lookup, THEME_VARIABLE, "light, dark or system", |value| {
            ThemePreference::from_key(value)
        })?;
        let view = parse_variable(&lookup, VIEW_VARIABLE, "details or an icon size", |value| {
            FolderView::from_key(value)
        })?;
        let size = parse_variable(&lookup, SIZE_VARIABLE, "<width>x<height>", WindowSize::parse)?;
        Ok(Some(Self {
            png: PathBuf::from(png),
            start: non_empty(&lookup, START_VARIABLE),
            theme,
            view,
            size,
        }))
    }
}

/// The value of the variable `name`, or `None` when it is unset or empty.
fn non_empty(lookup: impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    lookup(name).filter(|value| !value.is_empty())
}

/// Parses the optional variable `name` with `parse`.
///
/// # Errors
///
/// [`SnapshotError::InvalidValue`], naming `expected`, when `parse`
/// refuses the value.
fn parse_variable<T>(
    lookup: impl Fn(&str) -> Option<String>,
    name: &'static str,
    expected: &'static str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, SnapshotError> {
    let Some(value) = non_empty(lookup, name) else {
        return Ok(None);
    };
    match parse(&value) {
        Some(parsed) => Ok(Some(parsed)),
        None => Err(SnapshotError::InvalidValue {
            variable: name,
            value,
            expected,
        }),
    }
}

/// Saves `window` as `request` asks once its first listing is drawn (or
/// after [`LISTING_PATIENCE`]), then hands the outcome to `done`.
pub(crate) fn save_when_listed(
    window: &BrowserWindow,
    request: &SnapshotRequest,
    done: impl FnOnce(Result<(), SnapshotError>) + 'static,
) {
    let png = request.png.clone();
    let size = request.size;
    let milestones = Milestones::default();
    let timing = SaveTiming::starting_now();
    let done = RefCell::new(Some(done));
    window.add_tick_callback(move |window, _| {
        if let Some(size) = size {
            fit_content(window.upcast_ref(), size);
        }
        let listing = Listing::of(window);
        milestones.note_frame(listing);
        let readiness = timing.readiness(listing);
        if readiness == Readiness::Waiting {
            return glib::ControlFlow::Continue;
        }
        if readiness == Readiness::TimedOut {
            eprintln!("OpenXplorer snapshot: the first listing did not finish; saving the window as it is");
        }
        eprintln!("{}", milestones.summary());
        if let Some(done) = done.take() {
            done(save_png(window.upcast_ref(), &png));
        }
        glib::ControlFlow::Break
    });
}

/// Where the window's first listing is on a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    /// Still running, or not started yet.
    Running,
    /// Finished, so this frame draws it.
    Drawn,
}

impl Listing {
    /// Where the listing of `window`'s active tab is now.
    fn of(window: &BrowserWindow) -> Self {
        if window.is_listed() {
            Listing::Drawn
        } else {
            Listing::Running
        }
    }
}

/// Whether the window can be saved on this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    /// Not yet: the listing is running or has not settled.
    Waiting,
    /// The listing has been drawn for [`SETTLE_FRAMES`] frames.
    Settled,
    /// The listing did not finish within [`LISTING_PATIENCE`]; the window
    /// is saved as it is after [`SETTLE_FRAMES`] more frames.
    TimedOut,
}

/// Decides, frame by frame, when the snapshot is taken.
#[derive(Debug)]
struct SaveTiming {
    started: Instant,
    /// Frames drawn since the listing finished or the patience ran out.
    frames_since_listed: Cell<u32>,
}

impl SaveTiming {
    /// Starts the patience now, before the window's first frame.
    fn starting_now() -> Self {
        Self {
            started: Instant::now(),
            frames_since_listed: Cell::new(0),
        }
    }

    /// The readiness on a frame that shows `listing`.
    fn readiness(&self, listing: Listing) -> Readiness {
        let timed_out = self.started.elapsed() > LISTING_PATIENCE;
        if listing == Listing::Running && !timed_out {
            return Readiness::Waiting;
        }
        let frames = self.frames_since_listed.get() + 1;
        self.frames_since_listed.set(frames);
        if frames < SETTLE_FRAMES {
            Readiness::Waiting
        } else if timed_out {
            Readiness::TimedOut
        } else {
            Readiness::Settled
        }
    }
}

/// When the snapshot's window drew its first frame and its first
/// listing, in microseconds on the monotonic clock
/// (`g_get_monotonic_time`).
#[derive(Debug, Default)]
struct Milestones {
    first_frame: Cell<Option<i64>>,
    first_listing: Cell<Option<i64>>,
}

impl Milestones {
    /// Notes that a frame showing `listing` is about to be drawn.
    fn note_frame(&self, listing: Listing) {
        let now = glib::monotonic_time();
        if self.first_frame.get().is_none() {
            self.first_frame.set(Some(now));
        }
        if listing == Listing::Drawn && self.first_listing.get().is_none() {
            self.first_listing.set(Some(now));
        }
    }

    /// The line the hook prints for benchmarks.
    fn summary(&self) -> String {
        format!(
            "OpenXplorer snapshot: first frame at {} us, first listing at {} us (monotonic clock)",
            describe_moment(self.first_frame.get()),
            describe_moment(self.first_listing.get()),
        )
    }
}

/// A milestone's time in microseconds, or "never" before it is reached.
fn describe_moment(moment: Option<i64>) -> String {
    moment.map_or_else(|| "never".to_owned(), |micros| micros.to_string())
}

/// Resizes `window` so its title bar and contents take `size`: a window's
/// default size includes the frame GTK draws on a display without a
/// compositor, which the picture leaves out.
fn fit_content(window: &gtk::Window, size: WindowSize) {
    let (Some(outer), Some(content)) = (window.compute_bounds(window), content_bounds(window)) else {
        return;
    };
    let frame_width = pixels(outer.width() - content.width());
    let frame_height = pixels(outer.height() - content.height());
    let width = size.width + frame_width;
    let height = size.height + frame_height;
    if window.default_size() != (width, height) {
        window.set_default_size(width, height);
    }
}

/// Rounds a widget measure to whole pixels; window measures are far
/// inside `i32`.
#[expect(clippy::cast_possible_truncation, reason = "window measures are small")]
fn pixels(measure: f32) -> i32 {
    measure.round() as i32
}

/// Saves what `window` shows now as a PNG at `path`: its title bar and
/// contents, without the frame of a window on a display without a
/// compositor.
///
/// # Errors
///
/// [`SnapshotError::NotDrawn`] before the window is shown, and
/// [`SnapshotError::Write`] when the file cannot be written.
pub(crate) fn save_png(window: &gtk::Window, path: &Path) -> Result<(), SnapshotError> {
    let content = content_bounds(window).ok_or(SnapshotError::NotDrawn)?;
    render_png(window, Some(content), path)
}

/// Saves what the surface `native` (a window or a popover) draws now as a
/// PNG at `path`, cropped to `crop` in logical pixels from the surface's
/// outer edge, or whole.
///
/// # Errors
///
/// [`SnapshotError::NotDrawn`] before the surface is shown, and
/// [`SnapshotError::Write`] when the file cannot be written.
pub(crate) fn render_png(
    native: &impl IsA<gtk::Native>,
    crop: Option<graphene::Rect>,
    path: &Path,
) -> Result<(), SnapshotError> {
    let native = native.upcast_ref::<gtk::Native>();
    let renderer = native.renderer().ok_or(SnapshotError::NotDrawn)?;
    let paintable = gtk::WidgetPaintable::new(Some(native));
    let snapshot = gtk::Snapshot::new();
    // One picture pixel per device pixel, as the screen shows the surface
    // (two per logical pixel with GDK_SCALE=2). The paintable is drawn at
    // its own size: any other size scales the picture, which blurs
    // one-pixel lines.
    #[expect(clippy::cast_precision_loss, reason = "scale factors are small integers")]
    let scale = native.scale_factor() as f32;
    snapshot.scale(scale, scale);
    let width = f64::from(paintable.intrinsic_width());
    let height = f64::from(paintable.intrinsic_height());
    paintable.snapshot(&snapshot, width, height);
    let node = snapshot.to_node().ok_or(SnapshotError::NotDrawn)?;
    let device_crop = crop.map(|area| area.scale(scale, scale));
    let texture = renderer.render_texture(&node, device_crop.as_ref());
    texture.save_to_png(path).map_err(|source| SnapshotError::Write {
        path: path.to_owned(),
        source,
    })
}

/// The area of `window` taken by its title bar and child, where the
/// window's paintable draws them. The paintable starts at the window's
/// outer edge; widget coordinates start inside the window's frame.
fn content_bounds(window: &gtk::Window) -> Option<graphene::Rect> {
    let outer = window.compute_bounds(window)?;
    let child = window.child()?.compute_bounds(window)?;
    let content = match window.titlebar() {
        Some(title_bar) => title_bar.compute_bounds(window)?.union(&child),
        None => child,
    };
    Some(content.offset_r(-outer.x(), -outer.y()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::folder_view::grid::IconSize;

    fn request(variables: &[(&str, &str)]) -> Result<Option<SnapshotRequest>, SnapshotError> {
        let variables: HashMap<String, String> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        SnapshotRequest::from_variables(|name| variables.get(name).cloned())
    }

    #[test]
    fn without_a_snapshot_file_the_app_runs_normally() {
        let asked = request(&[(THEME_VARIABLE, "dark")]).expect("valid variables");
        assert_eq!(asked, None);
    }

    #[test]
    fn the_snapshot_variables_describe_the_window() {
        let asked = request(&[
            (SNAPSHOT_VARIABLE, "/tmp/window.png"),
            (START_VARIABLE, "pc:"),
            (THEME_VARIABLE, "dark"),
            (VIEW_VARIABLE, "large"),
            (SIZE_VARIABLE, "1440x900"),
        ]);
        let expected = SnapshotRequest {
            png: PathBuf::from("/tmp/window.png"),
            start: Some("pc:".to_owned()),
            theme: Some(ThemePreference::Dark),
            view: Some(FolderView::Icons(IconSize::Large)),
            size: Some(WindowSize {
                width: 1440,
                height: 900,
            }),
        };
        assert_eq!(asked.expect("valid variables"), Some(expected));
    }

    #[test]
    fn an_unknown_value_is_refused_with_its_variable() {
        let refused = request(&[(SNAPSHOT_VARIABLE, "a.png"), (VIEW_VARIABLE, "tiles")]);
        let message = refused.expect_err("tiles is not a view").to_string();
        assert_eq!(
            message,
            "OPENXPLORER_VIEW cannot be “tiles”: expected details or an icon size"
        );
        let refused = request(&[(SNAPSHOT_VARIABLE, "a.png"), (SIZE_VARIABLE, "wide")]);
        assert!(refused.is_err(), "a size needs a width and a height");
    }

    #[test]
    fn the_window_is_saved_once_the_listing_has_settled() {
        let timing = SaveTiming::starting_now();
        assert_eq!(timing.readiness(Listing::Running), Readiness::Waiting);
        for _ in 1..SETTLE_FRAMES {
            assert_eq!(timing.readiness(Listing::Drawn), Readiness::Waiting);
        }
        assert_eq!(timing.readiness(Listing::Drawn), Readiness::Settled);
    }

    #[test]
    fn the_start_up_milestones_read_never_until_reached() {
        let milestones = Milestones::default();
        assert!(milestones.summary().contains("first frame at never us"));
        milestones.note_frame(Listing::Running);
        let first_frame = milestones.first_frame.get().expect("a frame was noted");
        milestones.note_frame(Listing::Running);
        assert_eq!(
            milestones.first_frame.get(),
            Some(first_frame),
            "later frames keep the first"
        );
        assert!(milestones
            .summary()
            .contains(&format!("first frame at {first_frame} us")));
        assert!(milestones.summary().contains("first listing at never us"));
        milestones.note_frame(Listing::Drawn);
        assert!(
            milestones.first_listing.get().is_some(),
            "a listed frame is noted"
        );
    }
}
