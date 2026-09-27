// SPDX-License-Identifier: AGPL-3.0-only
//! Text size: the app's own zoom, independent of desktop scaling.
//!
//! Ports `desktop/ui/text-size.js` and its tests
//! (`desktop/tests/text_size.test.cjs`). Ctrl+plus makes text larger,
//! Ctrl+minus smaller and Ctrl+0 resets it; row heights and icon-view cells
//! follow the text so large text never clips.
//!
//! The keys live in one table ([`Step::keys`]) that the window installs as
//! GTK application accelerators. GTK matches accelerators with exactly the
//! listed modifiers, so Ctrl+Alt and `AltGr` combinations never resize text,
//! as `action()` in text-size.js requires.

/// Supported text sizes, in percent.
pub const LEVELS: [u32; 8] = [80, 90, 100, 110, 125, 150, 175, 200];

/// The default text size, in percent.
pub const DEFAULT: u32 = 100;

/// A supported size, or the default for anything else.
pub fn normalize(value: u32) -> u32 {
    if LEVELS.contains(&value) {
        value
    } else {
        DEFAULT
    }
}

/// A text-size keyboard or menu command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// One level larger.
    Increase,
    /// One level smaller.
    Decrease,
    /// Back to 100%.
    Reset,
}

impl Step {
    /// Every command, in menu order.
    pub const ALL: [Step; 3] = [Step::Increase, Step::Decrease, Step::Reset];

    /// The size after applying this step to `current`, bounded by the levels.
    pub fn apply(self, current: u32) -> u32 {
        match self {
            Step::Increase => step(current, 1),
            Step::Decrease => step(current, -1),
            Step::Reset => DEFAULT,
        }
    }

    /// The window action that performs the step.
    pub const fn action_name(self) -> &'static str {
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
    pub const fn keys(self) -> &'static [&'static str] {
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
    pub fn accelerators(self) -> Vec<String> {
        self.keys().iter().map(|key| format!("<Primary>{key}")).collect()
    }
}

/// Moves `direction` levels from `value`, stopping at the smallest and
/// largest levels.
///
/// # Panics
///
/// Never: [`normalize`] always returns one of the [`LEVELS`].
pub fn step(value: u32, direction: isize) -> u32 {
    let index = LEVELS
        .iter()
        .position(|level| *level == normalize(value))
        .expect("normalize always returns a listed level");
    let last = LEVELS.len() - 1;
    let target = index.saturating_add_signed(direction).min(last);
    LEVELS[target]
}

/// Layout sizes derived from the text size, in pixels at scale 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Font scale factor (1.0 at 100%).
    pub scale: f64,
    /// Height of a details-view row.
    pub detail_row: i32,
    /// Height of an icon-view cell.
    pub grid_row: i32,
    /// Width of an icon-view cell.
    pub grid_width: i32,
}

/// Rounds a layout measure up to whole pixels. Measures are a few hundred
/// pixels at most, far inside `i32`.
#[expect(clippy::cast_possible_truncation, reason = "layout measures are small")]
pub(crate) fn ceil_pixels(value: f64) -> i32 {
    value.ceil() as i32
}

/// Metrics for a text size, matching `metrics()` in text-size.js.
pub fn metrics(value: u32) -> Metrics {
    let scale = f64::from(normalize(value)) / 100.0;
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

    /// The step whose table lists `key`, as `action()` in text-size.js
    /// would classify a Ctrl press of that key.
    fn step_for(key: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|step| step.keys().contains(&key))
    }

    /// Ported from `desktop/tests/text_size.test.cjs::supported`
    #[test]
    fn supported_levels_are_kept() {
        for value in LEVELS {
            assert_eq!(normalize(value), value);
        }
    }

    /// Ported from `desktop/tests/text_size.test.cjs::fallback`
    #[test]
    fn other_values_fall_back_to_the_default() {
        for value in [0, 99, 125 + 1, 1000, u32::MAX] {
            assert_eq!(normalize(value), DEFAULT, "{value}");
        }
    }

    /// Ported from `desktop/tests/text_size.test.cjs::Ctrl + / Ctrl - / Ctrl 0`
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
    #[test]
    fn keypad_keys_are_recognised() {
        assert_eq!(step_for("KP_Add"), Some(Step::Increase));
        assert_eq!(step_for("KP_Subtract"), Some(Step::Decrease));
        assert_eq!(step_for("KP_0"), Some(Step::Reset));
        assert_eq!(step_for("KP_Insert"), Some(Step::Reset));
    }

    /// Ported from `desktop/tests/text_size.test.cjs::Ctrl+C not captured`
    #[test]
    fn other_shortcuts_are_not_captured() {
        assert_eq!(step_for("c"), None);
    }

    #[test]
    fn every_accelerator_holds_ctrl_and_menus_show_ctrl_plus_first() {
        for step in Step::ALL {
            let accelerators = step.accelerators();
            assert_eq!(accelerators.len(), step.keys().len());
            assert!(accelerators.iter().all(|accel| accel.starts_with("<Primary>")));
        }
        assert_eq!(Step::Increase.accelerators()[0], "<Primary>plus");
    }

    /// Ported from `desktop/tests/text_size.test.cjs::bounded stepping`
    #[test]
    fn stepping_is_bounded() {
        assert_eq!(step(80, -1), 80);
        assert_eq!(step(200, 1), 200);
        assert_eq!(step(100, 1), 110);
        assert_eq!(step(150, -1), 125);
        assert_eq!(Step::Reset.apply(175), DEFAULT);
    }

    /// Ported from `desktop/tests/text_size.test.cjs::default metrics unchanged`
    #[test]
    fn default_metrics_are_unchanged() {
        let expected = Metrics {
            scale: 1.0,
            detail_row: 38,
            grid_row: 130,
            grid_width: 135,
        };
        assert_eq!(metrics(100), expected);
    }

    /// Ported from `desktop/tests/text_size.test.cjs::large text row clearance`
    #[test]
    fn large_text_keeps_row_clearance() {
        for value in LEVELS {
            let m = metrics(value);
            assert!(f64::from(m.detail_row) >= 12.0 * m.scale * 1.45, "{value}");
            assert!(m.grid_row >= 130);
            assert!(m.grid_width >= 135);
        }
    }
}
