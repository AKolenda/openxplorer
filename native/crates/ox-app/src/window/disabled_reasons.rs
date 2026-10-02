// SPDX-License-Identifier: AGPL-3.0-only
//! Why a window command is disabled, for the tooltip and accessible
//! description of its menu item (CMD-031), as Windows 11 and Dolphin
//! explain a greyed-out command.
//!
//! Each reason follows the rule that disables the command: the file
//! commands' [`CommandFacts::refusal`](super::file_ops::CommandFacts),
//! the archive and folder-size commands' own checks, and a fixed reason
//! for the commands that need one item or a folder.

use crate::locations::Page;

use super::file_ops::FileCommand;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Why `action` is disabled now; `None` when nothing says.
    pub(super) fn disabled_reason(&self, action: WindowAction) -> Option<&'static str> {
        let file_command = FileCommand::ALL
            .into_iter()
            .find(|command| command.actions().contains(&action));
        if let Some(command) = file_command {
            return self.command_facts().refusal(command);
        }
        if let Some(reason) = self.archive_refusal(action).or_else(|| self.size_refusal(action)) {
            return Some(reason);
        }
        let on_page = self.current_uri().as_deref().and_then(Page::from_uri).is_some();
        match action {
            WindowAction::PinFolder => Some("Only a folder can be pinned to Quick access."),
            WindowAction::CacheFolder => Some("Only a folder on a drive or share can be cached for search."),
            WindowAction::Properties | WindowAction::PreviousVersions if on_page => {
                Some("Pages such as This PC have no properties.")
            }
            WindowAction::Open
            | WindowAction::PinSelected
            | WindowAction::Properties
            | WindowAction::PreviousVersions => Some("Select exactly one item."),
            WindowAction::OpenWith | WindowAction::OpenInTerminal | WindowAction::OpenInEditor => {
                Some("Select one item at a time.")
            }
            WindowAction::OpenFileLocation => Some("Only a search result has a file location to open."),
            _ => None,
        }
    }
}
