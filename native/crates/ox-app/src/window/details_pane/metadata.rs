// SPDX-License-Identifier: AGPL-3.0-only
//! The rows a picture's, video's or audio file's own metadata adds to the
//! details pane (PROP-009): Date taken, Dimensions and the camera of a
//! photo, the length, bit rates and artists of a recording.
//!
//! They are read by Properties' metadata reader
//! ([`crate::media_details`], PROP-034), in its separate, time-limited
//! process, and worded as its Details tab words them
//! ([`crate::properties::pane_media_rows`]). Reading never holds up the
//! pane: its file rows show at once, and the reader starts only once the
//! selection has rested for [`SETTLE_DELAY`], so arrowing through a
//! folder starts no reader per item. A new selection stops the reader of
//! the one before and drops anything it still sends; a file already read
//! is not read again when the pane is redrawn for it.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::content::MetadataRequest;
use super::DetailsPane;
use crate::media_details::{read_in_reader, ReaderProcess};

/// How long the selection rests before its file is read.
pub(super) const SETTLE_DELAY: Duration = Duration::from_millis(250);

/// A row the reader found: its key, its shown name and its value.
pub(super) type MetadataRow = (&'static str, String, String);

/// What the pane knows of the selected file's own details.
#[derive(Debug, Default)]
pub(super) struct MetadataState {
    /// The file being read or read, if any.
    request: RefCell<Option<MetadataRequest>>,
    /// Its rows, once read.
    rows: RefCell<Vec<MetadataRow>>,
    /// The wait for the selection to rest.
    settle: RefCell<Option<glib::SourceId>>,
    /// Counts the requests, so rows read for an earlier one are dropped.
    generation: Cell<u64>,
    /// The reader of the latest request, stopped by the next.
    process: ReaderProcess,
}

impl MetadataState {
    /// The rows read for the file shown.
    pub(super) fn rows(&self) -> Vec<MetadataRow> {
        self.rows.borrow().clone()
    }

    /// Stops waiting and reading for the file before.
    fn cancel(&self) {
        if let Some(settle) = self.settle.take() {
            settle.remove();
        }
        self.process.stop();
        self.generation.set(self.generation.get().wrapping_add(1));
    }
}

impl DetailsPane {
    /// Reads the details of `request`'s file once the selection rests;
    /// the same file as before keeps its rows. A hidden pane reads
    /// nothing, and catches up when it is shown.
    pub(super) fn follow_metadata(&self, request: Option<&MetadataRequest>) {
        let state = &self.imp().metadata;
        let request = request.filter(|_| self.is_visible());
        if state.request.borrow().as_ref() == request {
            return;
        }
        state.cancel();
        state.request.replace(request.cloned());
        state.rows.borrow_mut().clear();
        let Some(request) = request.cloned() else {
            return;
        };
        let generation = state.generation.get();
        let pane = self.downgrade();
        let settle = glib::timeout_add_local_once(SETTLE_DELAY, move || {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            pane.imp().metadata.settle.replace(None);
            pane.read_metadata(request, generation);
        });
        state.settle.replace(Some(settle));
    }

    /// Reads `request`'s file in the reader process and shows its rows,
    /// unless another file was selected meanwhile.
    fn read_metadata(&self, request: MetadataRequest, generation: u64) {
        let process = self.imp().metadata.process.clone();
        let pane = self.downgrade();
        glib::spawn_future_local(async move {
            let media = read_in_reader(&request.uri, request.kind, request.size, &process).await;
            let Some(pane) = pane.upgrade() else {
                return;
            };
            let state = &pane.imp().metadata;
            if state.generation.get() != generation {
                return;
            }
            let Some(media) = media else {
                return;
            };
            state
                .rows
                .replace(crate::properties::pane_media_rows(request.kind, &media));
            pane.redraw_properties();
        });
    }

    /// Whether the reader is still to answer for the file shown, for
    /// tests.
    #[cfg(test)]
    pub(in crate::window) fn reading_metadata(&self) -> bool {
        let state = &self.imp().metadata;
        state.request.borrow().is_some() && state.rows.borrow().is_empty()
    }
}
