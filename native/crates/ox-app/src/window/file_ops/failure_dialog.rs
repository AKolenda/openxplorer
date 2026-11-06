// SPDX-License-Identifier: AGPL-3.0-only
//! The questions about an item that failed for a reason other than a
//! name conflict (OPS-047), in Windows 11's words.
//!
//! | Cause | Caption | Says | Buttons |
//! |---|---|---|---|
//! | The item may not be read or written | File Access Denied (Folder …) | You need permission to perform this action | Try Again · Skip · Cancel |
//! | The destination folder may not be written | Destination Folder Access Denied | You'll need to provide administrator permission to copy to this folder | Continue (shield) · Skip · Cancel |
//! | Another program holds it | File In Use (Folder …) | The action can't be completed because the file is open in another program · Close the file and try again. | Try Again · Skip · Cancel |
//! | It is gone | Item Not Found | Could not find this item · This is no longer located in <folder>. Verify the item's location and try again. | Try Again · Skip · Cancel |
//! | Anything else | Interrupted Action | An unexpected error is keeping you from copying the file. … · the system's error | Try Again · Skip · Cancel |
//!
//! Each names the item as Windows does: its icon and name, then its Type,
//! Size and Date modified (a folder's Date created; for the destination
//! folder, the folder's). "Do this for all current items", shown while
//! other items remain, makes Skip skip, and Continue continue, every later
//! item without asking. The system's own words are kept only for the
//! unexpected error, as Windows shows its error text there.
//!
//! Continue, with the administrator shield "Open as administrator" shows,
//! copies or moves the item again into the destination folder under
//! `admin://` ([`ox_core::admin_copy`]). The folder is checked first; one
//! that may be written as administrator is mounted under `admin://`, which
//! is where the desktop asks for the administrator's password, and the
//! engine then runs the item there, checking the folder again. Closing that
//! prompt shows the question again. A folder the check refuses is answered
//! with Continue all the same, without a prompt, so the engine's refusal is
//! asked about in "Interrupted Action" with its reason.
//!
//! More details, at the bottom of every question, shows where the item is
//! and where it was going; whether it is open is kept, as the progress
//! window keeps its own.
//!
//! A full drive asks in the progress window instead, as Windows' "There is
//! not enough space" does ([`TransferPanel::ask_about_space`]).
//!
//! The engine asks from its worker thread and waits (see
//! `worker_question`); a question the window cannot show is answered with
//! Skip, the rule without a question. Cancel, Escape and closing the dialog
//! cancel the operation, keeping what is done.
//!
//! [`TransferPanel::ask_about_space`]: crate::window::transfer_panel::TransferPanel::ask_about_space

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{entry_from_info, Entry, ATTRIBUTES};
use ox_core::format;
use ox_core::i18n::{format_message, gettext};
use ox_core::location::parent_location;
use ox_core::ops::FailureAsker;
use ox_core::settings::PreferencesUpdate;
use ox_core::transfer::{Cancellation, FailedItem, FailureAnswer, FailureCause, TransferMode};

use super::worker_question::worker_question;
use crate::dialog::{Dialog, PropertyGrid};
use crate::icons::{self, FileType, Icon};
use crate::window::transfer_panel::{location_name, SpaceTexts};
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The edge of the item's icon.
const ITEM_ICON_SIZE: i32 = 48;

/// The edge of the More details chevron, as in the progress window.
const CHEVRON_SIZE: i32 = 16;

/// Which of Windows' questions an item gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    /// "File Access Denied", or "Folder Access Denied".
    AccessDenied,
    /// "Destination Folder Access Denied".
    DestinationDenied,
    /// "File In Use", or "Folder In Use".
    InUse,
    /// "Item Not Found".
    NotFound,
    /// "Interrupted Action": an unexpected error.
    Unexpected,
}

/// What the dialog says, in Windows' words.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Wording {
    /// Windows' caption, the dialog's heading and title.
    caption: String,
    /// Windows' main instruction, the first line under it.
    heading: String,
    /// The lines under it, if any.
    message: String,
}

/// The wording of `question` about `item`, a folder when `is_folder`, whose
/// folder reads `folder` where it is shown.
fn wording(question: Question, item: &FailedItem, is_folder: bool, folder: &str) -> Wording {
    let (caption, heading, message) = match question {
        Question::AccessDenied => (
            if is_folder {
                gettext("Folder Access Denied")
            } else {
                gettext("File Access Denied")
            },
            gettext("You need permission to perform this action"),
            String::new(),
        ),
        Question::DestinationDenied => (
            gettext("Destination Folder Access Denied"),
            if item.mode == TransferMode::Move {
                gettext("You'll need to provide administrator permission to move to this folder")
            } else {
                gettext("You'll need to provide administrator permission to copy to this folder")
            },
            String::new(),
        ),
        Question::InUse if is_folder => (
            gettext("Folder In Use"),
            gettext(
                "The action can't be completed because the folder or a file in it is open in another program",
            ),
            gettext("Close the folder or file and try again."),
        ),
        Question::InUse => (
            gettext("File In Use"),
            gettext("The action can't be completed because the file is open in another program"),
            gettext("Close the file and try again."),
        ),
        Question::NotFound => (
            gettext("Item Not Found"),
            gettext("Could not find this item"),
            format_message(
                "This is no longer located in {folder}. Verify the item's location and try again.",
                &[("folder", folder)],
            ),
        ),
        Question::Unexpected => (
            gettext("Interrupted Action"),
            unexpected_heading(item.mode, is_folder),
            item.error.clone(),
        ),
    };
    Wording {
        caption,
        heading,
        message,
    }
}

/// "An unexpected error is keeping you from copying the file. …", by what
/// the run does to a file or a folder.
fn unexpected_heading(mode: TransferMode, is_folder: bool) -> String {
    let template = match (mode, is_folder) {
        (TransferMode::Copy, false) => crate::i18n::message_id("An unexpected error is keeping you from copying the file. If you continue to receive this error, you can use the error code to search for help with this problem."),
        (TransferMode::Copy, true) => crate::i18n::message_id("An unexpected error is keeping you from copying the folder. If you continue to receive this error, you can use the error code to search for help with this problem."),
        (TransferMode::Move, false) => crate::i18n::message_id("An unexpected error is keeping you from moving the file. If you continue to receive this error, you can use the error code to search for help with this problem."),
        (TransferMode::Move, true) => crate::i18n::message_id("An unexpected error is keeping you from moving the folder. If you continue to receive this error, you can use the error code to search for help with this problem."),
        (TransferMode::Trash | TransferMode::Delete, false) => crate::i18n::message_id("An unexpected error is keeping you from deleting the file. If you continue to receive this error, you can use the error code to search for help with this problem."),
        (TransferMode::Trash | TransferMode::Delete, true) => crate::i18n::message_id("An unexpected error is keeping you from deleting the folder. If you continue to receive this error, you can use the error code to search for help with this problem."),
    };
    gettext(template)
}

/// "There is not enough space on {drive}.", then what `needed` bytes the
/// run needs, as Windows says it.
fn space_texts(
    mode: TransferMode,
    drive: &str,
    needed: Option<u64>,
    space: Option<(u64, u64)>,
) -> SpaceTexts {
    let summary = format_message("There is not enough space on {drive}.", &[("drive", drive)]);
    let advice = gettext("Delete or move files so you have enough space.");
    let needed = match needed {
        Some(bytes) => {
            let template = if mode == TransferMode::Move {
                crate::i18n::message_id("{size} is needed to move this item.")
            } else {
                crate::i18n::message_id("{size} is needed to copy this item.")
            };
            let size = format::pretty_bytes(bytes);
            format!("{} {advice}", format_message(template, &[("size", &size)]))
        }
        None => advice,
    };
    let drive = space
        .map(|(free, total)| {
            vec![
                format_message("Space free: {size}", &[("size", &format::pretty_bytes(free))]),
                format_message("Total size: {size}", &[("size", &format::pretty_bytes(total))]),
            ]
        })
        .unwrap_or_default();
    SpaceTexts {
        summary,
        needed,
        drive,
    }
}

/// What `entry` shows under its name: "Type", "Size" and "Date modified"
/// for a file, "Date created" for a folder, each only when known, as
/// Explorer's infotip and its questions list them.
fn facts_lines(entry: &Entry) -> Vec<String> {
    let mut lines = Vec::new();
    if entry.is_dir {
        if let Some(created) = entry.meta.created {
            let date = format::date_short_time_text(Some(created));
            lines.push(format_message("Date created: {date}", &[("date", &date)]));
        }
        return lines;
    }
    if !entry.type_label.is_empty() {
        lines.push(format_message("Type: {type}", &[("type", &entry.type_label)]));
    }
    if let Some(size) = entry.size {
        let size = format::size_text(Some(size));
        lines.push(format_message("Size: {size}", &[("size", &size)]));
    }
    if entry.modified.is_some() {
        let date = format::date_short_time_text(entry.modified);
        lines.push(format_message("Date modified: {date}", &[("date", &date)]));
    }
    lines
}

/// The entry at `uri`, `None` when it cannot be read, as an item that is
/// gone.
async fn read_entry(uri: &str) -> Option<Entry> {
    if uri.is_empty() {
        return None;
    }
    let file = gio::File::for_uri(uri);
    let info = file
        .query_info_future(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    Some(entry_from_info(&file, &info))
}

/// Whether the folder at `uri` may be written to; true when that cannot be
/// read, so an unknown answer never blames the destination.
async fn can_write(uri: &str) -> bool {
    let info = gio::File::for_uri(uri)
        .query_info_future(
            gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await;
    match info {
        Ok(info) if info.has_attribute(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE) => {
            info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE)
        }
        _ => true,
    }
}

/// The drive that holds `uri`, as the question names it: its mount's name,
/// or "Local Disk" for the system's own disk; and its free and total
/// bytes, when known.
async fn drive_of(uri: &str) -> (String, Option<(u64, u64)>) {
    let file = gio::File::for_uri(uri);
    // Local and GVfs mounts answer from their mount tables, at once.
    let name = match file.find_enclosing_mount(gio::Cancellable::NONE) {
        Ok(mount) => mount.name().to_string(),
        Err(_) if uri.starts_with("file://") => gettext("Local Disk"),
        Err(_) => location_name(uri),
    };
    let space = file
        .query_filesystem_info_future(
            &format!(
                "{},{}",
                gio::FILE_ATTRIBUTE_FILESYSTEM_FREE,
                gio::FILE_ATTRIBUTE_FILESYSTEM_SIZE
            ),
            glib::Priority::DEFAULT,
        )
        .await
        .ok()
        .filter(|info| {
            info.has_attribute(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE)
                && info.has_attribute(gio::FILE_ATTRIBUTE_FILESYSTEM_SIZE)
        })
        .map(|info| {
            (
                info.attribute_uint64(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE),
                info.attribute_uint64(gio::FILE_ATTRIBUTE_FILESYSTEM_SIZE),
            )
        });
    (name, space)
}

/// The item's icon and name, with `lines` under the name, as Windows'
/// questions show the item.
fn item_card(name: &str, icon: Icon, lines: &[String]) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    card.add_css_class("failed-item");
    let image = gtk::Image::new();
    image.set_valign(gtk::Align::Start);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    icons::set_icon(&image, icon, ITEM_ICON_SIZE);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    // The facts take the card's width rather than wrapping at their
    // narrowest.
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(name));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_selectable(true);
    title.add_css_class("failed-item-name");
    text.append(&title);
    for line in lines {
        let label = gtk::Label::new(Some(line));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.append(&label);
    }
    card.append(&image);
    card.append(&text);
    card
}

/// The icon of `entry`, called `name`: a folder's, or its type's.
fn icon_of(entry: Option<&Entry>, name: &str) -> Icon {
    match entry {
        Some(entry) if entry.is_dir => Icon::FileFolder,
        Some(entry) => FileType::of(name, entry.content_type.as_deref()).icon(ITEM_ICON_SIZE),
        None => FileType::of(name, None).icon(ITEM_ICON_SIZE),
    }
}

/// The flat "More details" toggle at the bottom left of a question.
fn details_toggle() -> gtk::Button {
    let toggle = gtk::Button::new();
    toggle.add_css_class("flat");
    toggle.add_css_class("failure-details-toggle");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let image = gtk::Image::new();
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    content.append(&image);
    content.append(&gtk::Label::new(None));
    toggle.set_child(Some(&content));
    toggle
}

/// Shows or hides `panel`, and words and draws `toggle` to match:
/// "More details" with a chevron down, or "Fewer details" with one up.
fn set_details(toggle: &gtk::Button, panel: &impl IsA<gtk::Widget>, shown: bool) {
    panel.set_visible(shown);
    let (icon, text) = if shown {
        (Icon::ChevronUp16, gettext("Fewer details"))
    } else {
        (Icon::ChevronDown16, gettext("More details"))
    };
    let content = toggle.child().and_downcast::<gtk::Box>();
    if let Some(content) = content {
        if let Some(image) = content.first_child().and_downcast::<gtk::Image>() {
            icons::set_icon(&image, icon, CHEVRON_SIZE);
        }
        if let Some(label) = content.last_child().and_downcast::<gtk::Label>() {
            label.set_text(&text);
        }
    }
    toggle.update_state(&[gtk::accessible::State::Expanded(Some(shown))]);
}

/// The last part of `name`, an item's path below the item the run handles.
fn last_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

impl BrowserWindow {
    /// An asker for the worker of the operation `cancel` stops: each
    /// question opens over this window, or in that operation's action for a
    /// full drive, and waits.
    pub(in crate::window) fn failure_asker(&self, cancel: &Cancellation) -> FailureAsker {
        let cancel = cancel.clone();
        FailureAsker::new(worker_question(
            self,
            FailureAnswer::Skip,
            move |window, item: FailedItem| {
                let cancel = cancel.clone();
                async move { window.ask_about_failure(&item, &cancel).await }
            },
        ))
    }

    /// Asks about `item`; Cancel, Escape and closing the dialog cancel the
    /// operation.
    async fn ask_about_failure(&self, item: &FailedItem, cancel: &Cancellation) -> FailureAnswer {
        if item.cause == FailureCause::NoSpace {
            return self.ask_about_space(item, cancel).await;
        }
        let (question, shown_uri) = self.question_about(item).await;
        let entry = read_entry(&shown_uri).await;
        let is_folder = entry.as_ref().is_some_and(|entry| entry.is_dir);
        let folder = parent_location(&item.uri)
            .map(|folder| self.imp().locations.borrow().display_location(&folder))
            .unwrap_or_default();
        let wording = wording(question, item, is_folder, &folder);
        let name = match (question, &entry) {
            (Question::DestinationDenied, Some(entry)) => entry.name.clone(),
            (Question::DestinationDenied, None) => location_name(&shown_uri),
            _ => last_name(&item.name).to_owned(),
        };
        // Windows' caption is the dialog's heading here, as the dialogs
        // draw no title bar of their own; its instruction follows.
        let text = [wording.heading.as_str(), wording.message.as_str()]
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let lines = entry.as_ref().map(facts_lines).unwrap_or_default();
        let details = self.failure_details(item);
        loop {
            let dialog = Dialog::new(self, &wording.caption, &text);
            dialog.add_field(&item_card(&name, icon_of(entry.as_ref(), &name), &lines));
            let for_all = item
                .more_items
                .then(|| dialog.add_check_button(&gettext("Do this for all current items"), false));
            self.add_details(&dialog, &details);
            // Windows' Continue asks for administrator permission; the
            // other questions keep Try Again.
            let first = if question == Question::DestinationDenied {
                dialog.add_button_with_icon(&gettext("Continue"), Icon::ShieldLock, ButtonStyle::Accent)
            } else {
                dialog.add_button(&gettext("Try Again"), ButtonStyle::Accent)
            };
            let skip = dialog.add_button(&gettext("Skip"), ButtonStyle::Bordered);
            dialog.add_cancel_button();
            dialog.open();
            let pressed = dialog.next_response().await;
            let for_all = for_all.is_some_and(|check| check.is_active());
            dialog.finish();
            return match pressed {
                Some(button) if button == first && question == Question::DestinationDenied => {
                    let destination = item.destination_folder.as_deref().unwrap_or_default();
                    if !self.authorize_administrator(destination).await {
                        // The administrator prompt was closed: ask again.
                        continue;
                    }
                    if for_all {
                        FailureAnswer::ElevateAll
                    } else {
                        FailureAnswer::Elevate
                    }
                }
                Some(button) if button == first => FailureAnswer::Retry,
                Some(button) if button == skip && for_all => FailureAnswer::SkipAll,
                Some(button) if button == skip => FailureAnswer::Skip,
                _ => FailureAnswer::Cancel,
            };
        }
    }

    /// What More details shows about `item`: where it is, and the folder
    /// a copy or move takes it to, as the user sees each location.
    fn failure_details(&self, item: &FailedItem) -> Vec<(String, String)> {
        let locations = self.imp().locations.borrow();
        let mut rows = Vec::new();
        if !item.uri.is_empty() {
            rows.push((gettext("Source"), locations.display_location(&item.uri)));
        }
        if let Some(destination) = &item.destination_folder {
            rows.push((gettext("Destination"), locations.display_location(destination)));
        }
        rows
    }

    /// Adds More details to `dialog`: a toggle at the bottom left that
    /// shows or hides `rows` under the item, open as it was last left.
    fn add_details(&self, dialog: &Dialog, rows: &[(String, String)]) {
        if rows.is_empty() {
            return;
        }
        let grid = PropertyGrid::new();
        for (name, value) in rows {
            grid.add_row(name, value);
        }
        let shown = self.context().settings_data().preferences.failure_details;
        let panel = grid.widget().clone();
        panel.add_css_class("failure-details");
        dialog.add_field(&panel);
        let toggle = details_toggle();
        set_details(&toggle, &panel, shown);
        let context = self.context().clone();
        toggle.connect_clicked(move |toggle| {
            let shown = !panel.is_visible();
            set_details(toggle, &panel, shown);
            context.remember_preferences(PreferencesUpdate {
                failure_details: Some(shown),
                ..PreferencesUpdate::default()
            });
        });
        dialog.add_footer_start(&toggle);
    }

    /// Readies Continue for the destination folder at `destination`:
    /// mounts the folder under `admin://`, where the desktop asks for the
    /// administrator's password. False when the user closed that prompt.
    ///
    /// A folder that may not be written as administrator is not mounted,
    /// so no password is asked for: the engine refuses it, with its
    /// reason, in the next question.
    async fn authorize_administrator(&self, destination: &str) -> bool {
        let Ok(admin) = ox_core::admin_copy::elevated_destination(destination) else {
            return true;
        };
        let file = gio::File::for_uri(&admin);
        let probe = file
            .query_info_future(
                gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await;
        if !matches!(&probe, Err(error) if error.matches(gio::IOErrorEnum::NotMounted)) {
            return true;
        }
        let operation = gtk::MountOperation::new(Some(self));
        let mounted = file
            .mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(&operation))
            .await;
        // Anything but the user closing the prompt is the engine's to
        // report, in the next question.
        !matches!(&mounted, Err(error)
            if error.matches(gio::IOErrorEnum::Cancelled)
                || error.matches(gio::IOErrorEnum::FailedHandled))
    }

    /// Which question `item` gets, and the URI of what it shows: the item,
    /// or the destination folder that may not be written to.
    async fn question_about(&self, item: &FailedItem) -> (Question, String) {
        match item.cause {
            FailureCause::AccessDenied => {
                if let Some(destination) = &item.destination_folder {
                    let writes = matches!(item.mode, TransferMode::Copy | TransferMode::Move);
                    if writes && !can_write(destination).await {
                        return (Question::DestinationDenied, destination.clone());
                    }
                }
                (Question::AccessDenied, item.uri.clone())
            }
            FailureCause::InUse => (Question::InUse, item.uri.clone()),
            FailureCause::NotFound => (Question::NotFound, item.uri.clone()),
            FailureCause::NoSpace | FailureCause::Other => (Question::Unexpected, item.uri.clone()),
        }
    }

    /// Asks in the operation's action whether to try a copy or move the
    /// destination's drive has no room for again: Try Again or Cancel, as
    /// Windows offers no Skip there.
    async fn ask_about_space(&self, item: &FailedItem, cancel: &Cancellation) -> FailureAnswer {
        let destination = item.destination_folder.clone().unwrap_or_default();
        let (drive, space) = drive_of(&destination).await;
        let needed = match item.needed {
            Some(bytes) => Some(bytes),
            None => read_entry(&item.uri)
                .await
                .filter(|entry| !entry.is_dir)
                .and_then(|entry| entry.size),
        };
        let texts = space_texts(item.mode, &drive, needed, space);
        let panel = self.panel_for_operation(cancel);
        if panel.ask_about_space(&texts).await {
            FailureAnswer::Retry
        } else {
            FailureAnswer::Cancel
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(mode: TransferMode, cause: FailureCause) -> FailedItem {
        FailedItem {
            mode,
            name: "Reports/2026/plan.docx".into(),
            error: "Input/output error".into(),
            more_items: true,
            cause,
            uri: "file:///home/sample/Reports/2026/plan.docx".into(),
            destination_folder: Some("file:///media/sample/USB".into()),
            needed: None,
        }
    }

    /// Each failure takes Windows 11's caption and words.
    ///
    /// parity: OPS-047
    #[test]
    fn each_failure_takes_windows_wording() {
        let copy = |cause| item(TransferMode::Copy, cause);
        let denied = wording(
            Question::AccessDenied,
            &copy(FailureCause::AccessDenied),
            false,
            "",
        );
        assert_eq!(denied.caption, "File Access Denied");
        assert_eq!(denied.heading, "You need permission to perform this action");
        assert_eq!(
            wording(
                Question::AccessDenied,
                &copy(FailureCause::AccessDenied),
                true,
                ""
            )
            .caption,
            "Folder Access Denied"
        );
        let destination = wording(
            Question::DestinationDenied,
            &copy(FailureCause::AccessDenied),
            true,
            "",
        );
        assert_eq!(destination.caption, "Destination Folder Access Denied");
        assert_eq!(
            destination.heading,
            "You'll need to provide administrator permission to copy to this folder"
        );
        let in_use = wording(Question::InUse, &copy(FailureCause::InUse), false, "");
        assert_eq!(
            (
                in_use.caption.as_str(),
                in_use.heading.as_str(),
                in_use.message.as_str()
            ),
            (
                "File In Use",
                "The action can't be completed because the file is open in another program",
                "Close the file and try again."
            )
        );
        let gone = wording(
            Question::NotFound,
            &copy(FailureCause::NotFound),
            false,
            "/home/sample/Reports/2026",
        );
        assert_eq!(gone.caption, "Item Not Found");
        assert_eq!(gone.heading, "Could not find this item");
        assert_eq!(
            gone.message,
            "This is no longer located in /home/sample/Reports/2026. Verify the item's location and try again."
        );
        let unexpected = wording(Question::Unexpected, &copy(FailureCause::Other), false, "");
        assert_eq!(unexpected.caption, "Interrupted Action");
        assert!(unexpected
            .heading
            .starts_with("An unexpected error is keeping you from copying the file."));
        assert_eq!(unexpected.message, "Input/output error");
    }

    /// A full drive says how much the item needs and what to do, as
    /// Windows does, and the drive's space when known.
    ///
    /// parity: OPS-047, XFER-028
    #[test]
    fn a_full_drive_says_what_is_needed() {
        let texts = space_texts(
            TransferMode::Copy,
            "USB Drive",
            Some(4_400_000_000),
            Some((3_800_000_000, 7_500_000_000)),
        );
        assert_eq!(texts.summary, "There is not enough space on USB Drive.");
        assert_eq!(
            texts.needed,
            format!(
                "{} is needed to copy this item. Delete or move files so you have enough space.",
                format::pretty_bytes(4_400_000_000)
            )
        );
        assert_eq!(texts.drive.len(), 2);
        assert!(texts.drive[0].starts_with("Space free: "));
        let unknown = space_texts(TransferMode::Move, "Local Disk", None, None);
        assert_eq!(unknown.needed, "Delete or move files so you have enough space.");
        assert!(unknown.drive.is_empty());
    }
}
