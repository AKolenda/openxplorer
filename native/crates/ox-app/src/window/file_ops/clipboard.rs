// SPDX-License-Identifier: AGPL-3.0-only
//! Cut, Copy and the desktop's clipboard (CLIP-001 to CLIP-011, CLIP-015,
//! CLIP-016).
//!
//! Ports `copySelection`, `refreshClipboard` and `clipboardConsume` of
//! `v2.0.0:desktop/ui/app.js` with the GTK half of `v2.0.0:desktop/file_clipboard.py`.
//! ox-core encodes and decodes the formats ([`ox_core::clipboard`]); this
//! module claims and reads the display's clipboard:
//!
//! - Copy and Cut publish the five formats at once as one clipboard owner,
//!   so Nautilus, Dolphin, terminals and other windows of this app can paste
//!   them.
//! - Reading tries the app's own format, then GNOME's, then the URI
//!   list with the KDE cut marker, each asynchronously, so a slow owner
//!   never freezes the window. When none of them holds a file list, the
//!   clipboard's picture is read, which a paste writes as one new file
//!   (CLIP-015). Plain text is neither a file list nor a file: as in
//!   Windows Explorer, it leaves Paste disabled.
//! - An empty or failed conversion, an oversized payload or an owner
//!   change during the read all mean "no data" (fail closed, CLIP-005,
//!   CLIP-016).
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
use ox_core::clipboard::{
    decode, ClipboardFiles, ClipboardMode, FileListFormat, PastedImage, IMAGE_MIME_TYPES, KDE_CUT, MAX_BYTES,
    TEXTURE_EXTENSION,
};
use ox_core::location::LocationContext;

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

/// What the desktop's clipboard offers Paste, as the window last read it:
/// enough to enable the command without keeping the payload in memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ClipboardKind {
    /// No files and no picture: Paste has nothing to write, as Windows
    /// Explorer has nothing to paste from text (CLIP-016).
    #[default]
    Empty,
    /// A file list, from this window, another window of this app, or
    /// another file manager.
    Files,
    /// A picture copied in another application, pasted as one image file
    /// (CLIP-015).
    Image,
    /// A picture larger than [`MAX_BYTES`], which is no data (CLIP-015);
    /// Paste says so, instead of doing nothing.
    TooLarge,
}

impl ClipboardKind {
    /// Whether Paste has something to write.
    pub(crate) fn is_pasteable(self) -> bool {
        matches!(self, Self::Files | Self::Image)
    }
}

/// What the desktop's clipboard holds for a paste (CLIP-005, CLIP-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClipboardContent {
    /// Nothing a paste could write: no files and no picture.
    Nothing,
    /// A picture over [`MAX_BYTES`], refused as no data.
    TooLarge,
    /// A file list, which paste copies or moves item by item.
    Files(ClipboardFiles),
    /// A picture, which paste writes as one new file (CLIP-015).
    Image(PastedImage),
}

impl ClipboardContent {
    /// What Paste makes of it.
    pub(crate) fn kind(&self) -> ClipboardKind {
        match self {
            Self::Nothing => ClipboardKind::Empty,
            Self::TooLarge => ClipboardKind::TooLarge,
            Self::Files(_) => ClipboardKind::Files,
            Self::Image(_) => ClipboardKind::Image,
        }
    }

    /// The file list, when the clipboard holds one.
    fn files(&self) -> Option<&ClipboardFiles> {
        match self {
            Self::Files(files) => Some(files),
            _ => None,
        }
    }
}

/// What one read of a clipboard format gave.
#[derive(Debug)]
enum Payload {
    /// The owner's bytes, within [`MAX_BYTES`].
    Bytes(Vec<u8>),
    /// The owner offers the format, but its payload is over [`MAX_BYTES`].
    TooLarge,
    /// The owner does not offer it or the conversion failed: no data, and
    /// never a crash (CLIP-016).
    Unavailable,
}

impl Payload {
    /// The bytes, when the read gave any within the limit.
    fn bytes(self) -> Option<Vec<u8>> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::TooLarge | Self::Unavailable => None,
        }
    }
}

/// Reads the whole payload of `mime_type` from `clipboard`; `Unavailable`
/// when the owner does not offer it or the conversion fails, and
/// `TooLarge` when the payload is larger than [`MAX_BYTES`].
async fn read_payload(clipboard: &gdk::Clipboard, mime_type: &str) -> Payload {
    if !clipboard.formats().contain_mime_type(mime_type) {
        return Payload::Unavailable;
    }
    let Ok((stream, _)) = clipboard.read_future(&[mime_type], glib::Priority::DEFAULT).await else {
        return Payload::Unavailable;
    };
    let mut payload = Vec::new();
    loop {
        let Ok(chunk) = stream
            .read_bytes_future(READ_CHUNK_BYTES, glib::Priority::DEFAULT)
            .await
        else {
            return Payload::Unavailable;
        };
        if chunk.is_empty() {
            return Payload::Bytes(payload);
        }
        payload.extend_from_slice(&chunk);
        if payload.len() > MAX_BYTES {
            return Payload::TooLarge;
        }
    }
}

/// Reads `format` from `clipboard` and decodes it; a URI list is read
/// with its KDE cut marker. A payload over the limit is no file list.
async fn read_file_list(clipboard: &gdk::Clipboard, format: FileListFormat<'_>) -> Option<ClipboardFiles> {
    let payload = read_payload(clipboard, format.mime_type()).await.bytes()?;
    if !matches!(format, FileListFormat::UriList { .. }) {
        return decode(format, &payload);
    }
    let marker = read_payload(clipboard, KDE_CUT).await.bytes();
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

/// The picture on `clipboard`, as the file a paste writes (CLIP-015).
///
/// The owner's own encoding is written as it is (PNG, JPEG, WebP, GIF or
/// BMP), under the extension of that format, so a pasted screenshot is the
/// screenshot and nothing re-encodes it. An owner that offers another
/// `image/*` format is asked for a texture instead, which GTK encodes as
/// PNG.
async fn read_image(clipboard: &gdk::Clipboard) -> ClipboardContent {
    let mut too_large = false;
    for (mime_type, extension) in IMAGE_MIME_TYPES {
        match read_payload(clipboard, mime_type).await {
            Payload::Bytes(bytes) if bytes.is_empty() => {}
            Payload::Bytes(bytes) => return ClipboardContent::Image(PastedImage { extension, bytes }),
            Payload::TooLarge => too_large = true,
            Payload::Unavailable => {}
        }
    }
    if too_large {
        return ClipboardContent::TooLarge;
    }
    if !offers_a_picture(clipboard) {
        return ClipboardContent::Nothing;
    }
    let Ok(Some(texture)) = clipboard.read_texture_future().await else {
        return ClipboardContent::Nothing;
    };
    let bytes = texture.save_to_png_bytes().to_vec();
    if bytes.is_empty() {
        return ClipboardContent::Nothing;
    }
    // GTK's decoding is not bounded by the size of what the owner holds, so
    // the limit is enforced on what the paste would write: a picture of this
    // branch is as much "too large" as one whose own bytes were over it.
    if bytes.len() > MAX_BYTES {
        return ClipboardContent::TooLarge;
    }
    ClipboardContent::Image(PastedImage {
        extension: TEXTURE_EXTENSION,
        bytes,
    })
}

/// True when `clipboard` offers a picture at all, in any format: an owner
/// that offers none is never asked for a texture, which would be a read it
/// cannot answer.
fn offers_a_picture(clipboard: &gdk::Clipboard) -> bool {
    clipboard
        .formats()
        .mime_types()
        .iter()
        .any(|mime_type| mime_type.starts_with("image/"))
}

/// What `clipboard` holds that a paste could write, in the order a paste
/// reads it: the file list first, then a picture (CLIP-005, CLIP-015).
/// Text is neither: as in Windows Explorer, it leaves Paste disabled.
async fn read_clipboard_content(clipboard: &gdk::Clipboard) -> ClipboardContent {
    match read_files(clipboard).await {
        Some(files) => ClipboardContent::Files(files),
        None => read_image(clipboard).await,
    }
}

/// The URIs of the items a cut put on the clipboard, which the views dim;
/// none for a copy or a picture (`state.clipboard?.mode==='move'` in
/// `renderRows`).
fn cut_uris(files: Option<&ClipboardFiles>) -> HashSet<String> {
    match files {
        Some(files) if files.mode() == ClipboardMode::Cut => files.uris().iter().cloned().collect(),
        _ => HashSet::new(),
    }
}

/// Makes `files` the clipboard's content, in every format at once.
/// `locations` names the devices and the home folder of the paths the text
/// format shows.
fn publish(
    clipboard: &gdk::Clipboard,
    files: &ClipboardFiles,
    locations: &LocationContext,
) -> Result<(), glib::BoolError> {
    let providers: Vec<gdk::ContentProvider> = files
        .encode(locations)
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
            self.show_message(ox_core::i18n::gettext_static(
                "Open the share first, then select its files or folders.",
            ));
            return;
        }
        let uris: Vec<String> = items.iter().map(|item| item.entry().uri.clone()).collect();
        // Inside a ZIP opened like a folder, copies are extracted first
        // (ARC-026).
        if let Some(inside) = crate::window::zip_copies::zip_items(&uris) {
            self.copy_zip_selection(mode, inside);
            return;
        }
        if mode == ClipboardMode::Cut && facts.has_read_only {
            self.show_message(ox_core::i18n::gettext_static(
                "Previous versions are read-only. Use Restore a copy.",
            ));
            return;
        }
        self.copy_items(mode, &uris);
    }

    /// Copy or Cut of the folder tree's folder at `uri` (SIDE-028).
    pub(crate) fn copy_folder_at(&self, mode: ClipboardMode, uri: &str) {
        self.copy_items(mode, &[uri.to_owned()]);
    }

    /// Puts `uris` on the desktop's clipboard and says so.
    fn copy_items(&self, mode: ClipboardMode, uris: &[String]) {
        let validated = ClipboardFiles::new(mode, uris, &self.imp().locations.borrow());
        let files = match validated {
            Ok(files) => files,
            Err(error) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        self.put_files_on_clipboard(files);
    }

    /// Makes `files` the desktop's clipboard, without a message, as
    /// Windows Explorer.
    pub(crate) fn put_files_on_clipboard(&self, files: ClipboardFiles) {
        self.put_files_on_clipboard_noting(files, None);
    }

    /// Makes `files` the desktop's clipboard and shows `note`, such as
    /// what was left out, when there is one.
    pub(crate) fn put_files_on_clipboard_noting(&self, files: ClipboardFiles, note: Option<&str>) {
        let published = publish(&self.clipboard(), &files, &self.imp().locations.borrow());
        if published.is_err() {
            self.show_message(ox_core::i18n::gettext_static(
                "The desktop clipboard could not be claimed.",
            ));
            return;
        }
        self.remember_clipboard(&ClipboardContent::Files(files));
        if let Some(note) = note {
            self.show_message(note);
        }
    }

    /// Reads the desktop's clipboard again and returns what it holds
    /// (`refreshClipboard`).
    pub(crate) async fn refresh_file_clipboard(&self) -> ClipboardContent {
        let clipboard = self.clipboard();
        let generation = self.clipboard_generation();
        let content = read_clipboard_content(&clipboard).await;
        // Safety rule (one owner per read, CLIP-006, CLIP-016): formats
        // read before and after an owner change could mix two clipboards,
        // such as one owner's URI list with another's cut marker, so such a
        // read holds no data. It is not remembered either: the change
        // started a read of its own, which a stale answer must not undo.
        if self.clipboard_generation() != generation {
            return ClipboardContent::Nothing;
        }
        self.remember_clipboard(&content);
        content
    }

    /// Removes the items a move-paste of `pasted` moved from the clipboard,
    /// when it still holds that cut; clears it once nothing is left
    /// (`clipboardConsume`).
    pub(crate) async fn consume_cut(&self, pasted: &ClipboardFiles, moved: &[String]) {
        let ClipboardContent::Files(mut current) = self.refresh_file_clipboard().await else {
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
            self.remember_clipboard(&ClipboardContent::Nothing);
        } else if publish(&clipboard, &current, &self.imp().locations.borrow()).is_ok() {
            self.remember_clipboard(&ClipboardContent::Files(current));
        }
    }

    /// Keeps `content` as what the clipboard holds, dims the items it cut
    /// and updates Paste.
    fn remember_clipboard(&self, content: &ClipboardContent) {
        let cut_uris = cut_uris(content.files());
        for pane in self.folder_panes() {
            pane.owners().show_cut_items(cut_uris.clone());
        }
        self.imp().file_operations.borrow_mut().clipboard = content.kind();
        self.update_file_commands();
    }

    /// How many times the clipboard has changed while the window watched.
    pub(crate) fn clipboard_generation(&self) -> u64 {
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

    /// Reads the clipboard in the background, so opening, activating or
    /// changing a clipboard with a large picture never holds up the
    /// window.
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
