// SPDX-License-Identifier: AGPL-3.0-only
//! Copies of items taken out of a ZIP browsed as a folder (ARC-026).
//!
//! Copy (Ctrl+C) and dragging items out of a ZIP need real files, which
//! other folders and applications can read. The selected members are
//! extracted, with every rule of the extractor, into a new folder below
//! [`copies_root`] (the user's cache, on disk rather than in memory), and
//! the clipboard or the drag hands over those files, as Explorer hands
//! over files it extracts to its temporary folder. Copies are left for
//! the paste or drop to read and are removed a day later
//! ([`remove_old_copies`]), when the app next starts or copies, with
//! what an extraction stopped by a crash left there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::extract::is_staging_name;
use crate::random::{random_hex, NAME_BYTES};

/// How long copies are kept for a later paste or drop.
pub const COPY_LIFETIME: Duration = Duration::from_hours(24);

/// The prefix of each copy's folder name.
const COPY_PREFIX: &str = "copy-";

/// Where copies taken out of ZIPs go: `$XDG_CACHE_HOME/winspace/zip-copies`.
pub fn copies_root() -> PathBuf {
    glib::user_cache_dir().join("winspace").join("zip-copies")
}

/// Creates `root` as a private folder (`0700`) when it is missing.
///
/// # Errors
///
/// When the folder cannot be created or is a symlink.
pub fn prepare_copies_root(root: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::other("the folder for copies from ZIPs is a symlink"));
        }
        Ok(metadata) if metadata.is_dir() => {
            return fs::set_permissions(root, fs::Permissions::from_mode(0o700));
        }
        Ok(_) => return Err(io::Error::other("the folder for copies from ZIPs is a file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::DirBuilder::new().recursive(true).mode(0o700).create(root)
}

/// A new, unpredictable folder name for one copy.
///
/// # Errors
///
/// When the system's random source cannot be read.
pub fn copy_folder_name() -> io::Result<String> {
    Ok(format!("{COPY_PREFIX}{}", random_hex(NAME_BYTES)?))
}

/// The path of `member` (as the archive browser names it) inside the copy
/// folder `folder`.
pub fn copied_member(folder: &Path, member: &str) -> PathBuf {
    member
        .trim_end_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .fold(folder.to_path_buf(), |path, segment| path.join(segment))
}

/// Removes the copies in `root` older than `lifetime`, and the staging
/// folders of extractions into `root` that never finished. Best effort
/// and blocking: a copy that cannot be removed is left for the next
/// time. Only folders this module or the extractor names are touched,
/// and links are never followed.
pub fn remove_old_copies(root: &Path, lifetime: Duration) {
    remove_old_folders(
        root,
        |name| name.starts_with(COPY_PREFIX) || is_staging_name(name),
        lifetime,
    );
}

/// Removes the folders in `root` whose names `is_ours` accepts and that
/// are older than `lifetime`, by their modification time. Best effort and
/// blocking; links are never followed.
pub(super) fn remove_old_folders(root: &Path, is_ours: impl Fn(&str) -> bool, lifetime: Duration) {
    let Ok(children) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for child in children.flatten() {
        let is_copy = child.file_name().to_str().is_some_and(&is_ours);
        let Ok(metadata) = child.path().symlink_metadata() else {
            continue;
        };
        let is_old = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > lifetime);
        if is_copy && metadata.is_dir() && is_old {
            let _ = fs::remove_dir_all(child.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// parity: ARC-026
    #[test]
    fn old_copies_and_staging_folders_are_removed_and_nothing_else() {
        let root = tempfile::tempdir().expect("a folder");
        let copies = root.path().join("zip-copies");
        prepare_copies_root(&copies).expect("created");
        let old = copies.join(copy_folder_name().expect("random"));
        let fresh = copies.join(copy_folder_name().expect("random"));
        let other = copies.join("notes");
        let old_staging = copies.join(format!(".openxplorer-extract-{}.part", "0a".repeat(NAME_BYTES)));
        let fresh_staging = copies.join(format!(".openxplorer-extract-{}.part", "1b".repeat(NAME_BYTES)));
        for folder in [&old, &fresh, &other, &old_staging, &fresh_staging] {
            fs::create_dir_all(folder.join("inner")).expect("a folder");
        }
        let long_ago = SystemTime::now() - Duration::from_hours(3 * 24);
        for folder in [&old, &other, &old_staging] {
            fs::File::open(folder)
                .and_then(|folder| folder.set_modified(long_ago))
                .expect("aged");
        }

        remove_old_copies(&copies, COPY_LIFETIME);

        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(!old_staging.exists(), "an extraction stopped by a crash");
        assert!(fresh_staging.exists(), "an extraction that may still run");
        assert!(other.exists(), "only the copies' own folders");
        assert_eq!(
            copied_member(&fresh, "tidewater/maps/"),
            fresh.join("tidewater").join("maps")
        );
        let mode = fs::metadata(&copies).expect("listed").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
