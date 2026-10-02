// SPDX-License-Identifier: AGPL-3.0-only
//! Rubber-band selection: dragging from blank space draws a rectangle that
//! selects the items it touches while it moves (SEL-012).
//!
//! Ports Dolphin's `KItemListRubberBand` and `KItemListController`'s band
//! handling: a plain band selects exactly the items it touches, Shift adds
//! them to the selection the band started from, and Ctrl toggles them. The
//! view scrolls when the pointer nears its edge, and in the details view the
//! band takes whole rows. GTK 4.14's own band applies only when it ends and
//! adds with Ctrl, so the window draws its own: [`banded_selection`] is the
//! rule, the drag gesture feeds it, and the folder pane draws the rectangle.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::BrowserWindow;

/// How far the pointer moves before a press on blank space becomes a band.
const BAND_THRESHOLD: f64 = 4.0;

/// How close to the view's edge the pointer scrolls it, in pixels.
const SCROLL_EDGE: f64 = 24.0;

/// How far the view scrolls each tick while the pointer is at its edge.
const SCROLL_STEP: f64 = 14.0;

/// How often the view scrolls while the pointer is at its edge.
const SCROLL_TICK: Duration = Duration::from_millis(30);

/// What a band does with the selection it starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BandMode {
    /// Selects exactly the touched items.
    Replace,
    /// Adds the touched items (Shift).
    Add,
    /// Toggles the touched items (Ctrl).
    Toggle,
}

impl BandMode {
    /// The mode of a band started with `modifiers` held, as in Dolphin.
    pub(super) fn from_modifiers(modifiers: gdk::ModifierType) -> Self {
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
            BandMode::Toggle
        } else if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
            BandMode::Add
        } else {
            BandMode::Replace
        }
    }
}

/// The selection a band in `mode` gives: from `initial`, the selection when
/// it started, and `touched`, the items it touches now.
pub(super) fn banded_selection(
    initial: &BTreeSet<u32>,
    touched: &BTreeSet<u32>,
    mode: BandMode,
) -> BTreeSet<u32> {
    match mode {
        BandMode::Replace => touched.clone(),
        BandMode::Add => initial.union(touched).copied().collect(),
        BandMode::Toggle => initial.symmetric_difference(touched).copied().collect(),
    }
}

/// A band being drawn.
#[derive(Debug)]
pub(super) struct Band {
    /// The view it is drawn in.
    view: gtk::Widget,
    /// Where it started, in the view's scrolled content.
    start: (f64, f64),
    /// Where the pointer is, in the view.
    pointer: (f64, f64),
    /// The selection it started from.
    initial: BTreeSet<u32>,
    /// What it does with that selection.
    mode: BandMode,
    /// Content bounds of items seen while dragging. Items scrolled away
    /// still leave the selection when the rectangle shrinks past them.
    bounds: BTreeMap<u32, graphene::Rect>,
    /// Scrolls the view while the pointer is at its edge.
    scroll_timer: Option<glib::SourceId>,
}

/// The view's scroll offsets, which turn view points into content points.
fn scroll_offsets(view: &gtk::Widget) -> (f64, f64) {
    let Some(scrollable) = view.dynamic_cast_ref::<gtk::Scrollable>() else {
        return (0.0, 0.0);
    };
    let value = |adjustment: Option<gtk::Adjustment>| adjustment.map_or(0.0, |adjustment| adjustment.value());
    (value(scrollable.hadjustment()), value(scrollable.vadjustment()))
}

/// The rectangle between two points.
fn rect_between(a: (f64, f64), b: (f64, f64)) -> graphene::Rect {
    #[expect(clippy::cast_possible_truncation, reason = "pixel coordinates")]
    let rect = graphene::Rect::new(
        a.0.min(b.0) as f32,
        a.1.min(b.1) as f32,
        (a.0 - b.0).abs() as f32,
        (a.1 - b.1).abs() as f32,
    );
    rect
}

impl BrowserWindow {
    /// Lets a drag from blank space in `view` draw a rubber band.
    pub(super) fn attach_rubber_band(&self, view: &gtk::Widget) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |drag, x, y| {
                if window.are_item_clicks_paused() || !window.is_blank_space(&view, x, y) {
                    drag.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                let mode = BandMode::from_modifiers(drag.current_event_state());
                window.begin_band(&view, (x, y), mode);
            }
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |drag, dx, dy| {
                let Some((x, y)) = drag.start_point() else { return };
                if dx.hypot(dy) < BAND_THRESHOLD && !window.band_is_shown() {
                    return;
                }
                if let Some(band) = window.imp().rubber_band.borrow_mut().as_mut() {
                    band.mode = BandMode::from_modifiers(drag.current_event_state());
                }
                drag.set_state(gtk::EventSequenceState::Claimed);
                window.move_band((x + dx, y + dy));
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| window.end_band()
        ));
        drag.connect_cancel(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.end_band()
        ));
        view.connect_unmap(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.end_band()
        ));
        self.folder_pane()
            .model()
            .selection()
            .connect_items_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, _, _, _| window.end_band()
            ));
        view.add_controller(drag);
    }

    /// Starts a band at `point` in `view`, in `mode`.
    pub(super) fn begin_band(&self, view: &gtk::Widget, point: (f64, f64), mode: BandMode) {
        self.end_band();
        let (dx, dy) = scroll_offsets(view);
        let initial = self
            .folder_pane()
            .model()
            .selected_positions()
            .into_iter()
            .collect();
        self.imp().rubber_band.replace(Some(Band {
            view: view.clone(),
            start: (point.0 + dx, point.1 + dy),
            pointer: point,
            initial,
            mode,
            bounds: BTreeMap::new(),
            scroll_timer: None,
        }));
    }

    /// Whether a band is drawn now.
    fn band_is_shown(&self) -> bool {
        self.folder_pane().rubber_band_shown()
    }

    /// Moves the band's free corner to `pointer` in its view: selects what
    /// it touches, draws it, and scrolls the view at its edge.
    pub(super) fn move_band(&self, pointer: (f64, f64)) {
        if let Some(band) = self.imp().rubber_band.borrow_mut().as_mut() {
            band.pointer = pointer;
        }
        self.update_band();
        self.scroll_at_edge();
    }

    /// Selects what the band touches and draws it where it is now.
    fn update_band(&self) {
        let pane = self.folder_pane();
        let mut state = self.imp().rubber_band.borrow_mut();
        let Some(band) = state.as_mut() else { return };
        let (dx, dy) = scroll_offsets(&band.view);
        let start = (band.start.0 - dx, band.start.1 - dy);
        let mut rect = rect_between(start, band.pointer);
        let content_rect = rect_between(band.start, (band.pointer.0 + dx, band.pointer.1 + dy));
        let full_rows = band.view.is::<gtk::ColumnView>();
        for (position, row) in pane.owners().shown_items(&band.view) {
            let Some(bounds) = row.compute_bounds(&band.view) else {
                continue;
            };
            #[expect(clippy::cast_possible_truncation, reason = "pixel coordinates")]
            let content_bounds = bounds.offset(dx as f32, dy as f32);
            band.bounds.insert(position, content_bounds);
        }
        let touched = touched_positions(&band.bounds, &content_rect, full_rows);
        if full_rows {
            #[expect(clippy::cast_precision_loss, reason = "a widget's pixel width")]
            let width = band.view.width() as f32;
            rect = graphene::Rect::new(0.0, rect.y(), width, rect.height());
        }
        let selected = banded_selection(&band.initial, &touched, band.mode);
        let positions: Vec<u32> = selected.into_iter().collect();
        let shown = band.view.clone();
        drop(state);
        pane.model().select_positions(&positions);
        pane.show_rubber_band(&shown, Some(&rect));
    }

    /// Scrolls the band's view while the pointer is near its edge, moving
    /// the band with it.
    fn scroll_at_edge(&self) {
        let needs_timer = {
            let state = self.imp().rubber_band.borrow();
            state
                .as_ref()
                .is_some_and(|band| band.scroll_timer.is_none() && edge_step(band) != (0.0, 0.0))
        };
        if !needs_timer {
            return;
        }
        let timer = glib::timeout_add_local(
            SCROLL_TICK,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || window.scroll_band_view()
            ),
        );
        if let Some(band) = self.imp().rubber_band.borrow_mut().as_mut() {
            band.scroll_timer = Some(timer);
        }
    }

    /// One scroll tick: scrolls the view toward the pointer's edge, or
    /// stops once the pointer has left it.
    fn scroll_band_view(&self) -> glib::ControlFlow {
        let step = {
            let mut state = self.imp().rubber_band.borrow_mut();
            let Some(band) = state.as_mut() else {
                return glib::ControlFlow::Break;
            };
            let step = edge_step(band);
            if step == (0.0, 0.0) {
                band.scroll_timer = None;
                return glib::ControlFlow::Break;
            }
            let scrollable = band.view.dynamic_cast_ref::<gtk::Scrollable>().cloned();
            (scrollable, step)
        };
        let mut moved = false;
        if let (Some(scrollable), (dx, dy)) = step {
            for (adjustment, delta) in [(scrollable.hadjustment(), dx), (scrollable.vadjustment(), dy)] {
                if let Some(adjustment) = adjustment.filter(|_| delta != 0.0) {
                    let before = adjustment.value();
                    adjustment.set_value(before + delta);
                    moved |= adjustment.value() != before;
                }
            }
        }
        if !moved {
            if let Some(band) = self.imp().rubber_band.borrow_mut().as_mut() {
                band.scroll_timer = None;
            }
            return glib::ControlFlow::Break;
        }
        self.update_band();
        glib::ControlFlow::Continue
    }

    /// Ends the band: the selection stays as it was drawn.
    pub(super) fn end_band(&self) {
        let Some(mut band) = self.imp().rubber_band.take() else {
            return;
        };
        if let Some(timer) = band.scroll_timer.take() {
            timer.remove();
        }
        self.folder_pane().show_rubber_band(&band.view, None);
    }
}

/// How far a tick scrolls the band's view: toward the edge the pointer is
/// near, across and down; nothing away from the edges.
fn edge_step(band: &Band) -> (f64, f64) {
    let (x, y) = band.pointer;
    let width = f64::from(band.view.width());
    let height = f64::from(band.view.height());
    let along = |at: f64, size: f64| {
        if at < SCROLL_EDGE {
            -SCROLL_STEP
        } else if at > size - SCROLL_EDGE {
            SCROLL_STEP
        } else {
            0.0
        }
    };
    (along(x, width), along(y, height))
}

/// The band being drawn, kept by the window.
pub(super) type BandState = RefCell<Option<Band>>;

/// Hit-tests in content coordinates, including cached bounds of rows that
/// have scrolled outside GTK's realized viewport.
fn touched_positions(
    bounds: &BTreeMap<u32, graphene::Rect>,
    rect: &graphene::Rect,
    full_rows: bool,
) -> BTreeSet<u32> {
    bounds
        .iter()
        .filter_map(|(position, bounds)| {
            let inside = if full_rows {
                bounds.y() < rect.y() + rect.height() && rect.y() < bounds.y() + bounds.height()
            } else {
                bounds.intersection(rect).is_some()
            };
            inside.then_some(*position)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(positions: &[u32]) -> BTreeSet<u32> {
        positions.iter().copied().collect()
    }

    /// A plain band selects what it touches, Shift adds that to the
    /// selection it started from and Ctrl toggles it, as in Dolphin.
    ///
    /// parity: SEL-012
    #[test]
    fn a_band_replaces_adds_or_toggles_what_it_touches() {
        let initial = set(&[1, 2]);
        let touched = set(&[2, 3]);
        assert_eq!(
            banded_selection(&initial, &touched, BandMode::Replace),
            set(&[2, 3])
        );
        assert_eq!(
            banded_selection(&initial, &touched, BandMode::Add),
            set(&[1, 2, 3])
        );
        assert_eq!(
            banded_selection(&initial, &touched, BandMode::Toggle),
            set(&[1, 3])
        );
        let ctrl_shift = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        assert_eq!(BandMode::from_modifiers(ctrl_shift), BandMode::Toggle);
        assert_eq!(
            BandMode::from_modifiers(gdk::ModifierType::SHIFT_MASK),
            BandMode::Add
        );
        let bounds = BTreeMap::from([
            (1, graphene::Rect::new(0.0, 100.0, 80.0, 30.0)),
            (2, graphene::Rect::new(0.0, 200.0, 80.0, 30.0)),
        ]);
        let wide = graphene::Rect::new(0.0, 0.0, 80.0, 250.0);
        let short = graphene::Rect::new(0.0, 0.0, 80.0, 150.0);
        assert_eq!(touched_positions(&bounds, &wide, false), set(&[1, 2]));
        assert_eq!(
            touched_positions(&bounds, &short, false),
            set(&[1]),
            "scrolled-away rows leave a shrinking band"
        );
    }
}
