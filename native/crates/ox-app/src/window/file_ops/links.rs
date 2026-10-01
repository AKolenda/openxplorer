// SPDX-License-Identifier: AGPL-3.0-only
//! Create links, as a drop with Ctrl+Shift or the drop menu's "Create
//! links here" asks (DND-019), and New ▸ Link to file or folder (OPS-004).
//!
//! New in the native app, from the Dolphin baseline. A drop's links are
//! made by ox-core's [`create_links`] as the window's one operation; New ▸
//! Link asks for what the link points to and its name, and a refusal, such
//! as a path where nothing is, stays in the dialog. The new links are
//! selected afterwards, as other operations' items are, and Undo moves
//! them to the Trash (OPS-029).

use glib::clone;
use gtk::glib;
use gtk::prelude::*;
use ox_core::ops::{
    create_link, create_links, starting_label, summarize_links, LinkRequest, NewLink, OperationContext,
};
use ox_core::transfer::TransferMode;

use super::running::FinishedOperation;
use super::FileCommand;
use crate::window::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// New ▸ Link's title.
const NEW_LINK_TITLE: &str = "New link";

/// What New ▸ Link makes.
const NEW_LINK_MESSAGE: &str =
    "A link opens the file or folder it points to. Leave the name empty to use the name of that item.";

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
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// New ▸ Link to file or folder: asks for the path and the name,
    /// makes the link, then selects it.
    pub(crate) async fn create_link(&self) {
        if !self.allows(FileCommand::New) {
            return;
        }
        let Some(folder) = self.current_uri() else {
            return;
        };
        let dialog = Dialog::new(self, NEW_LINK_TITLE, NEW_LINK_MESSAGE);
        let target = dialog.add_text_field("Link to", "");
        target.set_placeholder_text(Some("For example ~/Documents"));
        let name = dialog.add_text_field("Name", "");
        dialog.add_cancel_button();
        dialog.add_button("Create", ButtonStyle::Accent);
        dialog.open();
        let protection = self.context().write_protection();
        loop {
            if dialog.next_response().await.is_none() {
                return;
            }
            let request = NewLink {
                folder: folder.clone(),
                name: name.text().to_string(),
                target: target.text().to_string(),
            };
            let context = OperationContext::new(protection.clone());
            dialog.set_busy(Some(&context.cancel));
            let outcome = create_link(&request, &context).await;
            dialog.set_busy(None);
            match outcome {
                Ok(link) => {
                    dialog.finish();
                    self.context().record_operation(link.undo_record());
                    self.reload_selecting(vec![link.uri]);
                    return;
                }
                Err(error) => dialog.show_error(&error.to_string()),
            }
        }
    }
}
