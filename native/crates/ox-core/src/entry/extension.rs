// SPDX-License-Identifier: AGPL-3.0-only
//! Whether a name ends in the extension of a file type the desktop knows.
//!
//! Windows treats a name's extension as a file type only when the type is
//! registered: Explorer hides only those extensions (VIEW-069), and a
//! symbolic link named for one (`photo.jpg`) is typed and renamed by it,
//! while a link named with none is a shortcut. The desktop's register of
//! file types here is the shared MIME database, which GIO reads: an
//! extension is known when GIO names a type for it from the name alone,
//! without guessing.

use std::cell::RefCell;
use std::collections::HashMap;

/// The extension of a known file type that `name` ends in, with its dot,
/// or `None`: a name with no dot, a name that starts with its only dot
/// (`.bashrc`), a name ending in a dot, and an extension that names no
/// type (`v1.2 final`, `Report.txt - Shortcut`) have none. Only the last
/// extension counts, so `backup.tar.gz` ends in `.gz`.
pub fn known_extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    let extension = &name[dot..];
    is_known_extension(extension).then_some(extension)
}

/// Whether `extension`, with its dot, names a file type the desktop
/// knows: GIO names a type for it from the name alone, sure of it, and
/// not the type of unknown data. Each extension is asked once per thread.
fn is_known_extension(extension: &str) -> bool {
    thread_local! {
        static KNOWN: RefCell<HashMap<String, bool>> = RefCell::default();
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
