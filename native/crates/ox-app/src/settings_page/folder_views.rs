// SPDX-License-Identifier: AGPL-3.0-only
//! The Appearance page's "Files and folders" group: previews (VIEW-058),
//! as Dolphin's Previews settings offer them, Windows 11's Compact view
//! (VIEW-067), Dolphin's view options, and item counts in a folder's Size
//! (VIEW-037), as its "Number of items". Each switch saves its preference
//! or the folder views' options at once, and every window follows.

use gtk::glib;
use ox_core::settings::{PreferencesUpdate, ViewOptions};

use super::bindings::PreferenceBinding;
use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;

const RELATIVE_DATES: RowText = RowText {
    title: "Relative dates",
    description: "Show “Today” and “Yesterday” with the time; off, every date is shown in full.",
    keywords: "date modified time today yesterday absolute short format column",
};

const FOLDER_STYLES: RowText = RowText {
    title: "Remember each folder's view",
    description: "Each folder keeps its own layout, sorting and grouping; off, every folder shares one.",
    keywords: "view properties per folder display style layout sort group remember global",
};

const SELECTION_MARKER: RowText = RowText {
    title: "Selection marker",
    description: "Hovering an item shows a button that adds it to the selection or takes it out.",
    keywords: "check box checkbox select toggle hover marker plus minus item",
};

const EXPANDABLE_FOLDERS: RowText = RowText {
    title: "Expandable folders",
    description: "In the details view, a folder's arrow lists its contents beneath it.",
    keywords: "tree expand collapse arrow chevron details subfolders nested",
};

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

const COMPACT_DENSITY: RowText = RowText {
    title: "Compact view",
    description: "Rows in the file list and the navigation pane stand closer, so more items fit.",
    keywords: "density spacing padding rows tight dense smaller",
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
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Files and folders"));
    add_preview_rows(page, &group);
    let rows = [
        (
            COMPACT_DENSITY,
            PreferenceBinding {
                read: |preferences| preferences.compact_density,
                write: |on| PreferencesUpdate {
                    compact_density: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            RELATIVE_DATES,
            PreferenceBinding {
                read: |preferences| !preferences.absolute_dates,
                write: |on| PreferencesUpdate {
                    absolute_dates: Some(!on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            FOLDER_STYLES,
            PreferenceBinding {
                read: |preferences| preferences.per_folder_views,
                write: |on| PreferencesUpdate {
                    per_folder_views: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            SELECTION_MARKER,
            PreferenceBinding {
                read: |preferences| preferences.selection_marker,
                write: |on| PreferencesUpdate {
                    selection_marker: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
        (
            EXPANDABLE_FOLDERS,
            PreferenceBinding {
                read: |preferences| preferences.expandable_folders,
                write: |on| PreferencesUpdate {
                    expandable_folders: Some(on),
                    ..PreferencesUpdate::default()
                },
            },
        ),
    ];
    for (text, binding) in rows {
        let row = SettingRow::new(text);
        row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
        group.add_row(&row);
    }
    group
}

/// Preview and item-count settings share the same options record.
fn add_preview_rows(page: &SettingsPage, group: &SettingsGroup) {
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
