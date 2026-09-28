// SPDX-License-Identifier: AGPL-3.0-only
//! Commands the Python app has whose native workflow does not exist yet.
//!
//! Owner rule 1 ("only gain functionality", `native/docs/ui-spec.md` §1.2)
//! forbids dropping a control, and a button that silently does nothing
//! would be worse than none. So each of these commands is a real
//! window action that stays disabled: its button and menu row are shown
//! greyed out with a tooltip naming the milestone that brings it
//! (`native/ROADMAP.md`). Porting a command means replacing its entry here
//! with a working action.

use gtk::gio;
use gtk::prelude::*;

use super::window_action::WindowAction;
use super::BrowserWindow;

/// The `native/ROADMAP.md` milestone that brings a command or a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Milestone {
    /// "Complete safe file-operation workflows", which also brings file
    /// and tab drag-and-drop between windows.
    FileOperations,
    /// The "Search and metadata" service: the search index and folder
    /// sizes.
    SearchAndMetadata,
    /// The "Desktop integration" service: default apps, Show in folder and
    /// the Brave download folder.
    DesktopIntegration,
    /// The "Distribution" service: packages, the source they ship with and
    /// the update flow.
    Distribution,
}

impl Milestone {
    /// How tooltips and disabled settings name the milestone.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Milestone::FileOperations => "file operations",
            Milestone::SearchAndMetadata => "cached search",
            Milestone::DesktopIntegration => "desktop integration",
            Milestone::Distribution => "packaging and updates",
        }
    }

    /// The sentence a disabled control shows under its usual text: "Not in
    /// the native preview yet: arrives with file operations."
    pub(crate) fn notice(self) -> String {
        format!(
            "Not in the native preview yet: arrives with {}.",
            self.description()
        )
    }
}

/// A disabled command and the milestone that brings it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UnportedCommand {
    /// The disabled window action.
    pub action: WindowAction,
    /// When it arrives.
    pub milestone: Milestone,
}

const fn command(action: WindowAction, milestone: Milestone) -> UnportedCommand {
    UnportedCommand { action, milestone }
}

/// Every command that is shown but disabled.
pub(super) const UNPORTED_COMMANDS: [UnportedCommand; 16] = [
    command(WindowAction::NewFolder, Milestone::FileOperations),
    command(WindowAction::NewTextDocument, Milestone::FileOperations),
    command(WindowAction::NewFile, Milestone::FileOperations),
    command(WindowAction::NewMarkdownDocument, Milestone::FileOperations),
    command(WindowAction::NewCsvFile, Milestone::FileOperations),
    command(WindowAction::NewJsonFile, Milestone::FileOperations),
    command(WindowAction::NewHtmlDocument, Milestone::FileOperations),
    command(WindowAction::NewFromTemplate, Milestone::FileOperations),
    command(WindowAction::Cut, Milestone::FileOperations),
    command(WindowAction::Copy, Milestone::FileOperations),
    command(WindowAction::Paste, Milestone::FileOperations),
    command(WindowAction::Rename, Milestone::FileOperations),
    command(WindowAction::Trash, Milestone::FileOperations),
    command(WindowAction::CacheFolder, Milestone::SearchAndMetadata),
    // The dialog names where the installed source and the corresponding
    // source archive are, which packaging decides.
    command(WindowAction::License, Milestone::Distribution),
    command(WindowAction::CheckUpdates, Milestone::Distribution),
];

/// The milestone that brings the command `action`, or `None` for a
/// command that works.
fn milestone_of(action: WindowAction) -> Option<Milestone> {
    let unported = UNPORTED_COMMANDS.iter().find(|command| command.action == action);
    unported.map(|command| command.milestone)
}

/// Whether `action` is a command that is shown but disabled.
pub(super) fn is_unported(action: WindowAction) -> bool {
    milestone_of(action).is_some()
}

/// The tooltip of a disabled command's control: its usual tooltip, then
/// the milestone that enables it. Other commands keep `tooltip` as it is.
pub(super) fn tooltip(action: WindowAction, tooltip: &str) -> String {
    match milestone_of(action) {
        Some(milestone) => format!("{tooltip}\n{}", milestone.notice()),
        None => tooltip.to_owned(),
    }
}

impl BrowserWindow {
    /// Adds every unported command as a disabled window action.
    pub(super) fn install_unported_actions(&self) {
        for command in UNPORTED_COMMANDS {
            let action = gio::SimpleAction::new(command.action.name(), None);
            action.set_enabled(false);
            self.add_action(&action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_command_names_the_milestone_that_brings_it() {
        assert_eq!(
            tooltip(WindowAction::Cut, "Cut (Ctrl+X)"),
            "Cut (Ctrl+X)\nNot in the native preview yet: arrives with file operations."
        );
    }

    #[test]
    fn a_working_command_keeps_its_tooltip() {
        assert_eq!(tooltip(WindowAction::CopyPath, "Copy path"), "Copy path");
    }
}
