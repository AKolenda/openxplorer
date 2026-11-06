// SPDX-License-Identifier: AGPL-3.0-only
//! What the strip shows between and beside its tabs, as Windows 11's File
//! Explorer does (ui-spec.md §4.1, C38 and R06).
//!
//! A thin separator stands in the gap between two tabs, unless one of them
//! is in front or under the pointer, and the tab in front flares out at its
//! foot in two small concave curves that join it to the navigation row
//! below, as the Windows 11 tab view draws them. A tab cannot show them
//! itself: a widget paints only inside itself, and the separator belongs to
//! the gap between two tabs. So the skin draws them on three small widgets
//! of the strip (`.tab-separator` and `.tab-flare` in
//! `skin/title-bar.css`, the curves as radial gradients), and the strip
//! shows them over its tabs where the tabs are drawn, which keeps them in
//! place while tabs slide during a drag.

use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::drag_marks::DRAGGED_CLASS;
use super::TabStrip;
use crate::window::widget_tree::children;

/// The widgets the skin draws a separator and the two curves on, at the
/// sizes the skin gives them. They take no pointer and sit at the strip's
/// top left, where the strip never draws them: it draws them where they
/// belong instead (`draw_edges`), the separator once for each gap that has
/// one.
#[derive(Debug)]
pub(super) struct Edges {
    separator: gtk::Widget,
    start_flare: gtk::Widget,
    end_flare: gtk::Widget,
}

impl Edges {
    /// The widgets, made children of `strip`.
    pub(super) fn new(strip: &TabStrip) -> Self {
        let piece = |classes: &[&str]| {
            let piece = gtk::Box::builder()
                .can_target(false)
                .halign(gtk::Align::Start)
                .valign(gtk::Align::Start)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .css_classes(classes)
                .build();
            piece.set_parent(strip);
            piece.upcast::<gtk::Widget>()
        };
        Self {
            separator: piece(&["tab-separator"]),
            start_flare: piece(&["tab-flare", "start"]),
            end_flare: piece(&["tab-flare", "end"]),
        }
    }

    /// Takes the widgets out of the strip, as it goes.
    pub(super) fn unparent(&self) {
        for piece in [&self.separator, &self.start_flare, &self.end_flare] {
            piece.unparent();
        }
    }
}

/// A tab as the strip shows around it.
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
    /// Shows the separators between the tabs and the curves at the foot of
    /// the tab in front, over the tabs.
    pub(super) fn draw_edges(&self, snapshot: &gtk::Snapshot) {
        let Some(edges) = self.imp().edges.get() else {
            return;
        };
        let tabs = self.drawn_tabs();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a strip is far narrower than 2^24 pixels"
        )]
        let width = self.width() as f32;
        let size = |piece: &gtk::Widget| {
            let (_, natural) = piece.preferred_size();
            #[expect(clippy::cast_precision_loss, reason = "a few pixels")]
            (natural.width() as f32, natural.height() as f32)
        };
        let (_, separator_height) = size(&edges.separator);
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
            let y = left.bounds.y() + ((left.bounds.height() - separator_height) / 2.0).round();
            self.show_at(snapshot, &edges.separator, x, y);
        }
        let Some(front) = tabs.iter().find(|tab| tab.front) else {
            return;
        };
        // Only beside an edge of the tab that shows, so a tab scrolled
        // partly out of the strip has none on its cut side, and one
        // scrolled out of it has none.
        let shows = |edge: f32| (-0.5..=width + 0.5).contains(&edge);
        let start = front.bounds.x();
        let end = start + front.bounds.width();
        let (flare_width, flare_height) = size(&edges.start_flare);
        let top = front.bounds.y() + front.bounds.height() - flare_height;
        if shows(start) {
            self.show_at(snapshot, &edges.start_flare, start - flare_width, top);
        }
        if shows(end) {
            self.show_at(snapshot, &edges.end_flare, end, top);
        }
    }

    /// Shows `piece`, one of the strip's edge widgets, with its top left at
    /// (`x`, `y`) of the strip. The piece is drawn from where it is laid
    /// out, its start corner, which is the top right in a right-to-left
    /// language, so that place is taken off.
    fn show_at(&self, snapshot: &gtk::Snapshot, piece: &gtk::Widget, x: f32, y: f32) {
        let laid_out = piece
            .compute_point(self, &graphene::Point::zero())
            .unwrap_or_else(graphene::Point::zero);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x - laid_out.x(), y - laid_out.y()));
        self.snapshot_child(piece, snapshot);
        snapshot.restore();
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
