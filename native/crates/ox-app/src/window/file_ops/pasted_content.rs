// SPDX-License-Identifier: AGPL-3.0-only
//! Text or an image on the clipboard pasted as a new file (CLIP-015).
//!
//! Dolphin asks for a name first ("Filename for clipboard content:"),
//! Nautilus writes the picture at once; as Windows Explorer does with a
//! new item, the name is made without a question and the file can be
//! renamed in place afterwards. The name is the local time of the paste and
//! the extension of what was copied, `2026-02-05 14-30-00.png` or
//! `2026-02-05 14-30-00.txt`; a name that is taken moves the new file to
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

use ox_core::clipboard::PastedContent;
use ox_core::ops::create_pasted_file;

use crate::window::BrowserWindow;

/// What the transfer panel says while a paste writes its file.
const PASTING_A_FILE: &str = crate::i18n::message_id("Pasting a file…");

impl BrowserWindow {
    /// Writes `content`, the clipboard's text or image, into
    /// `destination_folder` as one new file, and selects it once the
    /// folder is listed again.
    pub(crate) async fn paste_clipboard_data(&self, content: PastedContent, destination_folder: String) {
        let name = content.file_name_now();
        let (context, _writing) = match self.admit_write() {
            Ok(admitted) => admitted,
            Err(refusal) => {
                self.show_message(&refusal);
                return;
            }
        };
        let label = ox_core::i18n::gettext_static(PASTING_A_FILE);
        let Some(write) = self.begin_quick_write(label, &context.cancel) else {
            return;
        };
        let created = create_pasted_file(&destination_folder, &name, &content, &context).await;
        self.end_quick_write(write, &context.cancel);
        match created {
            // A cancelled write says nothing, as any cancelled operation.
            Err(error) if error.is_cancelled() => {}
            Ok(created) => {
                self.context().record_operation(created.undo_record());
                self.reload_selecting(vec![created.uri]);
            }
            Err(error) => self.show_message(&error.to_string()),
        }
    }
}
