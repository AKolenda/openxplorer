// SPDX-License-Identifier: AGPL-3.0-only
//! Create links, as a drop with Ctrl+Shift or the drop menu's "Create
//! links here" asks (DND-019).
//!
//! New in the native app, from the Dolphin baseline. The links are made
//! by ox-core's [`create_links`] as the window's one operation, and the
//! new links are selected afterwards, as other operations' items are.

use glib::clone;
use gtk::glib;
use ox_core::ops::{create_links, starting_label, summarize_links, LinkRequest};
use ox_core::transfer::TransferMode;

use super::running::FinishedOperation;
use crate::window::BrowserWindow;

impl BrowserWindow {
    /// Creates the links `request` asks for once the drop handler has
    /// returned, so the drag has finished first.
    pub(in crate::window) fn spawn_links(&self, request: LinkRequest) {
        glib::spawn_future_local(clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window.run_links(&request).await;
            }
        ));
    }

    /// Runs `request` as the window's one operation and reports it; does
    /// nothing while another operation runs.
    async fn run_links(&self, request: &LinkRequest) {
        let Some(context) = self.begin_operation(starting_label(TransferMode::Copy)) else {
            return;
        };
        let outcome = create_links(request, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_links(&outcome.result),
            undo: None,
            created: outcome.created,
        });
        self.conclude_operation(finished).await;
    }
}
