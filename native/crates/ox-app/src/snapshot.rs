// SPDX-License-Identifier: AGPL-3.0-only
//! The developer snapshot hook, for visual checks of the window.
//!
//! With `OPENXPLORER_SNAPSHOT=<file.png>` set, the app opens one window,
//! waits until it has the size asked for and its first listing is drawn,
//! saves the window as a PNG and quits. These variables shape the picture,
//! for this run only (nothing is saved to the settings file):
//!
//! - `OPENXPLORER_START`: the first tab's location, a path or URI (the
//!   home folder by default; `ox:settings` opens Settings);
//! - `OPENXPLORER_THEME`: `light`, `dark` or `system`;
//! - `OPENXPLORER_VIEW`: `details` or an icon size (`large`, `medium`, ...);
//! - `OPENXPLORER_SIZE`: the window's size, `<width>x<height>`;
//! - `OPENXPLORER_SETTINGS`: opens Settings at a category (`general`,
//!   `appearance`, `files`, `archives`, `confirmations`, `search`,
//!   `default-apps`, `about`) or a page one of them opens
//!   (`indexed-folders`, `folder-sizes`, `troubleshooting`);
//! - `OPENXPLORER_SETTINGS_SEARCH`: types this into the settings search,
//!   opening Settings when it is not open;
//! - `OPENXPLORER_SEARCH`: types this into the window's search box, and
//!   waits for the search to show its results;
//! - `OPENXPLORER_SCENE`: test-only steps run once the window is listed,
//!   such as selecting an item or opening its context menu (see
//!   [`scene`]); menus they open are drawn over the window in the picture;
//! - `OPENXPLORER_HOTSPOTS`: writes the rectangles of the picture's
//!   controls to this JSON file (see [`hotspots`]).
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
//! started the app. The variables are read in [`request`]; this module
//! waits for the window and runs the scene, and [`render`] saves the picture.
//! `tools/capture-native-tour.py` uses the hook for the website's tour.

mod hotspots;
mod render;
mod request;
mod scene;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{glib, graphene};

use crate::window::BrowserWindow;

use render::{open_menus, render_png_with_menus};
#[cfg(test)]
pub(crate) use render::{render_png, save_png};
pub(crate) use request::{SnapshotRequest, WindowSize};

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
    /// A scene step could not run.
    #[error("the scene step {step} failed: {reason}")]
    Scene {
        /// The step, as the hook read it.
        step: String,
        /// Why it failed.
        reason: String,
    },
    /// The hotspot file could not be written.
    #[error("could not write the hotspots to {path}: {source}")]
    Hotspots {
        /// The file that could not be written.
        path: PathBuf,
        /// Why it could not.
        source: std::io::Error,
    },
    /// The PNG could not be written.
    #[error("could not save the snapshot to {path}: {source}")]
    Write {
        /// The file that could not be written.
        path: PathBuf,
        /// GDK's reason for refusing the write.
        source: glib::BoolError,
    },
}

/// Saves `window` as `request` asks once it has the size asked for and its
/// first listing is drawn (or after [`LISTING_PATIENCE`]), and the scene's
/// steps have run, each once the window has settled, then hands the
/// outcome to `done`.
pub(crate) fn save_when_listed(
    window: &BrowserWindow,
    request: &SnapshotRequest,
    done: impl FnOnce(Result<(), SnapshotError>) + 'static,
) {
    let request = request.clone();
    let steps = RefCell::new(VecDeque::from(request.scene.clone()));
    let milestones = Milestones::default();
    let timing = RefCell::new(SaveTiming::starting_now());
    let done = RefCell::new(Some(done));
    window.add_tick_callback(move |window, _| {
        if let Some(size) = request.size {
            fit_content(window.upcast_ref(), size);
        }
        let listing = Listing::of(window);
        milestones.note_frame(listing);
        // A window that changes its layout as it narrows (Settings does)
        // takes a few frames to reach the size; the frames that settle the
        // picture count from then on.
        let is_resized = request
            .size
            .is_none_or(|size| has_content_size(window.upcast_ref(), size));
        if !is_resized && !timing.borrow().has_timed_out() {
            return glib::ControlFlow::Continue;
        }
        let readiness = timing.borrow().count_frame(listing);
        if readiness == Readiness::Waiting {
            return glib::ControlFlow::Continue;
        }
        if readiness == Readiness::TimedOut {
            eprintln!("OpenXplorer snapshot: the listing did not finish; going on with the window as it is");
        }
        let step = steps.borrow_mut().pop_front();
        let outcome = if let Some(step) = step {
            match step.run(window) {
                Ok(pause) => {
                    // The next step, or the picture, waits for the pause
                    // and for the window to settle again.
                    timing.replace(SaveTiming::after(pause));
                    return glib::ControlFlow::Continue;
                }
                Err(reason) => Err(SnapshotError::Scene {
                    step: format!("{step:?}"),
                    reason,
                }),
            }
        } else {
            eprintln!("{}", milestones.summary());
            save_scene(window.upcast_ref(), &request)
        };
        if let Some(done) = done.take() {
            done(outcome);
        }
        glib::ControlFlow::Break
    });
}

/// Saves the picture `request` asks for, with the menus open over the
/// window, and the hotspots when asked.
fn save_scene(window: &gtk::Window, request: &SnapshotRequest) -> Result<(), SnapshotError> {
    let content = content_bounds(window).ok_or(SnapshotError::NotDrawn)?;
    let outer = window.compute_bounds(window).ok_or(SnapshotError::NotDrawn)?;
    let menus = open_menus(window);
    render_png_with_menus(window, &menus, content, &request.png)?;
    let Some(path) = &request.hotspots else {
        return Ok(());
    };
    // The picture starts at the content's corner, in window coordinates.
    let origin = graphene::Point::new(content.x() + outer.x(), content.y() + outer.y());
    let size = graphene::Size::new(content.width(), content.height());
    let found = hotspots::find(window, &menus, origin, size);
    hotspots::write(path, size, &found).map_err(|source| SnapshotError::Hotspots {
        path: path.clone(),
        source,
    })
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
    /// Where the listing of `window`'s active tab is now; a search typed
    /// into the window counts as part of it until it shows its results.
    fn of(window: &BrowserWindow) -> Self {
        if window.is_listed() && !window.is_search_running() {
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
    /// When the patience for the first listing started.
    started: Instant,
    /// Until when no frame counts: a scene's pause.
    paused_until: Instant,
    /// Frames drawn since the listing finished or the patience ran out.
    frames_since_listed: Cell<u32>,
}

impl SaveTiming {
    /// Starts the patience now, before the window's first frame.
    fn starting_now() -> Self {
        Self::after(Duration::ZERO)
    }

    /// Starts the patience now, and counts frames only after `pause`.
    fn after(pause: Duration) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            paused_until: now + pause,
            frames_since_listed: Cell::new(0),
        }
    }

    /// Whether the patience has run out.
    fn has_timed_out(&self) -> bool {
        self.started.elapsed() > LISTING_PATIENCE
    }

    /// Counts a frame that shows `listing` and says whether the window can
    /// be saved on it. Once the listing is drawn, or the patience ran out,
    /// every call counts towards [`SETTLE_FRAMES`], so call it once per
    /// frame.
    fn count_frame(&self, listing: Listing) -> Readiness {
        if Instant::now() < self.paused_until {
            return Readiness::Waiting;
        }
        let timed_out = self.has_timed_out();
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
            "OpenXplorer snapshot: first frame {}, first listing {} (monotonic clock)",
            describe_moment(self.first_frame.get()),
            describe_moment(self.first_listing.get()),
        )
    }
}

/// When a milestone was reached ("at 1234 us"), or "never" before it is.
fn describe_moment(moment: Option<i64>) -> String {
    match moment {
        Some(micros) => format!("at {micros} us"),
        None => "never".to_owned(),
    }
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

/// Whether the title bar and contents of `window` take `size` now.
fn has_content_size(window: &gtk::Window, size: WindowSize) -> bool {
    let Some(content) = content_bounds(window) else {
        return false;
    };
    pixels(content.width()) == size.width && pixels(content.height()) == size.height
}

/// Rounds a widget measure to whole pixels; window measures are far
/// inside `i32`.
#[expect(clippy::cast_possible_truncation, reason = "window measures are small")]
fn pixels(measure: f32) -> i32 {
    measure.round() as i32
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
    use super::*;

    #[test]
    fn the_window_is_saved_once_the_listing_has_settled() {
        let timing = SaveTiming::starting_now();
        assert_eq!(timing.count_frame(Listing::Running), Readiness::Waiting);
        for _ in 1..SETTLE_FRAMES {
            assert_eq!(timing.count_frame(Listing::Drawn), Readiness::Waiting);
        }
        assert_eq!(timing.count_frame(Listing::Drawn), Readiness::Settled);
    }

    #[test]
    fn the_start_up_milestones_read_never_until_reached() {
        let milestones = Milestones::default();
        assert_eq!(
            milestones.summary(),
            "OpenXplorer snapshot: first frame never, first listing never (monotonic clock)"
        );
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
            .contains(&format!("first frame at {first_frame} us,")));
        assert!(milestones
            .summary()
            .contains("first listing never (monotonic clock)"));
        milestones.note_frame(Listing::Drawn);
        assert!(
            milestones.first_listing.get().is_some(),
            "a listed frame is noted"
        );
    }
}
