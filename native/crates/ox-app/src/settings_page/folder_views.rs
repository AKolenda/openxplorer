// SPDX-License-Identifier: AGPL-3.0-only
//! The Appearance page's "Files and folders" group: previews (VIEW-058),
//! as Dolphin's Previews settings offer them, and item counts in a
//! folder's Size (VIEW-037), as its "Number of items". Each switch saves
//! the folder views' options at once, and every window follows.

use gtk::glib;
use ox_core::settings::{PreferencesUpdate, ViewOptions};

use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;

const SHOW_PREVIEWS: RowText = RowText {
    title: "Show previews",
    description: "Pictures, videos and documents show their contents instead of an icon.",
    keywords: "thumbnails thumbnail image photo preview icons cache",
};

const REMOTE_PREVIEWS: RowText = RowText {
    title: "Show previews in network folders",
    description: "Off: files on SMB shares, servers and phones keep their icons, so browsing \
                  them reads no file contents.",
    keywords: "thumbnails remote network smb nas sftp slow",
};

const LARGE_PREVIEWS: RowText = RowText {
    title: "Skip previews of large files",
    description: "Files larger than 50 MB keep their icon.",
    keywords: "thumbnails size limit big files",
};

const PREVIEW_PICTURES: RowText = RowText {
    title: "Preview pictures",
    description: "Photos, drawings and other images.",
    keywords: "thumbnails types plugins jpeg png images",
};

const PREVIEW_VIDEOS: RowText = RowText {
    title: "Preview videos",
    description: "A frame of each video, made by the desktop's thumbnailer.",
    keywords: "thumbnails types plugins movies films",
};

const PREVIEW_DOCUMENTS: RowText = RowText {
    title: "Preview documents and other files",
    description: "PDFs, office documents, fonts and every other type the desktop can preview.",
    keywords: "thumbnails types plugins pdf office fonts",
};

const ITEM_COUNTS: RowText = RowText {
    title: "Show the number of items in folders",
    description: "The Size column says how many items a folder on this computer holds.",
    keywords: "details folder size count items contents",
};

/// How a switch reads and changes one of the folder views' options.
#[derive(Clone, Copy)]
struct ViewOptionBinding {
    read: fn(&ViewOptions) -> bool,
    write: fn(&mut ViewOptions, bool),
}

/// The "Files and folders" group.
pub(super) fn group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Files and folders");
    let rows = [
        (
            SHOW_PREVIEWS,
            ViewOptionBinding {
                read: |options| options.show_previews,
                write: |options, on| options.show_previews = on,
            },
        ),
        (
            REMOTE_PREVIEWS,
            ViewOptionBinding {
                read: |options| options.remote_previews,
                write: |options, on| options.remote_previews = on,
            },
        ),
        (
            LARGE_PREVIEWS,
            ViewOptionBinding {
                read: |options| options.skip_large_previews,
                write: |options, on| options.skip_large_previews = on,
            },
        ),
        (
            PREVIEW_PICTURES,
            ViewOptionBinding {
                read: |options| options.preview_pictures,
                write: |options, on| options.preview_pictures = on,
            },
        ),
        (
            PREVIEW_VIDEOS,
            ViewOptionBinding {
                read: |options| options.preview_videos,
                write: |options, on| options.preview_videos = on,
            },
        ),
        (
            PREVIEW_DOCUMENTS,
            ViewOptionBinding {
                read: |options| options.preview_documents,
                write: |options, on| options.preview_documents = on,
            },
        ),
        (
            ITEM_COUNTS,
            ViewOptionBinding {
                read: |options| options.count_folder_items,
                write: |options, on| options.count_folder_items = on,
            },
        ),
    ];
    for (text, binding) in rows {
        let row = SettingRow::new(text);
        row.add_control(&view_option_switch(page, binding), ControlName::RowTitle);
        group.add_row(&row);
    }
    group
}

/// A switch showing the option `binding` reads, which saves the user's
/// changes into the current options.
fn view_option_switch(page: &SettingsPage, binding: ViewOptionBinding) -> gtk::Switch {
    let switch = parts::switch();
    let read = binding.read;
    page.follow_preferences(glib::clone!(
        #[weak]
        switch,
        move |preferences| switch.set_active(read(&preferences.view_options))
    ));
    let write = binding.write;
    switch.connect_active_notify(glib::clone!(
        #[weak]
        page,
        move |switch| {
            if !page.is_user_change() {
                return;
            }
            let mut options = page.context().settings_data().preferences.view_options;
            write(&mut options, switch.is_active());
            page.save_preferences(PreferencesUpdate {
                view_options: Some(options),
                ..PreferencesUpdate::default()
            });
        }
    ));
    switch
}
