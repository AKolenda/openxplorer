// SPDX-License-Identifier: AGPL-3.0-only
//! Super+E opens `OpenXplorer` on KDE Plasma, as Win+E opens File Explorer
//! (INT-033): the line under the Default apps switch and the changes the
//! switch asks for, run off the main thread. KDE Plasma 6 follows a changed
//! launch shortcut only after the user logs out and back in, so the line
//! and the messages say so.

use ox_core::integration::{LaunchShortcut, LaunchShortcutStatus, RestoredShortcut};

use super::changes::IntegrationError;

/// The line under the switch where it can be used: KDE Plasma follows the
/// change only from the next login.
const AFTER_LOGIN: &str =
    crate::i18n::message_id("Log out and back in after changing this for Super+E to follow.");
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

    /// The line under the switch: that the change needs a new login, or
    /// why the switch cannot be used here.
    pub(crate) fn text(&self) -> String {
        match &self.0 {
            LaunchShortcutStatus::Unsupported => {
                ox_core::i18n::gettext("Available on KDE Plasma with the installed package.")
            }
            LaunchShortcutStatus::Ours | LaunchShortcutStatus::Other(_) | LaunchShortcutStatus::Free => {
                ox_core::i18n::gettext_static(AFTER_LOGIN).to_owned()
            }
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
        Ok(ox_core::i18n::gettext(
            "Super+E will open OpenXplorer once you log out and back in.",
        ))
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
            RestoredShortcut::GivenBack => ox_core::i18n::gettext(
                "Super+E will open the app it opened before once you log out and back in.",
            ),
            RestoredShortcut::Freed => {
                ox_core::i18n::gettext("Super+E will stop opening OpenXplorer once you log out and back in.")
            }
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
    fn the_line_says_to_log_in_again_or_why_super_e_cannot_change() {
        let after_login = "Log out and back in after changing this for Super+E to follow.";
        let other = ShortcutStatus(LaunchShortcutStatus::Other("Dolphin".to_owned()));
        assert_eq!(other.text(), after_login);
        assert!(other.is_available() && !other.is_enabled());
        let ours = ShortcutStatus(LaunchShortcutStatus::Ours);
        assert_eq!(ours.text(), after_login);
        assert!(ours.is_enabled());
        let elsewhere = ShortcutStatus::default();
        assert!(!elsewhere.is_available());
        assert_eq!(
            elsewhere.text(),
            "Available on KDE Plasma with the installed package."
        );
        let silent = ShortcutStatus(LaunchShortcutStatus::Unreachable("no answer".to_owned()));
        assert!(!silent.is_available());
        assert_eq!(silent.text(), "no answer");
    }
}
