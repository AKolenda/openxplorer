// SPDX-License-Identifier: AGPL-3.0-only
//! Copy and drag items out of a ZIP opened like a folder (ARC-026), as
//! Windows Explorer lets them leave a "Compressed (zipped) Folder".
//!
//! Other folders and applications need real files, so the selected items
//! are extracted, with every rule of the extractor and the transfer panel,
//! into a new folder of copies (`ox_core::archive::copies_root`, in the
//! user's cache), and those files are what the clipboard or the drag hands
//! over:
//!
//! - Copy (Ctrl+C) extracts first, then puts the copies on the clipboard;
//!   Paste anywhere then copies them as any files.
//! - A drag offers `text/uri-list` lazily ([`ZipDragContent`]): nothing is
//!   extracted until a folder or an application accepts the drop and asks
//!   for the files.
//!
//! Cut is refused: the ZIP is read-only. Copies older than a day are
//! removed at the next copy.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::archive::{
    copied_member, copies_root, copy_folder_name, prepare_copies_root, remove_old_copies, ExtractionRequest,
    COPY_LIFETIME,
};
use ox_core::clipboard::{ClipboardFiles, ClipboardMode};
use ox_core::location::{file_uri, ArchiveLocation};

use super::zip_folder::archive_location;
use super::BrowserWindow;

/// Cut inside a ZIP.
pub(super) const NO_CUT: &str = "Items in a ZIP are read-only. Use Copy, then Paste where you want them.";

/// Extracts the copies of a drag when the drop asks for them.
type Materialise = Rc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>>>>>;

impl BrowserWindow {
    /// Extracts the items at `locations` (all inside one ZIP) into a new
    /// folder of copies and returns the copies' file URIs, in order.
    ///
    /// # Errors
    ///
    /// The message to show: another operation runs, the folder of copies
    /// cannot be made, or the extraction refused or failed.
    pub(super) async fn copy_out_of_zip(
        &self,
        locations: Vec<ArchiveLocation>,
    ) -> Result<Vec<String>, String> {
        let first = locations
            .first()
            .ok_or_else(|| "Select the items to copy.".to_owned())?;
        if locations
            .iter()
            .any(|location| location.archive_uri != first.archive_uri)
        {
            return Err("Copy items from one ZIP at a time.".to_owned());
        }
        if !self.may_start_archive_operation() {
            return Err("Wait for the running file operation to finish.".to_owned());
        }
        let archive_uri = first.archive_uri.clone();
        let root = copies_root();
        let prepared_root = root.clone();
        let folder_name = gio::spawn_blocking(move || {
            prepare_copies_root(&prepared_root)?;
            remove_old_copies(&prepared_root, COPY_LIFETIME);
            copy_folder_name()
        })
        .await
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .map_err(|error| format!("Could not prepare the copies: {error}"))?;
        let members: Vec<String> = locations.iter().map(|location| location.member.clone()).collect();
        let request = ExtractionRequest {
            archive_uri,
            destination_uri: file_uri(&root),
            folder_name: folder_name.clone(),
        };
        let extracted = self
            .run_extraction(request, Some(&members))
            .await
            .map_err(|error| crate::archive_view::extraction_failure_text(&error))?;
        let folder: PathBuf = gio::File::for_uri(&extracted.uri)
            .path()
            .unwrap_or_else(|| root.join(folder_name));
        Ok(members
            .iter()
            .map(|member| file_uri(&copied_member(&folder, member)))
            .collect())
    }

    /// Copy inside a ZIP: extracts the selection, then puts the copies on
    /// the clipboard. Cut is refused.
    pub(super) fn copy_zip_selection(&self, mode: ClipboardMode, locations: Vec<ArchiveLocation>) {
        if mode == ClipboardMode::Cut {
            self.show_message(NO_CUT);
            return;
        }
        let generation = self.clipboard_generation();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let copies = match window.copy_out_of_zip(locations).await {
                    Ok(copies) => copies,
                    Err(message) => {
                        window.show_message(&message);
                        return;
                    }
                };
                // Something else was copied meanwhile: it stays on the
                // clipboard.
                if window.clipboard_generation() != generation {
                    return;
                }
                match ClipboardFiles::new(ClipboardMode::Copy, &copies) {
                    Ok(files) => window.put_files_on_clipboard(files),
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        ));
    }

    /// What dragging the selected items inside a ZIP offers: their copies,
    /// extracted only when the drop asks for them.
    pub(super) fn zip_drag_content(&self, locations: Vec<ArchiveLocation>) -> gdk::ContentProvider {
        let window = self.downgrade();
        let materialise: Materialise = Rc::new(move || {
            let window = window.clone();
            let locations = locations.clone();
            Box::pin(async move {
                let Some(window) = window.upgrade() else {
                    return Err("The window closed.".to_owned());
                };
                window.copy_out_of_zip(locations).await
            })
        });
        ZipDragContent::new(materialise).upcast()
    }
}

/// Whether `uri` is one of the copies taken out of a ZIP, which last a day
/// and must not be pinned.
pub(crate) fn is_zip_copy(uri: &str) -> bool {
    gio::File::for_uri(uri)
        .path()
        .is_some_and(|path| path.starts_with(copies_root()))
}

/// The locations inside a ZIP of `uris`, if every one is inside one.
pub(super) fn zip_items(uris: &[String]) -> Option<Vec<ArchiveLocation>> {
    uris.iter().map(|uri| archive_location(uri)).collect()
}

mod imp {
    use std::cell::RefCell;
    use std::future::Future;
    use std::pin::Pin;

    use gtk::gio::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, gio, glib};

    use super::Materialise;

    /// The mime type of a file list.
    pub(super) const URI_LIST: &str = "text/uri-list";

    /// Where the copies of a drag stand.
    #[derive(Default)]
    pub(super) enum Copies {
        /// Not asked for yet.
        #[default]
        Idle,
        /// Being extracted; these requests wait for the result.
        Running(Vec<async_channel::Sender<Result<Vec<String>, String>>>),
        /// Extracted, or refused, for every later request.
        Done(Result<Vec<String>, String>),
    }

    /// Private state of [`super::ZipDragContent`].
    #[derive(Default)]
    pub(crate) struct ZipDragContent {
        /// Extracts the copies; set by [`super::ZipDragContent::new`].
        pub(super) materialise: RefCell<Option<Materialise>>,
        /// One extraction for every request of the drop.
        pub(super) copies: std::rc::Rc<RefCell<Copies>>,
    }

    impl std::fmt::Debug for ZipDragContent {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.debug_struct("ZipDragContent").finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ZipDragContent {
        const NAME: &'static str = "OxZipDragContent";
        type Type = super::ZipDragContent;
        type ParentType = gdk::ContentProvider;
    }

    impl ObjectImpl for ZipDragContent {}

    impl ContentProviderImpl for ZipDragContent {
        fn formats(&self) -> gdk::ContentFormats {
            gdk::ContentFormatsBuilder::new().add_mime_type(URI_LIST).build()
        }

        fn write_mime_type_future(
            &self,
            mime_type: &str,
            stream: &gio::OutputStream,
            io_priority: glib::Priority,
        ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'static>> {
            let is_uri_list = mime_type == URI_LIST;
            let materialise = self.materialise.borrow().clone();
            let copies = std::rc::Rc::clone(&self.copies);
            let stream = stream.clone();
            Box::pin(async move {
                let unsupported =
                    || glib::Error::new(gio::IOErrorEnum::NotSupported, "Only a file list is offered.");
                if !is_uri_list {
                    return Err(unsupported());
                }
                let materialise = materialise.ok_or_else(unsupported)?;
                let uris = super::shared_copies(&copies, &materialise)
                    .await
                    .map_err(|message| glib::Error::new(gio::IOErrorEnum::Failed, &message))?;
                let text = super::uri_list(&uris);
                stream
                    .write_all_future(text.into_bytes(), io_priority)
                    .await
                    .map_err(|(_, error)| error)?;
                stream.close_future(io_priority).await
            })
        }
    }
}

glib::wrapper! {
    /// A drag's content for items inside a ZIP: a file list of their
    /// copies, extracted when the drop first asks for it.
    pub(crate) struct ZipDragContent(ObjectSubclass<imp::ZipDragContent>)
        @extends gdk::ContentProvider;
}

impl ZipDragContent {
    /// Content whose file list `materialise` makes.
    fn new(materialise: Materialise) -> Self {
        let content: Self = glib::Object::new();
        content.imp().materialise.replace(Some(materialise));
        content
    }
}

/// The copies of a drag: the first request extracts them, requests that
/// come meanwhile wait for that extraction, later ones reuse its result,
/// so a drop that asks several times never extracts twice.
async fn shared_copies(
    copies: &Rc<std::cell::RefCell<imp::Copies>>,
    materialise: &Materialise,
) -> Result<Vec<String>, String> {
    let waiting = {
        let mut state = copies.borrow_mut();
        match &mut *state {
            imp::Copies::Done(result) => return result.clone(),
            imp::Copies::Running(waiters) => {
                let (sender, receiver) = async_channel::bounded(1);
                waiters.push(sender);
                Some(receiver)
            }
            imp::Copies::Idle => {
                *state = imp::Copies::Running(Vec::new());
                None
            }
        }
    };
    if let Some(receiver) = waiting {
        return receiver
            .recv()
            .await
            .unwrap_or_else(|_| Err("The copies could not be made.".to_owned()));
    }
    let result = materialise().await;
    let waiters = match copies.replace(imp::Copies::Done(result.clone())) {
        imp::Copies::Running(waiters) => waiters,
        imp::Copies::Idle | imp::Copies::Done(_) => Vec::new(),
    };
    for waiter in waiters {
        let _ = waiter.try_send(result.clone());
    }
    result
}

/// `uris` as `text/uri-list` (RFC 2483): one per line, CRLF.
fn uri_list(uris: &[String]) -> String {
    let mut text = String::new();
    for uri in uris {
        text.push_str(uri);
        text.push_str("\r\n");
    }
    text
}
