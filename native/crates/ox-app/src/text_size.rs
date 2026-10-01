// SPDX-License-Identifier: AGPL-3.0-only
//! Text size: the app's own zoom, independent of desktop scaling.
//!
//! Ports `desktop/ui/text-size.js` and its tests
//! (`desktop/tests/text_size.test.cjs`). Ctrl+plus makes text larger,
//! Ctrl+minus smaller and Ctrl+0 resets it; row heights and icon-view cells
//! follow the text so large text never clips.
//!
//! A [`TextSize`] is always one of the levels: the saved percentage is
//! normalised once, when it is read, and nothing downstream checks it
//! again.
//!
//! The keys live in one table ([`Step::keys`]) that the window installs as
//! GTK application accelerators. GTK matches accelerators with exactly the
//! listed modifiers, so Ctrl+Alt and `AltGr` combinations never resize text,
//! as `action()` in text-size.js requires.

use ox_core::settings::{DEFAULT_TEXT_SIZE, TEXT_SIZES};

/// Supported text sizes, in percent (`levels` in text-size.js). They are
/// the sizes the settings accept, so a size the app offers is always one
/// the settings save, and the reverse.
const LEVELS: [u32; 8] = TEXT_SIZES;

/// The position of the default size, 100%, in [`LEVELS`].
const DEFAULT_LEVEL: usize = level_of(DEFAULT_TEXT_SIZE);

/// The position of `percent` in [`LEVELS`], worked out while compiling.
///
/// # Panics
///
/// While compiling, when `percent` is not one of the levels, so a default
/// the settings and the app disagree on never builds.
const fn level_of(percent: u32) -> usize {
    let mut level = 0;
    while level < LEVELS.len() {
        if LEVELS[level] == percent {
            return level;
        }
        level += 1;
    }
    panic!("the default text size is one of the levels");
}

/// A supported text size: always one of the levels of text-size.js.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextSize {
    /// The position in [`LEVELS`].
    level: usize,
}

impl TextSize {
    /// 100%: the size a new installation starts at and Ctrl+0 returns to.
    pub(crate) const DEFAULT: TextSize = TextSize { level: DEFAULT_LEVEL };

    /// The size for a saved `percent`, or [`Self::DEFAULT`] when it is not
    /// one of the levels, as `normalize` in text-size.js.
    pub(crate) fn from_percent(percent: u32) -> Self {
        let level = LEVELS.iter().position(|listed| *listed == percent);
        level.map_or(Self::DEFAULT, |level| Self { level })
    }

    /// The size in percent, as the settings file stores it.
    pub(crate) fn percent(self) -> u32 {
        LEVELS[self.level]
    }

    /// Every supported size, smallest first, as Settings offers them.
    pub(crate) fn all() -> impl Iterator<Item = TextSize> {
        (0..LEVELS.len()).map(|level| TextSize { level })
    }
}

impl Default for TextSize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How large text is drawn: the app's own [`TextSize`] on top of the
/// desktop's text scaling (GNOME's Large Text, which GTK reports as the
/// font resolution), so row heights and tiles grow with both (ACC-013).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextScale {
    /// The size chosen in the app.
    pub size: TextSize,
    /// The desktop's text scaling factor, 1.0 when it has none.
    pub desktop: f64,
}

impl TextScale {
    /// The factor every font and text-sized measure is multiplied by.
    pub(crate) fn factor(self) -> f64 {
        f64::from(self.size.percent()) / 100.0 * self.desktop
    }
}

impl From<TextSize> for TextScale {
    fn from(size: TextSize) -> Self {
        Self { size, desktop: 1.0 }
    }
}

/// The font resolution GTK reports at no desktop text scaling, in the
/// 1/1024 dots per inch of `gtk-xft-dpi`.
const UNSCALED_XFT_DPI: f64 = 96.0 * 1024.0;

/// The desktop's text scaling factor for GTK's `gtk-xft-dpi`, 1.0 when it
/// is unset (-1 or 0); kept between half and three times, as GNOME offers.
pub(crate) fn desktop_text_scale(xft_dpi: i32) -> f64 {
    if xft_dpi <= 0 {
        return 1.0;
    }
    (f64::from(xft_dpi) / UNSCALED_XFT_DPI).clamp(0.5, 3.0)
}

/// A text-size keyboard or menu command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// One level larger.
    Increase,
    /// One level smaller.
    Decrease,
    /// Back to 100%.
    Reset,
}

impl Step {
    /// Every command, in menu order.
    pub(crate) const ALL: [Step; 3] = [Step::Increase, Step::Decrease, Step::Reset];

    /// The size after this step from `size`, as `step` in text-size.js:
    /// stepping stops at the smallest and the largest level.
    pub(crate) fn apply(self, size: TextSize) -> TextSize {
        let largest = LEVELS.len() - 1;
        let level = match self {
            Step::Increase => (size.level + 1).min(largest),
            Step::Decrease => size.level.saturating_sub(1),
            Step::Reset => DEFAULT_LEVEL,
        };
        TextSize { level }
    }

    /// The window action that performs the step.
    pub(crate) const fn action_name(self) -> &'static str {
        match self {
            Step::Increase => "text-larger",
            Step::Decrease => "text-smaller",
            Step::Reset => "text-reset",
        }
    }

    /// The GDK key names that perform the step with Ctrl held, from
    /// `action()` in text-size.js: `+`, `=` and keypad Add; `-`, `_` and
    /// keypad Subtract; `0` and keypad 0. `KP_Insert` is keypad 0 with
    /// `NumLock` off, which the web app matched by its physical code.
    ///
    /// The first key is the one menus show.
    pub(crate) const fn keys(self) -> &'static [&'static str] {
        match self {
            Step::Increase => &["plus", "equal", "KP_Add"],
            Step::Decrease => &["minus", "underscore", "KP_Subtract"],
            Step::Reset => &["0", "KP_0", "KP_Insert"],
        }
    }

    /// The GTK accelerators for [`Self::keys`].
    ///
    /// Application accelerators run before the focused widget sees the
    /// key, so `<Primary>KP_Insert` also wins over the address entry's
    /// Ctrl+Insert copy binding. The web app's capture-phase handler did
    /// the same.
    pub(crate) fn accelerators(self) -> Vec<String> {
        self.keys().iter().map(|key| format!("<Primary>{key}")).collect()
    }
}

/// Layout sizes derived from the text size, in pixels at scale 1 (the
/// object `metrics()` in text-size.js returns).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Metrics {
    /// Font scale factor (1.0 at 100%).
    pub scale: f64,
    /// Height of a details-view row (`detailRow`).
    pub detail_row: i32,
    /// Height of an icon-view cell (`gridRow`).
    pub grid_row: i32,
    /// Width of an icon-view cell (`gridWidth`).
    pub grid_width: i32,
}

/// Rounds a layout measure up to whole pixels. Measures are a few hundred
/// pixels at most, far inside `i32`.
#[expect(clippy::cast_possible_truncation, reason = "layout measures are small")]
pub(crate) fn ceil_pixels(value: f64) -> i32 {
    value.ceil() as i32
}

/// The metrics at `scale`, with the formulas and minimums of `metrics()`
/// in text-size.js.
pub(crate) fn metrics(scale: impl Into<TextScale>) -> Metrics {
    let scale = scale.into().factor();
    let detail_row = ceil_pixels(24.0 * scale + 14.0).max(38);
    let grid_growth = ceil_pixels((scale - 1.0) * 46.0).max(0);
    let grid_width = ceil_pixels(90.0 * scale + 45.0).max(135);
    Metrics {
        scale,
        detail_row,
        grid_row: 130 + grid_growth,
        grid_width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size for `percent`, which the test means to be a level.
    fn size(percent: u32) -> TextSize {
        TextSize::from_percent(percent)
    }

    /// The step whose table lists `key`, as `action()` in text-size.js
    /// would classify a Ctrl press of that key.
    fn step_for(key: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|step| step.keys().contains(&key))
    }

    /// Ported from `desktop/tests/text_size.test.cjs::supported`
    ///
    /// parity: VIEW-044
    #[test]
    fn supported_levels_are_kept() {
        for percent in LEVELS {
            assert_eq!(size(percent).percent(), percent);
        }
    }

    /// Ported from `desktop/tests/text_size.test.cjs::fallback`
    ///
    /// parity: VIEW-044
    #[test]
    fn other_values_fall_back_to_the_default() {
        for percent in [0, 99, 125 + 1, 1000, u32::MAX] {
            assert_eq!(TextSize::from_percent(percent).percent(), 100, "{percent}");
        }
    }

    /// The levels are `levels` in text-size.js, which the settings accept
    /// too.
    ///
    /// parity: VIEW-044
    #[test]
    fn every_level_is_listed_once_smallest_first() {
        let percents: Vec<u32> = TextSize::all().map(TextSize::percent).collect();
        assert_eq!(percents, [80, 90, 100, 110, 125, 150, 175, 200]);
        assert_eq!(TextSize::DEFAULT.percent(), 100);
        assert_eq!(TextSize::default(), TextSize::DEFAULT);
    }

    /// Ported from `desktop/tests/text_size.test.cjs::Ctrl + / Ctrl - / Ctrl 0`
    ///
    /// parity: VIEW-043
    #[test]
    fn control_plus_minus_and_zero_are_recognised() {
        for key in ["plus", "equal"] {
            assert_eq!(step_for(key), Some(Step::Increase), "{key}");
        }
        for key in ["minus", "underscore"] {
            assert_eq!(step_for(key), Some(Step::Decrease), "{key}");
        }
        assert_eq!(step_for("0"), Some(Step::Reset));
    }

    /// Ported from `desktop/tests/text_size.test.cjs` (keypad add, keypad
    /// subtract and keypad zero)
    ///
    /// parity: VIEW-043
    #[test]
    fn keypad_keys_are_recognised() {
        assert_eq!(step_for("KP_Add"), Some(Step::Increase));
        assert_eq!(step_for("KP_Subtract"), Some(Step::Decrease));
        assert_eq!(step_for("KP_0"), Some(Step::Reset));
        assert_eq!(step_for("KP_Insert"), Some(Step::Reset));
    }

    /// Ported from `desktop/tests/text_size.test.cjs::Ctrl+C not captured`
    ///
    /// parity: VIEW-043
    #[test]
    fn other_shortcuts_are_not_captured() {
        assert_eq!(step_for("c"), None);
    }

    /// parity: VIEW-043
    #[test]
    fn every_accelerator_holds_ctrl_and_menus_show_ctrl_plus_first() {
        for step in Step::ALL {
            let accelerators = step.accelerators();
            assert_eq!(accelerators.len(), step.keys().len());
            assert!(accelerators
                .iter()
                .all(|accelerator| accelerator.starts_with("<Primary>")));
        }
        assert_eq!(Step::Increase.accelerators()[0], "<Primary>plus");
    }

    /// Ported from `desktop/tests/text_size.test.cjs::bounded stepping`
    ///
    /// parity: VIEW-044
    #[test]
    fn stepping_is_bounded() {
        assert_eq!(Step::Decrease.apply(size(80)).percent(), 80);
        assert_eq!(Step::Increase.apply(size(200)).percent(), 200);
        assert_eq!(Step::Increase.apply(size(100)).percent(), 110);
        assert_eq!(Step::Decrease.apply(size(150)).percent(), 125);
        assert_eq!(Step::Reset.apply(size(175)).percent(), 100);
    }

    /// parity: VIEW-044
    #[test]
    fn a_size_that_is_not_a_level_steps_from_the_default() {
        assert_eq!(Step::Increase.apply(TextSize::from_percent(101)).percent(), 110);
        assert_eq!(Step::Decrease.apply(TextSize::from_percent(0)).percent(), 90);
    }

    /// Ported from `desktop/tests/text_size.test.cjs::default metrics unchanged`
    ///
    /// parity: VIEW-044
    #[test]
    fn default_metrics_are_unchanged() {
        let expected = Metrics {
            scale: 1.0,
            detail_row: 38,
            grid_row: 130,
            grid_width: 135,
        };
        assert_eq!(metrics(size(100)), expected);
    }

    /// Ported from `desktop/tests/text_size.test.cjs::large text row clearance`
    ///
    /// parity: VIEW-044
    #[test]
    fn large_text_keeps_row_clearance() {
        for percent in LEVELS {
            let sized = metrics(size(percent));
            let line_height = 12.0 * sized.scale * 1.45;
            assert!(f64::from(sized.detail_row) >= line_height, "{percent}");
            assert!(sized.grid_row >= 130);
            assert!(sized.grid_width >= 135);
        }
    }
}
