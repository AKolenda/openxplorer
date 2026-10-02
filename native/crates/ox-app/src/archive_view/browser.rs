// SPDX-License-Identifier: AGPL-3.0-only
//! The "Compressed folder" dialog: a ZIP browsed read-only (ARC-003).
//!
//! Ports `archiveDialog` in `v2.0.0:desktop/ui/app.js`: Up and the path inside
//! the archive, one row per folder and file with its size, a folder opened
//! by a double-click or Enter, and a supported file opened as a private
//! read-only copy. The dialog says when a folder is empty, how many
//! unsafe names or links it hides (ARC-004) and when it shows only the
//! first 5,000 entries; a listing that arrives after a newer one started
//! is dropped. Its buttons are Extract all…, Open in archive manager and
//! Close.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::archive::{ArchiveBrowser, ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveListing};
use ox_core::format;
use ox_core::transfer::Cancellation;

use super::{archive_art, ArchiveTarget};
use crate::dialog::{quiet_text, DialogFrame, DialogWidth};
use crate::icons::{self, ArtImage, Icon};
use crate::window::ButtonStyle;

/// Under the heading: what the dialog is for.
const BROWSE_MESSAGE: &str = "Browse without extracting the entire archive.";
/// Above the list: what browsing does and does not do.
const READ_ONLY_HELP: &str = "Read-only ZIP. Double-click a folder to browse it. Opening a supported file \
                              creates only that file’s temporary copy; it does not update the ZIP.";
/// Shown while a folder of the archive is read.
const READING: &str = "Reading ZIP directory…";
/// Shown for a folder without members.
const EMPTY_FOLDER: &str = "This archive folder is empty.";
/// Shown for a member that cannot be opened as a copy.
const NOT_OPENABLE: &str = "Use an archive manager for encrypted, large or unsupported members.";
/// Shown while a member is copied out.
const OPENING: &str = "Opening a read-only temporary copy…";
/// Shown once the copy opened.
const OPENED: &str = "Opened a temporary copy. Changes are not saved back to the ZIP.";
/// The size of a row's picture (`fileIcon(item, 25)`).
const ROW_ART_SIZE: i32 = 25;

/// What the dialog's buttons and rows ask the window to do.
pub(crate) struct ArchiveDialogActions {
    /// Extract all…: closes this dialog and opens the Extract dialog.
    pub extract_all: Box<dyn Fn()>,
    /// Open in archive manager: opens the archive with the desktop's
    /// application for ZIP files.
    pub open_externally: Box<dyn Fn()>,
    /// Opens the private copy of a member at the URI with its
    /// application.
    pub open_copy: Rc<dyn Fn(String)>,
}

/// The "<name> — Compressed folder" dialog for `archive`, showing its
/// location as `shown_path`.
pub(crate) fn archive_dialog(
    archive: &ArchiveTarget,
    browser: ArchiveBrowser,
    shown_path: &str,
    actions: ArchiveDialogActions,
) -> DialogFrame {
    let frame = DialogFrame::new(
        &format!("{} — Compressed folder", archive.name),
        DialogWidth::Archive,
    );
    frame.set_message(BROWSE_MESSAGE);
    let view = ArchiveBrowserView::new(archive.clone(), browser, shown_path, actions.open_copy);
    frame.body().append(&view);
    frame.add_closing_button("Extract all…", ButtonStyle::Bordered, actions.extract_all);
    frame.add_closing_button(
        "Open in archive manager",
        ButtonStyle::Bordered,
        actions.open_externally,
    );
    frame.add_closing_button("Close", ButtonStyle::Accent, || {});
    frame.connect_closed(glib::clone!(
        #[weak]
        view,
        move |_| view.cancel()
    ));
    view.show_folder("");
    frame
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;

    use gtk::glib;
    use gtk::subclass::prelude::*;
    use ox_core::archive::ArchiveBrowser;
    use ox_core::transfer::Cancellation;

    use super::super::ArchiveTarget;

    /// Private state of [`super::ArchiveBrowserView`].
    #[derive(Default)]
    pub(crate) struct ArchiveBrowserView {
        /// The archive; set by `new`.
        pub(super) archive: OnceCell<ArchiveTarget>,
        /// Reads the archive; set by `new`.
        pub(super) browser: OnceCell<ArchiveBrowser>,
        /// Opens a member's private copy; set by `new`.
        pub(super) open_copy: OnceCell<Rc<dyn Fn(String)>>,
        /// The archive's location as the path line shows it.
        pub(super) shown_path: RefCell<String>,
        /// The folder shown: empty for the top, else ending with `/`.
        pub(super) prefix: RefCell<String>,
        /// What each row shows, in row order.
        pub(super) entries: RefCell<Vec<ox_core::archive::ArchiveEntry>>,
        /// Counts listings, so an older one's result is dropped.
        pub(super) generation: Cell<u64>,
        /// Stops the listing or copy in progress.
        pub(super) work: RefCell<Cancellation>,
        /// Where in the archive the dialog is.
        pub(super) path_label: gtk::Label,
        /// The rows.
        pub(super) list: gtk::ListBox,
        /// Hidden members, truncation, errors and notices.
        pub(super) status: gtk::Label,
    }

    impl std::fmt::Debug for ArchiveBrowserView {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("ArchiveBrowserView")
                .field("archive", &self.archive.get())
                .field("prefix", &self.prefix)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArchiveBrowserView {
        const NAME: &'static str = "OxArchiveBrowserView";
        type Type = super::ArchiveBrowserView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for ArchiveBrowserView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for ArchiveBrowserView {}
    impl BoxImpl for ArchiveBrowserView {}
}

glib::wrapper! {
    /// The toolbar, rows and status line of the archive browser.
    pub(crate) struct ArchiveBrowserView(ObjectSubclass<imp::ArchiveBrowserView>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl ArchiveBrowserView {
    /// The browser of `archive`, which `open_copy` opens members of.
    fn new(
        archive: ArchiveTarget,
        browser: ArchiveBrowser,
        shown_path: &str,
        open_copy: Rc<dyn Fn(String)>,
    ) -> Self {
        let view: Self = glib::Object::new();
        let imp = view.imp();
        imp.archive.set(archive).expect("a new view has no archive yet");
        imp.browser.set(browser).expect("a new view has no reader yet");
        assert!(
            imp.open_copy.set(open_copy).is_ok(),
            "a new view has no opener yet"
        );
        imp.shown_path.replace(shown_path.to_owned());
        view
    }

    /// Lays out the toolbar, the help, the list and the status line.
    fn build(&self) {
        let imp = self.imp();
        self.set_orientation(gtk::Orientation::Vertical);
        let toolbar = gtk::Box::builder().css_classes(["archive-toolbar"]).build();
        let up = up_button();
        up.connect_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.go_up()
        ));
        toolbar.append(&up);
        imp.path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        imp.path_label.set_xalign(0.0);
        imp.path_label.add_css_class("archive-path");
        toolbar.append(&imp.path_label);
        self.append(&toolbar);
        self.append(&quiet_text(READ_ONLY_HELP));
        imp.list.add_css_class("archive-list");
        imp.list.set_selection_mode(gtk::SelectionMode::None);
        imp.list
            .update_property(&[gtk::accessible::Property::Label("Archive contents")]);
        imp.list.connect_row_activated(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, row| view.row_activated(row)
        ));
        self.append(&imp.list);
        imp.status.set_xalign(0.0);
        imp.status.set_wrap(true);
        imp.status.add_css_class("archive-status");
        imp.status.set_accessible_role(gtk::AccessibleRole::Status);
        self.append(&imp.status);
    }

    fn browser(&self) -> &ArchiveBrowser {
        self.imp().browser.get().expect("new sets the reader")
    }

    fn archive(&self) -> &ArchiveTarget {
        self.imp().archive.get().expect("new sets the archive")
    }

    /// Lists the folder `prefix` of the archive (empty for the top).
    pub(crate) fn show_folder(&self, prefix: &str) {
        let imp = self.imp();
        let generation = imp.generation.get().wrapping_add(1);
        imp.generation.set(generation);
        imp.prefix.replace(prefix.to_owned());
        let shown_path = imp.shown_path.borrow().clone();
        let path = if prefix.is_empty() {
            shown_path
        } else {
            format!("{shown_path} › {prefix}")
        };
        imp.path_label.set_text(&path);
        imp.list.remove_all();
        imp.entries.borrow_mut().clear();
        imp.list.append(&quiet_text(READING));
        imp.status.set_text("");
        let cancel = self.restart_work();
        let browser = self.browser().clone();
        let uri = self.archive().uri.clone();
        let prefix = prefix.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let result = browser.list_in_background(uri, prefix, cancel).await;
                view.listing_arrived(generation, result);
            }
        ));
    }

    /// Shows the answer of listing number `generation`, unless a newer
    /// listing started since: a late answer never replaces a newer one.
    fn listing_arrived(&self, generation: u64, result: Result<ArchiveListing, ArchiveError>) {
        if self.imp().generation.get() == generation {
            self.show_listing(result);
        }
    }

    /// Cancels the work in progress and returns a fresh cancellation.
    fn restart_work(&self) -> Cancellation {
        let fresh = Cancellation::new();
        let previous = self.imp().work.replace(fresh.clone());
        previous.cancel();
        fresh
    }

    /// Stops the listing or copy in progress, as closing the dialog does.
    pub(crate) fn cancel(&self) {
        self.imp().work.borrow().cancel();
    }

    /// Shows the rows of a listing, or why the folder could not be read.
    fn show_listing(&self, result: Result<ArchiveListing, ArchiveError>) {
        let imp = self.imp();
        imp.list.remove_all();
        let listing = match result {
            Ok(listing) => listing,
            Err(ArchiveError::Cancelled) => return,
            Err(error) => {
                imp.status.set_text(&error.to_string());
                return;
            }
        };
        for entry in &listing.entries {
            imp.list.append(&archive_row(entry));
        }
        if listing.entries.is_empty() {
            imp.list.append(&quiet_text(EMPTY_FOLDER));
        }
        imp.status.set_text(&listing_notice(&listing));
        imp.entries.replace(listing.entries);
    }

    /// Opens the folder that contains the one shown.
    fn go_up(&self) {
        let prefix = self.imp().prefix.borrow().clone();
        self.show_folder(&parent_prefix(&prefix));
    }

    /// A double-click or Enter on a row: a folder opens, a supported file
    /// opens as a copy, anything else says to use an archive manager.
    fn row_activated(&self, row: &gtk::ListBoxRow) {
        let Some(entry) = self.row_entry(row) else {
            return;
        };
        match entry.kind {
            ArchiveEntryKind::Folder => self.show_folder(&entry.member),
            ArchiveEntryKind::File { .. } if entry.can_open => self.open_member(&entry.member),
            ArchiveEntryKind::File { .. } => self.imp().status.set_text(NOT_OPENABLE),
        }
    }

    /// The entry `row` shows; `None` for the placeholder rows.
    fn row_entry(&self, row: &gtk::ListBoxRow) -> Option<ArchiveEntry> {
        let index = usize::try_from(row.index()).ok()?;
        self.imp().entries.borrow().get(index).cloned()
    }

    /// Copies `member` out privately and opens the copy.
    fn open_member(&self, member: &str) {
        let imp = self.imp();
        imp.status.set_text(OPENING);
        let cancel = self.restart_work();
        let browser = self.browser().clone();
        let uri = self.archive().uri.clone();
        let member = member.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                match browser.preview_member_in_background(uri, member, cancel).await {
                    Ok(copy) => {
                        let open_copy = view.imp().open_copy.get().expect("new sets the opener");
                        open_copy(copy.uri());
                        view.imp().status.set_text(OPENED);
                    }
                    Err(ArchiveError::Cancelled) => {}
                    Err(error) => view.imp().status.set_text(&error.to_string()),
                }
            }
        ));
    }

    /// Starts listing `prefix` and returns the number its answer carries,
    /// for tests that deliver an answer late with [`Self::deliver_listing`].
    #[cfg(test)]
    pub(crate) fn show_folder_numbered(&self, prefix: &str) -> u64 {
        self.show_folder(prefix);
        self.imp().generation.get()
    }

    /// Delivers `listing` as the answer of listing number `generation`,
    /// as the worker does, for tests.
    #[cfg(test)]
    pub(crate) fn deliver_listing(&self, generation: u64, listing: ArchiveListing) {
        self.listing_arrived(generation, Ok(listing));
    }

    /// Lists `prefix` of the archive at once on this thread, for tests.
    #[cfg(test)]
    pub(crate) fn list_now(&self, prefix: &str) -> ArchiveListing {
        self.browser()
            .list(&self.archive().uri, prefix, &Cancellation::new())
            .expect("the fixture archive is listed")
    }

    /// The names of the rows shown, for tests.
    #[cfg(test)]
    pub(crate) fn row_names(&self) -> Vec<String> {
        let entries = self.imp().entries.borrow();
        entries.iter().map(|entry| entry.name.clone()).collect()
    }

    /// The status line, for tests.
    #[cfg(test)]
    pub(crate) fn status_text(&self) -> String {
        self.imp().status.text().to_string()
    }

    /// The path line, for tests.
    #[cfg(test)]
    pub(crate) fn path_text(&self) -> String {
        self.imp().path_label.text().to_string()
    }

    /// Activates the row named `name`, as a double-click does, for tests.
    #[cfg(test)]
    pub(crate) fn activate_row(&self, name: &str) {
        let index = self
            .imp()
            .entries
            .borrow()
            .iter()
            .position(|entry| entry.name == name);
        let row = index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.imp().list.row_at_index(index));
        if let Some(row) = row {
            self.row_activated(&row);
        }
    }
}

/// The notices under the list: hidden unsafe members and truncation.
fn listing_notice(listing: &ArchiveListing) -> String {
    let mut notice = String::new();
    if listing.hidden_unsafe_count > 0 {
        notice = format!(
            "{} unsafe names or links are hidden.",
            listing.hidden_unsafe_count
        );
    }
    if listing.is_truncated {
        notice.push_str(" Showing the first 5,000 entries.");
    }
    notice
}

/// The folder above `prefix`: `a/b/` gives `a/`, `a/` gives the top.
fn parent_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/"),
        None => String::new(),
    }
}

/// Up, with its arrow.
fn up_button() -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icons::image(Icon::ArrowUp, 16));
    content.append(&gtk::Label::new(Some("Up")));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class(ButtonStyle::Bordered.css_class());
    button.update_property(&[gtk::accessible::Property::Label("Up")]);
    button
}

/// The row of `entry`: its picture, name, and size or "Folder".
fn archive_row(entry: &ArchiveEntry) -> gtk::ListBoxRow {
    let content = gtk::Box::builder().css_classes(["archive-row"]).build();
    content.append(&ArtImage::new(archive_art(entry), ROW_ART_SIZE));
    let name = gtk::Label::builder()
        .label(&entry.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    content.append(&name);
    let size = match entry.kind {
        ArchiveEntryKind::Folder => "Folder".to_owned(),
        ArchiveEntryKind::File { size, .. } => format::pretty_bytes(size),
    };
    content.append(
        &gtk::Label::builder()
            .label(size)
            .css_classes(["archive-size"])
            .build(),
    );
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&entry.name)]);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-004, ARC-003
    #[test]
    fn hidden_members_and_truncation_are_announced() {
        let listing = ArchiveListing {
            archive_uri: "file:///tmp/ox-test/a.zip".to_owned(),
            prefix: String::new(),
            entries: Vec::new(),
            hidden_unsafe_count: 2,
            is_truncated: true,
        };

        assert_eq!(
            listing_notice(&listing),
            "2 unsafe names or links are hidden. Showing the first 5,000 entries."
        );
    }

    #[test]
    fn up_goes_one_folder_higher() {
        assert_eq!(parent_prefix("Docs/Reports/"), "Docs/");
        assert_eq!(parent_prefix("Docs/"), "");
        assert_eq!(parent_prefix(""), "");
    }
}
