// SPDX-License-Identifier: AGPL-3.0-only
//! The drives being formatted and the writes running outside the
//! operation panel, which wait for each other in every window (DEV-012):
//! a write going on when the drive is unmounted would go on in the empty
//! mount point or on the new file system.
//!
//! Transfers and other operations count through their window's panel;
//! a new folder, file or link, a rename, a permission change and the
//! steps of an extraction around its panel count with a hold here.

use std::cell::Cell;
use std::rc::Rc;

use gtk::subclass::prelude::*;

use super::AppContext;

/// Counts a drive being formatted, or a write, until it is dropped.
#[derive(Debug)]
pub(crate) struct WriteHold(Rc<Cell<u32>>);

impl WriteHold {
    fn new(count: &Rc<Cell<u32>>) -> Self {
        count.set(count.get() + 1);
        Self(Rc::clone(count))
    }
}

impl Drop for WriteHold {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

impl AppContext {
    /// Holds every window's file writes while a drive is formatted and
    /// mounted again, until the returned hold is dropped.
    pub(crate) fn hold_writes_for_formatting(&self) -> WriteHold {
        WriteHold::new(&self.imp().drives_formatting)
    }

    /// Whether a drive is being formatted, so no window starts writing.
    pub(crate) fn formats_a_drive(&self) -> bool {
        self.imp().drives_formatting.get() > 0
    }

    /// Counts a write outside the operation panel as running, so no
    /// drive is formatted, until the returned hold is dropped.
    pub(crate) fn hold_write(&self) -> WriteHold {
        WriteHold::new(&self.imp().held_writes)
    }

    /// Whether a write outside the operation panel runs in any window.
    pub(crate) fn holds_writes(&self) -> bool {
        self.imp().held_writes.get() > 0
    }
}
