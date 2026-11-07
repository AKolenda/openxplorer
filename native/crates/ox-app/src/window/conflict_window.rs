// SPDX-License-Identifier: AGPL-3.0-only
//! The "N File Conflicts" window that "Compare info for both files" or
//! "Let me decide for each file" opens (OPS-028), as Windows 11 shows it.
//!
//! "Which files do you want to keep?" and "If you select both versions,
//! the copied file will have a number added to its name.", a check box
//! for each side ("Files from Downloads", "Files already in Desktop",
//! each folder a link), then one row per file: its name, and for each
//! side a check box with its thumbnail (the shared thumbnail cache, as
//! the folder views read it; the type's icon when there is none), its
//! date modified and its size. "Skip N files with the same date and
//! size" skips those at once. Continue is enabled once every file has a
//! side; only the incoming side replaces, only the existing side skips,
//! and both keep both, the copy numbered `report (2).png`. Cancel stops
//! the whole action.
//!
//! The window belongs to the progress window: it is its transient child,
//! modal within a window group of the two alone, so it blocks only the
//! progress window, as in Windows, while every browsing window and every
//! other action goes on. It sets no position: the desktop places it, as
//! Wayland requires. Escape is Cancel; every check box and button is
//! reachable with Tab and named for assistive technologies. Nothing in
//! it moves, so reduced motion needs nothing more.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::format::{date_short_time_text, pretty_bytes};
use ox_core::i18n::{format_message, gettext, ngettext};
use ox_core::ops::{ConflictScan, ItemFacts, NameConflict, Resolution};

use crate::icons::{self, FileType, Icon};
use crate::thumbnails::{preview, PreviewRequest};
use crate::window::progress_window::ProgressWindow;
use crate::window::transfer_panel::{location_name, TransferPanel};

/// The edge of a side's thumbnail.
const THUMBNAIL_SIZE: i32 = 64;

/// The two check boxes of one file.
#[derive(Debug, Clone)]
struct RowChecks {
    incoming: gtk::CheckButton,
    existing: gtk::CheckButton,
    /// The thumbnails of the two sides.
    pictures: [gtk::Image; 2],
    /// Both files have the same date and size.
    same: bool,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;
    use ox_core::ops::Resolution;

    use super::RowChecks;

    /// Private state of [`super::FileConflictWindow`].
    #[derive(Debug, Default)]
    pub(crate) struct FileConflictWindow {
        pub(super) rows: RefCell<Vec<RowChecks>>,
        pub(super) incoming_header: RefCell<Option<gtk::CheckButton>>,
        pub(super) existing_header: RefCell<Option<gtk::CheckButton>>,
        pub(super) skip_same: RefCell<Option<gtk::CheckButton>>,
        pub(super) continue_button: RefCell<Option<gtk::Button>>,
        /// Set while the window changes check boxes itself.
        pub(super) updating: Cell<bool>,
        /// Receives the answers, or `None` for Cancel.
        pub(super) answer: RefCell<Option<async_channel::Sender<Option<Vec<Resolution>>>>>,
        pub(super) receiver: RefCell<Option<async_channel::Receiver<Option<Vec<Resolution>>>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileConflictWindow {
        const NAME: &'static str = "OxFileConflictWindow";
        type Type = super::FileConflictWindow;
        type ParentType = gtk::Window;
    }

    impl ObjectImpl for FileConflictWindow {}
    impl WidgetImpl for FileConflictWindow {}

    impl WindowImpl for FileConflictWindow {
        /// Closing the window is Cancel.
        fn close_request(&self) -> glib::Propagation {
            // GTK destroys the window once this returns.
            self.obj().send(None);
            glib::Propagation::Proceed
        }
    }
}

glib::wrapper! {
    /// The File Conflict window of an action.
    pub(crate) struct FileConflictWindow(ObjectSubclass<imp::FileConflictWindow>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager;
}

/// The window's title: "1 File Conflict", "3 File Conflicts".
fn title(count: usize) -> String {
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    ngettext("{count} File Conflict", "{count} File Conflicts", count).replace("{count}", &count.to_string())
}

/// "Skip 1 file with the same date and size".
fn skip_same_label(count: usize) -> String {
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    ngettext(
        "Skip {count} file with the same date and size",
        "Skip {count} files with the same date and size",
        count,
    )
    .replace("{count}", &count.to_string())
}

/// What each file's check boxes answer: both kept, the incoming one, or
/// the existing one; a file skipped for its same date and size, or with
/// no side, is skipped.
fn resolution(incoming: bool, existing: bool, skipped_as_same: bool) -> Resolution {
    match (incoming, existing) {
        _ if skipped_as_same => Resolution::Skip,
        (true, true) => Resolution::KeepBoth,
        (true, false) => Resolution::Replace,
        _ => Resolution::Skip,
    }
}

/// A side's lines: the date modified, then the size (or "Folder").
fn side_lines(facts: ItemFacts) -> (String, String) {
    let date = date_short_time_text(facts.modified);
    let size = if facts.is_folder {
        gettext("Folder")
    } else {
        facts.size.map(pretty_bytes).unwrap_or_default()
    };
    (date, size)
}

/// "Files from {source}" with the folder a link, as Pango markup.
fn header_markup(template: &str, uri: &str) -> String {
    let link = format!(
        "<a href=\"{}\">{}</a>",
        glib::markup_escape_text(uri),
        glib::markup_escape_text(&location_name(uri))
    );
    let escaped = glib::markup_escape_text(template).to_string();
    escaped.replacen("{folder}", &link, 1)
}

impl FileConflictWindow {
    /// Opens the window of `panel`'s action over `progress` for `scan`'s
    /// conflicts, from the folder `source` into `destination`.
    pub(crate) fn new(
        progress: &ProgressWindow,
        panel: &TransferPanel,
        scan: &ConflictScan,
        source: &str,
        destination: &str,
    ) -> Self {
        let window: Self = glib::Object::builder()
            .property("title", title(scan.conflicts.len()))
            .property("modal", true)
            .property("destroy-with-parent", true)
            .property("default-width", 560)
            .property("default-height", 520)
            .build();
        window.add_css_class("ox");
        window.add_css_class("file-conflicts");
        window.set_transient_for(Some(progress));
        progress.group_for_questions().add_window(&window);
        let (sender, receiver) = async_channel::bounded(1);
        window.imp().answer.replace(Some(sender));
        window.imp().receiver.replace(Some(receiver));
        window.build(panel, scan, source, destination);
        progress.set_conflict_window(Some(&window));
        window.connect_map(Self::forget_focus);
        window.present();
        window.forget_focus();
        window
    }

    /// Leaves no widget focused and no focus ring showing, as the Windows
    /// window opens; Tab still moves into it and shows the ring, Escape
    /// cancels and Enter presses Continue.
    fn forget_focus(&self) {
        GtkWindowExt::set_focus(self, None::<&gtk::Widget>);
        self.set_focus_visible(false);
    }

    /// Waits for Continue (an answer per conflict) or Cancel (`None`).
    pub(crate) async fn answers(&self) -> Option<Vec<Resolution>> {
        let receiver = self.imp().receiver.take()?;
        receiver.recv().await.ok().flatten()
    }

    /// Closes the window unanswered, as Cancel does.
    pub(crate) fn cancel(&self) {
        self.respond(None);
    }

    /// Sends `answers` once, and closes the window.
    fn respond(&self, answers: Option<Vec<Resolution>>) {
        if self.send(answers) {
            self.close();
        }
    }

    /// Sends `answers` once, and forgets the window in the progress
    /// window; false when it was already answered.
    fn send(&self, answers: Option<Vec<Resolution>>) -> bool {
        let Some(sender) = self.imp().answer.take() else {
            return false;
        };
        let _ = sender.try_send(answers);
        if let Some(progress) = self.transient_for().and_downcast::<ProgressWindow>() {
            progress.set_conflict_window(None);
        }
        true
    }

    /// Lays out the window's content and connects its controls.
    fn build(&self, panel: &TransferPanel, scan: &ConflictScan, source: &str, destination: &str) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&intro());
        let (headers, incoming_header, existing_header) = side_headers(panel, source, destination);
        content.append(&headers);
        let (list, rows) = conflict_list(scan, &location_name(source), &location_name(destination));
        content.append(&list);
        let same = rows.iter().filter(|row| row.same).count();
        let (footer, skip_same, continue_button, cancel_button) = footer(same);
        content.append(&footer);
        self.set_child(Some(&content));
        self.set_default_widget(Some(&continue_button));

        let imp = self.imp();
        imp.rows.replace(rows);
        imp.incoming_header.replace(Some(incoming_header));
        imp.existing_header.replace(Some(existing_header));
        imp.skip_same.replace(skip_same);
        imp.continue_button.replace(Some(continue_button.clone()));
        self.connect_checks();
        self.connect_answers(&continue_button, &cancel_button);
        for (conflict, row) in scan.conflicts.iter().zip(self.imp().rows.borrow().iter()) {
            load_pictures(conflict, &row.pictures, THUMBNAIL_SIZE);
        }
    }

    /// Continue answers, Cancel and Escape cancel.
    fn connect_answers(&self, continue_button: &gtk::Button, cancel_button: &gtk::Button) {
        continue_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                let answers = window.current_answers();
                window.respond(Some(answers));
            }
        ));
        cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.respond(None)
        ));
        let escape = gtk::ShortcutController::new();
        escape.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("Escape"),
            Some(gtk::CallbackAction::new(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, _| {
                    window.respond(None);
                    glib::Propagation::Stop
                }
            ))),
        ));
        self.add_controller(escape);
    }

    /// Keeps the header check boxes, "Skip … same date and size" and
    /// Continue in step with the rows.
    fn connect_checks(&self) {
        let imp = self.imp();
        let rows = imp.rows.borrow().clone();
        for row in &rows {
            for check in [&row.incoming, &row.existing] {
                check.connect_toggled(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |_| window.refresh()
                ));
            }
        }
        for (header, incoming) in [
            (imp.incoming_header.borrow().clone(), true),
            (imp.existing_header.borrow().clone(), false),
        ] {
            let Some(header) = header else { continue };
            header.connect_toggled(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |header| window.choose_side(incoming, header.is_active())
            ));
        }
        if let Some(skip_same) = imp.skip_same.borrow().clone() {
            skip_same.connect_toggled(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_| window.refresh()
            ));
        }
        self.refresh();
    }

    /// A header check box was toggled: every free row takes that side.
    fn choose_side(&self, incoming: bool, active: bool) {
        let imp = self.imp();
        if imp.updating.get() {
            return;
        }
        imp.updating.set(true);
        let skipping = self.skips_same();
        for row in imp.rows.borrow().iter().filter(|row| !(skipping && row.same)) {
            let check = if incoming { &row.incoming } else { &row.existing };
            check.set_active(active);
        }
        imp.updating.set(false);
        self.refresh();
    }

    /// Whether "Skip … same date and size" is checked.
    fn skips_same(&self) -> bool {
        self.imp()
            .skip_same
            .borrow()
            .as_ref()
            .is_some_and(gtk::CheckButton::is_active)
    }

    /// Updates the headers and Continue from the rows.
    fn refresh(&self) {
        let imp = self.imp();
        if imp.updating.get() {
            return;
        }
        imp.updating.set(true);
        let skipping = self.skips_same();
        let rows = imp.rows.borrow();
        for row in rows.iter() {
            let free = !(skipping && row.same);
            row.incoming.set_sensitive(free);
            row.existing.set_sensitive(free);
        }
        let free: Vec<&RowChecks> = rows.iter().filter(|row| !(skipping && row.same)).collect();
        for (header, pick) in [
            (imp.incoming_header.borrow().clone(), true),
            (imp.existing_header.borrow().clone(), false),
        ] {
            let Some(header) = header else { continue };
            let checked = free
                .iter()
                .filter(|row| {
                    if pick {
                        row.incoming.is_active()
                    } else {
                        row.existing.is_active()
                    }
                })
                .count();
            header.set_active(!free.is_empty() && checked == free.len());
            header.set_inconsistent(checked > 0 && checked < free.len());
            header.set_sensitive(!free.is_empty());
        }
        let answered = free
            .iter()
            .all(|row| row.incoming.is_active() || row.existing.is_active());
        if let Some(button) = imp.continue_button.borrow().as_ref() {
            button.set_sensitive(answered);
        }
        imp.updating.set(false);
    }

    /// The answer of every row, in order.
    fn current_answers(&self) -> Vec<Resolution> {
        let skipping = self.skips_same();
        self.imp()
            .rows
            .borrow()
            .iter()
            .map(|row| {
                resolution(
                    row.incoming.is_active(),
                    row.existing.is_active(),
                    skipping && row.same,
                )
            })
            .collect()
    }

    /// The check boxes "Files from …" and "Files already in …", for
    /// tests.
    #[cfg(test)]
    pub(crate) fn header_checks(&self) -> (gtk::CheckButton, gtk::CheckButton) {
        let imp = self.imp();
        let incoming = imp.incoming_header.borrow().clone().expect("built");
        let existing = imp.existing_header.borrow().clone().expect("built");
        (incoming, existing)
    }

    /// The two check boxes of row `index`, for tests.
    #[cfg(test)]
    pub(crate) fn row_checks(&self, index: usize) -> (gtk::CheckButton, gtk::CheckButton) {
        let rows = self.imp().rows.borrow();
        (rows[index].incoming.clone(), rows[index].existing.clone())
    }
}

/// "Which files do you want to keep?" and the hint under it.
fn intro() -> gtk::Box {
    let top = gtk::Box::new(gtk::Orientation::Vertical, 4);
    top.add_css_class("file-conflicts-top");
    let heading = gtk::Label::new(Some(&gettext("Which files do you want to keep?")));
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    heading.add_css_class("file-conflicts-heading");
    heading.set_accessible_role(gtk::AccessibleRole::Heading);
    let hint = gtk::Label::new(Some(&gettext(
        "If you select both versions, the copied file will have a number added to its name.",
    )));
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    top.append(&heading);
    top.append(&hint);
    top
}

/// The check boxes "Files from <source>" and "Files already in
/// <destination>", each folder a link that `panel` opens.
fn side_headers(
    panel: &TransferPanel,
    source: &str,
    destination: &str,
) -> (gtk::Grid, gtk::CheckButton, gtk::CheckButton) {
    let headers = gtk::Grid::new();
    headers.set_column_homogeneous(true);
    headers.add_css_class("file-conflicts-sides");
    let header = |column: i32, template: String, uri: &str| {
        let check = gtk::CheckButton::new();
        let label = gtk::Label::new(None);
        label.set_markup(&header_markup(&template, uri));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.connect_activate_link(glib::clone!(
            #[weak]
            panel,
            #[upgrade_or]
            glib::Propagation::Stop,
            move |_, uri| {
                panel.open_location(uri);
                glib::Propagation::Stop
            }
        ));
        let plain = template.replacen("{folder}", &location_name(uri), 1);
        check.update_property(&[gtk::accessible::Property::Label(&plain)]);
        label.set_mnemonic_widget(Some(&check));
        let side = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        side.append(&check);
        side.append(&label);
        headers.attach(&side, column, 0, 1, 1);
        check
    };
    let incoming = header(0, gettext("Files from {folder}"), source);
    let existing = header(1, gettext("Files already in {folder}"), destination);
    (headers, incoming, existing)
}

/// The scrolled list of the conflicts, one row each, from the folders
/// called `source` and `destination`.
fn conflict_list(
    scan: &ConflictScan,
    source: &str,
    destination: &str,
) -> (gtk::ScrolledWindow, Vec<RowChecks>) {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_accessible_role(gtk::AccessibleRole::List);
    list.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Files with the same names",
    ))]);
    let mut rows = Vec::new();
    for conflict in &scan.conflicts {
        let (row, checks) = conflict_row(conflict, source, destination);
        list.append(&row);
        rows.push(checks);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    scroller.add_css_class("file-conflicts-list");
    (scroller, rows)
}

/// "Skip N files with the same date and size" when `same` is not 0, then
/// Continue and Cancel.
fn footer(same: usize) -> (gtk::Box, Option<gtk::CheckButton>, gtk::Button, gtk::Button) {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.add_css_class("file-conflicts-footer");
    let skip_same = (same > 0).then(|| gtk::CheckButton::with_label(&skip_same_label(same)));
    let start: gtk::Widget = match &skip_same {
        Some(check) => check.clone().upcast(),
        None => gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
    };
    start.set_hexpand(true);
    footer.append(&start);
    let continue_button = gtk::Button::with_label(&gettext("Continue"));
    continue_button.set_sensitive(false);
    let cancel_button = gtk::Button::with_label(&gettext("Cancel"));
    for button in [&continue_button, &cancel_button] {
        button.add_css_class("bordered");
        button.add_css_class("file-conflicts-button");
    }
    footer.append(&continue_button);
    footer.append(&cancel_button);
    (footer, skip_same, continue_button, cancel_button)
}

/// The row of one conflict: the name, then for each side a check box, its
/// thumbnail, and its date modified over its size, all aligned at the
/// top as in Windows. Clicking the thumbnail or the text toggles the box.
fn conflict_row(conflict: &NameConflict, source: &str, destination: &str) -> (gtk::Box, RowChecks) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.add_css_class("file-conflict");
    row.set_accessible_role(gtk::AccessibleRole::ListItem);
    let name = conflict.name();
    let title = gtk::Label::new(Some(&name));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title.add_css_class("file-conflict-name");
    row.append(&title);
    let sides = gtk::Grid::new();
    sides.set_column_homogeneous(true);
    let side = |facts: ItemFacts, label: String| {
        let check = gtk::CheckButton::new();
        check.set_valign(gtk::Align::Start);
        let picture = gtk::Image::new();
        picture.set_pixel_size(THUMBNAIL_SIZE);
        picture.set_valign(gtk::Align::Start);
        picture.set_accessible_role(gtk::AccessibleRole::Presentation);
        picture.add_css_class("file-conflict-picture");
        let (date, size) = side_lines(facts);
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_valign(gtk::Align::Start);
        for text in [&date, &size] {
            let line = gtk::Label::new(Some(text));
            line.set_xalign(0.0);
            lines.append(&line);
        }
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.add_css_class("file-conflict-side");
        content.append(&check);
        content.append(&picture);
        content.append(&lines);
        let click = gtk::GestureClick::new();
        click.connect_released(glib::clone!(
            #[weak]
            check,
            move |gesture, _, x, y| {
                // A click on the check box itself toggles it already.
                let on_check = gesture
                    .widget()
                    .and_then(|content| content.pick(x, y, gtk::PickFlags::DEFAULT))
                    .is_some_and(|picked| {
                        picked == check.clone().upcast::<gtk::Widget>() || picked.is_ancestor(&check)
                    });
                if !on_check {
                    check.set_active(!check.is_active());
                }
            }
        ));
        content.add_controller(click);
        let described = format!("{label}, {date}, {size}");
        check.update_property(&[gtk::accessible::Property::Label(&described)]);
        (content, check, picture)
    };
    let (incoming_side, incoming, incoming_picture) = side(
        conflict.incoming,
        format_message("{name} from {folder}", &[("name", &name), ("folder", source)]),
    );
    let (existing_side, existing, existing_picture) = side(
        conflict.existing,
        format_message(
            "{name} already in {folder}",
            &[("name", &name), ("folder", destination)],
        ),
    );
    sides.attach(&incoming_side, 0, 0, 1, 1);
    sides.attach(&existing_side, 1, 0, 1, 1);
    row.append(&sides);
    let same = conflict.same_date_and_size();
    (
        row,
        RowChecks {
            incoming,
            existing,
            pictures: [incoming_picture, existing_picture],
            same,
        },
    )
}

/// Shows each side's thumbnail, `size` pixels square, when the cache has or can make one, and
/// the type's icon until then or without one.
fn load_pictures(conflict: &NameConflict, pictures: &[gtk::Image; 2], size: i32) {
    let sides = [
        (&conflict.incoming_uri, conflict.incoming),
        (&conflict.existing_uri, conflict.existing),
    ];
    for ((uri, facts), picture) in sides.into_iter().zip(pictures.iter()) {
        let name = conflict.name();
        let icon = if facts.is_folder {
            Icon::FileFolder
        } else {
            FileType::of(&name, None).icon(size)
        };
        icons::set_icon(picture, icon, size);
        if facts.is_folder || !uri.starts_with("file://") {
            continue;
        }
        let uri = uri.clone();
        let modified = facts.modified;
        let picture = picture.clone();
        glib::spawn_future_local(async move {
            let file = gio::File::for_uri(&uri);
            let Ok(info) = file
                .query_info_future(
                    gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::LOW,
                )
                .await
            else {
                return;
            };
            let Some(content_type) = info.content_type() else {
                return;
            };
            let scale = picture.scale_factor().max(1);
            let request = PreviewRequest {
                uri,
                content_type: content_type.to_string(),
                modified,
                pixels: size * scale,
            };
            if let Some(texture) = preview(request).await {
                picture.set_paintable(Some(&texture));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window's words are Windows', singular and plural.
    ///
    /// parity: OPS-028
    #[test]
    fn the_window_takes_windows_wording() {
        assert_eq!(title(1), "1 File Conflict");
        assert_eq!(title(3), "3 File Conflicts");
        assert_eq!(skip_same_label(1), "Skip 1 file with the same date and size");
        assert_eq!(skip_same_label(2), "Skip 2 files with the same date and size");
        let markup = header_markup("Files from {folder}", "file:///home/sample/Down%26loads");
        assert_eq!(
            markup,
            "Files from <a href=\"file:///home/sample/Down%26loads\">Down&amp;loads</a>"
        );
    }

    /// One side replaces or skips, both keep both, and a file skipped for
    /// its date and size is skipped whatever its check boxes say.
    ///
    /// parity: OPS-028
    #[test]
    fn the_check_boxes_answer_as_windows_does() {
        assert_eq!(resolution(true, false, false), Resolution::Replace);
        assert_eq!(resolution(false, true, false), Resolution::Skip);
        assert_eq!(resolution(true, true, false), Resolution::KeepBoth);
        assert_eq!(resolution(true, true, true), Resolution::Skip);
    }
}
