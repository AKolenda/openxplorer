// SPDX-License-Identifier: AGPL-3.0-only
//! Where the controls are in a snapshot (`OPENXPLORER_HOTSPOTS`), so the
//! website's tour can make the real controls of a picture clickable.
//!
//! Every control the user can click that is drawn in the picture is
//! listed with its text and its rectangle in the picture's logical pixels:
//! buttons, list and sidebar rows, the items of the file list, tabs and
//! text entries, in the window and in the menus open over it. The text is
//! what the control shows (its first label), else its tooltip, else an
//! entry's placeholder; the tooltip is listed as well. The list is JSON:
//!
//! ```json
//! {"width": 1440, "height": 900, "hotspots": [
//!   {"text": "Documents", "tooltip": "", "kind": "row",
//!    "x": 8, "y": 240, "width": 196, "height": 34}]}
//! ```
//!
//! The rectangles are read from the widgets' allocations, so they are
//! where the controls are drawn, not estimates.

use std::fmt::Write as _;
use std::path::Path;

use gtk::graphene;
use gtk::prelude::*;

use super::render::OpenMenu;
use crate::window::widget_tree::descendants;

/// The CSS names of the items of GTK's list, column and grid views.
const ITEM_CSS_NAMES: [&str; 2] = ["row", "child"];

/// A control drawn in the picture.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hotspot {
    /// What it shows: its first label, tooltip or placeholder.
    pub text: String,
    /// Its tooltip, or empty.
    pub tooltip: String,
    /// Its CSS name, such as `button` or `row`.
    pub kind: String,
    /// Its rectangle in the picture, in logical pixels.
    pub area: graphene::Rect,
}

/// The controls of `window` and of the `menus` open over it; `origin` is
/// where the picture starts, in the window's coordinates, and `size` is
/// the picture's size, which clips the rectangles. A menu's controls come
/// after the window's, as the menu is drawn over them.
pub(crate) fn find(
    window: &gtk::Window,
    menus: &[OpenMenu],
    origin: graphene::Point,
    size: graphene::Size,
) -> Vec<Hotspot> {
    let picture = graphene::Rect::new(0.0, 0.0, size.width(), size.height());
    let mut found = controls_of(window.upcast_ref(), origin, picture);
    for menu in menus {
        // The menu's corner in the picture.
        let corner = graphene::Point::new(menu.corner.x() - origin.x(), menu.corner.y() - origin.y());
        let menu_origin = graphene::Point::new(-corner.x(), -corner.y());
        found.extend(controls_of(menu.popover.upcast_ref(), menu_origin, picture));
    }
    found
}

/// The controls drawn on the surface of `native`, whose widgets' corner
/// is at `-origin` in the picture, clipped to `picture`. Widgets of other
/// surfaces have no bounds on this one and are left out.
fn controls_of(native: &gtk::Widget, origin: graphene::Point, picture: graphene::Rect) -> Vec<Hotspot> {
    let mut found = Vec::new();
    for widget in std::iter::once(native.clone()).chain(descendants::<gtk::Widget>(native)) {
        if !widget.is_drawable() || !is_control(&widget) {
            continue;
        }
        let Some(bounds) = widget.compute_bounds(native) else {
            continue;
        };
        let area = bounds.offset_r(-origin.x(), -origin.y());
        let Some(area) = area.intersection(&picture) else {
            continue;
        };
        if area.width() < 1.0 || area.height() < 1.0 {
            continue;
        }
        let tooltip = widget.tooltip_text().map(String::from).unwrap_or_default();
        let text = shown_text(&widget).unwrap_or_else(|| tooltip.clone());
        if text.is_empty() {
            continue;
        }
        found.push(Hotspot {
            text,
            tooltip,
            kind: widget.css_name().to_string(),
            area,
        });
    }
    found
}

/// Whether the user can click or type into `widget`: a button, a row,
/// an item of a list or grid view, a tab or a text entry.
fn is_control(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Button>()
        || widget.is::<gtk::ListBoxRow>()
        || widget.is::<gtk::SearchEntry>()
        || widget.is::<gtk::Entry>()
        || widget.has_css_class("tab")
        || ITEM_CSS_NAMES.contains(&widget.css_name().as_str())
}

/// The first non-empty label inside `widget`, or an entry's placeholder.
fn shown_text(widget: &gtk::Widget) -> Option<String> {
    if let Some(entry) = widget.downcast_ref::<gtk::SearchEntry>() {
        return entry.placeholder_text().map(String::from);
    }
    if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
        return entry.placeholder_text().map(String::from);
    }
    let labels = widget
        .downcast_ref::<gtk::Label>()
        .cloned()
        .into_iter()
        .chain(descendants::<gtk::Label>(widget));
    labels
        .filter(WidgetExt::is_drawable)
        .map(|label| label.text().trim().to_owned())
        .find(|text| !text.is_empty())
}

/// Writes `hotspots` of a picture of `size` to `path` as JSON.
///
/// # Errors
///
/// The error of writing the file.
pub(crate) fn write(path: &Path, size: graphene::Size, hotspots: &[Hotspot]) -> std::io::Result<()> {
    std::fs::write(path, to_json(size, hotspots))
}

/// The JSON the hook writes, one hotspot per line.
fn to_json(size: graphene::Size, hotspots: &[Hotspot]) -> String {
    let mut json = format!(
        "{{\"width\": {}, \"height\": {}, \"hotspots\": [",
        pixels(size.width()),
        pixels(size.height())
    );
    for (index, hotspot) in hotspots.iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };
        let area = hotspot.area;
        // Writing to a String cannot fail.
        let _ = write!(
            json,
            "{separator}\n  {{\"text\": {}, \"tooltip\": {}, \"kind\": {}, \"x\": {}, \"y\": {}, \
             \"width\": {}, \"height\": {}}}",
            json_string(&hotspot.text),
            json_string(&hotspot.tooltip),
            json_string(&hotspot.kind),
            pixels(area.x()),
            pixels(area.y()),
            pixels(area.width()),
            pixels(area.height()),
        );
    }
    json.push_str("\n]}\n");
    json
}

/// `text` as a JSON string literal.
fn json_string(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    for character in text.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            character if u32::from(character) < 0x20 => {
                let _ = write!(literal, "\\u{:04x}", u32::from(character));
            }
            character => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

/// Rounds a widget measure to whole pixels; window measures are far
/// inside `i32`.
#[expect(clippy::cast_possible_truncation, reason = "window measures are small")]
fn pixels(measure: f32) -> i32 {
    measure.round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{wait_until, Fixture, TestWindow};

    /// The listed items and the window's buttons are found where they
    /// are drawn, named by what they show.
    #[gtk::test]
    fn the_controls_of_a_window_are_found_with_their_text() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let first = test.names().first().cloned().expect("the fixture lists items");
        let window: &gtk::Window = test.window.upcast_ref();
        #[expect(clippy::cast_precision_loss, reason = "window sizes are small")]
        let size = graphene::Size::new(window.width() as f32, window.height() as f32);
        let picture = graphene::Rect::new(0.0, 0.0, size.width(), size.height());
        let controls = || find(window, &[], graphene::Point::zero(), size);
        wait_until("the first item to be laid out", || {
            controls()
                .iter()
                .any(|hotspot| hotspot.text == first && hotspot.kind == "row")
        });
        let found = controls();
        assert!(found
            .iter()
            .any(|hotspot| hotspot.tooltip == "New tab (Ctrl+T)" && hotspot.kind == "button"));
        assert!(found
            .iter()
            .all(|hotspot| picture.contains_rect(&hotspot.area) && !hotspot.text.is_empty()));
    }

    #[test]
    fn hotspots_are_written_as_json_in_picture_pixels() {
        let hotspots = [Hotspot {
            text: "Say \"hi\"\\".to_owned(),
            tooltip: String::new(),
            kind: "button".to_owned(),
            area: graphene::Rect::new(10.4, 20.6, 30.0, 40.0),
        }];
        let json = to_json(graphene::Size::new(1440.0, 900.0), &hotspots);
        assert_eq!(
            json,
            "{\"width\": 1440, \"height\": 900, \"hotspots\": [\n  {\"text\": \"Say \\\"hi\\\"\\\\\", \
             \"tooltip\": \"\", \"kind\": \"button\", \"x\": 10, \"y\": 21, \"width\": 30, \"height\": 40}\n]}\n"
        );
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(json_string("a\nb\u{1}"), "\"a\\nb\\u0001\"");
    }
}
