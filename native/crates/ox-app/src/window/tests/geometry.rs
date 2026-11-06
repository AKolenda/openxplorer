// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers for the tests that measure the window against the current app.
//!
//! The expected numbers in those tests are the current app's layout as
//! Chromium draws `v2.0.0:desktop/ui/index.html` with `style.css` at 100% text
//! size (the reference captures of `tools/capture-screenshots.py`), in
//! window coordinates, except where Windows 11's density replaced it
//! (owner decision 2026-11-06): the 48-pixel navigation row and command
//! bar, the 32-pixel column header, rows and sidebar rows, and the
//! 16-pixel icons in rows and the sidebar.

use gtk::graphene;
use gtk::prelude::*;

use crate::test_support::harness::{descendants, wait_for_frames, TestWindow};

/// A widget's place, in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Bounds {
    /// The left edge.
    pub x: i32,
    /// The top edge.
    pub y: i32,
    /// The width.
    pub width: i32,
    /// The height.
    pub height: i32,
}

impl Bounds {
    /// `width` by `height` pixels at (`x`, `y`).
    pub(super) const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width, height }
    }

    /// `rect` rounded to whole pixels.
    pub(super) fn from_rect(rect: &graphene::Rect) -> Self {
        Self::new(
            pixels(rect.x()),
            pixels(rect.y()),
            pixels(rect.width()),
            pixels(rect.height()),
        )
    }

    /// Where the right edge is.
    pub(super) const fn right(self) -> i32 {
        self.x + self.width
    }
}

/// Rounds a widget coordinate to whole pixels.
#[expect(clippy::cast_possible_truncation, reason = "window coordinates are small")]
pub(super) fn pixels(value: f32) -> i32 {
    value.round() as i32
}

/// Where `widget` is in `test`'s window.
pub(super) fn bounds(test: &TestWindow, widget: &impl IsA<gtk::Widget>) -> Bounds {
    let rect = widget
        .compute_bounds(&test.window)
        .unwrap_or_else(graphene::Rect::zero);
    Bounds::from_rect(&rect)
}

/// A window on `uri`, listed and drawn.
pub(super) fn laid_out(uri: &str) -> TestWindow {
    let test = TestWindow::open(uri);
    wait_for_frames(&test.window, 3);
    test
}

/// [`laid_out`] with the details pane open, which a new window has
/// closed.
pub(super) fn laid_out_with_details_pane(uri: &str) -> TestWindow {
    let test = TestWindow::open(uri);
    test.open_details_pane();
    wait_for_frames(&test.window, 3);
    test
}

/// The button in the window that runs `action`.
pub(super) fn button_for(test: &TestWindow, action: &str) -> gtk::Button {
    descendants::<gtk::Button>(&test.window)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some(action))
        .unwrap_or_else(|| panic!("a button runs {action}"))
}
