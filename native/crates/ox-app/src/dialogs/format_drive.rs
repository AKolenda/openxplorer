// SPDX-License-Identifier: AGPL-3.0-only
//! "Format `<drive>`": Windows' Format dialog for a removable drive
//! (DEV-012).
//!
//! New in the native app, which formats through UDisks
//! ([`ox_core::integration::format_drive`]) instead of opening GNOME
//! Disks. Like Windows' dialog it shows the capacity and offers the file
//! system, the volume label and Quick format, then asks once more before
//! anything is erased. It only asks; the window formats.

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{default_file_system, DriveFacts, FileSystem, FormatRequest};

use crate::window::{ButtonStyle, Dialog};

/// The question asked before the drive is erased.
const WARNING: &str = crate::i18n::message_id(
    "Formatting erases all data on this drive. Copy anything you want to keep to another drive first.",
);

/// The Format dialog for the drive `drive_name` that `facts` describes,
/// over `parent`, offering the file systems in `available` (at least
/// one). Its Start button calls `on_start` with the dialog, to format in,
/// and what to make.
pub(crate) fn format_drive_dialog(
    parent: &impl IsA<gtk::Window>,
    drive_name: &str,
    facts: &DriveFacts,
    available: &[FileSystem],
    on_start: impl Fn(&Dialog, FormatRequest) + 'static,
) -> Dialog {
    let title = ox_core::i18n::format_message("Format {drive}", &[("drive", drive_name)]);
    let dialog = Dialog::new(parent, &title, "");
    let capacity = gtk::Label::builder()
        .label(ox_core::format::pretty_bytes(facts.size))
        .xalign(0.0)
        .selectable(true)
        .build();
    dialog.add_labelled(&ox_core::i18n::gettext("Capacity"), &capacity);
    let available = available.to_vec();
    let names: Vec<&str> = available.iter().map(|system| system.name()).collect();
    let choice = gtk::DropDown::from_strings(&names);
    let first = default_file_system(facts.file_system, facts.size, &available).unwrap_or(available[0]);
    let first_index = available
        .iter()
        .position(|system| *system == first)
        .unwrap_or_default();
    choice.set_selected(u32::try_from(first_index).unwrap_or_default());
    dialog.add_labelled(&ox_core::i18n::gettext("File system"), &choice);
    let label = dialog.add_text_field(
        &ox_core::i18n::gettext("Volume label"),
        &first.fitted_label(&facts.label),
    );
    fit_label_field(&label, first);
    let chosen = {
        let available = available.clone();
        move |choice: &gtk::DropDown| {
            let index = usize::try_from(choice.selected()).unwrap_or_default();
            available.get(index).copied().unwrap_or(first)
        }
    };
    choice.connect_selected_notify(glib::clone!(
        #[weak]
        label,
        #[strong]
        chosen,
        move |choice| fit_label_field(&label, chosen(choice))
    ));
    let quick = dialog.add_check_button(&ox_core::i18n::gettext("Quick format"), true);
    dialog.add_cancel_button();
    dialog.add_button(&ox_core::i18n::gettext("Start"), ButtonStyle::Accent);
    dialog.connect_confirmed(move |dialog| {
        let file_system = chosen(&choice);
        on_start(
            dialog,
            FormatRequest {
                file_system,
                label: file_system.fitted_label(&label.text()),
                quick: quick.is_active(),
            },
        );
    });
    dialog
}

/// Limits the label `field` to what `file_system` keeps, cutting the
/// text already typed.
fn fit_label_field(field: &gtk::Entry, file_system: FileSystem) {
    let length = i32::try_from(file_system.max_label_length()).unwrap_or(i32::MAX);
    field.set_max_length(length);
    let fitted = file_system.fitted_label(&field.text());
    if field.text() != fitted {
        field.set_text(&fitted);
    }
}

/// Asks, over `dialog`, whether to erase the drive; true for Format.
pub(crate) async fn confirm_erase(dialog: &Dialog) -> bool {
    let question = Dialog::new(
        dialog,
        &ox_core::i18n::gettext("Format this drive?"),
        ox_core::i18n::gettext_static(WARNING),
    );
    question.add_cancel_button();
    let format = question.add_button(&ox_core::i18n::gettext("Format"), ButtonStyle::Danger);
    // Without a field Cancel has focus, so Enter does not erase the drive.
    question.open();
    let answer = question.next_response().await;
    if answer.is_some() {
        question.finish();
    }
    answer == Some(format)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::test_support::harness::{descendants, settle};

    /// A 16 GB FAT32 drive labelled "old stick".
    fn usb_stick() -> DriveFacts {
        DriveFacts {
            object: "/org/freedesktop/UDisks2/block_devices/sdb1".into(),
            drive: "/org/freedesktop/UDisks2/drives/SanDisk_Cruzer_4C530001".into(),
            uuid: "4E2A-1B3C".into(),
            size: 16_000_000_000,
            label: "old stick".into(),
            file_system: Some(FileSystem::Fat32),
            read_only: false,
            has_file_system: true,
        }
    }

    /// The dialog shows the drive's capacity, keeps its file system and
    /// label, and Start asks for what the fields say.
    ///
    /// parity: DEV-012
    #[gtk::test]
    fn the_dialog_offers_the_drives_file_system_label_and_quick_format() {
        let parent = gtk::Window::new();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&requests);
        let dialog = format_drive_dialog(
            &parent,
            "USB",
            &usb_stick(),
            &FileSystem::ALL,
            move |_, request| heard.borrow_mut().push(request),
        );

        let texts = dialog.texts();
        for expected in [
            "Format USB",
            "Capacity",
            "14.9 GB",
            "File system",
            "Volume label",
            "Quick format",
            "Cancel",
            "Start",
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected} in {texts:?}"
            );
        }
        let [choice] = <[gtk::DropDown; 1]>::try_from(descendants::<gtk::DropDown>(&dialog)).unwrap();
        let label = descendants::<gtk::Entry>(&dialog).remove(0);
        assert_eq!(choice.selected(), 1, "FAT32, the drive's own");
        assert_eq!(label.text(), "OLD STICK");
        assert_eq!(label.max_length(), 11);
        dialog.press("Start");
        settle();
        choice.set_selected(3);
        label.set_text("Backups for the studio");
        descendants::<gtk::CheckButton>(&dialog)[0].set_active(false);
        dialog.press("Start");
        settle();

        let expected = [
            FormatRequest {
                file_system: FileSystem::Fat32,
                label: "OLD STICK".into(),
                quick: true,
            },
            FormatRequest {
                file_system: FileSystem::Ext4,
                label: "Backups for the".into(),
                quick: false,
            },
        ];
        assert_eq!(*requests.borrow(), expected);
        dialog.close();
        parent.close();
    }

    /// Only the file systems the system can make are listed, and a new
    /// file system cuts the label already typed to its length.
    ///
    /// parity: DEV-012
    #[gtk::test]
    fn the_dialog_lists_what_can_be_made_and_fits_the_label() {
        let parent = gtk::Window::new();
        let facts = DriveFacts {
            file_system: None,
            label: String::new(),
            size: 64_000_000_000,
            ..usb_stick()
        };
        let available = [FileSystem::ExFat, FileSystem::Fat32];
        let dialog = format_drive_dialog(&parent, "USB", &facts, &available, |_, _| {});

        let [choice] = <[gtk::DropDown; 1]>::try_from(descendants::<gtk::DropDown>(&dialog)).unwrap();
        let label = descendants::<gtk::Entry>(&dialog).remove(0);
        assert_eq!(choice.model().map(|model| model.n_items()), Some(2));
        assert_eq!(choice.selected(), 0, "exFAT for a drive above 32 GB");
        label.set_text("Holiday photos");
        choice.set_selected(1);
        assert_eq!(label.text(), "HOLIDAY PHO");
        assert_eq!(label.max_length(), 11);
        dialog.close();
        parent.close();
    }
}
