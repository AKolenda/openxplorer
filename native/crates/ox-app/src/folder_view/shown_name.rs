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

use gtk::gio;

/// Whether names show their extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Extensions {
    /// Known types' extensions are hidden, as in Windows by default.
    #[default]
    Hidden,
    /// Every name shows whole.
    Shown,
}

/// The extension a view hides of `name`, with its dot, or `None` when the
/// name shows whole.
pub(crate) fn hidden_extension(name: &str, is_folder: bool, extensions: Extensions) -> Option<&str> {
    if is_folder || extensions == Extensions::Shown {
        return None;
    }
    let dot = name.rfind('.')?;
    // ".bashrc" is all name; "draft." has no extension to hide.
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    let extension = &name[dot..];
    is_known_extension(extension).then_some(extension)
}

/// Whether `extension`, with its dot, names a file type the desktop
/// knows: GIO names a type for it from the name alone, sure of it, and
/// not the type of unknown data. Each extension is asked once.
fn is_known_extension(extension: &str) -> bool {
    thread_local! {
        static KNOWN: std::cell::RefCell<std::collections::HashMap<String, bool>> =
            std::cell::RefCell::default();
    }
    let key = extension.to_lowercase();
    if let Some(known) = KNOWN.with(|known| known.borrow().get(&key).copied()) {
        return known;
    }
    let sample = format!("name{key}");
    let (content_type, uncertain) = gio::content_type_guess(Some(sample.as_str()), None::<&[u8]>);
    let known = !uncertain && !gio::content_type_is_unknown(&content_type);
    KNOWN.with(|cache| cache.borrow_mut().insert(key, known));
    known
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
}
