// SPDX-License-Identifier: AGPL-3.0-only
//! Opt-in timestamps of the steps of a start, for measuring how quickly
//! the app opens.
//!
//! With `OX_STARTUP_TRACE=1` in the environment the app writes one line
//! per step to standard error, `ox-startup <step> <microseconds>`, where
//! the time is `CLOCK_MONOTONIC` (the monotonic clock of `GLib`), so a launcher
//! that notes the same clock before it starts the process can tell how
//! long each step took from the launch. Without the variable nothing is
//! written and each step costs one cached lookup. The startup budget test
//! (`window::tests::startup`) and the measurements in the pass notes read
//! these lines.
//!
//! The steps of the first window are `main`, `startup`, `state`, `built`,
//! `presented`, `painted` (its first frame on screen), `listed` (its first
//! tab's listing finished), `list-painted` (the first frame after that)
//! and `idle` (the main loop had nothing else to do after that). Later
//! windows of the same process write `window-built`, `window-painted` and
//! `window-listed`. Reading the digest of the whole executable, which a
//! process does at most once, writes `executable-digest`.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use gtk::glib;
use gtk::prelude::*;

/// Set to 1, the app writes the steps of its start to standard error.
pub(crate) const TRACE_VARIABLE: &str = "OX_STARTUP_TRACE";

/// Whether the steps are written; read once.
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var(TRACE_VARIABLE).is_ok_and(|value| value == "1"))
}

/// Writes that `step` happened now.
pub(crate) fn mark(step: &str) {
    if enabled() {
        eprintln!("ox-startup {step} {}", glib::monotonic_time());
    }
}

thread_local! {
    /// Whether the first window of the process was already traced; later
    /// windows write their own steps.
    static FIRST_WINDOW_TRACED: Cell<bool> = const { Cell::new(false) };
    /// The traced windows whose first listing has not finished yet.
    static AWAITING_LISTING: RefCell<Vec<(glib::WeakRef<gtk::Widget>, WindowSteps)>> =
        const { RefCell::new(Vec::new()) };
}

/// The steps a window writes: the first window's names, or a later
/// window's.
#[derive(Debug, Clone, Copy)]
struct WindowSteps {
    built: &'static str,
    painted: &'static str,
    listed: &'static str,
}

const FIRST_WINDOW: WindowSteps = WindowSteps {
    built: "built",
    painted: "painted",
    listed: "listed",
};

const LATER_WINDOW: WindowSteps = WindowSteps {
    built: "window-built",
    painted: "window-painted",
    listed: "window-listed",
};

/// Starts tracing `window`: writes that it was built, and its first frame
/// once it is painted. Its first listing is reported with
/// [`window_listed`].
pub(crate) fn trace_window(window: &impl IsA<gtk::Widget>) {
    if !enabled() {
        return;
    }
    let steps = if FIRST_WINDOW_TRACED.replace(true) {
        LATER_WINDOW
    } else {
        FIRST_WINDOW
    };
    mark(steps.built);
    let widget = window.upcast_ref::<gtk::Widget>();
    AWAITING_LISTING.with_borrow_mut(|windows| windows.push((widget.downgrade(), steps)));
    after_next_paint(widget, steps.painted, None);
}

/// Writes that `window`'s first listing finished, then the first frame
/// after it and the main loop's first idle moment after that. Only the
/// first call for a traced window writes anything.
pub(crate) fn window_listed(window: &impl IsA<gtk::Widget>) {
    if !enabled() {
        return;
    }
    let widget = window.upcast_ref::<gtk::Widget>();
    let found = AWAITING_LISTING.with_borrow_mut(|windows| {
        windows.retain(|(weak, _)| weak.upgrade().is_some());
        let index = windows
            .iter()
            .position(|(weak, _)| weak.upgrade().as_ref() == Some(widget))?;
        Some(windows.remove(index).1)
    });
    let Some(steps) = found else {
        return;
    };
    mark(steps.listed);
    let first = steps.listed == FIRST_WINDOW.listed;
    after_next_paint(
        widget,
        if first { "list-painted" } else { "" },
        first.then_some("idle"),
    );
}

/// Writes `step` after the next frame `widget` paints, then `then` once
/// the main loop is idle; an empty `step` writes only `then`.
fn after_next_paint(widget: &gtk::Widget, step: &'static str, then: Option<&'static str>) {
    let pending = std::rc::Rc::new(Cell::new(Some((step, then))));
    let connect = move |widget: &gtk::Widget| {
        let Some(clock) = widget.frame_clock() else {
            return;
        };
        let handler: std::rc::Rc<Cell<Option<glib::SignalHandlerId>>> = std::rc::Rc::default();
        let pending = std::rc::Rc::clone(&pending);
        let stored = std::rc::Rc::clone(&handler);
        let id = clock.connect_after_paint(move |clock| {
            let Some((step, then)) = pending.take() else {
                return;
            };
            if !step.is_empty() {
                mark(step);
            }
            if let Some(then) = then {
                glib::idle_add_local_full(glib::Priority::LOW, move || {
                    mark(then);
                    glib::ControlFlow::Break
                });
            }
            if let Some(id) = stored.take() {
                clock.disconnect(id);
            }
        });
        handler.set(Some(id));
        widget.queue_draw();
    };
    if widget.is_realized() {
        connect(widget);
    } else {
        let once: std::rc::Rc<Cell<Option<glib::SignalHandlerId>>> = std::rc::Rc::default();
        let stored = std::rc::Rc::clone(&once);
        let id = widget.connect_realize(move |widget| {
            if let Some(id) = stored.take() {
                widget.disconnect(id);
            }
            connect(widget);
        });
        once.set(Some(id));
    }
}
