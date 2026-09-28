// SPDX-License-Identifier: AGPL-3.0-only
//! Every icon the app shows, by name: the only place icon names live.
//!
//! Replaces the `paths` table and the art functions of `desktop/ui/app.js`
//! (`icon()`, `folderIcon`, `fileIcon`), which drew their own SVG, with the
//! Fluent icons the product owner approved. They are vendored unmodified in
//! `resources/icons/hicolor/scalable/<context>/`; `resources/icons/SOURCES.md`
//! records where each comes from.

use ox_core::places::KnownFolder;

/// An icon bundled with the app, named after its upstream Fluent file.
///
/// Monochrome glyphs (Fluent's regular and filled styles) are symbolic
/// icons: GTK paints them in the CSS `color` of their image, so they follow
/// the theme and the hover and disabled states. Colour art (the `...Color...`
/// variants and [`Icon::FileFolder`]) keeps its own colours.
///
/// Glyphs are Fluent's 20-pixel design, drawn at whatever size each place of
/// the window asks for. A number in a variant's name marks a second design of
/// the same picture: the 16-pixel design for the smallest glyphs (the icon
/// mapping's allowance for small carets and close buttons), and the colour
/// art's designs, which [`FileType::icon`](super::file_type::FileType::icon)
/// picks by size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Icon {
    /// `add_20_regular`: New, the new-tab "+", Map network location and
    /// Larger text.
    Add,
    /// `arrow_clockwise_20_regular`: Refresh and Check for updates.
    ArrowClockwise,
    /// `arrow_down_20_regular`: the Descending sort.
    ArrowDown,
    /// `arrow_download_20_regular`: the Downloads folder.
    ArrowDownload,
    /// `arrow_left_20_regular`: Back.
    ArrowLeft,
    /// `arrow_reset_20_regular`: Reset text size.
    ArrowReset,
    /// `arrow_right_20_regular`: Forward.
    ArrowRight,
    /// `arrow_sort_20_regular`: Sort and its columns.
    ArrowSort,
    /// `arrow_swap_20_regular`: Invert selection.
    ArrowSwap,
    /// `arrow_up_20_regular`: Up and the Ascending sort.
    ArrowUp,
    /// `braces_20_regular`: New JSON file.
    Braces,
    /// `checkmark_20_regular`: a checked menu item.
    Checkmark,
    /// `chevron_down_20_regular`: the menu carets of New, Sort and View.
    ChevronDown,
    /// `chevron_down_16_regular`: the small carets (the address bar's edit
    /// chevron, a column title's sort arrow and the sidebar's expanders).
    ChevronDown16,
    /// `clipboard_paste_20_regular`: Paste.
    ClipboardPaste,
    /// `code_20_regular`: New HTML document.
    Code,
    /// `code_20_color`: code, HTML and JSON files at small sizes.
    CodeColor20,
    /// `code_24_color`: code, HTML and JSON files at larger sizes, the
    /// largest design of this picture.
    CodeColor24,
    /// `copy_20_regular`: Copy, and the details pane's picture of several
    /// selected items.
    Copy,
    /// `cut_20_regular`: Cut.
    Cut,
    /// `delete_20_regular`: Move to Trash.
    Delete,
    /// `desktop_20_regular`: the Desktop folder, the open-windows button and
    /// its windows, Use system appearance and Discovered servers.
    Desktop,
    /// `dismiss_20_regular`: the window's close button and Quit
    /// `OpenXplorer`.
    Dismiss,
    /// `dismiss_16_regular`: the close buttons of tabs and the details pane.
    Dismiss16,
    /// `dismiss_circle_16_filled`: the badge of a disconnected network share
    /// or mapped drive, as Windows marks one with a red cross.
    DismissCircleFilled,
    /// `document_20_regular`: the Documents and Templates folders, and
    /// License & source.
    Document,
    /// `document_add_20_regular`: New file.
    DocumentAdd,
    /// `document_20_color`: documents, PDFs, slides and unknown files at
    /// small sizes.
    DocumentColor20,
    /// `document_32_color`: the same at medium sizes.
    DocumentColor32,
    /// `document_48_color`: the same at large sizes.
    DocumentColor48,
    /// `document_copy_20_regular`: New from template.
    DocumentCopy,
    /// `document_text_20_regular`: New text document.
    DocumentText,
    /// `document_text_20_color`: text and Markdown files at small sizes.
    DocumentTextColor20,
    /// `document_text_32_color`: the same at medium sizes.
    DocumentTextColor32,
    /// `document_text_48_color`: the same at large sizes.
    DocumentTextColor48,
    /// `eye_20_regular`: Show hidden files.
    Eye,
    /// Fluent Emoji `file_folder_flat`: the yellow folder of folders, tabs,
    /// the address bar, Quick access and network shares.
    FileFolder,
    /// `folder_20_regular`: the empty-folder page and Default file explorer.
    Folder,
    /// `folder_add_20_regular`: New folder.
    FolderAdd,
    /// `folder_zip_20_regular`: the badge that marks a ZIP archive.
    FolderZip,
    /// `grid_20_regular`: View and the icon views.
    Grid,
    /// `hard_drive_20_regular`: Local Disk, drives and mapped network drives.
    HardDrive,
    /// `headphones_20_color`: audio files at small sizes.
    HeadphonesColor20,
    /// `headphones_32_color`: the same at medium sizes.
    HeadphonesColor32,
    /// `headphones_48_color`: the same at large sizes.
    HeadphonesColor48,
    /// `home_20_regular`: Home.
    Home,
    /// `image_20_regular`: the Pictures folder.
    Image,
    /// `image_20_color`: pictures at small sizes.
    ImageColor20,
    /// `image_32_color`: the same at medium sizes.
    ImageColor32,
    /// `image_48_color`: the same at large sizes.
    ImageColor48,
    /// `info_20_regular`: About this build and the details pane's note.
    Info,
    /// `laptop_20_regular`: This PC.
    Laptop,
    /// `markdown_20_regular`: New Markdown document.
    Markdown,
    /// `maximize_20_regular`: the window's maximise button.
    Maximize,
    /// `more_horizontal_20_regular`: More options.
    MoreHorizontal,
    /// `music_note_2_20_regular`: the Music folder.
    MusicNote,
    /// `open_20_regular`: the details pane's Open.
    Open,
    /// `organization_20_regular`: Network, its page, SMB addresses, Map
    /// network location and a location that cannot be reached.
    Organization,
    /// `panel_right_20_regular`: the details pane toggle.
    PanelRight,
    /// `phone_20_regular`: phones and other devices.
    Phone,
    /// `pin_16_regular`: pins and the Quick access headings.
    Pin,
    /// `rename_20_regular`: Rename.
    Rename,
    /// `search_20_regular`: the search box and Cache this folder for search.
    Search,
    /// `select_all_off_20_regular`: Select none.
    SelectAllOff,
    /// `select_all_on_20_regular`: Select all.
    SelectAllOn,
    /// `server_20_regular`: an SMB server on the network bar.
    Server,
    /// `settings_20_regular`: Settings.
    Settings,
    /// `share_20_regular`: Copy path in the command bar.
    Share,
    /// `square_multiple_20_regular`: the window's restore button.
    SquareMultiple,
    /// `subtract_20_regular`: the window's minimise button and Smaller text.
    Subtract,
    /// `table_20_regular`: New CSV file.
    Table,
    /// `table_20_color`: spreadsheets and CSV files at small sizes.
    TableColor20,
    /// `table_32_color`: the same at medium sizes.
    TableColor32,
    /// `table_48_color`: the same at large sizes.
    TableColor48,
    /// `text_bullet_list_ltr_20_regular`: the details view.
    TextBulletList,
    /// `video_20_regular`: the Videos folder.
    Video,
    /// `video_20_color`: videos at small sizes.
    VideoColor20,
    /// `video_32_color`: the same at medium sizes.
    VideoColor32,
    /// `video_48_color`: the same at large sizes.
    VideoColor48,
    /// `weather_moon_20_regular`: the dark appearance.
    WeatherMoon,
    /// `weather_sunny_20_regular`: the light appearance.
    WeatherSunny,
    /// `window_new_20_regular`: New window.
    WindowNew,
}

impl Icon {
    /// The icon's name in the app's icon theme: its file name under
    /// `resources/icons/hicolor/scalable/`, without `.svg`. An exhaustive
    /// table, hence its length.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Icon::Add => "ox-add-20-symbolic",
            Icon::ArrowClockwise => "ox-arrow-clockwise-20-symbolic",
            Icon::ArrowDown => "ox-arrow-down-20-symbolic",
            Icon::ArrowDownload => "ox-arrow-download-20-symbolic",
            Icon::ArrowLeft => "ox-arrow-left-20-symbolic",
            Icon::ArrowReset => "ox-arrow-reset-20-symbolic",
            Icon::ArrowRight => "ox-arrow-right-20-symbolic",
            Icon::ArrowSort => "ox-arrow-sort-20-symbolic",
            Icon::ArrowSwap => "ox-arrow-swap-20-symbolic",
            Icon::ArrowUp => "ox-arrow-up-20-symbolic",
            Icon::Braces => "ox-braces-20-symbolic",
            Icon::Checkmark => "ox-checkmark-20-symbolic",
            Icon::ChevronDown => "ox-chevron-down-20-symbolic",
            Icon::ChevronDown16 => "ox-chevron-down-16-symbolic",
            Icon::ClipboardPaste => "ox-clipboard-paste-20-symbolic",
            Icon::Code => "ox-code-20-symbolic",
            Icon::CodeColor20 => "ox-code-20-color",
            Icon::CodeColor24 => "ox-code-24-color",
            Icon::Copy => "ox-copy-20-symbolic",
            Icon::Cut => "ox-cut-20-symbolic",
            Icon::Delete => "ox-delete-20-symbolic",
            Icon::Desktop => "ox-desktop-20-symbolic",
            Icon::Dismiss => "ox-dismiss-20-symbolic",
            Icon::Dismiss16 => "ox-dismiss-16-symbolic",
            Icon::DismissCircleFilled => "ox-dismiss-circle-16-filled-symbolic",
            Icon::Document => "ox-document-20-symbolic",
            Icon::DocumentAdd => "ox-document-add-20-symbolic",
            Icon::DocumentColor20 => "ox-document-20-color",
            Icon::DocumentColor32 => "ox-document-32-color",
            Icon::DocumentColor48 => "ox-document-48-color",
            Icon::DocumentCopy => "ox-document-copy-20-symbolic",
            Icon::DocumentText => "ox-document-text-20-symbolic",
            Icon::DocumentTextColor20 => "ox-document-text-20-color",
            Icon::DocumentTextColor32 => "ox-document-text-32-color",
            Icon::DocumentTextColor48 => "ox-document-text-48-color",
            Icon::Eye => "ox-eye-20-symbolic",
            Icon::FileFolder => "ox-file-folder-flat",
            Icon::Folder => "ox-folder-20-symbolic",
            Icon::FolderAdd => "ox-folder-add-20-symbolic",
            Icon::FolderZip => "ox-folder-zip-20-symbolic",
            Icon::Grid => "ox-grid-20-symbolic",
            Icon::HardDrive => "ox-hard-drive-20-symbolic",
            Icon::HeadphonesColor20 => "ox-headphones-20-color",
            Icon::HeadphonesColor32 => "ox-headphones-32-color",
            Icon::HeadphonesColor48 => "ox-headphones-48-color",
            Icon::Home => "ox-home-20-symbolic",
            Icon::Image => "ox-image-20-symbolic",
            Icon::ImageColor20 => "ox-image-20-color",
            Icon::ImageColor32 => "ox-image-32-color",
            Icon::ImageColor48 => "ox-image-48-color",
            Icon::Info => "ox-info-20-symbolic",
            Icon::Laptop => "ox-laptop-20-symbolic",
            Icon::Markdown => "ox-markdown-20-symbolic",
            Icon::Maximize => "ox-maximize-20-symbolic",
            Icon::MoreHorizontal => "ox-more-horizontal-20-symbolic",
            Icon::MusicNote => "ox-music-note-2-20-symbolic",
            Icon::Open => "ox-open-20-symbolic",
            Icon::Organization => "ox-organization-20-symbolic",
            Icon::PanelRight => "ox-panel-right-20-symbolic",
            Icon::Phone => "ox-phone-20-symbolic",
            Icon::Pin => "ox-pin-16-symbolic",
            Icon::Rename => "ox-rename-20-symbolic",
            Icon::Search => "ox-search-20-symbolic",
            Icon::SelectAllOff => "ox-select-all-off-20-symbolic",
            Icon::SelectAllOn => "ox-select-all-on-20-symbolic",
            Icon::Server => "ox-server-20-symbolic",
            Icon::Settings => "ox-settings-20-symbolic",
            Icon::Share => "ox-share-20-symbolic",
            Icon::SquareMultiple => "ox-square-multiple-20-symbolic",
            Icon::Subtract => "ox-subtract-20-symbolic",
            Icon::Table => "ox-table-20-symbolic",
            Icon::TableColor20 => "ox-table-20-color",
            Icon::TableColor32 => "ox-table-32-color",
            Icon::TableColor48 => "ox-table-48-color",
            Icon::TextBulletList => "ox-text-bullet-list-ltr-20-symbolic",
            Icon::Video => "ox-video-20-symbolic",
            Icon::VideoColor20 => "ox-video-20-color",
            Icon::VideoColor32 => "ox-video-32-color",
            Icon::VideoColor48 => "ox-video-48-color",
            Icon::WeatherMoon => "ox-weather-moon-20-symbolic",
            Icon::WeatherSunny => "ox-weather-sunny-20-symbolic",
            Icon::WindowNew => "ox-window-new-20-symbolic",
        }
    }

    /// The glyph of a standard folder, as Quick access shows it; `None` for
    /// the Public folder, which shows the folder art. Templates shares the
    /// Documents glyph, as `KnownFolder::glyph` in ox-core says.
    pub(crate) const fn for_known_folder(folder: KnownFolder) -> Option<Icon> {
        match folder {
            KnownFolder::Desktop => Some(Icon::Desktop),
            KnownFolder::Downloads => Some(Icon::ArrowDownload),
            KnownFolder::Documents | KnownFolder::Templates => Some(Icon::Document),
            KnownFolder::Pictures => Some(Icon::Image),
            KnownFolder::Music => Some(Icon::MusicNote),
            KnownFolder::Videos => Some(Icon::Video),
            KnownFolder::Public => None,
        }
    }
}

/// Every icon, in the order of [`Icon`], for the tests that check each one
/// ships, is recorded and resolves.
#[cfg(test)]
pub(crate) const ALL_ICONS: [Icon; 82] = [
    Icon::Add,
    Icon::ArrowClockwise,
    Icon::ArrowDown,
    Icon::ArrowDownload,
    Icon::ArrowLeft,
    Icon::ArrowReset,
    Icon::ArrowRight,
    Icon::ArrowSort,
    Icon::ArrowSwap,
    Icon::ArrowUp,
    Icon::Braces,
    Icon::Checkmark,
    Icon::ChevronDown,
    Icon::ChevronDown16,
    Icon::ClipboardPaste,
    Icon::Code,
    Icon::CodeColor20,
    Icon::CodeColor24,
    Icon::Copy,
    Icon::Cut,
    Icon::Delete,
    Icon::Desktop,
    Icon::Dismiss,
    Icon::Dismiss16,
    Icon::DismissCircleFilled,
    Icon::Document,
    Icon::DocumentAdd,
    Icon::DocumentColor20,
    Icon::DocumentColor32,
    Icon::DocumentColor48,
    Icon::DocumentCopy,
    Icon::DocumentText,
    Icon::DocumentTextColor20,
    Icon::DocumentTextColor32,
    Icon::DocumentTextColor48,
    Icon::Eye,
    Icon::FileFolder,
    Icon::Folder,
    Icon::FolderAdd,
    Icon::FolderZip,
    Icon::Grid,
    Icon::HardDrive,
    Icon::HeadphonesColor20,
    Icon::HeadphonesColor32,
    Icon::HeadphonesColor48,
    Icon::Home,
    Icon::Image,
    Icon::ImageColor20,
    Icon::ImageColor32,
    Icon::ImageColor48,
    Icon::Info,
    Icon::Laptop,
    Icon::Markdown,
    Icon::Maximize,
    Icon::MoreHorizontal,
    Icon::MusicNote,
    Icon::Open,
    Icon::Organization,
    Icon::PanelRight,
    Icon::Phone,
    Icon::Pin,
    Icon::Rename,
    Icon::Search,
    Icon::SelectAllOff,
    Icon::SelectAllOn,
    Icon::Server,
    Icon::Settings,
    Icon::Share,
    Icon::SquareMultiple,
    Icon::Subtract,
    Icon::Table,
    Icon::TableColor20,
    Icon::TableColor32,
    Icon::TableColor48,
    Icon::TextBulletList,
    Icon::Video,
    Icon::VideoColor20,
    Icon::VideoColor32,
    Icon::VideoColor48,
    Icon::WeatherMoon,
    Icon::WeatherSunny,
    Icon::WindowNew,
];

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    use gtk::glib;

    use super::*;

    /// The vendored icons: `resources/icons/hicolor/scalable`.
    fn scalable_directory() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/icons/hicolor/scalable")
    }

    /// Every vendored file, by icon name, with its path relative to
    /// [`scalable_directory`] (such as `actions/ox-add-20-symbolic.svg`).
    fn vendored_files() -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        for context in fs::read_dir(scalable_directory()).expect("the icons are vendored") {
            let context = context.expect("a readable icon context").path();
            let context_name = context
                .file_name()
                .expect("a context directory")
                .to_string_lossy()
                .into_owned();
            for file in fs::read_dir(&context).expect("a readable icon context") {
                let file_name = file
                    .expect("a readable icon")
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                let name = file_name
                    .strip_suffix(".svg")
                    .expect("icons are SVG files")
                    .to_owned();
                files.insert(name, format!("{context_name}/{file_name}"));
            }
        }
        files
    }

    /// The file path and SHA-256 of one row of the table in `SOURCES.md`,
    /// such as "| `actions/ox-add-20-symbolic.svg` | … | eace… |"; `None`
    /// for any other line.
    fn checksum_row(line: &str) -> Option<(String, String)> {
        let row = line.strip_prefix("| `")?;
        let (path, rest) = row.split_once('`')?;
        let checksum = rest.trim_end_matches(" |").rsplit("| ").next()?;
        Some((path.to_owned(), checksum.trim().to_owned()))
    }

    /// The SHA-256 each file has in `SOURCES.md`, by its path relative to
    /// [`scalable_directory`].
    fn recorded_checksums() -> BTreeMap<String, String> {
        let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/icons/SOURCES.md");
        let text = fs::read_to_string(sources).expect("SOURCES.md records the icons");
        text.lines().filter_map(checksum_row).collect()
    }

    #[test]
    fn every_icon_ships_as_a_file_and_every_file_is_an_icon() {
        let files: BTreeSet<String> = vendored_files().into_keys().collect();
        let names: BTreeSet<String> = ALL_ICONS.iter().map(|icon| icon.name().to_owned()).collect();
        assert_eq!(names.len(), ALL_ICONS.len(), "every icon has a name of its own");
        assert_eq!(names, files, "the icons and the vendored files match one to one");
    }

    /// Only colour art may keep its own colours; everything else must
    /// follow the text colour.
    #[test]
    fn only_colour_art_is_not_symbolic() {
        for icon in ALL_ICONS {
            let is_colour_art = icon.name().ends_with("-color") || icon == Icon::FileFolder;
            assert_eq!(!icon.name().ends_with("-symbolic"), is_colour_art, "{icon:?}");
        }
    }

    /// The files are exactly the upstream ones: none was edited after its
    /// checksum was recorded.
    #[test]
    fn every_vendored_file_matches_the_checksum_in_sources() {
        let recorded = recorded_checksums();
        let files = vendored_files();
        assert_eq!(recorded.len(), files.len(), "SOURCES.md lists every file once");
        for relative in files.values() {
            let bytes = fs::read(scalable_directory().join(relative)).expect("a readable icon");
            let checksum = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &bytes)
                .expect("GLib computes SHA-256");
            assert_eq!(
                recorded.get(relative).map(String::as_str),
                Some(checksum.as_str()),
                "{relative}"
            );
        }
    }

    /// parity: LOOK-015
    #[test]
    fn every_quick_access_folder_has_its_own_glyph() {
        let glyphs: BTreeSet<&str> = KnownFolder::QUICK_ACCESS
            .into_iter()
            .filter_map(Icon::for_known_folder)
            .map(Icon::name)
            .collect();
        assert_eq!(glyphs.len(), KnownFolder::QUICK_ACCESS.len());
        assert_eq!(
            Icon::for_known_folder(KnownFolder::Public),
            None,
            "Public shows the folder art"
        );
    }
}
