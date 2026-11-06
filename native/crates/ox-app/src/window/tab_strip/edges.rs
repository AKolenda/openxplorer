// SPDX-License-Identifier: AGPL-3.0-only
//! What the strip draws between and beside its tabs, as Windows 11's File
//! Explorer does (ui-spec.md §4.1, C38 and R06).
//!
//! A thin separator stands in the gap between two tabs, unless one of them
//! is in front or under the pointer, and the tab in front flares out at its
//! foot in two small concave curves that join it to the navigation row
//! below, as the Windows 11 tab view draws them. CSS can draw neither: a widget
//! paints only inside itself, the separator belongs to the gap between two
//! tabs, and GTK's CSS has no concave corner. So the strip draws them over
//! its tabs where they are laid out, which keeps them in place while tabs
//! slide during a drag, in the colours the skin gives two swatches
//! (`.tab-separator` and `.tab-flare` in `skin/title-bar.css`).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

use super::drag_marks::DRAGGED_CLASS;
use super::TabStrip;
use crate::window::widget_tree::children;

/// A separator's height: the 35-pixel tab less the 8-pixel margins above
/// and below a Windows 11 tab separator (`TabViewItemSeparator`, `0,8,0,8`).
const SEPARATOR_HEIGHT: f32 = 19.0;
/// How far the curves at the foot of the tab in front reach out, and
/// their radius (R06, 4 × 4 in the Windows 11 tab view).
const FLARE: f32 = 4.0;

/// Two widgets that draw nothing and take no pointer, whose CSS colour is
/// what the strip draws the separators and the curves in. They stay shown,
/// so the skin restyles them with everything else when the theme changes.
#[derive(Debug)]
pub(super) struct Swatches {
    separator: gtk::Widget,
    flare: gtk::Widget,
}

impl Swatches {
    /// The swatches, made children of `strip`.
    pub(super) fn new(strip: &TabStrip) -> Self {
        let swatch = |class: &str| {
            let swatch = gtk::Box::builder()
                .can_target(false)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .css_classes([class])
                .build();
            swatch.set_parent(strip);
            swatch.upcast::<gtk::Widget>()
        };
        Self {
            separator: swatch("tab-separator"),
            flare: swatch("tab-flare"),
        }
    }

    /// Takes the swatches out of the strip, as it goes.
    pub(super) fn unparent(&self) {
        self.separator.unparent();
        self.flare.unparent();
    }
}

/// A tab as the strip draws around it.
#[derive(Debug, Clone, Copy)]
struct DrawnTab {
    /// Where it is in the strip.
    bounds: graphene::Rect,
    /// It is in front.
    front: bool,
    /// The pointer is over it.
    hovered: bool,
}

impl DrawnTab {
    /// A separator may stand beside it: it is neither in front nor under
    /// the pointer.
    fn is_plain(self) -> bool {
        !self.front && !self.hovered
    }
}

impl TabStrip {
    /// Draws the separators between the tabs and the curves at the foot of
    /// the tab in front, over the tabs.
    pub(super) fn draw_edges(&self, snapshot: &gtk::Snapshot) {
        let Some(swatches) = self.imp().swatches.get() else {
            return;
        };
        let tabs = self.drawn_tabs();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a strip is far narrower than 2^24 pixels"
        )]
        let width = self.width() as f32;
        let separator = swatches.separator.color();
        for pair in tabs.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            if !(left.is_plain() && right.is_plain()) {
                continue;
            }
            let left_end = left.bounds.x() + left.bounds.width();
            let x = (f32::midpoint(left_end, right.bounds.x()) - 0.5).floor();
            if x < 0.0 || x + 1.0 > width {
                continue;
            }
            let y = left.bounds.y() + ((left.bounds.height() - SEPARATOR_HEIGHT) / 2.0).round();
            snapshot.append_color(&separator, &graphene::Rect::new(x, y, 1.0, SEPARATOR_HEIGHT));
        }
        if let Some(front) = tabs.iter().find(|tab| tab.front) {
            draw_flares(snapshot, &front.bounds, width, &swatches.flare.color());
        }
    }

    /// The tabs shown, left to right as they are drawn: during a drag the
    /// order of the widgets is not the order on screen. A tab carried out
    /// of the strip, which is drawn invisible there, is left out.
    fn drawn_tabs(&self) -> Vec<DrawnTab> {
        let mut tabs: Vec<DrawnTab> = children(&self.imp().tab_list.get())
            .filter(|tab| {
                tab.has_css_class("tab") && tab.should_layout() && !tab.has_css_class(DRAGGED_CLASS)
            })
            .filter_map(|tab| {
                Some(DrawnTab {
                    bounds: tab.compute_bounds(self)?,
                    front: tab.has_css_class("active"),
                    hovered: tab.state_flags().contains(gtk::StateFlags::PRELIGHT),
                })
            })
            .collect();
        tabs.sort_by(|a, b| a.bounds.x().total_cmp(&b.bounds.x()));
        tabs
    }
}

/// The two curves at the foot of the tab in front at `tab`, in `colour`,
/// on a strip `width` pixels wide: only beside an edge of the tab that
/// shows, so a tab scrolled partly out of the strip has none on its cut
/// side, and one scrolled out of it has none.
fn draw_flares(snapshot: &gtk::Snapshot, tab: &graphene::Rect, width: f32, colour: &gdk::RGBA) {
    let bottom = tab.y() + tab.height();
    let start = tab.x();
    let end = start + tab.width();
    let shows = |edge: f32| (-0.5..=width + 0.5).contains(&edge);
    let path = gsk::PathBuilder::new();
    if shows(start) {
        add_flare(&path, start, bottom, -FLARE);
    }
    if shows(end) {
        add_flare(&path, end, bottom, FLARE);
    }
    snapshot.append_fill(&path.to_path(), gsk::FillRule::Winding, colour);
}

/// One curve at the foot of the tab edge at `x`, reaching `out` pixels
/// beyond it (to the left when negative): the corner between the edge and
/// the bottom, less a quarter circle of radius [`FLARE`].
fn add_flare(path: &gsk::PathBuilder, x: f32, bottom: f32, out: f32) {
    path.move_to(x + out, bottom);
    path.line_to(x, bottom);
    path.line_to(x, bottom - FLARE);
    path.conic_to(x, bottom, x + out, bottom, std::f32::consts::FRAC_1_SQRT_2);
    path.close();
}
