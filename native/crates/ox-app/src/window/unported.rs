// SPDX-License-Identifier: AGPL-3.0-only
//! Commands the Python app has whose native workflow does not exist yet.
//!
//! Owner rule 1 forbids dropping a control, and a button that silently does
//! nothing would be worse than none. So each of these commands is a real
//! window action that stays disabled: its button and menu row are shown
//! greyed out with a tooltip naming the milestone that brings it
//! (`native/ROADMAP.md`). Porting a command means replacing its entry here
//! with a working action.

use super::BrowserWindow;
use gtk::prelude::*;

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
    /// The window action's name, without `win.`.
    pub action: &'static str,
    /// When it arrives.
    pub milestone: Milestone,
}

const fn command(action: &'static str, milestone: Milestone) -> UnportedCommand {
    UnportedCommand { action, milestone }
}

/// Every command that is shown but disabled.
pub(super) const UNPORTED_COMMANDS: [UnportedCommand; 20] = [
    command("new-folder", Milestone::FileOperations),
    command("new-text-document", Milestone::FileOperations),
    command("new-file", Milestone::FileOperations),
    command("new-markdown-document", Milestone::FileOperations),
    command("new-csv-file", Milestone::FileOperations),
    command("new-json-file", Milestone::FileOperations),
    command("new-html-document", Milestone::FileOperations),
    command("new-from-template", Milestone::FileOperations),
    command("cut", Milestone::FileOperations),
    command("copy", Milestone::FileOperations),
    command("paste", Milestone::FileOperations),
    command("rename", Milestone::FileOperations),
    command("trash", Milestone::FileOperations),
    command("map-network-location", Milestone::NetworkAndDevices),
    command("discover-servers", Milestone::NetworkAndDevices),
    command("cache-folder", Milestone::SearchAndMetadata),
    command("settings", Milestone::PreferencesAndSessions),
    command("license", Milestone::PreferencesAndSessions),
    command("default-file-explorer", Milestone::DesktopIntegration),
    command("check-updates", Milestone::Distribution),
];

/// The milestone that brings the command `action` (with or without its
/// `win.` prefix), or `None` for a command that works.
fn milestone_of(action: &str) -> Option<Milestone> {
    let name = action.strip_prefix("win.").unwrap_or(action);
    let unported = UNPORTED_COMMANDS.iter().find(|command| command.action == name);
    unported.map(|command| command.milestone)
}

/// Whether `action` is a command that is shown but disabled.
pub(super) fn is_unported(action: &str) -> bool {
    milestone_of(action).is_some()
}

/// The tooltip of a disabled command's control: its usual tooltip, then
/// the milestone that enables it. Other commands keep `tooltip` as it is.
pub(super) fn tooltip(action: &str, tooltip: &str) -> String {
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
            let action = gtk::gio::SimpleAction::new(command.action, None);
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
            tooltip("win.cut", "Cut (Ctrl+X)"),
            "Cut (Ctrl+X)\nNot in the native preview yet: arrives with file operations."
        );
    }

    #[test]
    fn a_working_command_keeps_its_tooltip() {
        assert_eq!(tooltip("win.copy-path", "Copy path"), "Copy path");
    }
}
