// SPDX-License-Identifier: AGPL-3.0-only
//! "Format `<drive>`": Windows' Format dialog for a removable drive
//! (DEV-012).
//!
//! New in the native app, which formats through UDisks
//! ([`ox_core::integration::format_drive`]) instead of opening GNOME
//! Disks. Like Windows' dialog it shows the capacity and offers the file
//! system, the allocation unit size, "Restore device defaults", the
//! volume label and Quick format, then asks once more, in Windows' words,
//! before anything is erased. Windows' MS-DOS startup disk has no
//! counterpart and is left out. It only asks; the window formats.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{default_file_system, DriveFacts, FileSystem, FormatRequest};

use crate::window::{ButtonStyle, Dialog};

/// The question asked before the drive is erased, in Windows' words.
const WARNING: &str = crate::i18n::message_id(
    "WARNING: Formatting will erase ALL data on this disk.\nTo format the disk, click OK. To quit, click CANCEL.",
);

/// The first allocation unit size listed, which leaves the size to the
/// file system's own `mkfs`.
const DEFAULT_ALLOCATION: &str = crate::i18n::message_id("Default allocation size");

/// An allocation unit size as Windows lists it: bytes up to 8192, then
/// kilobytes.
fn allocation_label(unit: u32) -> String {
    if unit < 16 * 1024 {
        ox_core::i18n::format_message("{size} bytes", &[("size", &unit.to_string())])
    } else {
        ox_core::i18n::format_message("{size} kilobytes", &[("size", &(unit / 1024).to_string())])
    }
}

/// The allocation unit sizes `file_system` offers on a drive of `size`
/// bytes, after the default, as the list shows them.
fn allocation_choices(file_system: FileSystem, size: u64) -> (Vec<Option<u32>>, gtk::StringList) {
    let units: Vec<Option<u32>> = std::iter::once(None)
        .chain(file_system.allocation_units(size).into_iter().map(Some))
        .collect();
    let labels = gtk::StringList::new(&[]);
    for unit in &units {
        match unit {
            Some(unit) => labels.append(&allocation_label(*unit)),
            None => labels.append(&ox_core::i18n::gettext(DEFAULT_ALLOCATION)),
        }
    }
    (units, labels)
}

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
    let dialog = Dialog::with_title_bar(parent, &title, "");
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
    let size = facts.size;
    let (units, labels) = allocation_choices(first, size);
    let units = Rc::new(RefCell::new(units));
    let allocation = gtk::DropDown::new(Some(labels), None::<gtk::Expression>);
    dialog.add_labelled(&ox_core::i18n::gettext("Allocation unit size"), &allocation);
    let restore = gtk::Button::with_label(&ox_core::i18n::gettext("Restore device defaults"));
    restore.set_halign(gtk::Align::Start);
    dialog.add_field(&restore);
    let label = dialog.add_text_field(
        &ox_core::i18n::gettext("Volume label"),
        &first.fitted_label(&facts.label),
    );
    fit_label_field(&label, first);
    let system = Rc::new(Cell::new(first));
    fit_while_typing(&label, Rc::clone(&system));
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
        #[weak]
        allocation,
        #[strong]
        chosen,
        #[strong]
        units,
        move |choice| {
            system.set(chosen(choice));
            fit_label_field(&label, system.get());
            // Each file system has sizes of its own; the default again.
            let (offered, labels) = allocation_choices(system.get(), size);
            units.replace(offered);
            allocation.set_model(Some(&labels));
            allocation.set_selected(0);
        }
    ));
    let quick = dialog.add_check_button(&ox_core::i18n::gettext("Quick format"), true);
    // As Windows' button does: the file system the drive starts with, the
    // default allocation size and a quick format; the label stays.
    restore.connect_clicked(glib::clone!(
        #[weak]
        choice,
        #[weak]
        allocation,
        #[weak]
        quick,
        move |_| {
            choice.set_selected(u32::try_from(first_index).unwrap_or_default());
            allocation.set_selected(0);
            quick.set_active(true);
        }
    ));
    dialog.add_cancel_button();
    dialog.add_button(&ox_core::i18n::gettext("Start"), ButtonStyle::Accent);
    dialog.connect_confirmed(move |dialog| {
        let file_system = chosen(&choice);
        let index = usize::try_from(allocation.selected()).unwrap_or_default();
        let allocation_unit = units.borrow().get(index).copied().flatten();
        on_start(
            dialog,
            FormatRequest {
                file_system,
                label: file_system.fitted_label(&label.text()),
                quick: quick.is_active(),
                allocation_unit,
            },
        );
    });
    dialog
}

/// The Rename dialog of the drive `drive_name` that `facts` describes,
/// over `parent`: its volume label, fitted to what its file system keeps
/// as it is typed, and Rename, which calls `on_rename` with the dialog,
/// to rename in, and the label typed. Windows renames a drive in place,
/// in This PC; here the drive's menu opens this dialog.
pub(crate) fn rename_drive_dialog(
    parent: &impl IsA<gtk::Window>,
    drive_name: &str,
    facts: &DriveFacts,
    on_rename: impl Fn(&Dialog, String) + 'static,
) -> Dialog {
    let title = ox_core::i18n::format_message("Rename {drive}", &[("drive", drive_name)]);
    let dialog = Dialog::with_title_bar(parent, &title, "");
    let current = facts
        .file_system
        .map_or_else(|| facts.label.clone(), |system| system.fitted_label(&facts.label));
    let label = dialog.add_text_field(&ox_core::i18n::gettext("Volume label"), &current);
    if let Some(system) = facts.file_system {
        fit_label_field(&label, system);
        fit_while_typing(&label, Rc::new(Cell::new(system)));
    }
    dialog.add_cancel_button();
    dialog.add_button(&ox_core::i18n::gettext("Rename"), ButtonStyle::Accent);
    dialog.connect_confirmed(move |dialog| on_rename(dialog, label.text().to_string()));
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

/// Keeps what is typed or pasted into the label `field` to what the
/// file system in `system` keeps, as it is inserted, so the field shows
/// the label that is written and Undo takes back what was inserted.
fn fit_while_typing(field: &gtk::Entry, system: Rc<Cell<FileSystem>>) {
    let Some(text) = field.delegate() else {
        return;
    };
    let handler = Rc::new(RefCell::new(None::<glib::SignalHandlerId>));
    let own = Rc::clone(&handler);
    let id = text.connect_insert_text(move |text, inserted, position| {
        let kept = system.get().inserted_label(&text.text(), inserted);
        if kept == inserted {
            return;
        }
        // What the file system keeps is inserted in place of what was.
        if let Some(id) = own.borrow().as_ref() {
            text.block_signal(id);
            text.insert_text(&kept, position);
            text.unblock_signal(id);
        }
        text.stop_signal_emission_by_name("insert-text");
    });
    handler.replace(Some(id));
}

/// Asks, over `dialog`, whether to erase the drive; true for Format.
pub(crate) async fn confirm_erase(dialog: &Dialog) -> bool {
    let question = Dialog::new(
        dialog,
        &ox_core::i18n::gettext("Format this drive?"),
        ox_core::i18n::gettext_static(WARNING),
    );
    question.add_cancel_button();
    let format = question.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Danger);
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
            "Allocation unit size",
            "Default allocation size",
            "Restore device defaults",
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
        let [choice, allocation] =
            <[gtk::DropDown; 2]>::try_from(descendants::<gtk::DropDown>(&dialog)).unwrap();
        let label = descendants::<gtk::Entry>(&dialog).remove(0);
        assert_eq!(choice.selected(), 1, "FAT32, the drive's own");
        assert_eq!(label.text(), "OLD STICK");
        assert_eq!(label.max_length(), 11);
        dialog.press("Start");
        settle();
        choice.set_selected(3);
        allocation.set_selected(2);
        label.set_text("Backups for the studio");
        descendants::<gtk::CheckButton>(&dialog)[0].set_active(false);
        dialog.press("Start");
        settle();

        let expected = [
            FormatRequest {
                file_system: FileSystem::Fat32,
                label: "OLD STICK".into(),
                quick: true,
                allocation_unit: None,
            },
            FormatRequest {
                file_system: FileSystem::Ext4,
                label: "Backups for the".into(),
                quick: false,
                allocation_unit: Some(2048),
            },
        ];
        assert_eq!(*requests.borrow(), expected);
        dialog.close();
        parent.close();
    }

    /// Allocation unit size lists the default first, then the file
    /// system's own sizes in Windows' words; a new file system lists its
    /// own, and "Restore device defaults" puts back the drive's file
    /// system, the default size and a quick format (#133).
    #[gtk::test]
    fn restore_device_defaults_resets_the_file_system_size_and_quick_format() {
        let parent = gtk::Window::new();
        let dialog = format_drive_dialog(&parent, "USB", &usb_stick(), &FileSystem::ALL, |_, _| {});
        let [choice, allocation] =
            <[gtk::DropDown; 2]>::try_from(descendants::<gtk::DropDown>(&dialog)).unwrap();
        let shown = |allocation: &gtk::DropDown| -> Vec<String> {
            let model = allocation
                .model()
                .and_downcast::<gtk::StringList>()
                .expect("a list of sizes");
            (0..model.n_items())
                .filter_map(|index| model.string(index))
                .map(String::from)
                .collect()
        };
        let fat32 = shown(&allocation);
        assert_eq!(fat32[0], "Default allocation size");
        assert!(fat32.contains(&"4096 bytes".to_owned()), "{fat32:?}");
        assert!(fat32.contains(&"32 kilobytes".to_owned()), "{fat32:?}");
        choice.set_selected(3);
        assert_eq!(
            shown(&allocation),
            [
                "Default allocation size",
                "1024 bytes",
                "2048 bytes",
                "4096 bytes"
            ]
        );
        allocation.set_selected(3);
        let quick = descendants::<gtk::CheckButton>(&dialog).remove(0);
        quick.set_active(false);
        descendants::<gtk::Button>(&dialog)
            .into_iter()
            .find(|button| button.label().as_deref() == Some("Restore device defaults"))
            .expect("the Restore device defaults button")
            .emit_clicked();
        settle();
        assert_eq!(choice.selected(), 1, "FAT32, the drive's own");
        assert_eq!(allocation.selected(), 0);
        assert!(quick.is_active());
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

        let [choice, _] = <[gtk::DropDown; 2]>::try_from(descendants::<gtk::DropDown>(&dialog)).unwrap();
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

    /// What is typed into the label is fitted as it is typed, so the
    /// field shows the label Start formats with; spaces stay until Start,
    /// so a word can follow, and Undo still takes back what was typed.
    ///
    /// parity: DEV-012
    #[gtk::test]
    fn the_label_is_fitted_as_it_is_typed() {
        let parent = gtk::Window::new();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&requests);
        let dialog = format_drive_dialog(
            &parent,
            "USB",
            &usb_stick(),
            &FileSystem::ALL,
            move |_, request| heard.borrow_mut().push(request.label),
        );

        let [choice, _] = <[gtk::DropDown; 2]>::try_from(descendants::<gtk::DropDown>(&dialog)).unwrap();
        let label = descendants::<gtk::Entry>(&dialog).remove(0);
        let typing = descendants::<gtk::Text>(&label).remove(0);
        label.set_text("");
        for key in "my: photos".chars() {
            typing.emit_insert_at_cursor(key.to_string());
            settle();
        }
        assert_eq!(label.text(), "MY PHOTOS", "FAT32 refuses ':' and keeps capitals");
        assert_eq!(label.position(), 9, "the cursor stays after the text typed");
        typing.activate_action("text.undo", None).expect("undo");
        assert_eq!(label.text(), "MY", "Undo takes back the last word typed");
        label.set_text("my ");
        dialog.press("Start");
        settle();
        choice.set_selected(3);
        label.set_text("ÄÄÄÄÄÄÄÄÄÄ");
        assert_eq!(label.text(), "ÄÄÄÄÄÄÄÄ", "ext4 keeps 16 bytes");
        dialog.press("Start");
        settle();

        assert_eq!(*requests.borrow(), ["MY", "ÄÄÄÄÄÄÄÄ"]);
        dialog.close();
        parent.close();
    }
}
