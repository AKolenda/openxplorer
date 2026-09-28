// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers for the tests that measure the window against the current app.
//!
//! The expected numbers in those tests are the current app's layout as
//! Chromium draws `desktop/ui/index.html` with `style.css` at 100% text
//! size (the reference captures of `tools/capture-screenshots.py`), in
//! window coordinates.

use gtk::graphene;
use gtk::prelude::*;

use crate::test_support::harness::{descendants, wait_for_frames, TestWindow};

/// A widget's place, in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bounds {
    /// `width` by `height` pixels at (`x`, `y`).
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width, height }
    }

    /// `rect` rounded to whole pixels.
    pub fn from_rect(rect: &graphene::Rect) -> Self {
        Self::new(
            pixels(rect.x()),
            pixels(rect.y()),
            pixels(rect.width()),
            pixels(rect.height()),
        )
    }

    /// Where the right edge is.
    pub const fn right(self) -> i32 {
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

/// The button in the window that runs `action`.
pub(super) fn button_for(test: &TestWindow, action: &str) -> gtk::Button {
    descendants::<gtk::Button>(&test.window)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some(action))
        .unwrap_or_else(|| panic!("a button runs {action}"))
}
