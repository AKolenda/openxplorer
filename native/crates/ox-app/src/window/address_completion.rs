// SPDX-License-Identifier: AGPL-3.0-only
//! What the list under the editable address shows: completions of the
//! typed address (NAV-030) and the typed history (NAV-043), which app.js
//! lacked (its address input had `autocomplete=off`).
//!
//! The text up to the last `/` or `\` is resolved as the address bar
//! resolves any address (`~`, relative paths, `smb://`), that folder is
//! listed, and the names starting with the rest, ignoring case, are
//! offered; folders end in the separator so typing can go on. Hidden
//! names are offered once the typed name starts with a dot.

use gtk::glib;
use ox_core::entry::enumerate_folder;

use crate::folder_view::sorting::SortKey;

use super::BrowserWindow;

/// The most completions offered.
const COMPLETION_LENGTH: usize = 12;

/// The folder part of `typed`, up to and including its last separator,
/// and the start of a name after it.
fn split_typed(typed: &str) -> Option<(&str, &str)> {
    let at = typed.rfind(['/', '\\'])?;
    Some((&typed[..=at], &typed[at + 1..]))
}

/// The completions of `prefix` in the folder typed as `base`, from its
/// `entries` (name, is a folder), in natural order.
fn completions(base: &str, prefix: &str, entries: &[(String, bool)]) -> Vec<String> {
    let separator = if base.ends_with('\\') { '\\' } else { '/' };
    let wanted = prefix.to_lowercase();
    let shows_hidden = prefix.starts_with('.');
    let mut matches: Vec<(SortKey, String)> = entries
        .iter()
        .filter(|(name, _)| shows_hidden || !name.starts_with('.'))
        .filter(|(name, _)| name.to_lowercase().starts_with(&wanted) && name != prefix)
        .map(|(name, is_dir)| {
            let end = if *is_dir {
                separator.to_string()
            } else {
                String::new()
            };
            (SortKey::new(name), format!("{base}{name}{end}"))
        })
        .collect();
    matches.sort_by(|(a, _), (b, _)| a.natural_cmp(b));
    matches
        .into_iter()
        .take(COMPLETION_LENGTH)
        .map(|(_, text)| text)
        .collect()
}

impl BrowserWindow {
    /// Offers completions of `typed`, the text being typed in the address.
    pub(super) fn complete_address(&self, typed: &str) {
        let bar = self.address_bar();
        let folder = split_typed(typed).and_then(|(base, _)| self.resolve_address(base).ok());
        let Some(folder) = folder else {
            bar.hide_suggestions();
            return;
        };
        let typed = typed.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak]
            bar,
            async move {
                let mut entries = Vec::new();
                let listed = enumerate_folder(&folder, |batch| {
                    entries.extend(batch.into_iter().map(|entry| (entry.name, entry.is_dir)));
                })
                .await;
                let Some((base, prefix)) = split_typed(&typed) else {
                    return;
                };
                let offered = if listed.is_ok() {
                    completions(base, prefix, &entries)
                } else {
                    Vec::new()
                };
                bar.show_completions(&typed, &offered);
            }
        ));
    }

    /// F4 and the chevron: edits the address with the typed history listed
    /// below it, as Explorer's address drop-down.
    pub(super) fn edit_address_from_history(&self) {
        if !self.address_bar().is_typing() {
            self.edit_address();
        }
        self.address_bar().show_history();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NAV-030
    #[test]
    fn names_starting_with_the_typed_text_complete_it() {
        let entries = [
            ("Documents".to_owned(), true),
            ("docs.txt".to_owned(), false),
            ("Downloads".to_owned(), true),
            (".dotfile".to_owned(), false),
            ("Music".to_owned(), true),
        ];

        let (base, prefix) = split_typed("/home/demo/Do").expect("a folder part");
        let offered = completions(base, prefix, &entries);

        assert_eq!((base, prefix), ("/home/demo/", "Do"));
        assert_eq!(
            offered,
            [
                "/home/demo/docs.txt",
                "/home/demo/Documents/",
                "/home/demo/Downloads/"
            ]
        );
        assert_eq!(completions("/home/demo/", ".", &entries), ["/home/demo/.dotfile"]);
        assert_eq!(
            completions("\\\\nas\\share\\", "mu", &entries),
            ["\\\\nas\\share\\Music\\"]
        );
        assert_eq!(split_typed("Documents"), None);
    }
}
