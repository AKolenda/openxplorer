// SPDX-License-Identifier: AGPL-3.0-only
//! The places and items a Windows icon file can draw, and which icon group
//! of which file draws each.
//!
//! The group numbers are the resource IDs of `imageres.dll.mun` and
//! `shell32.dll.mun` in Windows 10 and 11 (checked on Windows 11 build
//! 26200; Home's and Gallery's are the ones their `DefaultIcon` keys
//! name). A file is recognized by its name alone, with or without the
//! `.mun` of the copies in `C:\Windows\SystemResources`.

use std::path::Path;

/// A Windows file whose icon groups `OpenXplorer` knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconLibrary {
    /// `imageres.dll`, or its resources `imageres.dll.mun`.
    Imageres,
    /// `shell32.dll`, or its resources `shell32.dll.mun`.
    Shell32,
}

impl IconLibrary {
    /// The library `path` names, by its file name in any letter case, with
    /// or without `.mun`; `None` for any other file.
    pub fn of_file(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        let name = name.strip_suffix(".mun").unwrap_or(&name);
        match name {
            "imageres.dll" => Some(Self::Imageres),
            "shell32.dll" => Some(Self::Shell32),
            _ => None,
        }
    }
}

/// A place or item `OpenXplorer` can draw with an icon from a Windows file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IconRole {
    /// A folder.
    Folder,
    /// The Desktop folder.
    Desktop,
    /// The Downloads folder.
    Downloads,
    /// The Documents folder.
    Documents,
    /// The Pictures folder.
    Pictures,
    /// The Music folder.
    Music,
    /// The Videos folder.
    Videos,
    /// Home.
    Home,
    /// Gallery.
    Gallery,
    /// This PC.
    ThisPc,
    /// Network, in the navigation pane and as a page.
    Network,
    /// A computer or server Network lists.
    NetworkComputer,
    /// A shared folder on a server.
    NetworkShare,
    /// A share mapped as a drive, connected.
    NetworkDrive,
    /// A share mapped as a drive, not connected.
    NetworkDriveDisconnected,
    /// A fixed drive, Local Disk among them: the plain drive, never
    /// Windows' drive with its logo, on a system that is not Windows.
    Drive,
    /// A removable drive: a USB stick, an SD card, an external disk.
    RemovableDrive,
    /// An optical drive: a CD, DVD or Blu-ray disc.
    OpticalDrive,
    /// The Recycle Bin with nothing in it.
    RecycleBinEmpty,
    /// The Recycle Bin with items in it.
    RecycleBinFull,
    /// A ZIP archive, shown as a folder, and the other archives the app
    /// opens (7z, TAR and its compressed forms).
    ZipFolder,
    /// A text file.
    TextFile,
    /// A picture.
    ImageFile,
    /// Music or other audio.
    AudioFile,
    /// A video.
    VideoFile,
    /// A program.
    Executable,
    /// A script: shell, batch or `PowerShell`.
    Script,
    /// A shortcut: a `.desktop` launcher or a Windows `.lnk`.
    Shortcut,
    /// A file of a type without an icon of its own in these files: PDFs,
    /// office documents, spreadsheets, code and unknown files, as Windows
    /// shows a file no installed app claims.
    GenericFile,
    /// The Settings page.
    Settings,
    /// The back of a folder whose contents peek out of it, drawn behind
    /// the previews of its pictures in the larger icon views.
    FolderBack,
    /// The front of a folder whose contents peek out of it, drawn over
    /// the previews of its pictures.
    FolderFront,
    /// A folder with a white page sticking out: a folder that holds files
    /// but no picture, in the larger icon views.
    FolderWithFiles,
}

/// Where a role's group number comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Read from Windows 11's registry (a `DefaultIcon` value) or the
    /// owner's check of the icon in a Windows 11 virtual machine.
    Verified,
    /// The closest picture on the icon sheet, not confirmed in Windows.
    BestMatch,
}

impl IconRole {
    /// Every role, in the order of [`IconRole`].
    pub const ALL: [IconRole; 33] = [
        IconRole::Folder,
        IconRole::Desktop,
        IconRole::Downloads,
        IconRole::Documents,
        IconRole::Pictures,
        IconRole::Music,
        IconRole::Videos,
        IconRole::Home,
        IconRole::Gallery,
        IconRole::ThisPc,
        IconRole::Network,
        IconRole::NetworkComputer,
        IconRole::NetworkShare,
        IconRole::NetworkDrive,
        IconRole::NetworkDriveDisconnected,
        IconRole::Drive,
        IconRole::RemovableDrive,
        IconRole::OpticalDrive,
        IconRole::RecycleBinEmpty,
        IconRole::RecycleBinFull,
        IconRole::ZipFolder,
        IconRole::TextFile,
        IconRole::ImageFile,
        IconRole::AudioFile,
        IconRole::VideoFile,
        IconRole::Executable,
        IconRole::Script,
        IconRole::Shortcut,
        IconRole::GenericFile,
        IconRole::Settings,
        IconRole::FolderBack,
        IconRole::FolderFront,
        IconRole::FolderWithFiles,
    ];

    /// The file and icon group that draw this role, and where the number
    /// comes from. Several roles may share a group. The special folders
    /// are the flat tiles their `HKCR\CLSID\{known folder}\DefaultIcon`
    /// values name on Windows 11 build 26200, not the yellow folders with
    /// an overlay (1001 to 1005) older Windows used.
    pub const fn source(self) -> (IconLibrary, u16, Provenance) {
        use IconLibrary::{Imageres, Shell32};
        use Provenance::{BestMatch, Verified};
        match self {
            IconRole::Folder => (Imageres, 3, Verified),
            IconRole::Desktop => (Imageres, 183, Verified),
            IconRole::Downloads => (Imageres, 184, Verified),
            IconRole::Documents => (Imageres, 112, Verified),
            IconRole::Pictures => (Imageres, 113, Verified),
            IconRole::Music => (Imageres, 108, Verified),
            IconRole::Videos => (Imageres, 189, Verified),
            IconRole::Home => (Shell32, 51380, Verified),
            IconRole::Gallery => (Shell32, 51586, Verified),
            // This PC, and a computer on the network: the plain monitor, as
            // Network shows computers; checked on the user's extracted
            // imageres, like the groups below marked so.
            IconRole::ThisPc | IconRole::NetworkComputer => (Imageres, 109, Verified),
            IconRole::Network => (Imageres, 25, Verified),
            // The yellow folder with a green bar, Windows' network folder.
            IconRole::NetworkShare => (Imageres, 143, Verified),
            IconRole::NetworkDrive => (Imageres, 33, Verified),
            IconRole::NetworkDriveDisconnected => (Imageres, 31, Verified),
            IconRole::Drive => (Imageres, 32, Verified),
            // The plain dark drive, as a USB stick shows in Windows 11.
            IconRole::RemovableDrive => (Imageres, 35, Verified),
            // A disc in its drive: the app lists an optical drive only
            // while a disc is mounted (an empty one is imageres 30).
            IconRole::OpticalDrive => (Imageres, 56, Verified),
            IconRole::RecycleBinEmpty => (Imageres, 55, Verified),
            IconRole::RecycleBinFull => (Imageres, 54, Verified),
            IconRole::ZipFolder => (Imageres, 174, Verified),
            IconRole::TextFile => (Imageres, 102, Verified),
            IconRole::ImageFile => (Imageres, 132, Verified),
            IconRole::AudioFile => (Imageres, 131, Verified),
            IconRole::VideoFile => (Imageres, 133, Verified),
            IconRole::Executable => (Imageres, 15, Verified),
            IconRole::Shortcut => (Imageres, 15, BestMatch),
            IconRole::Script => (Imageres, 68, BestMatch),
            IconRole::GenericFile => (Imageres, 2, Verified),
            IconRole::Settings => (Shell32, 16826, BestMatch),
            // The full back and the flat front flap a folder with a
            // picture is composed of, and the folder with a page sticking
            // out, checked on Windows 11's imageres icon sheet.
            IconRole::FolderBack => (Imageres, 5, Verified),
            IconRole::FolderFront => (Imageres, 6, Verified),
            IconRole::FolderWithFiles => (Imageres, 162, Verified),
        }
    }

    /// The role's name in the cache's file names and index.
    pub const fn key(self) -> &'static str {
        match self {
            IconRole::Folder => "folder",
            IconRole::Desktop => "desktop",
            IconRole::Downloads => "downloads",
            IconRole::Documents => "documents",
            IconRole::Pictures => "pictures",
            IconRole::Music => "music",
            IconRole::Videos => "videos",
            IconRole::Home => "home",
            IconRole::Gallery => "gallery",
            IconRole::ThisPc => "this-pc",
            IconRole::Network => "network",
            IconRole::NetworkComputer => "network-computer",
            IconRole::NetworkShare => "network-share",
            IconRole::NetworkDrive => "network-drive",
            IconRole::NetworkDriveDisconnected => "network-drive-disconnected",
            IconRole::Drive => "drive",
            IconRole::RemovableDrive => "removable-drive",
            IconRole::OpticalDrive => "optical-drive",
            IconRole::RecycleBinEmpty => "recycle-bin-empty",
            IconRole::RecycleBinFull => "recycle-bin-full",
            IconRole::ZipFolder => "zip-folder",
            IconRole::TextFile => "text-file",
            IconRole::ImageFile => "image-file",
            IconRole::AudioFile => "audio-file",
            IconRole::VideoFile => "video-file",
            IconRole::Executable => "executable",
            IconRole::Script => "script",
            IconRole::Shortcut => "shortcut",
            IconRole::GenericFile => "generic-file",
            IconRole::Settings => "settings",
            IconRole::FolderBack => "folder-back",
            IconRole::FolderFront => "folder-front",
            IconRole::FolderWithFiles => "folder-with-files",
        }
    }

    /// The role whose [`Self::key`] is `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.key() == key)
    }
}
