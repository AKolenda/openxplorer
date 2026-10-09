// SPDX-License-Identifier: AGPL-3.0-only
//! Super+E opens `OpenXplorer` on KDE Plasma, as Win+E opens File Explorer
//! (INT-033): the status line and the changes the Default apps page asks
//! for, run off the main thread.

use ox_core::integration::{LaunchShortcut, LaunchShortcutStatus, RestoredShortcut};

use super::changes::IntegrationError;
use super::shortcut_backend::ShortcutBackend;
use super::DesktopIntegration;

/// What Super+E does now, as the Default apps page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShortcutStatus(pub(crate) LaunchShortcutStatus);

impl Default for ShortcutStatus {
    fn default() -> Self {
        Self(LaunchShortcutStatus::Unsupported)
    }
}

impl ShortcutStatus {
    /// True when the shortcut can be changed here: on KDE Plasma, with the
    /// installed package, and its service answers.
    pub(crate) fn is_available(&self) -> bool {
        matches!(
            self.0,
            LaunchShortcutStatus::Ours | LaunchShortcutStatus::Other(_) | LaunchShortcutStatus::Free
        )
    }

    /// True when Super+E opens `OpenXplorer`.
    pub(crate) fn is_enabled(&self) -> bool {
        self.0 == LaunchShortcutStatus::Ours
    }

    /// The row's status line.
    pub(crate) fn text(&self) -> String {
        match &self.0 {
            LaunchShortcutStatus::Unsupported => {
                ox_core::i18n::gettext("Available on KDE Plasma with the installed package.")
            }
            LaunchShortcutStatus::Ours => ox_core::i18n::gettext("Super+E opens OpenXplorer."),
            LaunchShortcutStatus::Other(name) => {
                ox_core::i18n::format_message("Super+E opens {name}.", &[("name", name)])
            }
            LaunchShortcutStatus::Free => ox_core::i18n::gettext("Super+E is not used."),
            LaunchShortcutStatus::Unreachable(reason) => reason.clone(),
        }
    }
}

impl DesktopIntegration {
    /// Makes Super+E open `OpenXplorer`.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::LaunchShortcut`] when KDE's shortcut service
    /// cannot be reached or kept Super+E, and off KDE Plasma.
    pub(crate) async fn enable_launch_shortcut(&self) -> Result<String, IntegrationError> {
        self.launch_shortcut()
            .run_in_background(LaunchShortcut::enable)
            .await?;
        self.notify_changed();
        Ok(ox_core::i18n::gettext("Super+E now opens OpenXplorer."))
    }

    /// Gives Super+E back to the app it opened before.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::LaunchShortcut`] when KDE's shortcut service
    /// cannot be reached, and off KDE Plasma.
    pub(crate) async fn restore_launch_shortcut(&self) -> Result<String, IntegrationError> {
        let restored = self
            .launch_shortcut()
            .run_in_background(LaunchShortcut::restore)
            .await?;
        self.notify_changed();
        Ok(match restored {
            RestoredShortcut::GivenBack => ox_core::i18n::gettext("Super+E opens the app it opened before."),
            RestoredShortcut::Freed => ox_core::i18n::gettext("Super+E no longer opens OpenXplorer."),
            RestoredShortcut::NotOurs => ox_core::i18n::gettext("Super+E was not opening OpenXplorer."),
        })
    }

    /// What Super+E does now, read off the main thread.
    pub(super) async fn launch_shortcut_status(&self) -> ShortcutStatus {
        ShortcutStatus(
            self.launch_shortcut()
                .run_in_background(LaunchShortcut::status)
                .await,
        )
    }

    fn launch_shortcut(&self) -> &LaunchShortcut<ShortcutBackend> {
        &self.services().launch_shortcut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: INT-033
    #[test]
    fn the_status_says_what_super_e_opens() {
        let other = ShortcutStatus(LaunchShortcutStatus::Other("Dolphin".to_owned()));
        assert_eq!(other.text(), "Super+E opens Dolphin.");
        assert!(other.is_available() && !other.is_enabled());
        let ours = ShortcutStatus(LaunchShortcutStatus::Ours);
        assert!(ours.is_enabled());
        let elsewhere = ShortcutStatus::default();
        assert!(!elsewhere.is_available());
        assert_eq!(
            elsewhere.text(),
            "Available on KDE Plasma with the installed package."
        );
        let silent = ShortcutStatus(LaunchShortcutStatus::Unreachable("no answer".to_owned()));
        assert!(!silent.is_available());
    }
}
