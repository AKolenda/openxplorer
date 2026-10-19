// SPDX-License-Identifier: AGPL-3.0-only
//! The drives being formatted, which every window's file writes wait for
//! (DEV-012): a transfer between two files when the drive is unmounted
//! would go on in the empty mount point or on the new file system.

use std::cell::Cell;
use std::rc::Rc;

use gtk::subclass::prelude::*;

use super::AppContext;

/// Keeps every window from starting to write files until it is dropped.
#[derive(Debug)]
pub(crate) struct FormattingHold(Rc<Cell<u32>>);

impl Drop for FormattingHold {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

impl AppContext {
    /// Holds every window's file writes while a drive is formatted and
    /// mounted again, until the returned hold is dropped.
    pub(crate) fn hold_writes_for_formatting(&self) -> FormattingHold {
        let formatting = Rc::clone(&self.imp().drives_formatting);
        formatting.set(formatting.get() + 1);
        FormattingHold(formatting)
    }

    /// Whether a drive is being formatted, so no window starts writing.
    pub(crate) fn formats_a_drive(&self) -> bool {
        self.imp().drives_formatting.get() > 0
    }
}
