// SPDX-License-Identifier: AGPL-3.0-only
//! The name-conflict dialog before a paste or drop (OPS-026, OPS-028).
//!
//! Ports the "Items already exist" dialog of `transferWithConflicts` in
//! `v2.0.0:desktop/ui/app.js`, with its message and its Cancel, "Skip
//! duplicates" and "Replace existing" buttons. The native dialog adds
//! "Keep both" (the engine's `(copy N)` names) and, with several
//! conflicts, "Apply to all": cleared, the answer is for the first
//! conflicting item only, and the dialog asks again about the next one,
//! as Windows' "Let me decide for each file" does. As in Dolphin and
//! Windows, the dialog shows the first conflicting item beside the one it
//! would replace, with sizes and dates and whether the files are
//! identical ([`super::conflict_compare`]), and when it is a file it
//! offers "Replace older", which replaces only files whose existing copy
//! is older and skips the rest. "Rename" puts the first item in under the
//! name typed in "New name", which starts as a free suggestion
//! ([`super::conflict_rename`]). An item copied into its own folder is
//! not offered Replace, since it cannot replace itself.

use gtk::prelude::*;
use ox_core::transfer::ConflictPolicy;

use super::conflict_compare::{compare, compare_dates, Comparison};
use super::conflict_rename::{checked_new_name, suggested_name};
use crate::dialog::{Dialog, DialogButton};
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The dialog's title.
const TITLE: &str = crate::i18n::message_id("Items already exist");

/// One conflicting item and the user's answer for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConflictAnswer {
    /// The item's URI.
    pub(super) uri: String,
    /// What happens to it.
    pub(super) policy: ConflictPolicy,
    /// The name it goes in under instead, when the answer was Rename.
    pub(super) rename_to: Option<String>,
}

/// The dialog's message for `count` conflicts in `destination`, word for
/// word as app.js writes it.
fn conflict_message(count: usize, destination: &str) -> String {
    ox_core::i18n::format_message("{count} matching name(s) in {destination}\n\nReplace existing files or skip conflicts. Same-name folders are merged; destination-only files stay in place.", &[("count", &count.to_string()), ("destination", destination)])
}

/// The label of "Apply to all" for `count` conflicts.
fn apply_to_all_label(count: usize) -> String {
    ox_core::i18n::format_message("Apply to all {count} items", &[("count", &count.to_string())])
}

/// The name an item's URI ends in, as the dialog shows it.
fn item_name(uri: &str) -> String {
    let file = gtk::gio::File::for_uri(uri);
    file.basename()
        .map_or_else(|| uri.to_owned(), |name| name.to_string_lossy().into_owned())
}

/// What the user answered in one dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    /// One policy.
    Policy(ConflictPolicy),
    /// Replace files whose existing copy is older, skip the others.
    ReplaceOlder,
    /// The first item goes in under this name.
    Rename(String),
}

/// The buttons of one dialog and the answer each gives.
#[derive(Debug)]
struct PolicyButtons {
    skip: DialogButton,
    keep_both: DialogButton,
    rename: DialogButton,
    replace_older: Option<DialogButton>,
    replace: Option<DialogButton>,
}

impl PolicyButtons {
    /// Adds Cancel, "Skip duplicates", "Keep both", "Rename", "Replace
    /// older" when `offers.replace_older`, and "Replace existing" when
    /// `offers.replace`, the primary button. Without it, "Keep both" is.
    fn add_to(dialog: &Dialog, offers: Offers) -> Self {
        dialog.add_cancel_button();
        let skip = dialog.add_button(
            ox_core::i18n::gettext_static("Skip duplicates"),
            ButtonStyle::Bordered,
        );
        let keep_both_style = if offers.replace {
            ButtonStyle::Bordered
        } else {
            ButtonStyle::Accent
        };
        let keep_both = dialog.add_button(ox_core::i18n::gettext_static("Keep both"), keep_both_style);
        let rename = dialog.add_button(ox_core::i18n::gettext_static("Rename"), ButtonStyle::Bordered);
        let replace_older = offers.replace_older.then(|| {
            dialog.add_button(
                ox_core::i18n::gettext_static("Replace older"),
                ButtonStyle::Bordered,
            )
        });
        let replace = offers.replace.then(|| {
            dialog.add_button(
                ox_core::i18n::gettext_static("Replace existing"),
                ButtonStyle::Accent,
            )
        });
        Self {
            skip,
            keep_both,
            rename,
            replace_older,
            replace,
        }
    }

    /// The answer `button` stands for; `None` for Rename, whose answer is
    /// the typed name.
    fn policy_choice(&self, button: DialogButton) -> Option<Choice> {
        if Some(button) == self.replace {
            Some(Choice::Policy(ConflictPolicy::Replace))
        } else if button == self.keep_both {
            Some(Choice::Policy(ConflictPolicy::KeepBoth))
        } else if Some(button) == self.replace_older {
            Some(Choice::ReplaceOlder)
        } else if button == self.rename {
            None
        } else {
            debug_assert_eq!(button, self.skip, "no other button answers");
            Some(Choice::Policy(ConflictPolicy::Skip))
        }
    }
}

/// Which of the replacing answers one dialog offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Offers {
    /// "Replace existing": not when the first item is the existing item
    /// itself, which may not replace itself.
    replace: bool,
    /// "Replace older": when both sides of the first item are files.
    replace_older: bool,
}

impl Offers {
    /// The answers offered for a first item compared as `first`.
    fn for_first(first: Option<&Comparison>) -> Self {
        let same_item = first.is_some_and(|comparison| comparison.same_item);
        let both_files =
            first.is_some_and(|comparison| !comparison.incoming.is_folder && !comparison.existing.is_folder);
        Self {
            replace: !same_item,
            replace_older: both_files && !same_item,
        }
    }
}

/// The policy `choice` gives the item compared in `comparison`.
fn policy_for(choice: &Choice, comparison: Option<&Comparison>) -> ConflictPolicy {
    match choice {
        Choice::Policy(policy) => *policy,
        Choice::ReplaceOlder if comparison.is_some_and(Comparison::existing_is_older) => {
            ConflictPolicy::Replace
        }
        Choice::ReplaceOlder | Choice::Rename(_) => ConflictPolicy::Skip,
    }
}

impl BrowserWindow {
    /// Asks what happens to each of `conflicts`, items whose names are
    /// taken in `destination_folder`, shown as `destination`. `None` when
    /// the user cancels: nothing may change then.
    pub(super) async fn ask_about_conflicts(
        &self,
        conflicts: &[String],
        destination_folder: &str,
        destination: &str,
    ) -> Option<Vec<ConflictAnswer>> {
        let mut answers = Vec::with_capacity(conflicts.len());
        while answers.len() < conflicts.len() {
            let remaining = &conflicts[answers.len()..];
            let first = compare(&remaining[0], destination_folder).await;
            let (choice, applies_to_all) = self
                .ask_once(remaining, destination_folder, destination, first.as_ref())
                .await?;
            if let Choice::Rename(name) = choice {
                // A typed name is for one item only.
                answers.push(ConflictAnswer {
                    uri: remaining[0].clone(),
                    policy: ConflictPolicy::Skip,
                    rename_to: Some(name),
                });
                continue;
            }
            let answered = if applies_to_all {
                remaining
            } else {
                &remaining[..1]
            };
            for (index, uri) in answered.iter().enumerate() {
                let comparison = match (&choice, index) {
                    (Choice::ReplaceOlder, 0) => first,
                    (Choice::ReplaceOlder, _) => compare_dates(uri, destination_folder).await,
                    _ => None,
                };
                answers.push(ConflictAnswer {
                    uri: uri.clone(),
                    policy: policy_for(&choice, comparison.as_ref()),
                    rename_to: None,
                });
            }
        }
        Some(answers)
    }

    /// One dialog about `remaining`, showing `first`, the comparison of
    /// the first of them: the answer, and whether it applies to all of
    /// them; `None` for Cancel.
    async fn ask_once(
        &self,
        remaining: &[String],
        destination_folder: &str,
        destination: &str,
        first: Option<&Comparison>,
    ) -> Option<(Choice, bool)> {
        let first_uri = &remaining[0];
        let suggestion = suggested_name(first_uri, destination_folder).await;
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(TITLE),
            &conflict_message(remaining.len(), destination),
        );
        if let Some(comparison) = first {
            dialog.add_note(&format!(
                "“{}”\n{}",
                item_name(first_uri),
                comparison.lines().join("\n")
            ));
        }
        let new_name = dialog.add_text_field(ox_core::i18n::gettext_static("New name"), &suggestion);
        let apply_to_all = (remaining.len() > 1).then(|| {
            let check = dialog.add_check_button(&apply_to_all_label(remaining.len()), true);
            let first = item_name(first_uri);
            dialog.add_hint(&ox_core::i18n::format_message(
                "Otherwise the choice is for “{first}” only. A new name is always for “{first}” only.",
                &[("first", &first)],
            ));
            check
        });
        let buttons = PolicyButtons::add_to(&dialog, Offers::for_first(first));
        dialog.submit_with(&new_name, buttons.rename);
        // Cancel keeps the focus, as before the dialog had a name field.
        dialog.open_on_first_button();
        let choice = loop {
            let pressed = dialog.next_response().await?;
            if let Some(choice) = buttons.policy_choice(pressed) {
                break choice;
            }
            match checked_new_name(&new_name.text(), first_uri, destination_folder).await {
                Ok(name) => break Choice::Rename(name),
                Err(message) => dialog.show_error(&message),
            }
        };
        let applies_to_all = apply_to_all.as_ref().is_none_or(gtk::CheckButton::is_active);
        dialog.finish();
        Some((choice, applies_to_all))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-026
    #[test]
    fn the_conflict_message_keeps_the_python_wording() {
        assert_eq!(
            conflict_message(2, "/home/user/Projects"),
            "2 matching name(s) in /home/user/Projects\n\nReplace existing files or skip conflicts. \
             Same-name folders are merged; destination-only files stay in place."
        );
        assert_eq!(apply_to_all_label(3), "Apply to all 3 items");
        assert_eq!(item_name("file:///tmp/a%20b.txt"), "a b.txt");
    }
}
