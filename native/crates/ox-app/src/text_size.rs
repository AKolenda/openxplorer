// SPDX-License-Identifier: AGPL-3.0-only
//! Text size: the app's own zoom, independent of desktop scaling.
//!
//! Ports `desktop/ui/text-size.js` and its tests
//! (`desktop/tests/text_size.test.cjs`). Ctrl+plus makes text larger,
//! Ctrl+minus smaller and Ctrl+0 resets it; row heights and icon-view cells
//! follow the text so large text never clips.

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
    /// The size after applying this step to `current`, bounded by the levels.
    pub fn apply(self, current: u32) -> u32 {
        match self {
            Step::Increase => step(current, 1),
            Step::Decrease => step(current, -1),
            Step::Reset => DEFAULT,
        }
    }
}

/// Moves `direction` levels from `value`, stopping at the smallest and
/// largest levels.
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

/// Metrics for a text size, matching `metrics()` in text-size.js.
pub fn metrics(value: u32) -> Metrics {
    let scale = f64::from(normalize(value)) / 100.0;
    let detail_row = ((24.0 * scale + 14.0).ceil() as i32).max(38);
    let grid_growth = (((scale - 1.0) * 46.0).ceil() as i32).max(0);
    let grid_width = ((90.0 * scale + 45.0).ceil() as i32).max(135);
    Metrics {
        scale,
        detail_row,
        grid_row: 130 + grid_growth,
        grid_width,
    }
}

/// A key press reduced to what text sizing needs, so the mapping can be
/// tested without a display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyPress<'a> {
    /// Ctrl (or Super, as Meta in the web app) is held.
    pub control: bool,
    /// Alt is held.
    pub alt: bool,
    /// AltGr is held (a third-level shift on many layouts).
    pub alt_graph: bool,
    /// An input method is composing text.
    pub composing: bool,
    /// The GDK key name, for example `plus`, `KP_Add` or `0`.
    pub key: &'a str,
}

/// The text-size command for a key press, if it is one.
pub fn action(press: KeyPress<'_>) -> Option<Step> {
    if !press.control || press.alt || press.alt_graph || press.composing {
        return None;
    }
    match press.key {
        "plus" | "equal" | "KP_Add" => Some(Step::Increase),
        "minus" | "underscore" | "KP_Subtract" => Some(Step::Decrease),
        "0" | "KP_0" | "KP_Insert" => Some(Step::Reset),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(key: &str) -> KeyPress<'_> {
        KeyPress {
            control: true,
            key,
            ..KeyPress::default()
        }
    }

    /// Ported from desktop/tests/text_size.test.cjs::supported
    #[test]
    fn supported_levels_are_kept() {
        for value in LEVELS {
            assert_eq!(normalize(value), value);
        }
    }

    /// Ported from desktop/tests/text_size.test.cjs::fallback
    #[test]
    fn other_values_fall_back_to_the_default() {
        for value in [0, 99, 125 + 1, 1000, u32::MAX] {
            assert_eq!(normalize(value), DEFAULT, "{value}");
        }
    }

    /// Ported from desktop/tests/text_size.test.cjs::Ctrl + / Ctrl - / Ctrl 0
    #[test]
    fn control_plus_minus_and_zero_are_recognised() {
        for key in ["plus", "equal"] {
            assert_eq!(action(ctrl(key)), Some(Step::Increase), "{key}");
        }
        for key in ["minus", "underscore"] {
            assert_eq!(action(ctrl(key)), Some(Step::Decrease), "{key}");
        }
        assert_eq!(action(ctrl("0")), Some(Step::Reset));
    }

    /// Ported from desktop/tests/text_size.test.cjs::keypad add / keypad
    /// subtract / keypad zero
    #[test]
    fn keypad_keys_are_recognised() {
        assert_eq!(action(ctrl("KP_Add")), Some(Step::Increase));
        assert_eq!(action(ctrl("KP_Subtract")), Some(Step::Decrease));
        assert_eq!(action(ctrl("KP_0")), Some(Step::Reset));
    }

    /// Ported from desktop/tests/text_size.test.cjs::unmodified typing ignored
    #[test]
    fn unmodified_typing_is_ignored() {
        let press = KeyPress {
            key: "plus",
            ..KeyPress::default()
        };
        assert_eq!(action(press), None);
    }

    /// Ported from desktop/tests/text_size.test.cjs::AltGraph ignored and
    /// Alt ignored
    #[test]
    fn alt_and_alt_graph_are_ignored() {
        let alt = KeyPress {
            alt: true,
            ..ctrl("plus")
        };
        let alt_graph = KeyPress {
            alt_graph: true,
            ..ctrl("plus")
        };
        assert_eq!(action(alt), None);
        assert_eq!(action(alt_graph), None);
    }

    /// Ported from desktop/tests/text_size.test.cjs::composing ignored and
    /// provisional IME ignored
    #[test]
    fn input_method_composition_is_ignored() {
        let composing = KeyPress {
            composing: true,
            ..ctrl("plus")
        };
        assert_eq!(action(composing), None);
    }

    /// Ported from desktop/tests/text_size.test.cjs::Ctrl+C not captured
    #[test]
    fn other_shortcuts_are_not_captured() {
        assert_eq!(action(ctrl("c")), None);
    }

    /// Ported from desktop/tests/text_size.test.cjs::bounded stepping
    #[test]
    fn stepping_is_bounded() {
        assert_eq!(step(80, -1), 80);
        assert_eq!(step(200, 1), 200);
        assert_eq!(step(100, 1), 110);
        assert_eq!(step(150, -1), 125);
        assert_eq!(Step::Reset.apply(175), DEFAULT);
    }

    /// Ported from desktop/tests/text_size.test.cjs::default metrics unchanged
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

    /// Ported from desktop/tests/text_size.test.cjs::large text row clearance
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
