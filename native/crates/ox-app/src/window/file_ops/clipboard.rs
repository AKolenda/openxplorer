// SPDX-License-Identifier: AGPL-3.0-only
//! Cut, Copy and the desktop's clipboard (CLIP-001 to CLIP-011, CLIP-015,
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
//!   never freezes the window. When none of them holds a file list, the
//!   clipboard's picture is read, and then its text, which a paste writes
//!   as one new file (CLIP-015). The file list always comes first: a
//!   window of this app offers the copied paths as text beside the file
//!   formats (CLIP-004), and text must never win, or Ctrl+C then Ctrl+V
//!   would write a .txt instead of copying the files.
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
    decode, ClipboardFiles, ClipboardMode, FileListFormat, PastedContent, IMAGE_MIME_TYPES, KDE_CUT,
    MAX_BYTES, TEXTURE_EXTENSION, TEXT_MIME_TYPE,
};

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

/// The text targets a paste accepts, besides any format of type `text/*`.
///
/// GTK converts whichever of them an owner declares into
/// `text/plain;charset=utf-8`, which is the one a read asks for, so an X11
/// owner that offers only `STRING` is read too. They are checked first
/// because an owner that offers no text at all is never asked for any: a
/// clipboard that a slow or half-gone owner still holds would otherwise
/// take a read it cannot answer, and hold up the next one.
const TEXT_TARGETS: [&str; 4] = [TEXT_MIME_TYPE, "text/plain", "UTF8_STRING", "STRING"];

/// What the desktop's clipboard offers Paste, as the window last read it:
/// enough to enable the command without keeping the payload in memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ClipboardKind {
    /// No files, no text and no image: Paste has nothing to write
    /// (CLIP-016).
    #[default]
    Empty,
    /// A file list, from this window, another window of this app, or
    /// another file manager.
    Files,
    /// Text copied in another application, pasted as one `.txt` file
    /// (CLIP-015).
    Text,
    /// An image copied in another application, pasted as one image file
    /// (CLIP-015).
    Image,
    /// Text or an image larger than [`MAX_BYTES`], which is no data
    /// (CLIP-015); Paste says so, instead of doing nothing.
    TooLarge,
}

impl ClipboardKind {
    /// Whether Paste has something to write.
    pub(crate) fn is_pasteable(self) -> bool {
        matches!(self, Self::Files | Self::Text | Self::Image)
    }
}

/// What the desktop's clipboard holds for a paste (CLIP-005, CLIP-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClipboardContent {
    /// Nothing a paste could write: no files, no text, no image.
    Nothing,
    /// Text or an image over [`MAX_BYTES`], refused as no data.
    TooLarge,
    /// A file list, which paste copies or moves item by item.
    Files(ClipboardFiles),
    /// Text or an image, which paste writes as one new file (CLIP-015).
    Data(PastedContent),
}

impl ClipboardContent {
    /// What Paste makes of it.
    pub(crate) fn kind(&self) -> ClipboardKind {
        match self {
            Self::Nothing => ClipboardKind::Empty,
            Self::TooLarge => ClipboardKind::TooLarge,
            Self::Files(_) => ClipboardKind::Files,
            Self::Data(PastedContent::Text(_)) => ClipboardKind::Text,
            Self::Data(PastedContent::Image { .. }) => ClipboardKind::Image,
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

/// What one format read gave, in the order paste reads the formats.
#[derive(Debug)]
enum Found<Content> {
    /// Something to paste.
    Content(Content),
    /// A payload over [`MAX_BYTES`], which is no data.
    TooLarge,
    /// Nothing in this format.
    Nothing,
}

/// Reads the whole payload of `mime_type` from `clipboard`; `Unavailable`
/// when the owner does not offer it, the conversion fails or the payload is
/// larger than [`MAX_BYTES`].
async fn read_payload(clipboard: &gdk::Clipboard, mime_type: &str) -> Payload {
    if !clipboard.formats().contain_mime_type(mime_type) {
        return Payload::Unavailable;
    }
    read_offered_payload(clipboard, mime_type).await
}

/// Reads `mime_type` from `clipboard` without asking whether it is offered.
///
/// Only text is read this way: GTK converts the text of any text target an
/// owner publishes into `text/plain;charset=utf-8`, so asking for that one
/// format reads a `text/plain` owner too, as `gdk_clipboard_read_text_async`
/// does in Nautilus. A read of anything else is worth asking for only when
/// the owner offers it, which [`read_payload`] checks.
async fn read_offered_payload(clipboard: &gdk::Clipboard, mime_type: &str) -> Payload {
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
/// with its KDE cut marker.
async fn read_file_list(clipboard: &gdk::Clipboard, format: FileListFormat<'_>) -> Found<ClipboardFiles> {
    let payload = match read_payload(clipboard, format.mime_type()).await {
        Payload::Bytes(payload) => payload,
        Payload::TooLarge => return Found::TooLarge,
        Payload::Unavailable => return Found::Nothing,
    };
    let decoded = match format {
        FileListFormat::UriList { .. } => {
            let marker = match read_payload(clipboard, KDE_CUT).await {
                Payload::Bytes(marker) => Some(marker),
                // The exact cut marker is `1`: a marker that is not there,
                // or one past the limit, makes the list a copy.
                Payload::TooLarge | Payload::Unavailable => None,
            };
            let with_marker = FileListFormat::UriList {
                kde_cut_marker: marker.as_deref(),
            };
            decode(with_marker, &payload)
        }
        format => decode(format, &payload),
    };
    decoded.map_or(Found::Nothing, Found::Content)
}

/// The file list on `clipboard`, in the first format that holds one.
async fn read_files(clipboard: &gdk::Clipboard) -> Found<ClipboardFiles> {
    let mut too_large = false;
    for format in FILE_LIST_FORMATS {
        match read_file_list(clipboard, format).await {
            Found::Content(files) => return Found::Content(files),
            Found::TooLarge => too_large = true,
            Found::Nothing => {}
        }
    }
    if too_large {
        Found::TooLarge
    } else {
        Found::Nothing
    }
}

/// The image on `clipboard`, as the file a paste writes (CLIP-015).
///
/// The owner's own encoding is written as it is (PNG, JPEG, WebP, GIF or
/// BMP), under the extension of that format, so a pasted screenshot is the
/// screenshot and nothing re-encodes it. An owner that offers no picture
/// this app keeps is asked for a texture instead, which GTK encodes as
/// PNG; that is also what a picture of a format outside
/// [`IMAGE_MIME_TYPES`] becomes.
async fn read_image(clipboard: &gdk::Clipboard) -> Found<PastedContent> {
    let mut too_large = false;
    for (mime_type, extension) in IMAGE_MIME_TYPES {
        match read_payload(clipboard, mime_type).await {
            Payload::Bytes(bytes) if bytes.is_empty() => {}
            Payload::Bytes(bytes) => return Found::Content(PastedContent::Image { extension, bytes }),
            Payload::TooLarge => too_large = true,
            Payload::Unavailable => {}
        }
    }
    if too_large {
        return Found::TooLarge;
    }
    if !offers_a_picture(clipboard) {
        return Found::Nothing;
    }
    let Ok(Some(texture)) = clipboard.read_texture_future().await else {
        return Found::Nothing;
    };
    let bytes = texture.save_to_png_bytes().to_vec();
    if bytes.is_empty() {
        return Found::Nothing;
    }
    Found::Content(PastedContent::Image {
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

/// True when `clipboard` offers text in a form [`TEXT_MIME_TYPE`] can be
/// converted from, so an owner without any text is not asked for one.
fn offers_text(clipboard: &gdk::Clipboard) -> bool {
    let formats = clipboard.formats();
    TEXT_TARGETS
        .iter()
        .any(|target| formats.contain_mime_type(target))
        || formats
            .mime_types()
            .iter()
            .any(|mime_type| mime_type.starts_with("text/"))
}

/// The text on `clipboard`, as the UTF-8 file a paste writes (CLIP-015).
///
/// The bytes are written as GTK converted them, line endings included: a
/// text copied with CRLF stays CRLF, so the file holds what was copied and
/// no editor shows a line the user never typed.
async fn read_text(clipboard: &gdk::Clipboard) -> Found<PastedContent> {
    if !offers_text(clipboard) {
        return Found::Nothing;
    }
    match read_offered_payload(clipboard, TEXT_MIME_TYPE).await {
        Payload::Bytes(bytes) if bytes.is_empty() => Found::Nothing,
        Payload::Bytes(bytes) => Found::Content(PastedContent::Text(bytes)),
        Payload::TooLarge => Found::TooLarge,
        Payload::Unavailable => Found::Nothing,
    }
}

/// What `clipboard` holds that a paste could write, in the order a paste
/// reads it: the file list first, then an image, then text (CLIP-005,
/// CLIP-015).
async fn read_clipboard_content(clipboard: &gdk::Clipboard) -> ClipboardContent {
    let mut too_large = false;
    match read_files(clipboard).await {
        Found::Content(files) => return ClipboardContent::Files(files),
        Found::TooLarge => too_large = true,
        Found::Nothing => {}
    }
    match read_image(clipboard).await {
        Found::Content(data) => return ClipboardContent::Data(data),
        Found::TooLarge => too_large = true,
        Found::Nothing => {}
    }
    match read_text(clipboard).await {
        Found::Content(data) => return ClipboardContent::Data(data),
        Found::TooLarge => too_large = true,
        Found::Nothing => {}
    }
    if too_large {
        ClipboardContent::TooLarge
    } else {
        ClipboardContent::Nothing
    }
}

/// The URIs of the items a cut put on the clipboard, which the views dim;
/// none for a copy, and none for text or an image
/// (`state.clipboard?.mode==='move'` in `renderRows`).
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
        let files = match ClipboardFiles::new(mode, uris) {
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
        if publish(&self.clipboard(), &files).is_err() {
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
        } else if publish(&clipboard, &current).is_ok() {
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
    /// changing a clipboard with a large picture or a long text never
    /// holds up the window.
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
