// SPDX-License-Identifier: AGPL-3.0-only
//! Keeps a list at its top when its items change while it is there.
//!
//! GTK keeps the row at the top edge where it was when a list changes, not
//! the top of the list. Rows that come before that row, such as the files
//! another type shows in an Open or Save dialog (`*.svg`, then All files),
//! then push the list down, and the first files are out of sight. Windows
//! Explorer and Dolphin show such a list from its top again.
//!
//! So when the items change while the list is at its top, the list goes
//! back there each time GTK lays it out again, until two frames have drawn
//! the change. GTK sets the scroll position during that layout, when it
//! does not listen to the position (it blocks its own `value-changed`
//! handler in `gtk_list_base_set_adjustment_values`), so the position is
//! announced again once GTK has finished: GTK then keeps the top of the
//! list, not the row that was there, also at its later layouts. A scroll
//! position the window restores (Back, a tab switch), an item it scrolls
//! to and a list the user scrolled away from the top are left alone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

/// What one list's [`TopKeeper`] remembers.
#[derive(Default)]
struct State {
    /// Set from a change at the top until two frames have drawn it.
    keeping: Cell<bool>,
    /// The frames drawn since the last change at the top.
    frames: Cell<u32>,
    /// The frame clock counting those frames, and its handler.
    watch: RefCell<Option<(gdk::FrameClock, glib::SignalHandlerId)>>,
    /// Set while GTK waits to be told the list is at its top again.
    reanchor_pending: Cell<bool>,
}

/// Keeps one list at its top when its items change there.
#[derive(Clone)]
pub(crate) struct TopKeeper {
    state: Rc<State>,
    list: glib::WeakRef<gtk::Widget>,
    /// The adjustment the list scrolls along now: a grid that switches
    /// between scrolling down and sideways has two.
    adjustment: Rc<dyn Fn() -> Option<gtk::Adjustment>>,
}

impl std::fmt::Debug for TopKeeper {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TopKeeper")
            .field("keeping", &self.state.keeping.get())
            .finish_non_exhaustive()
    }
}

impl TopKeeper {
    /// Keeps `list`, which shows `model`, at its top when `model` changes
    /// there. `adjustments` are every adjustment it may scroll along, and
    /// `current` names the one it scrolls along now.
    pub(crate) fn follow(
        list: &impl IsA<gtk::Widget>,
        model: &impl IsA<gtk::gio::ListModel>,
        adjustments: &[gtk::Adjustment],
        current: impl Fn() -> Option<gtk::Adjustment> + 'static,
    ) -> Self {
        let keeper = Self {
            state: Rc::default(),
            list: list.as_ref().downgrade(),
            adjustment: Rc::new(current),
        };
        let changed = keeper.clone();
        model.connect_items_changed(move |_, _, _, _| {
            // The change is not laid out yet, so the adjustment still says
            // where the list was.
            if changed
                .current()
                .is_some_and(|adjustment| adjustment.value() < 0.5)
            {
                changed.keep_until_drawn();
            }
        });
        for adjustment in adjustments {
            let laid_out = keeper.clone();
            let back_to_the_top = move |adjustment: &gtk::Adjustment| laid_out.back_to_the_top(adjustment);
            adjustment.connect_changed(back_to_the_top.clone());
            adjustment.connect_value_changed(back_to_the_top);
        }
        keeper
    }

    /// Lets the list go from its top: the window restores a scroll
    /// position or scrolls to an item, which wins.
    pub(crate) fn let_go(&self) {
        self.state.keeping.set(false);
    }

    /// The adjustment the list scrolls along now.
    fn current(&self) -> Option<gtk::Adjustment> {
        (self.adjustment)()
    }

    /// Puts the list back at its top after GTK moved it, while it is kept
    /// there and `adjustment` is the one it scrolls along.
    fn back_to_the_top(&self, adjustment: &gtk::Adjustment) {
        if !self.state.keeping.get() || adjustment.value() <= 0.0 {
            return;
        }
        if self.current().as_ref() != Some(adjustment) {
            return;
        }
        adjustment.set_value(0.0);
        self.reanchor(adjustment);
    }

    /// Tells GTK again, once it has finished laying the list out, that the
    /// list is at its top, so GTK keeps it there.
    fn reanchor(&self, adjustment: &gtk::Adjustment) {
        if self.state.reanchor_pending.replace(true) {
            return;
        }
        let state = Rc::downgrade(&self.state);
        let adjustment = adjustment.clone();
        glib::idle_add_local_full(glib::Priority::HIGH, move || {
            if let Some(state) = state.upgrade() {
                state.reanchor_pending.set(false);
                if adjustment.value() < 0.5 {
                    adjustment.emit_by_name::<()>("value-changed", &[]);
                }
            }
            glib::ControlFlow::Break
        });
    }

    /// Keeps the list at its top through the layouts of a change, until
    /// two frames have drawn it; another change starts the count again. A
    /// list that is not shown yet waits until it is.
    fn keep_until_drawn(&self) {
        self.state.keeping.set(true);
        self.state.frames.set(0);
        if self.state.watch.borrow().is_some() {
            return;
        }
        let Some(list) = self.list.upgrade() else {
            return;
        };
        if let Some(clock) = list.frame_clock() {
            self.count_drawn_frames(&clock);
            return;
        }
        let handler: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
        let first = Rc::clone(&handler);
        let keeper = self.clone();
        let realized = list.connect_realize(move |list| {
            if let Some(id) = first.take() {
                list.disconnect(id);
            }
            if let Some(clock) = list.frame_clock().filter(|_| keeper.state.keeping.get()) {
                keeper.count_drawn_frames(&clock);
            }
        });
        handler.set(Some(realized));
    }

    /// Lets the list go from its top once `clock` has drawn two frames
    /// since the last change: GTK lays a change out in the first, and lays
    /// it out again where it was put back in the second.
    fn count_drawn_frames(&self, clock: &gdk::FrameClock) {
        let state = Rc::downgrade(&self.state);
        let painted = clock.connect_after_paint(move |clock| {
            let Some(state) = state.upgrade() else {
                return;
            };
            let frames = state.frames.get() + 1;
            state.frames.set(frames);
            if frames < 2 && state.keeping.get() {
                clock.request_phase(gdk::FrameClockPhase::PAINT);
                return;
            }
            state.keeping.set(false);
            if let Some((clock, id)) = state.watch.take() {
                clock.disconnect(id);
            }
        });
        self.state.watch.replace(Some((clock.clone(), painted)));
        clock.request_phase(gdk::FrameClockPhase::PAINT);
    }
}
