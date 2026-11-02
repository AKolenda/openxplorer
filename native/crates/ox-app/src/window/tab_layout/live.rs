// SPDX-License-Identifier: AGPL-3.0-only
//! Where the tabs are while a tab is dragged along a strip, as in Windows
//! Explorer and Chrome (TAB-032, TAB-033).
//!
//! The dragged tab follows the pointer sideways, kept inside the row, and
//! the other tabs make room for it: each slides to its new place when the
//! dragged tab's leading edge passes its middle, so the order shown is the
//! order a release keeps. A strip that a tab from another window hovers
//! opens a gap of that tab's width where it would land instead. Nothing is
//! rebuilt while this lasts: the layout places the same tab widgets at
//! other positions. A release slides the dragged tab into its place before
//! the strip takes the new order.
//!
//! The slides last [`SLIDE`] and ease out as Chrome's tab strip does
//! (`BoundsAnimator`: 200 ms, `gfx::Tween::EASE_OUT`), driven by the
//! widget's frame clock and drawn at sub-pixel positions. A tab moves to
//! another place only after the pointer went [`REORDER_HYSTERESIS`] since
//! the last move (Chrome's `kHorizontalMoveThreshold`), so the order does
//! not flicker. Once the tab leaves the strip it vanishes there and the
//! others close up, as Chrome takes a dragged-out tab off its strip. While
//! the desktop turns animations off (`gtk-enable-animations`), tabs move
//! into place at once; the dragged tab still follows the pointer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

/// The width the layout gives a tab, padding and border included.
pub(super) type Width<'a> = &'a dyn Fn(&gtk::Widget) -> f64;

/// How long a tab takes to slide into its place.
pub(super) const SLIDE: Duration = Duration::from_millis(200);

/// How far the pointer must go after one move of the order before the
/// next, for a tab 215 pixels wide; narrower tabs need less.
const REORDER_HYSTERESIS: f64 = 16.0;

/// A tab's width when there is room, which [`REORDER_HYSTERESIS`] is for.
const STANDARD_WIDTH: f64 = 215.0;

/// One tab sliding into its place: its offset from there, shrinking to
/// nothing.
#[derive(Debug, Clone, Copy)]
struct Slide {
    /// The offset when the slide began, in pixels.
    from: f64,
    /// When it began, in the frame clock's microseconds.
    start: i64,
    /// How long it lasts, in microseconds; 0 without animations.
    length: i64,
}

impl Slide {
    /// The offset at frame time `now`, easing out as Chrome's
    /// `EASE_OUT` (1 - (1 - t)²).
    fn offset(self, now: i64) -> f64 {
        if self.is_done(now) {
            return 0.0;
        }
        #[expect(clippy::cast_precision_loss, reason = "a slide lasts a fraction of a second")]
        let progress = ((now - self.start) as f64 / self.length as f64).clamp(0.0, 1.0);
        self.from * (1.0 - progress).powi(2)
    }

    fn is_done(self, now: i64) -> bool {
        now - self.start >= self.length
    }
}

/// The tab that is dragged.
#[derive(Debug)]
struct Dragged {
    tab: gtk::Widget,
    /// Where its left edge would be, unclamped.
    left: f64,
    /// Where it was when the order last changed.
    reordered_at: f64,
    /// Released: it slides into its place like the others.
    settling: bool,
    /// Out of the strip: the others close up where it was.
    detached: bool,
}

/// What runs once a released tab is in place: once, and not when a new
/// drag of the tab calls it off first, even after it was handed to the
/// main loop to run; each clone is the same.
#[derive(Clone)]
pub(super) struct Settled(Rc<RefCell<Option<Then>>>);

/// What runs once, when it was not called off.
type Then = Box<dyn FnOnce()>;

impl std::fmt::Debug for Settled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Settled")
    }
}

impl Settled {
    fn new(then: Then) -> Self {
        Self(Rc::new(RefCell::new(Some(then))))
    }

    /// Runs it, unless it ran or was called off.
    pub(super) fn run(&self) {
        let then = self.0.borrow_mut().take();
        if let Some(then) = then {
            then();
        }
    }

    /// Calls it off.
    pub(super) fn cancel(&self) {
        self.0.borrow_mut().take();
    }
}

/// The live layout of a strip while a tab is dragged over it.
#[derive(Debug)]
pub(super) struct LiveTabs {
    /// The tabs, left to right, in the order shown.
    order: Vec<gtk::Widget>,
    dragged: Dragged,
    /// The tabs still sliding.
    slides: Vec<(gtk::Widget, Slide)>,
    /// The frame clock callback that moves them, while one runs.
    tick: Option<gtk::TickCallbackId>,
    /// What to do once a released tab is in place.
    on_settled: Option<Settled>,
}

/// The left edge of each of `order`'s tabs when packed from 0 with `gap`
/// pixels between them and no room for a `skipped` tab.
fn packed(order: &[gtk::Widget], gap: f64, skipped: Option<&gtk::Widget>, width: Width) -> Vec<f64> {
    let mut x = 0.0;
    order
        .iter()
        .map(|tab| {
            let left = x;
            if skipped != Some(tab) {
                x += width(tab) + gap;
            }
            left
        })
        .collect()
}

impl LiveTabs {
    /// `dragged`, one of `order`, follows the pointer from `left`.
    pub(super) fn dragging(order: Vec<gtk::Widget>, dragged: gtk::Widget, left: f64) -> Self {
        Self {
            order,
            dragged: Dragged {
                tab: dragged,
                left,
                // The first place is never held back.
                reordered_at: f64::NEG_INFINITY,
                settling: false,
                detached: false,
            },
            slides: Vec::new(),
            tick: None,
            on_settled: None,
        }
    }

    /// The dragged tab, while it follows the pointer in the strip.
    pub(super) fn following(&self) -> Option<&gtk::Widget> {
        let dragged = &self.dragged;
        (!dragged.settling && !dragged.detached).then_some(&dragged.tab)
    }

    /// The dragged tab, following, out of the strip or settling.
    pub(super) fn dragged(&self) -> &gtk::Widget {
        &self.dragged.tab
    }

    /// The tabs in the order shown.
    pub(super) fn order(&self) -> &[gtk::Widget] {
        &self.order
    }

    /// Where `tab` is drawn at frame time `now` in a row `row` pixels wide.
    fn shown_left(&self, tab: &gtk::Widget, slots: &[f64], row: f64, now: i64, width: Width) -> f64 {
        if self.following() == Some(tab) {
            let widest = (row - width(tab)).max(0.0);
            return self.dragged.left.clamp(0.0, widest);
        }
        let index = self.order.iter().position(|shown| shown == tab).unwrap_or(0);
        let slide = self
            .slides
            .iter()
            .find(|(sliding, _)| sliding == tab)
            .map_or(0.0, |(_, slide)| slide.offset(now));
        slots.get(index).copied().unwrap_or(0.0) + slide
    }

    fn slots(&self, gap: f64, width: Width) -> Vec<f64> {
        let skipped = self.dragged.detached.then_some(&self.dragged.tab);
        packed(&self.order, gap, skipped, width)
    }

    /// Each tab and its left edge at frame time `now`, for the layout.
    pub(super) fn placements(&self, gap: f64, row: f64, now: i64, width: Width) -> Vec<(gtk::Widget, f64)> {
        let slots = self.slots(gap, width);
        self.order
            .iter()
            .map(|tab| (tab.clone(), self.shown_left(tab, &slots, row, now, width)))
            .collect()
    }

    /// Changes the order or the dragged tab with `change`, and slides every
    /// tab whose place moved from where it is drawn now.
    fn rearrange(
        &mut self,
        (gap, row, now, length): (f64, f64, i64, i64),
        width: Width,
        change: impl FnOnce(&mut Self),
    ) {
        let before = self.slots(gap, width);
        let shown: Vec<(gtk::Widget, f64)> = self
            .order
            .iter()
            .map(|tab| (tab.clone(), self.shown_left(tab, &before, row, now, width)))
            .collect();
        change(self);
        let after = self.slots(gap, width);
        let following = self.following().cloned();
        let mut slides = Vec::new();
        for (index, tab) in self.order.iter().enumerate() {
            if following.as_ref() == Some(tab) {
                continue;
            }
            let from = shown
                .iter()
                .find(|(shown, _)| shown == tab)
                .map_or(0.0, |(_, left)| left - after[index]);
            if from.abs() >= 0.5 && length > 0 {
                slides.push((
                    tab.clone(),
                    Slide {
                        from,
                        start: now,
                        length,
                    },
                ));
            }
        }
        self.slides = slides;
    }

    /// The dragged tab's left edge moves to `left`; the others make room
    /// when its leading edge passes their middle. True when the order
    /// changed.
    pub(super) fn follow(&mut self, left: f64, frame: (f64, f64, i64, i64), width: Width) -> bool {
        let (gap, row, _, _) = frame;
        self.dragged.left = left;
        let reordered_at = self.dragged.reordered_at;
        let was_away = self.dragged.detached;
        if self.dragged.settling {
            // Back in the strip before it slid back: it follows again.
            self.dragged.settling = false;
            if let Some(settled) = self.on_settled.take() {
                settled.cancel();
            }
        }
        if was_away {
            // Back in the strip: the others open up for it again.
            self.rearrange(frame, width, |live| live.dragged.detached = false);
        }
        let dragged = self.dragged.tab.clone();
        let dragged_width = width(&dragged);
        // A tab just added to the row, as one dragged in from another
        // window, has no room in it until the row is laid out again: the
        // row is as wide as all its tabs.
        let row = packed(&self.order, gap, None, width)
            .last()
            .zip(self.order.last())
            .map_or(row, |(last, tab)| row.max(last + width(tab)));
        let left = left.clamp(0.0, (row - dragged_width).max(0.0));
        let mut order = self.order.clone();
        let Some(mut at) = order.iter().position(|tab| *tab == dragged) else {
            return false;
        };
        loop {
            let slots = packed(&order, gap, None, width);
            let middle = |index: usize| slots[index] + width(&order[index]) / 2.0;
            if at + 1 < order.len() && left + dragged_width > middle(at + 1) {
                order.swap(at, at + 1);
                at += 1;
            } else if at > 0 && left < middle(at - 1) {
                order.swap(at, at - 1);
                at -= 1;
            } else {
                break;
            }
        }
        if order == self.order {
            return false;
        }
        let hysteresis = REORDER_HYSTERESIS * dragged_width / STANDARD_WIDTH;
        if (left - reordered_at).abs() <= hysteresis && !was_away {
            return false;
        }
        self.rearrange(frame, width, |live| {
            live.order = order;
            live.dragged.reordered_at = left;
        });
        true
    }

    /// The dragged tab leaves the strip: the others close up where it was.
    pub(super) fn detach(&mut self, frame: (f64, f64, i64, i64), width: Width) {
        self.rearrange(frame, width, |live| live.dragged.detached = true);
    }

    /// The dragged tab is released: every tab, the dragged one too,
    /// slides to its place in `order`; `then` runs once all are there.
    pub(super) fn settle(
        &mut self,
        order: Vec<gtk::Widget>,
        frame: (f64, f64, i64, i64),
        width: Width,
        then: Box<dyn FnOnce()>,
    ) {
        self.rearrange(frame, width, |live| {
            live.order = order;
            live.dragged.settling = true;
            live.dragged.detached = false;
        });
        self.on_settled = Some(Settled::new(then));
    }

    /// Whether every slide is over at frame time `now`.
    pub(super) fn is_still(&self, now: i64) -> bool {
        self.slides.iter().all(|(_, slide)| slide.is_done(now))
    }

    pub(super) fn take_on_settled(&mut self) -> Option<Settled> {
        self.on_settled.take()
    }

    /// What waits for the released tab, which stays here until it ran.
    pub(super) fn on_settled(&self) -> Option<Settled> {
        self.on_settled.clone()
    }

    pub(super) fn set_tick(&mut self, tick: Option<gtk::TickCallbackId>) -> Option<gtk::TickCallbackId> {
        std::mem::replace(&mut self.tick, tick)
    }

    pub(super) fn has_tick(&self) -> bool {
        self.tick.is_some()
    }
}

/// How long slides last for `widget`: [`SLIDE`], or nothing while the
/// desktop turns animations off.
pub(super) fn slide_length(widget: &impl IsA<gtk::Widget>) -> i64 {
    if widget.settings().is_gtk_enable_animations() {
        i64::try_from(SLIDE.as_micros()).unwrap_or(i64::MAX)
    } else {
        0
    }
}

/// The frame time of `widget`'s frame clock, in microseconds, or the
/// monotonic time before it has one.
pub(super) fn frame_time(widget: &impl IsA<gtk::Widget>) -> i64 {
    widget
        .frame_clock()
        .map_or_else(glib::monotonic_time, |clock| clock.frame_time())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tab slides as Chrome's tab strip slides tabs: 200 ms, easing out
    /// as `gfx::Tween::EASE_OUT` (1 - (1 - t)²), so halfway through it has
    /// three quarters of the way behind it.
    ///
    /// parity: TAB-032
    #[test]
    fn a_tab_slides_for_200_ms_easing_out_as_chrome() {
        let length = i64::try_from(SLIDE.as_micros()).expect("a short slide");
        let slide = Slide {
            from: 100.0,
            start: 0,
            length,
        };

        assert_eq!(SLIDE, Duration::from_millis(200));
        assert!((slide.offset(0) - 100.0).abs() < 1e-9);
        assert!((slide.offset(length / 2) - 25.0).abs() < 1e-9);
        assert!(slide.offset(length).abs() < 1e-9);
        assert!(slide.is_done(length));
    }
}
