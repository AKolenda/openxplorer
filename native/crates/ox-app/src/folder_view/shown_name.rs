// SPDX-License-Identifier: AGPL-3.0-only
//! The name a view shows for an item: its real name, or, while file name
//! extensions are hidden, its name without the extension of a known file
//! type, as Windows Explorer's "Hide extensions for known file types" does
//! (VIEW-069).
//!
//! Windows hides an extension only when it is registered as a file type;
//! a name whose extension names no type keeps it, so it is never mistaken
//! for another file. The desktop's register of file types here is the
//! shared MIME database, which GIO reads: an extension is known when GIO
//! names a type for it from the name alone, without guessing. Folders
//! never lose anything, a name that starts with a dot and has no other
//! (`.bashrc`) keeps it whole, and only the last extension goes, so
//! `backup.tar.gz` shows as `backup.tar`, as in Windows.

use ox_core::entry::Entry;

/// Whether names show their extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Extensions {
    /// Known types' extensions are hidden, as in Windows by default.
    #[default]
    Hidden,
    /// Every name shows whole.
    Shown,
}

/// Whether `entry`'s name always shows whole: a folder's, or a
/// shortcut's ([`Entry::is_shortcut`]), whose name is all Windows shows
/// of it, as Windows hides only a shortcut's `.lnk`, which a link here
/// does not have. A symbolic link named for a type ("notes.txt") is a
/// file, as Windows treats one.
pub(crate) fn shows_whole(entry: &Entry) -> bool {
    entry.is_dir || entry.is_shortcut()
}

/// The extension a view hides of `name`, with its dot, or `None` when the
/// name shows whole; `is_folder` is true for a name that always shows
/// whole ([`shows_whole`]).
pub(crate) fn hidden_extension(name: &str, is_folder: bool, extensions: Extensions) -> Option<&str> {
    if is_folder || extensions == Extensions::Shown {
        return None;
    }
    ox_core::entry::known_extension(name)
}

/// The name a view shows for `name`.
pub(crate) fn shown_name(name: &str, is_folder: bool, extensions: Extensions) -> &str {
    match hidden_extension(name, is_folder, extensions) {
        Some(extension) => &name[..name.len() - extension.len()],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows' rule: a known type's extension is hidden, the rest of the
    /// name stays; folders, names with no known type, names that are only
    /// a dot and an extension, and names ending in a dot keep everything.
    ///
    /// parity: VIEW-069
    #[test]
    fn only_a_known_types_extension_is_hidden_and_never_a_folders() {
        fn hidden(name: &str) -> &str {
            shown_name(name, false, Extensions::Hidden)
        }
        assert_eq!(hidden("Notes 2.txt"), "Notes 2");
        assert_eq!(hidden("Budget.PDF"), "Budget");
        assert_eq!(hidden("holiday.photo.jpeg"), "holiday.photo");
        assert_eq!(hidden("backup.tar.gz"), "backup.tar");
        assert_eq!(hidden(".bashrc"), ".bashrc", "a dotfile keeps its name");
        assert_eq!(hidden(".private"), ".private");
        assert_eq!(hidden("README"), "README");
        assert_eq!(hidden("draft."), "draft.");
        assert_eq!(hidden("report v1.2 final"), "report v1.2 final", "no known type");
        assert_eq!(hidden("capture.zz9q7x"), "capture.zz9q7x", "no known type");
        assert_eq!(
            shown_name("Projects.txt", true, Extensions::Hidden),
            "Projects.txt"
        );
        assert_eq!(shown_name("Notes 2.txt", false, Extensions::Shown), "Notes 2.txt");
        assert_eq!(
            hidden_extension("Notes 2.txt", false, Extensions::Hidden),
            Some(".txt")
        );
        assert_eq!(hidden_extension("Notes 2.txt", false, Extensions::Shown), None);
    }

    /// Every file of a type the desktop knows hides its extension, also
    /// a type GIO is unsure of because more than one type uses the
    /// extension (`.png`, `.json`, `.html`, `.py`), as Windows hides the
    /// extension of every registered type. Only the last extension goes,
    /// and an extension of digits alone is a version, not a type.
    ///
    /// parity: VIEW-069
    #[test]
    fn every_known_type_hides_its_extension_also_one_gio_is_unsure_of() {
        fn hidden(name: &str) -> &str {
            shown_name(name, false, Extensions::Hidden)
        }
        assert_eq!(hidden("sunset.png"), "sunset");
        assert_eq!(hidden("data.json"), "data");
        assert_eq!(hidden("page.html"), "page");
        assert_eq!(hidden("script.py"), "script");
        assert_eq!(hidden("plan.pdf"), "plan");
        assert_eq!(hidden("Budget 2026.txt"), "Budget 2026");
        assert_eq!(hidden("Bundle.tar.gz"), "Bundle.tar");
        assert_eq!(hidden("Notes v2.1"), "Notes v2.1", "a version number");
        assert_eq!(hidden("Report.txt - Shortcut"), "Report.txt - Shortcut");
    }
}
