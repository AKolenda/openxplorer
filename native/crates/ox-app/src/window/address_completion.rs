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
//!
//! A folder is listed once while editing: typing on in it filters the
//! names kept, and a listing still running is dropped when the typed
//! folder changes or editing ends, so typing in a big or remote folder
//! does not start a listing per key.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::enumerate_folder;

use crate::folder_view::sorting::SortKey;

use super::BrowserWindow;

/// The most completions offered.
const COMPLETION_LENGTH: usize = 12;

/// A folder's names, with whether each is a folder.
type Names = Rc<Vec<(String, bool)>>;

/// The folder listed for completions while the address is edited.
#[derive(Debug, Default)]
pub(super) struct CompletionListing {
    /// The last folder listed and its names.
    listed: RefCell<Option<(String, Names)>>,
    /// The folder being listed, and the listing.
    running: RefCell<Option<(String, glib::JoinHandle<()>)>>,
}

impl CompletionListing {
    /// The names kept for `folder`, if it was the last listed.
    fn names_of(&self, folder: &str) -> Option<Names> {
        let listed = self.listed.borrow();
        let (kept, names) = listed.as_ref()?;
        (kept == folder).then(|| Rc::clone(names))
    }

    /// Whether `folder` is being listed now.
    fn is_listing(&self, folder: &str) -> bool {
        self.running
            .borrow()
            .as_ref()
            .is_some_and(|(running, _)| running == folder)
    }

    /// Drops the listing running, if any.
    fn stop(&self) {
        if let Some((_, listing)) = self.running.take() {
            listing.abort();
        }
    }
}

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
        let split = split_typed(typed);
        let folder = split.and_then(|(base, _)| self.resolve_address(base).ok());
        let (Some((base, prefix)), Some(folder)) = (split, folder) else {
            self.forget_address_completions();
            bar.hide_suggestions();
            return;
        };
        let state = &self.imp().completion_listing;
        if let Some(names) = state.names_of(&folder) {
            bar.show_completions(typed, &completions(base, prefix, &names));
            return;
        }
        if state.is_listing(&folder) {
            return;
        }
        state.stop();
        let listing = glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            folder,
            async move {
                let mut entries = Vec::new();
                let listed = enumerate_folder(&folder, |batch| {
                    entries.extend(batch.into_iter().map(|entry| (entry.name, entry.is_dir)));
                })
                .await;
                if listed.is_err() {
                    entries.clear();
                }
                let state = &window.imp().completion_listing;
                state.running.replace(None);
                state.listed.replace(Some((folder, Rc::new(entries))));
                window.complete_address(&window.address_bar().typed_text());
            }
        ));
        state.running.replace(Some((folder, listing)));
    }

    /// Editing ended: the folder listed is dropped, and any listing.
    pub(super) fn forget_address_completions(&self) {
        let state = &self.imp().completion_listing;
        state.stop();
        state.listed.replace(None);
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
