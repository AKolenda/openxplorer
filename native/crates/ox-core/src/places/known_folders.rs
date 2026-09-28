// SPDX-License-Identifier: AGPL-3.0-only
//! The XDG standard folders (Desktop, Downloads, ...) and where they are.
//!
//! Ports `FOLDERS` and `FolderLocations.paths` from
//! `desktop/folder_locations.py`, and the Quick access glyph colours from
//! `environment` in `desktop/winspace.py`. The paths are read from
//! `user-dirs.dirs` on every call instead of through the `GLib` special-folder
//! cache, which lives for the whole process: a folder moved with
//! `xdg-user-dirs-update` or the Python app shows up at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::user_dirs::{self, UserDirs};
use super::Place;
use crate::location::file_uri;

/// `GLib` log domain for problems with `user-dirs.dirs`.
const LOG_DOMAIN: &str = "openxplorer";

/// An XDG standard folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    /// `XDG_DESKTOP_DIR`.
    Desktop,
    /// `XDG_DOWNLOAD_DIR`.
    Downloads,
    /// `XDG_DOCUMENTS_DIR`.
    Documents,
    /// `XDG_PICTURES_DIR`.
    Pictures,
    /// `XDG_MUSIC_DIR`.
    Music,
    /// `XDG_VIDEOS_DIR`.
    Videos,
    /// `XDG_TEMPLATES_DIR`, the source of New > from template.
    Templates,
    /// `XDG_PUBLICSHARE_DIR`.
    Public,
}

impl KnownFolder {
    /// Every standard folder, in the order of `FOLDERS`.
    pub const ALL: [KnownFolder; 8] = [
        KnownFolder::Desktop,
        KnownFolder::Downloads,
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Music,
        KnownFolder::Videos,
        KnownFolder::Templates,
        KnownFolder::Public,
    ];

    /// The standard folders Quick access shows, in their default order.
    pub const QUICK_ACCESS: [KnownFolder; 6] = [
        KnownFolder::Desktop,
        KnownFolder::Downloads,
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Music,
        KnownFolder::Videos,
    ];

    /// The `<NAME>` in `XDG_<NAME>_DIR`.
    pub const fn xdg_key(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "DESKTOP",
            KnownFolder::Downloads => "DOWNLOAD",
            KnownFolder::Documents => "DOCUMENTS",
            KnownFolder::Pictures => "PICTURES",
            KnownFolder::Music => "MUSIC",
            KnownFolder::Videos => "VIDEOS",
            KnownFolder::Templates => "TEMPLATES",
            KnownFolder::Public => "PUBLICSHARE",
        }
    }

    /// The folder named `XDG_<key>_DIR`, or `None` for another name.
    pub fn from_xdg_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|folder| folder.xdg_key() == key)
    }

    /// The visible name, which is also the default folder name in the home
    /// folder.
    pub const fn label(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "Desktop",
            KnownFolder::Downloads => "Downloads",
            KnownFolder::Documents => "Documents",
            KnownFolder::Pictures => "Pictures",
            KnownFolder::Music => "Music",
            KnownFolder::Videos => "Videos",
            KnownFolder::Templates => "Templates",
            KnownFolder::Public => "Public",
        }
    }

    /// The glyph drawn for the folder.
    pub const fn glyph(self) -> &'static str {
        match self {
            KnownFolder::Desktop => "desktop",
            KnownFolder::Downloads => "downloads",
            KnownFolder::Documents | KnownFolder::Templates => "documents",
            KnownFolder::Pictures => "pictures",
            KnownFolder::Music => "music",
            KnownFolder::Videos => "videos",
            KnownFolder::Public => "folder",
        }
    }

    /// The glyph colour in Quick access, in CSS hex notation; `None` for
    /// folders Quick access does not show.
    pub const fn glyph_color(self) -> Option<&'static str> {
        match self {
            KnownFolder::Desktop => Some("#3b8ec7"),
            KnownFolder::Downloads => Some("#138266"),
            KnownFolder::Documents => Some("#4a94d1"),
            KnownFolder::Pictures => Some("#9a79cb"),
            KnownFolder::Music => Some("#c66b9c"),
            KnownFolder::Videos => Some("#b48540"),
            KnownFolder::Templates | KnownFolder::Public => None,
        }
    }
}

/// Where to look for the standard folders: the home folder and the
/// `user-dirs.dirs` file. Creating it reads nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderLocations {
    home: PathBuf,
    user_dirs_file: PathBuf,
}

impl FolderLocations {
    /// The user's home folder and `$XDG_CONFIG_HOME/user-dirs.dirs`
    /// (`~/.config/user-dirs.dirs` by default).
    pub fn from_environment() -> Self {
        Self::new(glib::home_dir(), &glib::user_config_dir())
    }

    /// The standard folders of `home`, configured in
    /// `config_directory/user-dirs.dirs`.
    pub fn new(home: PathBuf, config_directory: &Path) -> Self {
        Self {
            home,
            user_dirs_file: config_directory.join("user-dirs.dirs"),
        }
    }

    /// Reads `user-dirs.dirs` now and returns every standard folder. A
    /// folder without a valid line is `~/<Label>`; nothing is created.
    ///
    /// A missing file means every default. A file that cannot be read (too
    /// large, not UTF-8, not a regular file) also means every default,
    /// with a logged warning, where the Python app failed to build the
    /// sidebar at all.
    pub fn read_paths(&self) -> KnownFolderPaths {
        let mut configured = self.read_configured();
        let mut paths = HashMap::new();
        for folder in KnownFolder::ALL {
            let default_path = self.home.join(folder.label());
            let path = configured.remove(&folder).unwrap_or(default_path);
            paths.insert(folder, path);
        }
        KnownFolderPaths { paths }
    }

    /// The folders `user-dirs.dirs` configures; none, with a logged
    /// warning, if it cannot be read.
    fn read_configured(&self) -> UserDirs {
        match user_dirs::read(&self.user_dirs_file, &self.home) {
            Ok(configured) => configured,
            Err(error) => {
                let file = self.user_dirs_file.display();
                glib::g_warning!(LOG_DOMAIN, "{file}: {error} Using the default standard folders.");
                UserDirs::new()
            }
        }
    }
}

/// The path of every standard folder, as read at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownFolderPaths {
    paths: HashMap<KnownFolder, PathBuf>,
}

impl KnownFolderPaths {
    /// Where `folder` is.
    ///
    /// # Panics
    ///
    /// Never: [`FolderLocations::read_paths`] fills in every standard
    /// folder.
    pub fn path(&self, folder: KnownFolder) -> &Path {
        self.paths
            .get(&folder)
            .expect("FolderLocations::read_paths fills in every standard folder")
    }

    /// The Quick access rows of the six standard folders it shows, before
    /// hidden folders and the saved order are applied.
    pub fn quick_access_places(&self) -> Vec<Place> {
        let place = |folder: KnownFolder| Place {
            label: folder.label().to_owned(),
            uri: file_uri(self.path(folder)),
            known_folder: Some(folder),
            is_shared: false,
        };
        KnownFolder::QUICK_ACCESS.into_iter().map(place).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        home: PathBuf,
        config: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("a temporary directory");
            let home = root.path().join("home");
            let config = root.path().join("config");
            fs::create_dir_all(&config).expect("the config directory is created");
            Self {
                _root: root,
                home,
                config,
            }
        }

        fn locations(&self) -> FolderLocations {
            FolderLocations::new(self.home.clone(), &self.config)
        }

        fn write_user_dirs(&self, contents: impl AsRef<[u8]>) {
            fs::write(self.config.join("user-dirs.dirs"), contents).expect("user-dirs.dirs is written");
        }
    }

    #[test]
    fn xdg_keys_round_trip() {
        for folder in KnownFolder::ALL {
            assert_eq!(KnownFolder::from_xdg_key(folder.xdg_key()), Some(folder));
        }
        assert_eq!(KnownFolder::from_xdg_key("DOWNLOADS"), None);
    }

    /// A `user-dirs.dirs` that cannot be read.
    struct UnreadableCase {
        /// Why it cannot be read.
        name: &'static str,
        /// The file's bytes.
        contents: Vec<u8>,
    }

    #[test]
    fn a_missing_file_gives_every_default() {
        let fixture = Fixture::new();
        let paths = fixture.locations().read_paths();
        for folder in KnownFolder::ALL {
            assert_eq!(paths.path(folder), fixture.home.join(folder.label()));
        }
    }

    /// Python's `paths()` re-reads the file on every call, so a folder moved
    /// by another program shows up without a restart.
    #[test]
    fn every_call_reads_the_file_again() {
        let fixture = Fixture::new();
        let locations = fixture.locations();
        fixture.write_user_dirs("XDG_DOWNLOAD_DIR=\"$HOME/Incoming\"\n");
        let before = locations.read_paths();
        fixture.write_user_dirs("XDG_DOWNLOAD_DIR=\"/elsewhere/Downloads\"\n");
        let after = locations.read_paths();
        assert_eq!(before.path(KnownFolder::Downloads), fixture.home.join("Incoming"));
        assert_eq!(
            after.path(KnownFolder::Downloads),
            Path::new("/elsewhere/Downloads")
        );
    }

    #[test]
    fn an_unreadable_file_gives_every_default() {
        let fixture = Fixture::new();
        let oversized = format!("XDG_DESKTOP_DIR=\"/data/Desk\"\n{}", "#".repeat(128 * 1024));
        let cases = [
            UnreadableCase {
                name: "over 128 KiB",
                contents: oversized.into_bytes(),
            },
            UnreadableCase {
                name: "not UTF-8",
                contents: b"XDG_DESKTOP_DIR=\"/data/Caf\xe9\"\n".to_vec(),
            },
        ];
        for case in cases {
            fixture.write_user_dirs(&case.contents);
            let paths = fixture.locations().read_paths();
            let desktop = paths.path(KnownFolder::Desktop);
            assert_eq!(desktop, fixture.home.join("Desktop"), "{}", case.name);
        }
    }

    /// parity: SIDE-005
    #[test]
    fn quick_access_shows_six_folders_with_their_glyphs_and_colours() {
        let fixture = Fixture::new();
        fixture.write_user_dirs("XDG_PICTURES_DIR=\"/data//Photos (2024)/\"\n");
        let places = fixture.locations().read_paths().quick_access_places();
        let labels: Vec<&str> = places.iter().map(|place| place.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Desktop", "Downloads", "Documents", "Pictures", "Music", "Videos"]
        );
        let folders: Vec<Option<KnownFolder>> = places.iter().map(|place| place.known_folder).collect();
        assert_eq!(folders, KnownFolder::QUICK_ACCESS.map(Some));
        let pictures = &places[3];
        assert_eq!(pictures.uri, "file:///data/Photos%20%282024%29");
        assert_eq!(
            (pictures.glyph(), pictures.glyph_color()),
            (Some("pictures"), Some("#9a79cb"))
        );
        assert!(places.iter().all(|place| !place.is_shared));
    }
}
