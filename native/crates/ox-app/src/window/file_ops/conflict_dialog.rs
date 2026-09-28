// SPDX-License-Identifier: AGPL-3.0-only
//! The name-conflict dialog before a paste or drop (OPS-026, OPS-028).
//!
//! Ports the "Items already exist" dialog of `transferWithConflicts` in
//! `desktop/ui/app.js`, with its message and its Cancel, "Skip
//! duplicates" and "Replace existing" buttons. The native dialog adds
//! "Keep both" (the engine's `(copy N)` names) and, with several
//! conflicts, "Apply to all": cleared, the answer is for the first
//! conflicting item only, and the dialog asks again about the next one,
//! as Windows' "Let me decide for each file" does.

use gtk::prelude::*;
use ox_core::transfer::ConflictPolicy;

use crate::window::dialog::{ButtonStyle, Dialog, DialogButton};
use crate::window::BrowserWindow;

/// The dialog's title.
const TITLE: &str = "Items already exist";

/// One conflicting item and the user's answer for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConflictAnswer {
    /// The item's URI.
    pub(super) uri: String,
    /// What happens to it.
    pub(super) policy: ConflictPolicy,
}

/// The dialog's message for `count` conflicts in `destination`, word for
/// word as app.js writes it.
fn conflict_message(count: usize, destination: &str) -> String {
    format!(
        "{count} matching name(s) in {destination}\n\nReplace existing files or skip conflicts. Same-name \
         folders are merged; destination-only files stay in place."
    )
}

/// The label of "Apply to all" for `count` conflicts.
fn apply_to_all_label(count: usize) -> String {
    format!("Apply to all {count} items")
}

/// The name an item's URI ends in, as the dialog shows it.
fn item_name(uri: &str) -> String {
    let file = gtk::gio::File::for_uri(uri);
    file.basename()
        .map_or_else(|| uri.to_owned(), |name| name.to_string_lossy().into_owned())
}

/// The buttons of one dialog and the answer each gives.
#[derive(Debug)]
struct PolicyButtons {
    skip: DialogButton,
    keep_both: DialogButton,
    replace: DialogButton,
}

impl PolicyButtons {
    /// Adds Cancel, "Skip duplicates", "Keep both" and the primary
    /// "Replace existing".
    fn add_to(dialog: &Dialog) -> Self {
        dialog.add_cancel_button();
        Self {
            skip: dialog.add_button("Skip duplicates", ButtonStyle::Standard),
            keep_both: dialog.add_button("Keep both", ButtonStyle::Standard),
            replace: dialog.add_button("Replace existing", ButtonStyle::Primary),
        }
    }

    /// The policy `button` stands for.
    fn policy(&self, button: DialogButton) -> ConflictPolicy {
        if button == self.replace {
            ConflictPolicy::Replace
        } else if button == self.keep_both {
            ConflictPolicy::KeepBoth
        } else {
            debug_assert_eq!(button, self.skip, "the dialog has four buttons");
            ConflictPolicy::Skip
        }
    }
}

impl BrowserWindow {
    /// Asks what happens to each of `conflicts`, items whose names are
    /// taken in the folder shown as `destination`. `None` when the user
    /// cancels: nothing may change then.
    pub(super) async fn ask_about_conflicts(
        &self,
        conflicts: &[String],
        destination: &str,
    ) -> Option<Vec<ConflictAnswer>> {
        let mut answers = Vec::with_capacity(conflicts.len());
        while answers.len() < conflicts.len() {
            let remaining = &conflicts[answers.len()..];
            let (policy, applies_to_all) = self.ask_once(remaining, destination).await?;
            let answered = if applies_to_all {
                remaining
            } else {
                &remaining[..1]
            };
            answers.extend(answered.iter().map(|uri| ConflictAnswer {
                uri: uri.clone(),
                policy,
            }));
        }
        Some(answers)
    }

    /// One dialog about `remaining`: the chosen policy, and whether it
    /// applies to all of them; `None` for Cancel.
    async fn ask_once(&self, remaining: &[String], destination: &str) -> Option<(ConflictPolicy, bool)> {
        let dialog = Dialog::new(self, TITLE, &conflict_message(remaining.len(), destination));
        let apply_to_all = (remaining.len() > 1).then(|| {
            let check = dialog.add_check_button(&apply_to_all_label(remaining.len()), true);
            let first = item_name(&remaining[0]);
            dialog.add_hint(&format!("Otherwise the choice is for “{first}” only."));
            check
        });
        let buttons = PolicyButtons::add_to(&dialog);
        dialog.open();
        let pressed = dialog.next_response().await;
        let applies_to_all = apply_to_all.as_ref().is_none_or(gtk::CheckButton::is_active);
        dialog.finish();
        Some((buttons.policy(pressed?), applies_to_all))
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
