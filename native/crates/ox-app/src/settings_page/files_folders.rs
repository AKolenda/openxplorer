// SPDX-License-Identifier: AGPL-3.0-only
//! Files & folders: Dolphin's view options, item counts in a folder's Size
//! (VIEW-037), as its "Number of items", folder sizes (the Python "Folder
//! sizes" section, SET-008), and previews (VIEW-058), as Dolphin's
//! Previews settings offer them, folded away. Windows 11's Compact view
//! (VIEW-067) is built here and shown on the Appearance page. Each switch
//! saves its preference or the folder views' options at once, and every
//! window follows.

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, ViewOptions};

use super::bindings::PreferenceBinding;
use super::group::SettingsGroup;
use super::pages::{Category, SettingsView, Subpage};
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;

const RELATIVE_DATES: RowText = RowText {
    title: "Relative dates (Today, Yesterday)",
    description: "Show “Today” and “Yesterday” with the time; off, every date is shown in full.",
    keywords: "relative dates date modified time today yesterday absolute short format column",
};

const FOLDER_STYLES: RowText = RowText {
    title: "Remember each folder's view",
    description: "Each folder keeps its own layout, sorting and grouping; off, every folder shares one.",
    keywords: "view properties per folder display style layout sort group remember global",
};

const SELECTION_MARKER: RowText = RowText {
    title: "Selection marker on hover",
    description: "Hovering an item shows a button that adds it to the selection or takes it out.",
    keywords: "selection marker check box checkbox select toggle hover marker plus minus item",
};

const EXPANDABLE_FOLDERS: RowText = RowText {
    title: "Expandable folders in Details",
    description: "In the details view, a folder's arrow lists its contents beneath it.",
    keywords: "expandable folders tree expand collapse arrow chevron details subfolders nested",
};

const SHOW_PREVIEWS: RowText = RowText {
    title: "Show previews",
    description: "Pictures, videos and documents show their contents instead of an icon.",
    keywords: "thumbnails thumbnail image photo preview icons cache",
};

const REMOTE_PREVIEWS: RowText = RowText {
    title: "Also in network folders",
    description: "Off: files on SMB shares, servers and phones keep their icons, so browsing \
                  them reads no file contents.",
    keywords: "show previews in network folders thumbnails remote network smb nas sftp slow",
};

const LARGE_PREVIEWS: RowText = RowText {
    title: "Skip very large files",
    description: "Files larger than 50 MB keep their icon.",
    keywords: "skip previews of large files thumbnails size limit big files",
};

const PREVIEW_PICTURES: RowText = RowText {
    title: "Pictures",
    description: "Photos, drawings and other images.",
    keywords: "preview pictures thumbnails types plugins jpeg png images",
};

const PREVIEW_VIDEOS: RowText = RowText {
    title: "Videos",
    description: "A frame of each video, made by the desktop's thumbnailer.",
    keywords: "preview videos thumbnails types plugins movies films",
};

const PREVIEW_DOCUMENTS: RowText = RowText {
    title: "Documents and other files",
    description: "PDFs, office documents, fonts and every other type the desktop can preview.",
    keywords: "preview documents and other files thumbnails types plugins pdf office fonts",
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

const FOLDER_SIZES: RowText = RowText {
    title: "Calculate folder sizes",
    description: "Right-click a folder → Calculate folder size. Results last for this session.",
    keywords: "zfs logical bytes snapshots disk usage. Results are kept only for this window \
               session.",
};

const HOW_SIZES_ARE_COUNTED: RowText = RowText {
    title: "How sizes are counted",
    description: "Logical file bytes, what a scan skips, and its limits.",
    keywords: "folder sizes scans run on demand outside the browsing worker pool compressed zfs \
               space snapshot usage hidden files links nested filesystem mounts snapshot \
               collections 1 million entries 5 minutes partial total cancel recalculate",
};

/// The two paragraphs of the Python "Folder sizes" section.
const FOLDER_SIZES_HELP: [&str; 2] = [
    "Right-click a folder → Calculate folder size. Scans run on demand, outside the browsing worker \
     pool. Results are logical file bytes, not compressed ZFS space or snapshot usage. They are \
     kept only for this window session. Recalculate to pick up later changes.",
    "Scans include hidden files, but skip links, nested filesystem mounts and snapshot \
     collections. Each folder is limited to 1 million entries or 5 minutes between I/O calls. \
     Inaccessible or excluded entries produce a partial total. Cancel stops the active scan and \
     any queued folders.",
];

/// How a switch reads and changes one of the folder views' options.
#[derive(Clone, Copy)]
struct ViewOptionBinding {
    read: fn(&ViewOptions) -> bool,
    write: fn(&mut ViewOptions, bool),
}

/// The Files & folders page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::FilesAndFolders;
    let files = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    files.append_group(&folder_views_group(page));
    files.append_group(&details_group(page));
    files.append_group(&previews_group(page));
    files
}

/// "Compact view", which the Appearance page shows.
pub(super) fn compact_view_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(COMPACT_DENSITY);
    let binding = PreferenceBinding {
        read: |preferences| preferences.compact_density,
        write: |on| PreferencesUpdate {
            compact_density: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    row
}

/// "Remember each folder's view".
fn folder_views_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Folder views"));
    let row = SettingRow::new(FOLDER_STYLES);
    let binding = PreferenceBinding {
        read: |preferences| preferences.per_folder_views,
        write: |on| PreferencesUpdate {
            per_folder_views: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&row);
    group
}

/// What the views show of each item, and folder sizes.
fn details_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Details"));
    let relative_dates = SettingRow::new(RELATIVE_DATES);
    let binding = PreferenceBinding {
        read: |preferences| !preferences.absolute_dates,
        write: |on| PreferencesUpdate {
            absolute_dates: Some(!on),
            ..PreferencesUpdate::default()
        },
    };
    relative_dates.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&relative_dates);
    let item_counts = SettingRow::new(ITEM_COUNTS);
    let binding = ViewOptionBinding {
        read: |options| options.count_folder_items,
        write: |options, on| options.count_folder_items = on,
    };
    item_counts.add_control(&view_option_switch(page, binding), ControlName::RowTitle);
    group.add_row(&item_counts);
    let rows = [
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
    ];
    for (text, binding) in rows {
        let row = SettingRow::new(text);
        row.add_control(&page.preference_switch(binding), ControlName::RowTitle);
        group.add_row(&row);
    }
    add_folder_size_rows(page, &group);
    group
}

/// Previews, folded away: what most people never change.
fn previews_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new_folded(&ox_core::i18n::gettext("Previews and thumbnails"));
    add_preview_rows(page, &group);
    group
}

/// The preview settings, which share the folder views' options record.
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

/// "Calculate folder sizes", a command of the folder's context menu, and
/// the row that opens how sizes are counted.
fn add_folder_size_rows(page: &SettingsPage, group: &SettingsGroup) {
    group.add_row(&SettingRow::new(FOLDER_SIZES));
    let details = SettingRow::new(HOW_SIZES_ARE_COUNTED);
    let open = parts::chevron_button(HOW_SIZES_ARE_COUNTED.title);
    open.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.show_view(SettingsView::Subpage(Subpage::FolderSizes))
    ));
    details.add_control(&open, ControlName::OwnLabel);
    group.add_row(&details);
}

/// The page of how folder sizes are counted: the Python section's help.
pub(super) fn build_folder_sizes() -> SettingsSection {
    let sizes = SettingsSection::new(
        &ox_core::i18n::gettext("Folder sizes"),
        "How Calculate folder size counts a folder, and what it leaves out.",
        PageKind::Subpage,
    );
    for paragraph in FOLDER_SIZES_HELP {
        sizes.append_text(&parts::paragraph(paragraph));
    }
    sizes
}
