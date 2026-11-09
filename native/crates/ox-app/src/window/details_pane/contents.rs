// SPDX-License-Identifier: AGPL-3.0-only
//! The Preview pane's file contents (VIEW-070), drawn in the pane's own
//! place as Windows 11 File Explorer's Preview pane draws them: the
//! beginning of a text file as read-only, selectable text in a fixed-width
//! font that wraps, a PDF's pages one under the other, a picture as large
//! as the pane allows and never stretched, or a message in the middle of
//! the pane. What it shows is [`super::preview_page`]'s choice.
//!
//! Nothing is read on the main thread. A new selection waits
//! [`SETTLE`] before anything is read, so arrowing through a folder
//! reads only the file it stops on; it cancels the read of the one
//! before, and anything that arrives for an earlier selection is dropped.
//! A text file is read up to [`TEXT_LIMIT`]; a picture is decoded within
//! the app's limits for pictures ([`picture_guard`]); a PDF's pages are
//! drawn by `pdftoppm` in the thumbnailers' sandbox and limits a few at a
//! time, at most [`PDF_PAGE_LIMIT`] and [`PDF_PIXEL_BUDGET`]. With network
//! previews off, a file on a network file system is not read.

use std::fs;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use crate::picture_guard::{self, Limits};

use super::preview_page::{decode_text, PreviewPage, NO_PREVIEW, TEXT_LIMIT};
use super::DetailsPane;

/// How long a selection must stay before its file is read.
const SETTLE: Duration = Duration::from_millis(90);

/// The most pages of a PDF drawn.
const PDF_PAGE_LIMIT: u32 = 30;

/// The most pixels of PDF pages kept: 24 megapixels, about 96 MB, which
/// is about 40 pages at 2x and every one of [`PDF_PAGE_LIMIT`] at 1x.
const PDF_PIXEL_BUDGET: u64 = 24 * 1024 * 1024;

/// How many PDF pages each run of `pdftoppm` draws: the first show soon,
/// and a long document goes on in the background.
const PDF_BATCH: u32 = 4;

/// The room around the contents, in pixels.
const MARGIN: i32 = 12;

/// The room between a text preview's edges and its text, in pixels, as
/// Windows 11's text preview leaves it.
const TEXT_PADDING: i32 = 6;

/// The pages of the Preview pane's stack.
const MESSAGE: &str = "message";
const TEXT: &str = "text";
const PICTURE: &str = "picture";
const PDF: &str = "pdf";
/// The pages of [`DetailsPane`]'s own stack.
const DETAILS: &str = "details";
const PREVIEW: &str = "preview";

/// The widgets of the Preview pane's contents.
#[derive(Debug)]
pub(super) struct Contents {
    stack: gtk::Stack,
    message: gtk::Label,
    text: gtk::TextView,
    text_scroller: gtk::ScrolledWindow,
    /// Holds the picture shown, made for each picture so that no empty
    /// picture stays in the pane.
    picture_room: gtk::ScrolledWindow,
    pdf_pages: gtk::Box,
    pdf_scroller: gtk::ScrolledWindow,
}

impl Contents {
    /// The contents' widgets, as a page of `pages` named [`PREVIEW`].
    fn new(pages: &gtk::Stack) -> Self {
        let stack = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::None)
            .css_classes(["preview-contents"])
            .build();
        let message = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .justify(gtk::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(28)
            .css_classes(["preview-message"])
            .build();
        stack.add_named(&message, Some(MESSAGE));

        let text = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(TEXT_PADDING)
            .right_margin(TEXT_PADDING)
            .top_margin(TEXT_PADDING)
            .bottom_margin(TEXT_PADDING)
            .css_classes(["preview-text"])
            .build();
        let text_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&text)
            .vexpand(true)
            .build();
        stack.add_named(&text_scroller, Some(TEXT));

        // In a scroller that never scrolls, so the picture's own size
        // never widens the pane: it takes the room the pane has.
        let picture_room = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::External)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(&picture_room, Some(PICTURE));

        let pdf_pages = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(MARGIN)
            .margin_start(MARGIN)
            .margin_end(MARGIN)
            .margin_top(MARGIN)
            .margin_bottom(MARGIN)
            .css_classes(["preview-pages"])
            .build();
        let pdf_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&pdf_pages)
            .vexpand(true)
            .build();
        stack.add_named(&pdf_scroller, Some(PDF));

        let label = ox_core::i18n::gettext("Preview");
        for widget in [text.upcast_ref::<gtk::Widget>(), pdf_pages.upcast_ref()] {
            widget.update_property(&[gtk::accessible::Property::Label(&label)]);
        }
        pages.add_named(&stack, Some(PREVIEW));
        Self {
            stack,
            message,
            text,
            text_scroller,
            picture_room,
            pdf_pages,
            pdf_scroller,
        }
    }

    /// Shows `words`, translated, in the middle of the pane.
    fn show_message(&self, words: &str) {
        self.message.set_text(&ox_core::i18n::gettext(words));
        self.stack.set_visible_child_name(MESSAGE);
    }

    /// Empties every page, so nothing of the item before shows while the
    /// next one is read.
    fn clear(&self) {
        self.message.set_text("");
        self.stack.set_visible_child_name(MESSAGE);
        self.text.buffer().set_text("");
        self.picture_room.set_child(gtk::Widget::NONE);
        while let Some(page) = self.pdf_pages.first_child() {
            self.pdf_pages.remove(&page);
        }
    }
}

impl DetailsPane {
    /// Builds the Preview pane's contents beside the Details column.
    pub(super) fn build_contents(&self) {
        let imp = self.imp();
        let contents = Contents::new(&imp.pages);
        imp.contents.set(contents).expect("the contents are built once");
    }

    fn contents(&self) -> &Contents {
        self.imp().contents.get().expect("built in constructed")
    }

    /// Shows `page` in the Preview pane, or the Details column for `None`
    /// and for [`PreviewPage::Media`], whose player the column holds. The
    /// same page as before stays as it is; another one cancels whatever
    /// is being read for the one before. Nothing is read while the pane
    /// is hidden; it is read as soon as the pane is shown again.
    pub(super) fn show_page(&self, page: Option<&PreviewPage>) {
        let imp = self.imp();
        imp.wanted_page.replace(page.cloned());
        let page = page.filter(|_| self.is_visible());
        let uses_contents = page.is_some_and(|page| *page != PreviewPage::Media);
        imp.pages
            .set_visible_child_name(if uses_contents { PREVIEW } else { DETAILS });
        if imp.shown_page.borrow().as_ref() == page {
            return;
        }
        imp.shown_page.replace(page.cloned());
        self.stop_reading();
        let generation = imp.page_generation.get().wrapping_add(1);
        imp.page_generation.set(generation);
        let contents = self.contents();
        contents.clear();
        let Some(page) = page.filter(|_| uses_contents) else {
            return;
        };
        if let PreviewPage::Message(words) = page {
            contents.show_message(words);
            return;
        }
        let cancellation = gio::Cancellable::new();
        imp.page_cancellation.replace(Some(cancellation.clone()));
        let pane = self.downgrade();
        let page = page.clone();
        let timer = glib::timeout_add_local_once(SETTLE, move || {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            pane.imp().page_timer.replace(None);
            pane.read_page(generation, &page, cancellation);
        });
        imp.page_timer.replace(Some(timer));
    }

    /// Stops the wait and the read for the page shown.
    pub(super) fn stop_reading(&self) {
        let imp = self.imp();
        if let Some(timer) = imp.page_timer.take() {
            timer.remove();
        }
        if let Some(cancellation) = imp.page_cancellation.take() {
            cancellation.cancel();
        }
    }

    /// Stops the wait and the read for the page shown and drops what
    /// arrives for it, keeping the page it was asked for
    /// ([`super::imp::DetailsPane::wanted_page`]) to read again later.
    pub(super) fn forget_page(&self) {
        let imp = self.imp();
        self.stop_reading();
        imp.page_generation.set(imp.page_generation.get().wrapping_add(1));
        imp.shown_page.replace(None);
    }

    /// Whether `generation` is still the page shown.
    fn is_current(&self, generation: u64) -> bool {
        self.imp().page_generation.get() == generation
    }

    /// Reads `page`, the selection of `generation`, off the main thread
    /// and shows it if the selection is still the same.
    fn read_page(&self, generation: u64, page: &PreviewPage, cancellation: gio::Cancellable) {
        let file = match page {
            PreviewPage::Text(file) | PreviewPage::Picture(file) | PreviewPage::Pdf(file) => file,
            PreviewPage::Message(_) | PreviewPage::Media => return,
        };
        let local_only = file.local_only;
        let uri = file.uri.clone();
        let pane = self.downgrade();
        let scale = self.scale_factor().max(1);
        let (width, height) = self.contents_size();
        let page = page.clone();
        glib::spawn_future_local(async move {
            // The file's path: its own, or for an SMB file the path of a
            // kernel mount or GVfs's FUSE export, which reading the mount
            // table and the export can take a while to find.
            let path = gio::spawn_blocking(move || ox_core::network::local_path(&uri))
                .await
                .ok()
                .flatten();
            let Some(path) = path else {
                if let Some(pane) = pane.upgrade().filter(|pane| pane.is_current(generation)) {
                    pane.contents().show_message(NO_PREVIEW);
                }
                return;
            };
            match page {
                PreviewPage::Text(_) => {
                    let read = gio::spawn_blocking(move || {
                        allowed(&path, local_only)
                            .then(|| read_text(&path, &cancellation))
                            .flatten()
                    })
                    .await;
                    let Some(pane) = pane.upgrade().filter(|pane| pane.is_current(generation)) else {
                        return;
                    };
                    match read {
                        Ok(Some(text)) => pane.show_text(&text),
                        _ => pane.contents().show_message(NO_PREVIEW),
                    }
                }
                PreviewPage::Picture(_) => {
                    let edge = width.max(height) * scale;
                    let decoded = gio::spawn_blocking(move || {
                        allowed(&path, local_only)
                            .then(|| decode_picture(&path, edge, &cancellation))
                            .flatten()
                    })
                    .await;
                    let Some(pane) = pane.upgrade().filter(|pane| pane.is_current(generation)) else {
                        return;
                    };
                    match decoded {
                        Ok(Some(texture)) => pane.show_picture(&texture),
                        _ => pane.contents().show_message(NO_PREVIEW),
                    }
                }
                PreviewPage::Pdf(_) => {
                    pdf_pages(pane, generation, path, local_only, width * scale, cancellation).await;
                }
                PreviewPage::Message(_) | PreviewPage::Media => {}
            }
        });
    }

    /// The room the contents have, in logical pixels, inside the margins.
    fn contents_size(&self) -> (i32, i32) {
        let width = (self.width() - 2 * MARGIN).max(super::PANE_WIDTH - 2 * MARGIN);
        let height = (self.height() - 2 * MARGIN).max(width);
        (width, height)
    }

    /// Shows `text` from its top.
    fn show_text(&self, text: &str) {
        let contents = self.contents();
        let buffer = contents.text.buffer();
        buffer.set_text(text);
        buffer.place_cursor(&buffer.start_iter());
        contents.text_scroller.vadjustment().set_value(0.0);
        contents.stack.set_visible_child_name(TEXT);
    }

    /// Shows `texture`, as large as the pane allows and never larger than
    /// it is.
    fn show_picture(&self, texture: &gdk::Texture) {
        let contents = self.contents();
        let picture = gtk::Picture::builder()
            .paintable(texture)
            .content_fit(gtk::ContentFit::ScaleDown)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .margin_start(MARGIN)
            .margin_end(MARGIN)
            .margin_top(MARGIN)
            .margin_bottom(MARGIN)
            .css_classes(["preview-picture"])
            .build();
        picture.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "Preview",
        ))]);
        contents.picture_room.set_child(Some(&picture));
        contents.stack.set_visible_child_name(PICTURE);
    }

    /// Adds `page` under the PDF pages shown.
    fn add_pdf_page(&self, page: &gdk::Texture) {
        let contents = self.contents();
        let picture = gtk::Picture::builder()
            .paintable(page)
            .content_fit(gtk::ContentFit::ScaleDown)
            .can_shrink(true)
            .hexpand(true)
            .css_classes(["preview-page"])
            .build();
        // As tall as its shape needs at the pane's width.
        let (width, _) = self.contents_size();
        let height = i64::from(width) * i64::from(page.height()) / i64::from(page.width().max(1));
        picture.set_size_request(-1, i32::try_from(height).unwrap_or(i32::MAX).max(1));
        contents.pdf_pages.append(&picture);
        if contents.stack.visible_child_name().as_deref() != Some(PDF) {
            contents.pdf_scroller.vadjustment().set_value(0.0);
            contents.stack.set_visible_child_name(PDF);
        }
    }

    /// The text page's text, for tests.
    #[cfg(test)]
    pub(in crate::window) fn preview_text(&self) -> Option<String> {
        let contents = self.contents();
        (self.imp().pages.visible_child_name().as_deref() == Some(PREVIEW)
            && contents.stack.visible_child_name().as_deref() == Some(TEXT))
        .then(|| {
            let buffer = contents.text.buffer();
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string()
        })
    }

    /// The message in the middle of the pane, for tests.
    #[cfg(test)]
    pub(in crate::window) fn preview_message(&self) -> Option<String> {
        let contents = self.contents();
        (self.imp().pages.visible_child_name().as_deref() == Some(PREVIEW)
            && contents.stack.visible_child_name().as_deref() == Some(MESSAGE))
        .then(|| contents.message.text().to_string())
        .filter(|text| !text.is_empty())
    }

    /// The size of the picture shown, for tests.
    #[cfg(test)]
    pub(in crate::window) fn preview_picture_size(&self) -> Option<(i32, i32)> {
        let contents = self.contents();
        let shown = self.imp().pages.visible_child_name().as_deref() == Some(PREVIEW)
            && contents.stack.visible_child_name().as_deref() == Some(PICTURE);
        let picture = contents.picture_room.child().filter(|_| shown)?;
        let picture = picture
            .downcast::<gtk::Viewport>()
            .ok()?
            .child()
            .and_downcast::<gtk::Picture>()?;
        let paintable = picture.paintable()?;
        Some((paintable.intrinsic_width(), paintable.intrinsic_height()))
    }

    /// Whether the Preview pane shows the Details column (the player of
    /// a recording), for tests.
    #[cfg(test)]
    pub(in crate::window) fn shows_details_column(&self) -> bool {
        self.imp().pages.visible_child_name().as_deref() == Some(DETAILS)
    }
}

/// Draws the PDF at `path` page by page for `pane`'s selection
/// `generation`, `width` pixels wide, until it ends, [`PDF_PAGE_LIMIT`]
/// pages or [`PDF_PIXEL_BUDGET`] pixels are shown, or the selection
/// changes; only a plain file, and with `local_only` not on a network
/// file system.
async fn pdf_pages(
    pane: glib::WeakRef<DetailsPane>,
    generation: u64,
    path: std::path::PathBuf,
    local_only: bool,
    width: i32,
    cancellation: gio::Cancellable,
) {
    let mut first = 1;
    let mut pixels = 0_u64;
    while first <= PDF_PAGE_LIMIT && pixels < PDF_PIXEL_BUDGET {
        let last = (first + PDF_BATCH - 1).min(PDF_PAGE_LIMIT);
        let (path, cancel) = (path.clone(), cancellation.clone());
        let drawn = gio::spawn_blocking(move || {
            // Only a plain file reaches pdftoppm, which would wait on a
            // pipe until its time limit ([`open_plain_file`]).
            let plain = || fs::metadata(&path).is_ok_and(|metadata| metadata.is_file());
            if first == 1 && !(plain() && allowed(&path, local_only)) {
                return None;
            }
            crate::thumbnails::pdf_pages(&path, first, last, width, &cancel)
                .map(|pages| pages.iter().map(gdk::Texture::for_pixbuf).collect::<Vec<_>>())
        })
        .await
        .ok()
        .flatten();
        let Some(pane) = pane.upgrade().filter(|pane| pane.is_current(generation)) else {
            return;
        };
        let Some(pages) = drawn else {
            if first == 1 {
                pane.contents().show_message(NO_PREVIEW);
            }
            return;
        };
        for page in &pages {
            pane.add_pdf_page(page);
            let size = |side: i32| u64::try_from(side).unwrap_or_default();
            pixels += size(page.width()) * size(page.height());
        }
        if u32::try_from(pages.len()).unwrap_or(0) < last - first + 1 {
            return;
        }
        first = last + 1;
    }
}

/// Whether the file at `path` may be read: with `local_only` (network
/// previews off), only when the file system it is on, after its links,
/// is no network one, as its thumbnail ([`crate::thumbnails`]). Blocks.
pub(super) fn allowed(path: &Path, local_only: bool) -> bool {
    !local_only || !crate::thumbnails::is_on_network_filesystem(path)
}

/// The plain file at `path`, opened for reading: a link is followed, but
/// a pipe or a device is never opened (opening some devices does
/// something, as a tape rewinds), nor waited on when one is swapped in
/// after that check (the open does not block), and it is refused by the
/// opened descriptor's own type, so nothing swapped in is read. Blocks.
fn open_plain_file(path: &Path) -> Option<fs::File> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed())
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

/// The text the Preview pane shows of the file at `path`: its first
/// [`TEXT_LIMIT`] bytes, decoded by [`decode_text`]; `None` for a binary
/// file, one that is not a plain file ([`open_plain_file`]), or a
/// cancelled read. Blocks.
fn read_text(path: &Path, cancellation: &gio::Cancellable) -> Option<String> {
    let file = open_plain_file(path)?;
    let mut bytes = Vec::with_capacity(64 * 1024);
    let mut chunk = vec![0; 64 * 1024];
    let mut reader = file.take(u64::try_from(TEXT_LIMIT).unwrap_or(u64::MAX) + 1);
    loop {
        if cancellation.is_cancelled() {
            return None;
        }
        let read = reader.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let truncated = bytes.len() > TEXT_LIMIT;
    bytes.truncate(TEXT_LIMIT);
    decode_text(&bytes, truncated)
}

/// The picture at `path` decoded to fit `edge` pixels square, or at its
/// own size when smaller, and turned upright, within the app's limits for
/// a picture shown ([`picture_guard`]: its pixels, a GIF's first frame
/// only, a time limit, and `cancellation`); `None` when it is not a plain
/// file ([`open_plain_file`]) or a picture gdk-pixbuf reads, or was
/// refused. Blocks.
fn decode_picture(path: &Path, edge: i32, cancellation: &gio::Cancellable) -> Option<gdk::Texture> {
    let mut file = open_plain_file(path)?;
    let decoded =
        picture_guard::decode_reader_to_fit(&mut file, edge, edge, Limits::DISPLAY, Some(cancellation))
            .ok()?;
    Some(gdk::Texture::for_pixbuf(&decoded.pixbuf))
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::super::content::{pane_content, PaneFacts};
    use super::super::preview_page::PreviewRules;
    use super::super::{DetailsPane, PaneMode};
    use crate::folder_view::item::FileItem;
    use crate::test_support::harness::{wait_until, Fixture};
    use ox_core::location::LocationContext;

    /// A Preview pane hidden while it reads (closed, or a narrow window)
    /// stops reading, and reads again once it is shown.
    ///
    /// parity: VIEW-070
    #[gtk::test]
    fn a_pane_hidden_while_it_reads_stops_and_reads_again_when_shown() {
        let fixture = Fixture::standard();
        let (window, pane) = preview_pane_with(&fixture, "Notes 2.txt");
        pane.set_visible(true);
        assert!(pane.imp().page_timer.borrow().is_some(), "about to read");

        pane.set_visible(false);

        assert!(pane.imp().page_timer.borrow().is_none(), "the wait is stopped");
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(400));
        assert_eq!(pane.preview_text(), None, "nothing was read for a hidden pane");
        pane.set_visible(true);
        wait_until("the text", || {
            pane.preview_text().as_deref() == Some("Synthetic test data\n")
        });
        window.destroy();
    }

    /// A Preview pane, hidden, in a window of its own, given the content
    /// of the fixture's file `name`.
    fn preview_pane_with(fixture: &Fixture, name: &str) -> (gtk::Window, DetailsPane) {
        let pane: DetailsPane = gtk::glib::Object::new();
        let window = gtk::Window::builder().child(&pane).build();
        window.present();
        pane.set_mode(PaneMode::Preview);
        pane.set_visible(false);
        let mut entry = crate::test_support::file_entry(name);
        entry.uri = fixture.uri_of(name);
        entry.content_type = Some("text/plain".to_owned());
        let selection = [FileItem::new(entry)];
        let locations = LocationContext::default();
        let content = pane_content(&PaneFacts {
            selection: &selection,
            folder_uri: &fixture.uri(),
            folder_item_count: 1,
            locations: &locations,
            network: &[],
            condensed_dates: false,
            preview_rules: PreviewRules {
                remote: false,
                remote_previews: false,
            },
        });
        pane.set_content(&content);
        (window, pane)
    }

    /// A Preview pane given its content while hidden reads it once it is
    /// shown, without waiting for the window to ask again.
    ///
    /// parity: VIEW-070
    #[gtk::test]
    fn a_pane_shown_again_reads_the_page_it_was_given_while_hidden() {
        let fixture = Fixture::standard();
        let pane: DetailsPane = gtk::glib::Object::new();
        let window = gtk::Window::builder().child(&pane).build();
        window.present();
        pane.set_mode(PaneMode::Preview);
        pane.set_visible(false);
        let mut entry = crate::test_support::file_entry("Notes 2.txt");
        entry.uri = fixture.uri_of("Notes 2.txt");
        entry.content_type = Some("text/plain".to_owned());
        let selection = [FileItem::new(entry)];
        let locations = LocationContext::default();
        let content = pane_content(&PaneFacts {
            selection: &selection,
            folder_uri: &fixture.uri(),
            folder_item_count: 1,
            locations: &locations,
            network: &[],
            condensed_dates: false,
            preview_rules: PreviewRules {
                remote: false,
                remote_previews: false,
            },
        });
        pane.set_content(&content);

        pane.set_visible(true);

        wait_until("the text", || {
            pane.preview_text().as_deref() == Some("Synthetic test data\n")
        });
        window.destroy();
    }
}
