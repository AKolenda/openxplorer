// SPDX-License-Identifier: AGPL-3.0-only
//! The thin line that runs over the top of the folder pane while a folder
//! is listed.
//!
//! Ports `#loading-line` in `desktop/ui/index.html` and `.loading-line` in
//! `desktop/ui/style.css`: a 2-pixel line laid over the pane, so showing it
//! never moves the items, with a bar sliding across it
//! (`resources/skin/folder-views.css`). It appears only when a listing
//! takes longer than [`APPEARANCE_DELAY`], so folders that list at once
//! never flash it (ui-spec.md M06).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

/// How long a listing runs before the line shows: Fluent's `durationFast`.
const APPEARANCE_DELAY: Duration = Duration::from_millis(150);

/// The loading line and the timer that will show it.
#[derive(Debug)]
pub(super) struct LoadingLine {
    /// The line, to lay over the top of the folder pane.
    pub widget: gtk::Box,
    pending: Rc<RefCell<Option<glib::SourceId>>>,
}

impl LoadingLine {
    /// A hidden line.
    pub fn new() -> Self {
        let widget = gtk::Box::builder()
            .valign(gtk::Align::Start)
            .hexpand(true)
            .can_target(false)
            .visible(false)
            .css_classes(["loading-line"])
            .build();
        widget.update_property(&[gtk::accessible::Property::Label("Loading")]);
        Self {
            widget,
            pending: Rc::new(RefCell::new(None)),
        }
    }

    /// Shows the line [`APPEARANCE_DELAY`] after a listing starts, or hides
    /// it at once when the listing is over.
    pub fn set_loading(&self, loading: bool) {
        if !loading {
            self.cancel_pending();
            self.widget.set_visible(false);
            return;
        }
        let already_on_its_way = self.widget.is_visible() || self.pending.borrow().is_some();
        if already_on_its_way {
            return;
        }
        let line = self.widget.downgrade();
        let pending = Rc::clone(&self.pending);
        let timer = glib::timeout_add_local_once(APPEARANCE_DELAY, move || {
            pending.take();
            if let Some(line) = line.upgrade() {
                line.set_visible(true);
            }
        });
        self.pending.replace(Some(timer));
    }

    fn cancel_pending(&self) {
        if let Some(timer) = self.pending.take() {
            timer.remove();
        }
    }
}

impl Drop for LoadingLine {
    fn drop(&mut self) {
        self.cancel_pending();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::test_support::harness::{wait_for, wait_until};

    /// parity: VIEW-047
    #[gtk::test]
    fn the_line_shows_once_a_listing_has_run_for_the_delay() {
        let line = LoadingLine::new();
        let started = Instant::now();
        line.set_loading(true);
        assert!(
            !line.widget.is_visible(),
            "a listing that just started shows no line"
        );
        wait_until("the loading line", || line.widget.is_visible());
        assert!(started.elapsed() >= APPEARANCE_DELAY);
        line.set_loading(false);
        assert!(!line.widget.is_visible(), "the line goes when the listing ends");
    }

    #[gtk::test]
    fn a_listing_that_ends_at_once_never_shows_the_line() {
        let line = LoadingLine::new();
        line.set_loading(true);
        line.set_loading(false);
        wait_for(APPEARANCE_DELAY * 2);
        assert!(!line.widget.is_visible());
    }
}
