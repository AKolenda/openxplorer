// SPDX-License-Identifier: AGPL-3.0-only
//! Cut, Copy and the desktop's file clipboard (CLIP-001 to CLIP-011,
//! CLIP-016).
//!
//! Ports `copySelection`, `refreshClipboard` and `clipboardConsume` of
//! `v2.0.0:desktop/ui/app.js` with the GTK half of `v2.0.0:desktop/file_clipboard.py`.
//! ox-core encodes and decodes the formats ([`ox_core::clipboard`]); this
//! module claims and reads the display's clipboard:
//!
//! - Copy and Cut publish the four formats at once as one clipboard owner,
//!   so Nautilus, Dolphin and other windows of this app can paste them.
//! - Reading tries the app's own format, then GNOME's, then the URI
//!   list with the KDE cut marker, each asynchronously, so a slow owner
//!   never freezes the window. Plain text, an image, an empty or failed
//!   conversion, an oversized payload or an owner change during the read
//!   all mean "no files" (fail closed, CLIP-005, CLIP-016).
//! - The window reads the clipboard again whenever it changes and when the
//!   window becomes active (CLIP-009), which enables Paste and dims the
//!   items a cut put on the clipboard (CLIP-002).
//! - After a move-paste the moved items leave the clipboard, but only
//!   while it still holds the same cut (CLIP-008).
//! - The content is local and every format is storable, so GTK hands it
//!   to a clipboard manager when the application quits (`gtk_main_sync`
//!   stores the display's clipboard at shutdown), as
//!   `gtk_clipboard_set_can_store` did in Python (CLIP-010).

use std::collections::HashSet;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{decode, ClipboardFiles, ClipboardMode, FileListFormat, KDE_CUT, MAX_BYTES};

use crate::window::BrowserWindow;

/// How much of a payload one read takes.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// The formats a paste reads, in priority order; the URI list's KDE
/// marker is read beside it.
const FILE_LIST_FORMATS: [FileListFormat<'static>; 3] = [
    FileListFormat::Custom,
    FileListFormat::Gnome,
    FileListFormat::UriList { kde_cut_marker: None },
];

/// The toast after a copy or cut of `count` items.
fn published_message(mode: ClipboardMode, count: usize) -> String {
    let verb = match mode {
        ClipboardMode::Copy => "copied",
        ClipboardMode::Cut => "cut",
    };
    format!("{count} item(s) {verb} — ready to paste in another window.")
}

/// Reads the whole payload of `mime_type` from `clipboard`; `None` when
/// the owner does not offer it, the conversion fails or the payload is
/// larger than [`MAX_BYTES`].
async fn read_payload(clipboard: &gdk::Clipboard, mime_type: &str) -> Option<Vec<u8>> {
    if !clipboard.formats().contain_mime_type(mime_type) {
        return None;
    }
    let (stream, _) = clipboard
        .read_future(&[mime_type], glib::Priority::DEFAULT)
        .await
        .ok()?;
    let mut payload = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(READ_CHUNK_BYTES, glib::Priority::DEFAULT)
            .await
            .ok()?;
        if chunk.is_empty() {
            return Some(payload);
        }
        payload.extend_from_slice(&chunk);
        if payload.len() > MAX_BYTES {
            return None;
        }
    }
}

/// Reads `format` from `clipboard` and decodes it; a URI list is read
/// with its KDE cut marker.
async fn read_file_list(clipboard: &gdk::Clipboard, format: FileListFormat<'_>) -> Option<ClipboardFiles> {
    let payload = read_payload(clipboard, format.mime_type()).await?;
    if !matches!(format, FileListFormat::UriList { .. }) {
        return decode(format, &payload);
    }
    let marker = read_payload(clipboard, KDE_CUT).await;
    let with_marker = FileListFormat::UriList {
        kde_cut_marker: marker.as_deref(),
    };
    decode(with_marker, &payload)
}

/// The file list on `clipboard`, in the first format that holds one.
async fn read_files(clipboard: &gdk::Clipboard) -> Option<ClipboardFiles> {
    for format in FILE_LIST_FORMATS {
        if let Some(files) = read_file_list(clipboard, format).await {
            return Some(files);
        }
    }
    None
}

/// The URIs of the items a cut put on the clipboard, which the views dim;
/// none for a copy (`state.clipboard?.mode==='move'` in `renderRows`).
fn cut_uris(files: Option<&ClipboardFiles>) -> HashSet<String> {
    match files {
        Some(files) if files.mode() == ClipboardMode::Cut => files.uris().iter().cloned().collect(),
        _ => HashSet::new(),
    }
}

/// Makes `files` the clipboard's content, in every format at once.
fn publish(clipboard: &gdk::Clipboard, files: &ClipboardFiles) -> Result<(), glib::BoolError> {
    let providers: Vec<gdk::ContentProvider> = files
        .encode()
        .into_iter()
        .map(|payload| {
            gdk::ContentProvider::for_bytes(payload.mime_type, &glib::Bytes::from_owned(payload.bytes))
        })
        .collect();
    let content = gdk::ContentProvider::new_union(&providers);
    clipboard.set_content(Some(&content))
}

impl BrowserWindow {
    /// Copy (Ctrl+C) or Cut (Ctrl+X): puts the selection on the desktop's
    /// clipboard (`copySelection`). The keys run this even where the
    /// commands are disabled, so a share root or a previous version says
    /// why it cannot be copied or cut.
    pub(crate) fn copy_selection(&self, mode: ClipboardMode) {
        let items = self.folder_pane().model().selected_items();
        let command_facts = self.command_facts();
        // The Recycle Bin's items can only be restored or deleted, which is
        // why its Cut and Copy commands are off.
        if items.is_empty() || command_facts.folder.is_recycle_bin {
            return;
        }
        let facts = command_facts.selection;
        if facts.has_inoperable {
            self.show_message("Open the share first, then select its files or folders.");
            return;
        }
        if mode == ClipboardMode::Cut && facts.has_read_only {
            self.show_message("Previous versions are read-only. Use Restore a copy.");
            return;
        }
        let uris: Vec<String> = items.iter().map(|item| item.entry().uri.clone()).collect();
        let files = match ClipboardFiles::new(mode, &uris) {
            Ok(files) => files,
            Err(error) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        if publish(&self.clipboard(), &files).is_err() {
            self.show_message("The desktop clipboard could not be claimed.");
            return;
        }
        let message = published_message(mode, files.uris().len());
        self.remember_clipboard(Some(files));
        self.show_message(&message);
    }

    /// Reads the desktop's clipboard again and returns its file list
    /// (`refreshClipboard`).
    pub(crate) async fn refresh_file_clipboard(&self) -> Option<ClipboardFiles> {
        let clipboard = self.clipboard();
        let generation = self.clipboard_generation();
        let files = read_files(&clipboard).await;
        // Safety rule (one owner per read, CLIP-006, CLIP-016): formats
        // read before and after an owner change could mix two clipboards,
        // such as one owner's URI list with another's cut marker, so such a
        // read holds no files. It is not remembered either: the change
        // started a read of its own, which a stale answer must not undo.
        if self.clipboard_generation() != generation {
            return None;
        }
        self.remember_clipboard(files.clone());
        files
    }

    /// Removes the items a move-paste of `pasted` moved from the clipboard,
    /// when it still holds that cut; clears it once nothing is left
    /// (`clipboardConsume`).
    pub(crate) async fn consume_cut(&self, pasted: &ClipboardFiles, moved: &[String]) {
        let Some(mut current) = self.refresh_file_clipboard().await else {
            return;
        };
        // Safety rule (cut identity, CLIP-008): only the same cut, by its
        // token, loses items; ClipboardFiles::consume refuses any other.
        if !current.consume(pasted.token(), moved) {
            return;
        }
        let clipboard = self.clipboard();
        if current.uris().is_empty() {
            // Clearing fails only when another owner took the clipboard
            // meanwhile, and then there is nothing of this cut to clear.
            clipboard.set_content(None::<&gdk::ContentProvider>).ok();
            self.remember_clipboard(None);
        } else if publish(&clipboard, &current).is_ok() {
            self.remember_clipboard(Some(current));
        }
    }

    /// Keeps `files` as the clipboard's file list, dims the items it cut
    /// and updates Paste.
    fn remember_clipboard(&self, files: Option<ClipboardFiles>) {
        let cut_uris = cut_uris(files.as_ref());
        self.folder_pane().owners().show_cut_items(cut_uris);
        self.imp().file_operations.borrow_mut().clipboard = files;
        self.update_file_commands();
    }

    /// How many times the clipboard has changed while the window watched.
    fn clipboard_generation(&self) -> u64 {
        self.imp().file_operations.borrow().clipboard_generation
    }

    /// Reads the clipboard now, whenever it changes, and whenever the
    /// window becomes active (CLIP-009). Returns the handler on the
    /// display's clipboard, which outlives the window.
    pub(crate) fn follow_file_clipboard(&self) -> glib::SignalHandlerId {
        self.connect_is_active_notify(|window| {
            if window.is_active() {
                window.schedule_clipboard_read();
            }
        });
        self.schedule_clipboard_read();
        self.clipboard().connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.imp().file_operations.borrow_mut().clipboard_generation += 1;
                window.schedule_clipboard_read();
            }
        ))
    }

    /// Reads the clipboard in the background.
    fn schedule_clipboard_read(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window.refresh_file_clipboard().await;
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: CLIP-001, CLIP-002
    #[test]
    fn copy_and_cut_say_how_many_items_are_ready_to_paste() {
        assert_eq!(
            published_message(ClipboardMode::Copy, 2),
            "2 item(s) copied — ready to paste in another window."
        );
        assert_eq!(
            published_message(ClipboardMode::Cut, 1),
            "1 item(s) cut — ready to paste in another window."
        );
    }
}
