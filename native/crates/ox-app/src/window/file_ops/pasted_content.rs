// SPDX-License-Identifier: AGPL-3.0-only
//! A picture on the clipboard pasted as a new file (CLIP-015).
//!
//! Nautilus writes a pasted picture at once, and Dolphin asks for a name
//! first ("Filename for clipboard content:"); as Windows Explorer does
//! with a new item, the name is made without a question and the file can
//! be renamed in place afterwards. The name is the local time of the paste
//! and the extension of the format that was copied,
//! `2026-02-05 14-30-00.png`; a name that is taken moves the new file to
//! the next number, `2026-02-05 14-30-00 (2).png`, never over the item it
//! found.
//!
//! The file is written through ox-core's staged write, the one New ▸ Text
//! document uses ([`ox_core::ops::create_pasted_file`], OPS-048), so it
//! stages privately, publishes without overwriting, checks the free space
//! first and honours the write protection of the location (XFER-001,
//! XFER-007, XFER-028). One paste writes one file, and the write is the
//! window's running operation: other file commands wait, Cancel stops it,
//! and its panel shows only when the write takes a moment (OPS-024). Undo
//! moves the file to the Recycle Bin, as Undo of any new file (OPS-029).

use ox_core::clipboard::PastedImage;
use ox_core::ops::create_pasted_file;

use crate::search::changed_folders;
use crate::window::BrowserWindow;

/// What the transfer panel says while a paste writes its picture.
const PASTING_A_PICTURE: &str = crate::i18n::message_id("Pasting a picture…");

impl BrowserWindow {
    /// Writes `image`, the clipboard's picture, into `destination_folder`
    /// as one new file, and selects it once the folder is listed again.
    pub(crate) async fn paste_clipboard_image(&self, image: PastedImage, destination_folder: String) {
        let name = image.file_name_now();
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.show_message(&refusal);
                return;
            }
        };
        let label = ox_core::i18n::gettext_static(PASTING_A_PICTURE);
        let Some(write) = self.begin_quick_write(label, &context.cancel) else {
            return;
        };
        let created = create_pasted_file(&destination_folder, &name, &image, &context).await;
        self.end_quick_write(write, &context.cancel);
        match created {
            // A cancelled write says nothing, as any cancelled operation.
            Err(error) if error.is_cancelled() => {}
            Ok(created) => {
                self.context().record_operation(created.undo_record());
                // Paste into writes into a folder of another tab, and a slow
                // share can be navigated away from before the file lands:
                // reloading the tab that is active now would replace its
                // selection with an item it does not hold, so only the tab
                // still showing the destination selects it. Searches and the
                // other tabs on that folder learn of the file either way,
                // as after New ▸ Text document.
                let changed = changed_folders([destination_folder.as_str()], [created.uri.as_str()]);
                self.context().search_cache().folders_written(changed);
                if self.current_uri().as_deref() == Some(destination_folder.as_str()) {
                    self.reload_selecting(vec![created.uri]);
                } else {
                    self.mark_changed(&destination_folder);
                }
            }
            Err(error) => self.show_message(&error.to_string()),
        }
    }
}
