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
    /// The "Network and devices" service.
    NetworkAndDevices,
    /// The "Search and metadata" service: the search index.
    SearchAndMetadata,
    /// The rest of "Search and metadata": properties, folder sizes and
    /// Open with.
    ItemDetails,
    /// The "Archives and recovery" service: ZIP extraction and previous
    /// versions.
    ArchivesAndRecovery,
    /// The "Desktop integration" service: default apps, Show in folder,
    /// the terminal and the Brave download folder.
    DesktopIntegration,
    /// The "Distribution" service: packages, the source they ship with and
    /// the update flow.
    Distribution,
}

impl Milestone {
    /// How tooltips and disabled settings name the milestone.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Milestone::NetworkAndDevices => "network and device support",
            Milestone::SearchAndMetadata => "cached search",
            Milestone::ItemDetails => "properties, folder sizes and Open with",
            Milestone::ArchivesAndRecovery => "archives and previous versions",
            Milestone::DesktopIntegration => "desktop integration",
            Milestone::Distribution => "packaging and updates",
        }
    }

    /// The sentence a disabled control shows under its usual text: "Not in
    /// the native preview yet: arrives with packaging and updates."
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
pub(super) const UNPORTED_COMMANDS: [UnportedCommand; 12] = [
    command(WindowAction::MapNetworkLocation, Milestone::NetworkAndDevices),
    command(WindowAction::DiscoverServers, Milestone::NetworkAndDevices),
    command(WindowAction::SignOut, Milestone::NetworkAndDevices),
    command(WindowAction::CacheFolder, Milestone::SearchAndMetadata),
    command(WindowAction::OpenWith, Milestone::ItemDetails),
    command(WindowAction::CalculateFolderSize, Milestone::ItemDetails),
    command(WindowAction::Properties, Milestone::ItemDetails),
    command(WindowAction::ExtractAll, Milestone::ArchivesAndRecovery),
    command(WindowAction::PreviousVersions, Milestone::ArchivesAndRecovery),
    command(WindowAction::OpenInTerminal, Milestone::DesktopIntegration),
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
            tooltip(WindowAction::CheckUpdates, "Check for updates"),
            "Check for updates\nNot in the native preview yet: arrives with packaging and updates."
        );
    }

    #[test]
    fn a_working_command_keeps_its_tooltip() {
        assert_eq!(tooltip(WindowAction::CopyPath, "Copy path"), "Copy path");
        assert_eq!(tooltip(WindowAction::Rename, "Rename (F2)"), "Rename (F2)");
        assert_eq!(tooltip(WindowAction::Cut, "Cut (Ctrl+X)"), "Cut (Ctrl+X)");
    }
}
