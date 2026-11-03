// SPDX-License-Identifier: AGPL-3.0-only
//! Shortcuts: a symbolic link named as Windows Explorer names a shortcut
//! (DND-027, DND-019).
//!
//! Windows names a new shortcut after its target's whole displayed name,
//! extension included, followed by " - Shortcut" (its default
//! `ShortcutNameTemplate`, `%s - Shortcut.lnk`), and numbers it when that
//! name is taken, as its unique-name rule does for every new item:
//! `report.txt - Shortcut`, then `report.txt - Shortcut (2)`, `(3)` and so
//! on. A Linux shortcut is a symbolic link, so there is no hidden `.lnk`.
//!
//! Safety rule "a shortcut never replaces anything": each name is claimed
//! by the system call that makes the link, which fails on any existing
//! item, a broken link included, so a name taken between two tries is
//! skipped too.

use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// What follows the target's name in a shortcut's name.
const SHORTCUT_SUFFIX: &str = " - Shortcut";

/// The most numbered names tried before giving up.
const MAX_NUMBER: u32 = 10_000;

/// The name of the `number`th shortcut to `name`: `name - Shortcut`, then
/// `name - Shortcut (2)`.
fn shortcut_name(name: &str, number: u32) -> String {
    if number <= 1 {
        format!("{name}{SHORTCUT_SUFFIX}")
    } else {
        format!("{name}{SHORTCUT_SUFFIX} ({number})")
    }
}

/// Makes a shortcut to `source` in `folder`: a symbolic link to the
/// absolute path of `source`, named `name - Shortcut`, or numbered when
/// that is taken. It never replaces anything. It runs on the calling
/// thread; the link's path is returned.
///
/// # Errors
///
/// A `source` without a name (such as `/`), a name too long for the
/// folder, every numbered name taken, or the system's failure, such as a
/// folder that cannot be written or a file system without symbolic links.
pub fn create_shortcut(source: &Path, folder: &Path) -> io::Result<PathBuf> {
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the item has no name"))?
        .to_string_lossy()
        .into_owned();
    let target = if source.is_absolute() {
        source.to_path_buf()
    } else {
        std::env::current_dir()?.join(source)
    };
    for number in 1..=MAX_NUMBER {
        let link = folder.join(shortcut_name(&name, number));
        match symlink(&target, &link) {
            Ok(()) => return Ok(link),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "every shortcut name is taken",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temporary_folder;

    /// parity: DND-027
    #[test]
    fn a_shortcut_keeps_the_whole_name_and_says_shortcut() {
        let folder = temporary_folder();
        let file = folder.path().join("report.txt");
        let projects = folder.path().join("Projects");
        let archive = folder.path().join("notes.tar.gz");
        std::fs::write(&file, b"report").expect("fixture file");
        std::fs::create_dir(&projects).expect("fixture folder");
        std::fs::write(&archive, b"notes").expect("fixture file");
        let shortcuts = folder.path().join("Shortcuts");
        std::fs::create_dir(&shortcuts).expect("fixture folder");

        let to_file = create_shortcut(&file, &shortcuts).expect("a shortcut");
        let to_folder = create_shortcut(&projects, &shortcuts).expect("a shortcut");
        let to_archive = create_shortcut(&archive, &shortcuts).expect("a shortcut");

        assert_eq!(to_file, shortcuts.join("report.txt - Shortcut"));
        assert_eq!(to_folder, shortcuts.join("Projects - Shortcut"));
        assert_eq!(to_archive, shortcuts.join("notes.tar.gz - Shortcut"));
        assert_eq!(std::fs::read_link(&to_file).expect("a symbolic link"), file);
        assert_eq!(std::fs::read_link(&to_folder).expect("a symbolic link"), projects);
    }

    /// parity: DND-027
    #[test]
    fn a_taken_name_is_numbered_and_never_replaced() {
        let folder = temporary_folder();
        let file = folder.path().join("plan.odt");
        std::fs::write(&file, b"plan").expect("fixture file");
        std::fs::write(folder.path().join("plan.odt - Shortcut"), b"keep me").expect("fixture file");
        symlink("/no/such/place", folder.path().join("plan.odt - Shortcut (2)")).expect("a broken link");

        let made = create_shortcut(&file, folder.path()).expect("a shortcut");
        let next = create_shortcut(&file, folder.path()).expect("a shortcut");

        assert_eq!(made, folder.path().join("plan.odt - Shortcut (3)"));
        assert_eq!(next, folder.path().join("plan.odt - Shortcut (4)"));
        assert_eq!(
            std::fs::read(folder.path().join("plan.odt - Shortcut")).expect("kept"),
            b"keep me"
        );
        assert_eq!(
            std::fs::read_link(folder.path().join("plan.odt - Shortcut (2)")).expect("kept"),
            PathBuf::from("/no/such/place")
        );
    }

    /// parity: DND-027
    #[test]
    fn a_folder_that_cannot_take_it_is_an_error() {
        let folder = temporary_folder();
        let file = folder.path().join("a.txt");
        std::fs::write(&file, b"a").expect("fixture file");

        let missing = create_shortcut(&file, &folder.path().join("missing"));
        let nameless = create_shortcut(Path::new("/"), folder.path());

        assert_eq!(
            missing.map_err(|error| error.kind()),
            Err(io::ErrorKind::NotFound)
        );
        assert_eq!(
            nameless.map_err(|error| error.kind()),
            Err(io::ErrorKind::InvalidInput)
        );
    }
}
