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

/// The `native/ROADMAP.md` milestone that brings a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Milestone {
    /// "Complete safe file-operation workflows".
    FileOperations,
    /// The "Network and devices" service.
    NetworkAndDevices,
    /// The "Search and metadata" service.
    SearchAndMetadata,
    /// The "Preferences and sessions" service: the Settings page.
    PreferencesAndSessions,
    /// The "Desktop integration" service.
    DesktopIntegration,
    /// The "Distribution" service: the update flow.
    Distribution,
}

impl Milestone {
    /// How the tooltip names the milestone.
    const fn description(self) -> &'static str {
        match self {
            Milestone::FileOperations => "file operations",
            Milestone::NetworkAndDevices => "network and device support",
            Milestone::SearchAndMetadata => "cached search",
            Milestone::PreferencesAndSessions => "the Settings page",
            Milestone::DesktopIntegration => "desktop integration",
            Milestone::Distribution => "the update flow",
        }
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
pub(super) const UNPORTED_COMMANDS: [UnportedCommand; 20] = [
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
    command(WindowAction::MapNetworkLocation, Milestone::NetworkAndDevices),
    command(WindowAction::DiscoverServers, Milestone::NetworkAndDevices),
    command(WindowAction::CacheFolder, Milestone::SearchAndMetadata),
    command(WindowAction::Settings, Milestone::PreferencesAndSessions),
    command(WindowAction::License, Milestone::PreferencesAndSessions),
    command(WindowAction::DefaultFileExplorer, Milestone::DesktopIntegration),
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
        Some(milestone) => format!(
            "{tooltip}\nNot in the native preview yet: arrives with {}.",
            milestone.description()
        ),
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
