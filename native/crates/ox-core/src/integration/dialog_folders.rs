// SPDX-License-Identifier: AGPL-3.0-only
//! Where another application's Open or Save dialog starts: the folder the
//! user last chose in that application's dialogs, as Windows' common
//! file dialog remembers it.
//!
//! New in the native app (INT-035). Windows keeps, for each program, the
//! folder last used in its Open and Save dialogs, one value shared by both
//! (`ComDlg32\LastVisitedPidlMRU`, keyed by the program's file name), and
//! the first initial folder the program ever passed (`ComDlg32\FirstFolder`).
//! A dialog then starts, as `GetOpenFileName` documents for Windows 7 and
//! later:
//!
//! 1. in the remembered folder, when the program passes the same initial
//!    folder as it did the first time (also when it passes none, both
//!    times), so a program that always suggests one folder still opens
//!    where the user was last;
//! 2. else in the folder of a file the program names with its full path;
//! 3. else in the initial folder the program passes;
//! 4. else in the user's Documents folder, else the Desktop.
//!
//! The portal's options map onto these: `current_folder` is the initial
//! folder, `current_file` the full path, and `current_name` a name only,
//! which, as on Windows, never moves the dialog. One difference is kept on
//! purpose: a `current_file` comes first, as Windows' newer dialog does with
//! `IFileSaveDialog::SetSaveAsItem`, because the portal sends it only when
//! an application saves a file that already exists. A folder that no longer
//! exists, or a share that is offline, is skipped, and the next folder of
//! the list is used, so the dialog never starts on an error.
//!
//! The memory is `dialog-folders.json` in the settings folder: one folder
//! per application, at most [`MAX_APPS`] applications, the least recently
//! used dropped first, written privately (0600 in a 0700 folder) and
//! atomically like `settings.json`. A location is kept without any user
//! name or password ([`remembered_location`]), as settings keep locations
//! (SAFE-010).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::location::{normalise, unquote_lossy, without_user};
use crate::private_storage::{
    private_directory, private_file_if_present, read_limited_text, replace_file_atomically,
    PrivateFileOptions, StorageError,
};

/// The file in the settings folder.
pub const DIALOG_FOLDERS_FILE: &str = "dialog-folders.json";

/// The most applications remembered.
pub const MAX_APPS: usize = 50;

/// The longest application key or location kept, in bytes.
const MAX_TEXT_BYTES: usize = 4096;

/// The largest file read; [`MAX_APPS`] entries need far less.
const MAX_FILE_BYTES: u64 = 512 * 1024;

/// The prefix of the temporary files of an atomic save.
const TEMPORARY_PREFIX: &str = ".dialog-folders-";

/// One application's memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppFolders {
    /// The application, as [`caller_key`] names it.
    pub app: String,
    /// The initial folder the application passed the first time, as a
    /// `file://` URI; empty when it passed none.
    pub initial: String,
    /// The folder last chosen in its dialogs: a `file://` URI, or a
    /// network location such as `smb://nas/projects/Reports`.
    pub last: String,
}

/// The file's contents.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    apps: Vec<AppFolders>,
}

/// Where a dialog may start, in the order it tries them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartFolder {
    /// A local folder.
    Path(PathBuf),
    /// A remembered location, local or on the network.
    Location(String),
}

/// The remembered folders of every application's dialogs.
#[derive(Debug, Clone)]
pub struct DialogFolders {
    path: PathBuf,
    apps: Vec<AppFolders>,
}

impl DialogFolders {
    /// The memory in the settings folder `directory`; never fails. A
    /// missing, damaged or refused file is read as empty, and the next
    /// change replaces it.
    ///
    /// Every field is cleaned as it is read ([`read`]), and a file an
    /// earlier build wrote with a user name in it is written again
    /// without it, so none survives on disk.
    pub fn open(directory: &Path) -> Self {
        let path = directory.join(DIALOG_FOLDERS_FILE);
        let (apps, cleaned) = read(&path).unwrap_or_default();
        let folders = Self { path, apps };
        if cleaned {
            // Best effort: the memory in hand is clean either way.
            let _ = folders.save();
        }
        folders
    }

    /// What is remembered for `app`.
    pub fn of(&self, app: &str) -> Option<&AppFolders> {
        self.apps.iter().find(|entry| entry.app == app)
    }

    /// Remembers `folder` as `app`'s last folder, and, the first time,
    /// `initial`, the initial folder the application passed. A location
    /// that cannot be kept safely is not remembered.
    ///
    /// # Errors
    ///
    /// The [`StorageError`] of writing the file.
    pub fn remember(&mut self, app: &str, initial: Option<&Path>, folder: &str) -> Result<(), StorageError> {
        let Some(last) = remembered_location(folder) else {
            return Ok(());
        };
        if app.is_empty() || app.len() > MAX_TEXT_BYTES {
            return Ok(());
        }
        // Re-read, so a dialog of another window is not lost.
        self.apps = read(&self.path).unwrap_or_default().0;
        let initial = match self.apps.iter().position(|entry| entry.app == app) {
            Some(index) => self.apps.remove(index).initial,
            None => initial.map(initial_location).unwrap_or_default(),
        };
        self.apps.insert(
            0,
            AppFolders {
                app: app.to_owned(),
                initial,
                last,
            },
        );
        self.apps.truncate(MAX_APPS);
        self.save()
    }

    /// Writes the memory privately and atomically.
    fn save(&self) -> Result<(), StorageError> {
        let text = serde_json::to_string_pretty(&Stored {
            apps: self.apps.clone(),
        })
        .unwrap_or_default();
        private_directory(crate::private_storage::parent_directory(&self.path))?;
        replace_file_atomically(&self.path, TEMPORARY_PREFIX, text.as_bytes())
    }
}

/// Reads the memory at `path`, cleaning every field as [`remember`]
/// writes it: the folders through [`remembered_location`], so no user name
/// or password survives; an entry that cannot be cleaned is dropped. The
/// flag says whether anything changed. `None` when there is no memory or
/// it is unusable.
///
/// [`remember`]: DialogFolders::remember
fn read(path: &Path) -> Option<(Vec<AppFolders>, bool)> {
    let file = private_file_if_present(path, PrivateFileOptions::default()).ok()??;
    let text = read_limited_text(file, path, MAX_FILE_BYTES).ok()?;
    let stored: Stored = serde_json::from_str(&text).ok()?;
    let mut apps: Vec<AppFolders> = Vec::new();
    let mut cleaned = false;
    for entry in stored.apps {
        let last = remembered_location(&entry.last);
        let initial = if entry.initial.is_empty() {
            Some(String::new())
        } else {
            remembered_location(&entry.initial)
        };
        let sound_app = !entry.app.is_empty()
            && entry.app.len() <= MAX_TEXT_BYTES
            && !entry.app.chars().any(|c| c.is_control() || c == '/')
            && !apps.iter().any(|kept| kept.app == entry.app);
        match (last, initial) {
            (Some(last), Some(initial)) if sound_app && apps.len() < MAX_APPS => {
                cleaned |= last != entry.last || initial != entry.initial;
                apps.push(AppFolders {
                    app: entry.app,
                    initial,
                    last,
                });
            }
            _ => cleaned = true,
        }
    }
    Some((apps, cleaned))
}

/// How the memory keeps the initial folder `path` a caller passed: as
/// [`remembered_location`] keeps it, so a share's FUSE path becomes the
/// share's own address without its user. Empty when it cannot be kept.
fn initial_location(path: &Path) -> String {
    remembered_location(&folder_uri(path)).unwrap_or_default()
}

/// The remote address of a path inside `GVfs`'s FUSE export
/// (`…/gvfs/smb-share:server=nas,share=projects,user=sam/Film` is
/// `smb://nas/projects/Film`), without the user the export's name holds.
fn fuse_export_location(path: &Path) -> FusePath {
    use std::path::Component;
    let parts: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    let Some(index) = parts.iter().position(|part| *part == "gvfs").map(|gvfs| gvfs + 1) else {
        return FusePath::NotExport;
    };
    let Some(export) = parts.get(index).map(|export| export.to_str()) else {
        return FusePath::NotExport;
    };
    let Some((kind, fields)) = export.and_then(|export| export.split_once(':')) else {
        return FusePath::Unknown;
    };
    let field = |key: &str| {
        fields
            .split(',')
            .filter_map(|pair| pair.split_once('='))
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| unquote_lossy(value))
            .next_back()
    };
    let escape = |text: &str| glib::Uri::escape_string(text, None, false).to_string();
    let (scheme, host, first) = match kind {
        "smb-share" => ("smb", field("server"), field("share")),
        "sftp" | "ftp" | "ftps" | "dav" | "davs" | "afp-volume" => (
            kind.strip_suffix("-volume").unwrap_or(kind),
            field("host"),
            field("volume"),
        ),
        _ => return FusePath::Unknown,
    };
    let Some(host) = host.filter(|host| !host.is_empty()) else {
        return FusePath::Unknown;
    };
    let port = field("port").map(|port| format!(":{port}")).unwrap_or_default();
    let mut location = format!("{scheme}://{}{port}", escape(&host));
    for part in first.iter().map(String::as_str).chain(
        parts[index + 1..]
            .iter()
            .map(|part| part.to_str().unwrap_or_default()),
    ) {
        if part.is_empty() {
            return FusePath::Unknown;
        }
        location.push('/');
        location.push_str(&escape(part));
    }
    FusePath::Address(location)
}

/// What a local path is to [`fuse_export_location`].
enum FusePath {
    /// Not inside `GVfs`'s FUSE export.
    NotExport,
    /// Inside it: the remote address, without the export's user.
    Address(String),
    /// Inside an export it cannot turn into an address, which is then not
    /// kept at all, as the export's name may hold a user.
    Unknown,
}

/// The `file://` URI of a local folder, written from the path alone (never
/// the `smb://` URI `GVfs` would give a path inside its FUSE export).
pub fn folder_uri(path: &Path) -> String {
    glib::filename_to_uri(path, None).map_or_else(|_| String::new(), |uri| uri.to_string())
}

/// `location` as the memory keeps it: canonical, and without any user
/// name or password, which the address must never carry into a file
/// (SAFE-010); `None` for anything that is not a folder location.
pub fn remembered_location(location: &str) -> Option<String> {
    if location.is_empty() || location.len() > MAX_TEXT_BYTES || location.chars().any(char::is_control) {
        return None;
    }
    let (scheme, rest) = location.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    // The app's own pages (This PC, Network, a search) are not folders;
    // phones, cameras and iPhones GVfs mounts (mtp, gphoto2, afc) are.
    if !matches!(
        scheme.as_str(),
        "file" | "smb" | "sftp" | "ftp" | "ftps" | "dav" | "davs" | "nfs" | "afp" | "mtp" | "gphoto2" | "afc"
    ) {
        return None;
    }
    if scheme == "file" {
        // A share's FUSE path is kept as the share's address: its name
        // holds the share's user.
        if let Ok((path, _)) = glib::filename_from_uri(location) {
            match fuse_export_location(&path) {
                FusePath::NotExport => {}
                FusePath::Address(remote) => return remembered_location(&remote),
                FusePath::Unknown => return None,
            }
        }
    }
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let bare = format!("{scheme}://{host}{path}");
    normalise(&bare).ok().map(|uri| without_user(&uri))
}

/// The folders a dialog tries, in order, as the module describes:
/// `file_folder` is the folder of a file the caller named with its full
/// path, `initial` the folder it passed, `memory` what is remembered for
/// it, and `defaults` the Documents folder, the Desktop and the home
/// folder. The first one that is there is used.
pub fn start_folders(
    file_folder: Option<&Path>,
    initial: Option<&Path>,
    memory: Option<&AppFolders>,
    defaults: &[PathBuf],
) -> Vec<StartFolder> {
    let mut folders = Vec::new();
    if let Some(folder) = file_folder {
        folders.push(StartFolder::Path(folder.to_path_buf()));
    }
    let initial_uri = initial.map(initial_location).unwrap_or_default();
    if let Some(memory) = memory {
        if file_folder.is_none() && same_folder(&memory.initial, &initial_uri) {
            folders.push(StartFolder::Location(memory.last.clone()));
        }
    }
    if let Some(folder) = initial {
        folders.push(StartFolder::Path(folder.to_path_buf()));
    }
    // Where the caller's own folder is gone, its last folder still beats
    // Documents.
    if let Some(memory) = memory {
        let last = StartFolder::Location(memory.last.clone());
        if !folders.contains(&last) {
            folders.push(last);
        }
    }
    for folder in defaults {
        let folder = StartFolder::Path(folder.clone());
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

/// Whether two folder URIs (empty for none) name the same folder.
fn same_folder(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// The key of the calling application: the portal's application ID, or,
/// for an application the portal could not name (one started outside a
/// desktop launcher, so without a systemd unit to read its ID from),
/// `exe:` and its program's file name, as Windows keys by the program's
/// file name. `None` when neither is known: the dialog then remembers
/// nothing for it.
pub fn caller_key(app_id: &str, program: Option<&str>) -> Option<String> {
    let usable = |text: &str| {
        !text.is_empty() && text.len() <= 255 && !text.chars().any(|c| c.is_control() || c == '/')
    };
    if usable(app_id) {
        return Some(app_id.to_owned());
    }
    program
        .filter(|program| usable(program))
        .map(|program| format!("exe:{program}"))
}

/// The unique bus name of the application that opened a dialog, from the
/// request handle the portal made for it,
/// `/org/freedesktop/portal/desktop/request/<sender>/<token>`, where
/// `<sender>` is the caller's unique name without its `:` and with `_` for
/// each `.`.
pub fn caller_bus_name(handle: &str) -> Option<String> {
    let rest = handle.strip_prefix("/org/freedesktop/portal/desktop/request/")?;
    let (sender, token) = rest.split_once('/')?;
    let valid = !sender.is_empty()
        && !token.is_empty()
        && sender.chars().all(|c| c.is_ascii_digit() || c == '_')
        && sender.contains('_');
    valid.then(|| format!(":{}", sender.replace('_', ".")))
}

/// The file name of the program running as `pid`: its executable, else
/// the name the kernel keeps for it (`comm`), which the app can read also
/// when the executable link is closed to it.
pub fn program_of(pid: u32) -> Option<String> {
    let process = PathBuf::from(format!("/proc/{pid}"));
    let from_exe = fs::read_link(process.join("exe"))
        .ok()
        .and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().into_owned()))
        .map(|name| name.trim_end_matches(" (deleted)").to_owned());
    from_exe.or_else(|| {
        fs::read_to_string(process.join("comm"))
            .ok()
            .map(|comm| comm.trim_end().to_owned())
            .filter(|comm| !comm.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(initial: &str, last: &str) -> AppFolders {
        AppFolders {
            app: "org.example.Editor".to_owned(),
            initial: initial.to_owned(),
            last: last.to_owned(),
        }
    }

    fn defaults() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/home/demo/Documents"),
            PathBuf::from("/home/demo/Desktop"),
            PathBuf::from("/home/demo"),
        ]
    }

    fn path(text: &str) -> StartFolder {
        StartFolder::Path(PathBuf::from(text))
    }

    /// The same initial folder as the first time, or none both times,
    /// opens the remembered folder; a new one is the caller's choice.
    ///
    /// parity: INT-035
    #[test]
    fn the_remembered_folder_wins_while_the_caller_passes_its_first_folder() {
        let remembered = memory(
            "file:///home/demo/Downloads",
            "smb://nas.invalid/projects/Reports",
        );
        let same = start_folders(
            None,
            Some(Path::new("/home/demo/Downloads")),
            Some(&remembered),
            &defaults(),
        );
        assert_eq!(
            same[0],
            StartFolder::Location("smb://nas.invalid/projects/Reports".into())
        );
        assert_eq!(same[1], path("/home/demo/Downloads"), "then the caller's own");

        let other = start_folders(
            None,
            Some(Path::new("/home/demo/Pictures")),
            Some(&remembered),
            &defaults(),
        );
        assert_eq!(other[0], path("/home/demo/Pictures"), "a new folder is honoured");
        assert_eq!(
            other[1],
            StartFolder::Location("smb://nas.invalid/projects/Reports".into()),
            "the last folder if that one is gone"
        );

        let never = memory("", "file:///srv/demo/Out");
        let none = start_folders(None, None, Some(&never), &defaults());
        assert_eq!(none[0], StartFolder::Location("file:///srv/demo/Out".into()));
    }

    /// A full path opens its folder; a name alone does not move the
    /// dialog; nothing known opens Documents, then the Desktop, then home.
    ///
    /// parity: INT-035
    #[test]
    fn a_full_path_comes_first_and_nothing_known_opens_documents() {
        let remembered = memory("", "file:///srv/demo/Out");
        let saved = start_folders(
            Some(Path::new("/home/demo/Letters")),
            None,
            Some(&remembered),
            &defaults(),
        );
        assert_eq!(saved[0], path("/home/demo/Letters"));
        assert_eq!(saved[1], StartFolder::Location("file:///srv/demo/Out".into()));

        assert_eq!(
            start_folders(None, None, None, &defaults()),
            [
                path("/home/demo/Documents"),
                path("/home/demo/Desktop"),
                path("/home/demo")
            ]
        );
    }

    /// parity: INT-035, SAFE-010
    #[test]
    fn a_location_is_kept_without_its_user_or_password() {
        assert_eq!(
            remembered_location("smb://demo:Fict1onal-pass@nas.invalid/projects/Reports").as_deref(),
            Some("smb://nas.invalid/projects/Reports")
        );
        assert_eq!(
            remembered_location("sftp://anna@build.invalid/srv/out").as_deref(),
            Some("sftp://build.invalid/srv/out")
        );
        assert_eq!(
            remembered_location("file:///home/demo/My%20Files").as_deref(),
            Some("file:///home/demo/My%20Files")
        );
        assert_eq!(remembered_location("search:///?q=report"), None);
        assert_eq!(remembered_location("not a location"), None);
    }

    /// A phone, camera or iPhone that `GVfs` mounts is a folder the
    /// dialog opens like any other, so it is remembered too.
    ///
    /// parity: INT-035
    #[test]
    fn a_connected_device_folder_is_remembered() {
        for location in [
            "mtp://Example_Phone/Internal%20storage/DCIM",
            "gphoto2://Example_Camera/DCIM",
            "afc://00008030-example/Documents",
        ] {
            assert!(remembered_location(location).is_some(), "{location}");
        }
    }

    /// One folder per application, persisted privately, with no password;
    /// the first initial folder is kept, and only so many apps are.
    ///
    /// parity: INT-035, SAFE-010
    #[test]
    fn the_memory_persists_one_private_folder_per_app() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("a settings folder");
        let settings = directory.path().join("winspace");
        let mut folders = DialogFolders::open(&settings);
        assert_eq!(folders.of("org.example.Editor"), None);
        folders
            .remember(
                "org.example.Editor",
                Some(Path::new("/home/demo/Downloads")),
                "smb://demo:Fict1onal-pass@nas.invalid/projects/Reports/",
            )
            .expect("written");
        folders
            .remember(
                "org.example.Editor",
                Some(Path::new("/home/demo/Pictures")),
                "file:///srv/demo/Out",
            )
            .expect("written");
        folders
            .remember("exe:chrome", None, "file:///srv/demo/Pages")
            .expect("written");

        let again = DialogFolders::open(&settings);
        assert_eq!(
            again.of("org.example.Editor"),
            Some(&memory("file:///home/demo/Downloads", "file:///srv/demo/Out"))
        );
        assert_eq!(
            again.of("exe:chrome").map(|entry| entry.initial.as_str()),
            Some("")
        );
        let file = settings.join(DIALOG_FOLDERS_FILE);
        let text = fs::read_to_string(&file).expect("the file");
        assert!(!text.contains("Fict1onal"), "no password");
        assert!(!text.contains("demo@"), "no user name");
        let mode = fs::metadata(&file).expect("the file").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let mut many = DialogFolders::open(&settings);
        for index in 0..MAX_APPS + 5 {
            many.remember(&format!("org.example.App{index}"), None, "file:///srv/demo/Out")
                .expect("written");
        }
        let kept = DialogFolders::open(&settings);
        assert_eq!(kept.apps.len(), MAX_APPS);
        assert!(kept.of("org.example.App0").is_none(), "the oldest went first");
        assert!(kept.of(&format!("org.example.App{}", MAX_APPS + 4)).is_some());
    }

    /// A FUSE path names its share's user (`user=sam`): the first folder
    /// passed as one is kept as the share's own address without it, and
    /// still matches when the caller passes it again; so is a last folder.
    ///
    /// parity: INT-035, SAFE-010
    #[test]
    fn a_fuse_path_is_kept_as_its_share_without_the_user() {
        let directory = tempfile::tempdir().expect("a settings folder");
        let fuse = Path::new("/run/user/1000/gvfs/smb-share:server=studio-nas,share=projects,user=sam/Film");
        let mut folders = DialogFolders::open(directory.path());
        folders
            .remember(
                "org.example.Editor",
                Some(fuse),
                "file:///run/user/1000/gvfs/smb-share:server=studio-nas,share=projects,user=sam/Film/Cuts",
            )
            .expect("written");

        let text = fs::read_to_string(directory.path().join(DIALOG_FOLDERS_FILE)).expect("the file");
        assert!(!text.contains("sam"), "{text}");
        let memory = DialogFolders::open(directory.path());
        let entry = memory.of("org.example.Editor").expect("remembered");
        assert_eq!(entry.initial, "smb://studio-nas/projects/Film");
        assert_eq!(entry.last, "smb://studio-nas/projects/Film/Cuts");
        let folders = start_folders(None, Some(fuse), Some(entry), &[]);
        assert_eq!(
            folders[0],
            StartFolder::Location(entry.last.clone()),
            "the same first folder"
        );
    }

    /// A file an earlier build wrote with a user name in it is cleaned as
    /// it is read, on disk too, so no user name survives.
    ///
    /// parity: INT-035, SAFE-010
    #[test]
    fn an_older_memory_with_a_user_name_is_cleaned_when_read() {
        let directory = tempfile::tempdir().expect("a settings folder");
        let file = directory.path().join(DIALOG_FOLDERS_FILE);
        fs::write(
            &file,
            r#"{"apps":[{"app":"org.example.Editor","initial":"file:///run/user/1000/gvfs/smb-share:server=studio-nas,share=projects,user=sam","last":"smb://sam@studio-nas/projects/Reports"}]}"#,
        )
        .expect("written");

        let memory = DialogFolders::open(directory.path());

        let entry = memory.of("org.example.Editor").expect("kept, cleaned");
        assert_eq!(entry.initial, "smb://studio-nas/projects");
        assert_eq!(entry.last, "smb://studio-nas/projects/Reports");
        let text = fs::read_to_string(&file).expect("the file");
        assert!(!text.contains("sam"), "{text}");
    }

    /// A damaged file is read as empty and replaced by the next change.
    ///
    /// parity: INT-035
    #[test]
    fn a_damaged_memory_is_read_as_empty() {
        let directory = tempfile::tempdir().expect("a settings folder");
        fs::write(directory.path().join(DIALOG_FOLDERS_FILE), "{not json").expect("written");
        let mut folders = DialogFolders::open(directory.path());
        assert!(folders.of("org.example.Editor").is_none());
        folders
            .remember("org.example.Editor", None, "file:///srv/demo/Out")
            .expect("written");
        assert!(DialogFolders::open(directory.path())
            .of("org.example.Editor")
            .is_some());
    }

    /// parity: INT-035
    #[test]
    fn the_caller_is_its_app_id_else_its_program() {
        assert_eq!(
            caller_key("org.example.Editor", Some("editor")).as_deref(),
            Some("org.example.Editor")
        );
        assert_eq!(caller_key("", Some("chrome")).as_deref(), Some("exe:chrome"));
        assert_eq!(caller_key("", None), None);
        assert_eq!(caller_key("", Some("")), None);
        assert_eq!(
            caller_bus_name("/org/freedesktop/portal/desktop/request/1_26/AEC3D87C").as_deref(),
            Some(":1.26")
        );
        assert_eq!(
            caller_bus_name("/org/freedesktop/portal/desktop/request/x/y"),
            None
        );
        assert_eq!(caller_bus_name("/elsewhere/1_2/y"), None);
        let me = program_of(std::process::id()).expect("this test's own program");
        assert!(!me.is_empty() && !me.contains('/'));
    }
}
