// SPDX-License-Identifier: AGPL-3.0-only
//! Every icon the app shows, by name: the only place icon names live.
//!
//! Replaces the `paths` table and the art functions of `v2.0.0:desktop/ui/app.js`
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
    /// `add_20_regular`: New, the new-tab "+", Map network drive and
    /// Larger text.
    Add,
    /// Add a network location in This PC's "…": an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    AddNetworkLocation24,
    /// `apps_20_regular`: the Default apps settings.
    Apps,
    /// `apps_list_detail_20_regular`: View > Tiles.
    AppsListDetail,
    /// `arrow_autofit_height_20_regular`: View > Show > Compact view, as Windows 11 draws it.
    ArrowAutofitHeight,
    /// `arrow_clockwise_20_regular`: Refresh, Check for updates, and the
    /// Refresh and Reset buttons of Settings.
    ArrowClockwise,
    /// `arrow_counterclockwise_20_regular`: Restore, in the Recycle Bin.
    ArrowCounterclockwise,
    /// `arrow_down_20_regular`: the Descending sort.
    ArrowDown,
    /// `arrow_eject_20_regular`: Eject, Safely remove, Disconnect and
    /// Sign out of server, the commands app.js drew with its eject
    /// glyph, and the eject button of a removable drive's sidebar row.
    ArrowEject,
    /// `arrow_download_20_regular`: the Downloads folder and the Brave &
    /// downloads settings.
    ArrowDownload,
    /// `arrow_left_20_regular`: Back, Back to files and a settings page's
    /// way back.
    ArrowLeft,
    /// `arrow_redo_20_regular`: Redo.
    ArrowRedo,
    /// `arrow_reset_20_regular`: Reset text size.
    ArrowReset,
    /// `arrow_right_20_regular`: Forward.
    ArrowRight,
    /// `arrow_sort_20_regular`: Sort and its columns.
    ArrowSort,
    /// `arrow_swap_20_regular`: Invert selection until "…" took Windows
    /// 11's glyph for it (2026-11-03); still bundled with the approved
    /// set, for the owner to keep or remove.
    #[cfg_attr(not(test), expect(dead_code, reason = "no menu shows it since 2026-11-03"))]
    ArrowSwap,
    /// `arrow_undo_20_regular`: Undo, and "Skip this file" in Replace or
    /// Skip Files.
    ArrowUndo,
    /// `arrow_up_20_regular`: Up and the Ascending sort.
    ArrowUp,
    /// `bluetooth_20_regular`: Send to ▸ Bluetooth device.
    Bluetooth,
    /// `braces_20_regular`: New JSON file until New took Windows 11's list (2026-11-06);
    /// still bundled with the approved set, for the owner to keep or remove.
    #[cfg_attr(not(test), expect(dead_code, reason = "no menu shows it since 2026-11-06"))]
    Braces,
    /// `checkbox_checked_20_regular`: View > Show > Item check boxes.
    CheckboxChecked,
    /// `checkmark_20_regular`: a checked menu item, the chosen option of a
    /// settings drop-down, and "Replace the file in the destination".
    Checkmark,
    /// `chevron_down_20_regular`: the menu carets of New, Sort and View.
    ChevronDown,
    /// `chevron_double_left_16_regular`: the address bar's « button, which
    /// lists the crumbs a long path hides (named by
    /// `resources/skin/navigation.css`).
    #[cfg_attr(not(test), expect(dead_code, reason = "only the skin's CSS names it"))]
    ChevronDoubleLeft16,
    /// `chevron_double_right_16_regular`: the same button in a
    /// right-to-left language.
    #[cfg_attr(not(test), expect(dead_code, reason = "only the skin's CSS names it"))]
    ChevronDoubleRight16,
    /// `chevron_down_16_regular`: the small carets (the address bar's edit
    /// chevron, a column title's sort arrow, the sidebar's expanders and a
    /// settings drop-down).
    ChevronDown16,
    /// `chevron_left_16_regular`: the address bar's crumb dividers in a
    /// right-to-left language (named by `resources/skin/navigation.css`).
    #[cfg_attr(not(test), expect(dead_code, reason = "only the skin's CSS names it"))]
    ChevronLeft16,
    /// `chevron_right_16_regular`: a settings row that opens a page of its
    /// own.
    ChevronRight16,
    /// `chevron_up_16_regular`: the progress window's "Fewer details".
    ChevronUp16,
    /// `circle_small_20_filled`: the dot a menu shows beside the chosen view or sort, and the open
    /// Details pane, as Windows 11 marks a choice.
    CircleSmallFilled,
    /// `clipboard_paste_20_regular`: Paste.
    ClipboardPaste,
    /// `clock_20_regular`: Previous versions, and the banner of a tab that
    /// browses one (`icon('clock')` in app.js).
    Clock,
    /// `code_20_regular`: the address menu's Keep address editable.
    Code,
    /// `code_20_color`: code, HTML and JSON files at small sizes.
    CodeColor20,
    /// `code_24_color`: code, HTML and JSON files at larger sizes, the
    /// largest design of this picture.
    CodeColor24,
    /// `content_view_20_regular`: View > Content.
    ContentView,
    /// `copy_20_regular`: Copy, and the details pane's picture of several
    /// selected items.
    Copy,
    /// `cut_20_regular`: Cut.
    Cut,
    /// `delete_20_regular`: Move to Trash.
    Delete,
    /// `delete_dismiss_20_regular`: Empty Recycle Bin.
    DeleteDismiss,
    /// `desktop_20_regular`: the Desktop folder, the open-windows menu's
    /// windows, Use system appearance, Discovered servers and Settings'
    /// Open windows….
    Desktop,
    /// Disconnect network drive in This PC's "…": an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    DisconnectNetworkDrive24,
    /// `dismiss_20_regular`: the window's close button and Quit
    /// `OpenXplorer`.
    Dismiss,
    /// `dismiss_16_regular`: the close buttons of tabs and the details pane.
    Dismiss16,
    /// `dismiss_circle_16_filled`: the badge of a disconnected network share
    /// or mapped drive, as Windows marks one with a red cross.
    DismissCircleFilled,
    /// `document_20_regular`: the Documents and Templates folders, License &
    /// source, and New ▸ Rich Text Document and the user's templates.
    Document,
    /// `document_add_20_regular`: the panel of a file being created.
    DocumentAdd,
    /// `document_20_color`: documents, PDFs, slides and unknown files at
    /// small sizes, and Send to ▸ Documents.
    DocumentColor20,
    /// `document_32_color`: the same at medium sizes.
    DocumentColor32,
    /// `document_48_color`: the same at large sizes.
    DocumentColor48,
    /// `document_copy_20_regular`: Duplicate, Compare files, and "Compare
    /// info for both files".
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
    /// `folder_zip_20_regular`: the badge that marks a ZIP archive, and New
    /// ▸ Compressed (zipped) Folder.
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
    /// `history_20_regular`: Previous versions.
    History,
    /// `home_20_regular`: Home.
    Home,
    /// `image_20_regular`: the Pictures folder and New ▸ Bitmap image.
    Image,
    /// `image_20_color`: pictures at small sizes.
    ImageColor20,
    /// `image_32_color`: the same at medium sizes.
    ImageColor32,
    /// `image_48_color`: the same at large sizes.
    ImageColor48,
    /// `info_20_regular`: About this build, the details pane's note, and the
    /// About settings and the notes of Settings.
    Info,
    /// Invert selection in the "…" menu: an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    InvertSelection24,
    /// `laptop_20_regular`: This PC.
    Laptop,
    /// `link_20_regular`: Copy as path in the context menus, and New ▸
    /// Shortcut.
    Link,
    /// `mail_20_color`: Send to ▸ Mail recipient, in colour as in Windows.
    MailColor20,
    /// Map network drive in This PC's "…": an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    MapNetworkDrive24,
    /// `markdown_20_regular`: New Markdown document until New took Windows 11's list (2026-11-06);
    /// still bundled with the approved set, for the owner to keep or remove.
    #[cfg_attr(not(test), expect(dead_code, reason = "no menu shows it since 2026-11-06"))]
    Markdown,
    /// `maximize_20_regular`: the window's maximise button.
    Maximize,
    /// `more_horizontal_20_regular`: More options.
    MoreHorizontal,
    /// `music_note_2_20_regular`: the Music folder.
    MusicNote,
    /// `open_20_regular`: the details pane's Open and a link's Show target.
    Open,
    /// Options in the "…" menu: an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    Options24,
    /// `organization_20_regular`: Network, its page, SMB addresses, Map
    /// network location and a location that cannot be reached.
    Organization,
    /// `paint_brush_20_regular`: the Appearance settings.
    PaintBrush,
    /// `panel_left_20_regular`: View > Show > Navigation pane.
    PanelLeft,
    /// `panel_right_20_regular`: the details pane toggle.
    PanelRight,
    /// `panel_right_gallery_20_regular`: View > Preview pane.
    PanelRightGallery,
    /// `pause_16_regular`: pausing an action in the progress window.
    Pause16,
    /// `phone_20_regular`: phones and other devices.
    Phone,
    /// `play_16_regular`: resuming a paused action in the progress
    /// window.
    Play16,
    /// `pin_16_regular`: pins and the Quick access headings.
    Pin,
    /// `rectangle_landscape_20_regular`: View > Extra large icons and Large icons.
    RectangleLandscape,
    /// `rename_20_regular`: Rename.
    Rename,
    /// `search_20_regular`: the search box, Cache this folder for search,
    /// Select items matching, the settings search and the Search &
    /// indexing settings.
    Search,
    /// Select all in the "…" menu: an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    SelectAll24,
    /// `select_all_off_20_regular`: Select none until "…" took Windows 11's
    /// four squares for it (2026-11-03); still bundled with the approved
    /// set, for the owner to keep or remove.
    #[cfg_attr(not(test), expect(dead_code, reason = "no menu shows it since 2026-11-03"))]
    SelectAllOff,
    /// `select_all_on_20_regular`: Select all until "…" took Windows 11's
    /// blue squares for it (2026-11-04); still bundled with the approved
    /// set, for the owner to keep or remove.
    #[cfg_attr(not(test), expect(dead_code, reason = "no menu shows it since 2026-11-04"))]
    SelectAllOn,
    /// Select none in the "…" menu: an original icon
    /// made for `OpenXplorer` to match Windows 11's command-bar glyph
    /// (`native/icon-sources`).
    SelectNone24,
    /// `server_20_regular`: an SMB server on the network bar.
    Server,
    /// `settings_20_regular`: Settings.
    Settings,
    /// `share_20_regular`: Share in the command bar.
    Share,
    /// `shield_lock_20_regular`: the search index's privacy note.
    ShieldLock,
    /// `square_multiple_20_regular`: the window's restore button.
    SquareMultiple,
    /// `star_20_regular`: an empty star of the Details tab's Rating.
    Star,
    /// `star_20_filled`: a star the Rating gives.
    StarFilled,
    /// `subtract_20_regular`: the window's minimise button and Smaller text.
    Subtract,
    /// `table_20_regular`: New CSV file until New took Windows 11's list (2026-11-06);
    /// still bundled with the approved set, for the owner to keep or remove.
    #[cfg_attr(not(test), expect(dead_code, reason = "no menu shows it since 2026-11-06"))]
    Table,
    /// `table_20_color`: spreadsheets and CSV files at small sizes.
    TableColor20,
    /// `table_32_color`: the same at medium sizes.
    TableColor32,
    /// `table_48_color`: the same at large sizes.
    TableColor48,
    /// `text_bullet_list_ltr_20_regular`: the details view.
    TextBulletList,
    /// `text_column_two_20_regular`: View > List.
    TextColumnTwo,
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
    /// `window_console_20_regular`: Open in Terminal.
    WindowConsole,
    /// `window_multiple_20_regular`: the General settings.
    WindowMultiple,
    /// `window_new_20_regular`: New window, in menus and in Settings.
    WindowNew,
    /// `wrench_20_regular`: Properties in the "…" menu, as Windows 11 draws it.
    Wrench,
}

impl Icon {
    /// The icon's name in the app's icon theme: its file name under
    /// `resources/icons/hicolor/scalable/`, without `.svg`. An exhaustive
    /// table, hence its length.
    #[expect(clippy::too_many_lines, reason = "one arm per bundled icon")]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Icon::Add => "ox-add-20-symbolic",
            Icon::AddNetworkLocation24 => "ox-add-network-location-24-symbolic",
            Icon::Apps => "ox-apps-20-symbolic",
            Icon::AppsListDetail => "ox-apps-list-detail-20-symbolic",
            Icon::ArrowAutofitHeight => "ox-arrow-autofit-height-20-symbolic",
            Icon::ArrowClockwise => "ox-arrow-clockwise-20-symbolic",
            Icon::ArrowCounterclockwise => "ox-arrow-counterclockwise-20-symbolic",
            Icon::ArrowDown => "ox-arrow-down-20-symbolic",
            Icon::ArrowEject => "ox-arrow-eject-20-symbolic",
            Icon::ArrowDownload => "ox-arrow-download-20-symbolic",
            Icon::ArrowLeft => "ox-arrow-left-20-symbolic",
            Icon::ArrowRedo => "ox-arrow-redo-20-symbolic",
            Icon::ArrowReset => "ox-arrow-reset-20-symbolic",
            Icon::ArrowRight => "ox-arrow-right-20-symbolic",
            Icon::ArrowSort => "ox-arrow-sort-20-symbolic",
            Icon::ArrowSwap => "ox-arrow-swap-20-symbolic",
            Icon::ArrowUndo => "ox-arrow-undo-20-symbolic",
            Icon::ArrowUp => "ox-arrow-up-20-symbolic",
            Icon::Bluetooth => "ox-bluetooth-20-symbolic",
            Icon::Braces => "ox-braces-20-symbolic",
            Icon::CheckboxChecked => "ox-checkbox-checked-20-symbolic",
            Icon::Checkmark => "ox-checkmark-20-symbolic",
            Icon::ChevronDown => "ox-chevron-down-20-symbolic",
            Icon::ChevronDown16 => "ox-chevron-down-16-symbolic",
            Icon::ChevronDoubleLeft16 => "ox-chevron-double-left-16-symbolic",
            Icon::ChevronDoubleRight16 => "ox-chevron-double-right-16-symbolic",
            Icon::ChevronLeft16 => "ox-chevron-left-16-symbolic",
            Icon::ChevronRight16 => "ox-chevron-right-16-symbolic",
            Icon::ChevronUp16 => "ox-chevron-up-16-symbolic",
            Icon::CircleSmallFilled => "ox-circle-small-20-filled-symbolic",
            Icon::ClipboardPaste => "ox-clipboard-paste-20-symbolic",
            Icon::Clock => "ox-clock-20-symbolic",
            Icon::Code => "ox-code-20-symbolic",
            Icon::CodeColor20 => "ox-code-20-color",
            Icon::CodeColor24 => "ox-code-24-color",
            Icon::ContentView => "ox-content-view-20-symbolic",
            Icon::Copy => "ox-copy-20-symbolic",
            Icon::Cut => "ox-cut-20-symbolic",
            Icon::Delete => "ox-delete-20-symbolic",
            Icon::DeleteDismiss => "ox-delete-dismiss-20-symbolic",
            Icon::Desktop => "ox-desktop-20-symbolic",
            Icon::DisconnectNetworkDrive24 => "ox-disconnect-network-drive-24-symbolic",
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
            Icon::History => "ox-history-20-symbolic",
            Icon::Home => "ox-home-20-symbolic",
            Icon::Image => "ox-image-20-symbolic",
            Icon::ImageColor20 => "ox-image-20-color",
            Icon::ImageColor32 => "ox-image-32-color",
            Icon::ImageColor48 => "ox-image-48-color",
            Icon::Info => "ox-info-20-symbolic",
            Icon::InvertSelection24 => "ox-invert-selection-24-symbolic",
            Icon::Laptop => "ox-laptop-20-symbolic",
            Icon::Link => "ox-link-20-symbolic",
            Icon::MailColor20 => "ox-mail-20-color",
            Icon::MapNetworkDrive24 => "ox-map-network-drive-24-symbolic",
            Icon::Markdown => "ox-markdown-20-symbolic",
            Icon::Maximize => "ox-maximize-20-symbolic",
            Icon::MoreHorizontal => "ox-more-horizontal-20-symbolic",
            Icon::MusicNote => "ox-music-note-2-20-symbolic",
            Icon::Open => "ox-open-20-symbolic",
            Icon::Options24 => "ox-options-24-symbolic",
            Icon::Organization => "ox-organization-20-symbolic",
            Icon::PaintBrush => "ox-paint-brush-20-symbolic",
            Icon::PanelLeft => "ox-panel-left-20-symbolic",
            Icon::PanelRight => "ox-panel-right-20-symbolic",
            Icon::PanelRightGallery => "ox-panel-right-gallery-20-symbolic",
            Icon::Pause16 => "ox-pause-16-symbolic",
            Icon::Phone => "ox-phone-20-symbolic",
            Icon::Play16 => "ox-play-16-symbolic",
            Icon::Pin => "ox-pin-16-symbolic",
            Icon::RectangleLandscape => "ox-rectangle-landscape-20-symbolic",
            Icon::Rename => "ox-rename-20-symbolic",
            Icon::Search => "ox-search-20-symbolic",
            Icon::SelectAll24 => "ox-select-all-24-symbolic",
            Icon::SelectAllOff => "ox-select-all-off-20-symbolic",
            Icon::SelectAllOn => "ox-select-all-on-20-symbolic",
            Icon::SelectNone24 => "ox-select-none-24-symbolic",
            Icon::Server => "ox-server-20-symbolic",
            Icon::Settings => "ox-settings-20-symbolic",
            Icon::Share => "ox-share-20-symbolic",
            Icon::ShieldLock => "ox-shield-lock-20-symbolic",
            Icon::SquareMultiple => "ox-square-multiple-20-symbolic",
            Icon::Star => "ox-star-20-symbolic",
            Icon::StarFilled => "ox-star-20-filled-symbolic",
            Icon::Subtract => "ox-subtract-20-symbolic",
            Icon::Table => "ox-table-20-symbolic",
            Icon::TableColor20 => "ox-table-20-color",
            Icon::TableColor32 => "ox-table-32-color",
            Icon::TableColor48 => "ox-table-48-color",
            Icon::TextBulletList => "ox-text-bullet-list-ltr-20-symbolic",
            Icon::TextColumnTwo => "ox-text-column-two-20-symbolic",
            Icon::Video => "ox-video-20-symbolic",
            Icon::VideoColor20 => "ox-video-20-color",
            Icon::VideoColor32 => "ox-video-32-color",
            Icon::VideoColor48 => "ox-video-48-color",
            Icon::WeatherMoon => "ox-weather-moon-20-symbolic",
            Icon::WeatherSunny => "ox-weather-sunny-20-symbolic",
            Icon::WindowConsole => "ox-window-console-20-symbolic",
            Icon::WindowMultiple => "ox-window-multiple-20-symbolic",
            Icon::WindowNew => "ox-window-new-20-symbolic",
            Icon::Wrench => "ox-wrench-20-symbolic",
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
pub(crate) const ALL_ICONS: [Icon; 123] = [
    Icon::Add,
    Icon::AddNetworkLocation24,
    Icon::Apps,
    Icon::AppsListDetail,
    Icon::ArrowAutofitHeight,
    Icon::ArrowClockwise,
    Icon::ArrowCounterclockwise,
    Icon::ArrowDown,
    Icon::ArrowEject,
    Icon::ArrowDownload,
    Icon::ArrowLeft,
    Icon::ArrowRedo,
    Icon::ArrowReset,
    Icon::ArrowRight,
    Icon::ArrowSort,
    Icon::ArrowSwap,
    Icon::ArrowUndo,
    Icon::ArrowUp,
    Icon::Bluetooth,
    Icon::Braces,
    Icon::CheckboxChecked,
    Icon::Checkmark,
    Icon::ChevronDown,
    Icon::ChevronDoubleLeft16,
    Icon::ChevronDoubleRight16,
    Icon::ChevronDown16,
    Icon::ChevronLeft16,
    Icon::ChevronRight16,
    Icon::ChevronUp16,
    Icon::CircleSmallFilled,
    Icon::ClipboardPaste,
    Icon::Clock,
    Icon::Code,
    Icon::CodeColor20,
    Icon::CodeColor24,
    Icon::ContentView,
    Icon::Copy,
    Icon::Cut,
    Icon::Delete,
    Icon::DeleteDismiss,
    Icon::Desktop,
    Icon::DisconnectNetworkDrive24,
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
    Icon::History,
    Icon::Home,
    Icon::Image,
    Icon::ImageColor20,
    Icon::ImageColor32,
    Icon::ImageColor48,
    Icon::Info,
    Icon::InvertSelection24,
    Icon::Laptop,
    Icon::Link,
    Icon::MailColor20,
    Icon::MapNetworkDrive24,
    Icon::Markdown,
    Icon::Maximize,
    Icon::MoreHorizontal,
    Icon::MusicNote,
    Icon::Open,
    Icon::Options24,
    Icon::Organization,
    Icon::PaintBrush,
    Icon::PanelLeft,
    Icon::PanelRight,
    Icon::PanelRightGallery,
    Icon::Pause16,
    Icon::Phone,
    Icon::Play16,
    Icon::Pin,
    Icon::RectangleLandscape,
    Icon::Rename,
    Icon::Search,
    Icon::SelectAll24,
    Icon::SelectAllOff,
    Icon::SelectAllOn,
    Icon::SelectNone24,
    Icon::Server,
    Icon::Settings,
    Icon::Share,
    Icon::ShieldLock,
    Icon::SquareMultiple,
    Icon::Star,
    Icon::StarFilled,
    Icon::Subtract,
    Icon::Table,
    Icon::TableColor20,
    Icon::TableColor32,
    Icon::TableColor48,
    Icon::TextBulletList,
    Icon::TextColumnTwo,
    Icon::Video,
    Icon::VideoColor20,
    Icon::VideoColor32,
    Icon::VideoColor48,
    Icon::WeatherMoon,
    Icon::WeatherSunny,
    Icon::WindowConsole,
    Icon::WindowMultiple,
    Icon::WindowNew,
    Icon::Wrench,
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

    /// A glyph name of app.js, as ox-core's `KnownFolder::glyph` gives it,
    /// and the Fluent icon the approved icon mapping chose for it.
    struct GlyphCase {
        app_glyph: &'static str,
        icon: Option<Icon>,
    }

    /// [`Icon::for_known_folder`] repeats ox-core's glyph table in Fluent
    /// icons; the two must agree, Templates sharing the Documents glyph
    /// included.
    #[test]
    fn every_known_folder_shows_the_fluent_icon_of_its_ox_core_glyph() {
        let mapping = [
            GlyphCase {
                app_glyph: "desktop",
                icon: Some(Icon::Desktop),
            },
            GlyphCase {
                app_glyph: "downloads",
                icon: Some(Icon::ArrowDownload),
            },
            GlyphCase {
                app_glyph: "documents",
                icon: Some(Icon::Document),
            },
            GlyphCase {
                app_glyph: "pictures",
                icon: Some(Icon::Image),
            },
            GlyphCase {
                app_glyph: "music",
                icon: Some(Icon::MusicNote),
            },
            GlyphCase {
                app_glyph: "videos",
                icon: Some(Icon::Video),
            },
            // "folder" is the folder art, not a glyph.
            GlyphCase {
                app_glyph: "folder",
                icon: None,
            },
        ];
        for folder in KnownFolder::ALL {
            let app_glyph = folder.glyph();
            let case = mapping
                .iter()
                .find(|case| case.app_glyph == app_glyph)
                .unwrap_or_else(|| panic!("the icon mapping has no icon for {app_glyph:?}"));
            assert_eq!(Icon::for_known_folder(folder), case.icon, "{folder:?}");
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
