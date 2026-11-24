// SPDX-License-Identifier: AGPL-3.0-only
//! A row's controls that move onto further lines when they do not fit on
//! one, as CSS `flex-wrap` lays a toolbar out.
//!
//! A row such as "Open `OpenXplorer` to" holds a field and two buttons
//! side by side. At a large text size in a narrow window the three are
//! wider than the page, which would then scroll sideways; here the
//! buttons go under the field instead. Where there is room they stay on
//! one line as before, a field taking the space the buttons leave.

use gtk::glib;
use gtk::prelude::*;

/// The gap between two controls on a line, and between two lines, in
/// pixels: the controls box's own spacing.
pub(super) const GAP: i32 = 8;

/// One control's widths and whether it takes room left on its line.
#[derive(Debug, Clone)]
struct Control {
    widget: gtk::Widget,
    minimum: i32,
    natural: i32,
    expands: bool,
}

/// The visible children of `slot`, measured.
fn controls_of(slot: &gtk::Widget) -> Vec<Control> {
    let mut controls = Vec::new();
    let mut child = slot.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if !widget.should_layout() {
            continue;
        }
        let (minimum, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
        let expands = widget.compute_expand(gtk::Orientation::Horizontal);
        controls.push(Control {
            widget,
            minimum,
            natural,
            expands,
        });
    }
    controls
}

/// `controls` broken into lines `width` pixels wide: each line takes the
/// next controls while their natural widths fit, and at least one.
fn lines(controls: &[Control], width: i32) -> Vec<&[Control]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (index, control) in controls.iter().enumerate() {
        let needed = if index == start {
            control.natural
        } else {
            used + GAP + control.natural
        };
        if index > start && needed > width {
            lines.push(&controls[start..index]);
            start = index;
            used = control.natural;
        } else {
            used = needed;
        }
    }
    if start < controls.len() {
        lines.push(&controls[start..]);
    }
    lines
}

/// The widths `line`'s controls get on a line `width` pixels wide: their
/// natural widths, the room left shared among those that expand, or, on
/// a line too narrow, each narrowed towards its minimum alike.
fn widths(line: &[Control], width: i32) -> Vec<i32> {
    let gaps = GAP * i32::try_from(line.len().saturating_sub(1)).unwrap_or(0);
    let natural: i32 = line.iter().map(|control| control.natural).sum();
    let room = width - gaps;
    if natural > room {
        let minimum: i32 = line.iter().map(|control| control.minimum).sum();
        let spare = (room - minimum).max(0);
        let shrinkable = (natural - minimum).max(1);
        return line
            .iter()
            .map(|control| {
                let give =
                    i64::from(control.natural - control.minimum) * i64::from(spare) / i64::from(shrinkable);
                control.minimum + i32::try_from(give).unwrap_or(0)
            })
            .collect();
    }
    let expanding = i32::try_from(line.iter().filter(|control| control.expands).count()).unwrap_or(0);
    let extra = room - natural;
    let mut handed = 0;
    let mut seen = 0;
    line.iter()
        .map(|control| {
            if !control.expands || expanding == 0 {
                return control.natural;
            }
            seen += 1;
            // The last expanding control takes what rounding left.
            let share = if seen == expanding {
                extra - handed
            } else {
                extra / expanding
            };
            handed += share;
            control.natural + share
        })
        .collect()
}

/// The height of `line` with its controls `widths` wide.
fn line_height(line: &[Control], widths: &[i32]) -> i32 {
    line.iter()
        .zip(widths)
        .map(|(control, width)| control.widget.measure(gtk::Orientation::Vertical, *width).1)
        .max()
        .unwrap_or(0)
}

mod imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{controls_of, line_height, lines, widths, GAP};

    /// Private state of [`super::WrapLayout`]: none.
    #[derive(Debug, Default)]
    pub(crate) struct WrapLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for WrapLayout {
        const NAME: &'static str = "OxSettingsWrapLayout";
        type Type = super::WrapLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for WrapLayout {}

    impl LayoutManagerImpl for WrapLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(
            &self,
            slot: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let controls = controls_of(slot);
            if controls.is_empty() {
                return (0, 0, -1, -1);
            }
            let gaps = GAP * i32::try_from(controls.len() - 1).unwrap_or(0);
            let one_line: i32 = controls.iter().map(|control| control.natural).sum::<i32>() + gaps;
            if orientation == gtk::Orientation::Horizontal {
                let widest = controls.iter().map(|control| control.minimum).max().unwrap_or(0);
                return (widest, one_line, -1, -1);
            }
            let width = if for_size < 0 { one_line } else { for_size };
            let broken = lines(&controls, width);
            let count = i32::try_from(broken.len()).unwrap_or(1);
            let height: i32 = broken
                .iter()
                .map(|line| line_height(line, &widths(line, width)))
                .sum::<i32>()
                + GAP * (count - 1);
            (height, height, -1, -1)
        }

        fn allocate(&self, slot: &gtk::Widget, width: i32, _height: i32, _baseline: i32) {
            let controls = controls_of(slot);
            let mut top = 0;
            for line in lines(&controls, width) {
                let line_widths = widths(line, width);
                let height = line_height(line, &line_widths);
                let mut left = 0;
                for (control, control_width) in line.iter().zip(&line_widths) {
                    let x = if slot.direction() == gtk::TextDirection::Rtl {
                        width - left - control_width
                    } else {
                        left
                    };
                    #[expect(clippy::cast_precision_loss, reason = "a row is a few thousand pixels wide")]
                    let place = gtk::gsk::Transform::new()
                        .translate(&gtk::graphene::Point::new(x as f32, top as f32));
                    control.widget.allocate(*control_width, height, -1, Some(place));
                    left += control_width + GAP;
                }
                top += height + GAP;
            }
        }
    }
}

glib::wrapper! {
    /// Lays a row's controls out on as many lines as the row's width
    /// needs.
    pub(crate) struct WrapLayout(ObjectSubclass<imp::WrapLayout>)
        @extends gtk::LayoutManager;
}

/// Lets `slot`'s children move onto further lines when they do not fit
/// on one ([`WrapLayout`]).
pub(super) fn wrap_children_of(slot: &impl IsA<gtk::Widget>) {
    slot.set_layout_manager(Some(glib::Object::new::<WrapLayout>()));
}
