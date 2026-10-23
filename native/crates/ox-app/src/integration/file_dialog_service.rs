// SPDX-License-Identifier: AGPL-3.0-only
//! Other applications' Open and Save dialogs: the portal backend the app
//! serves, and the opt-in that sends the dialogs to it.
//!
//! New in the native app (INT-032). The backend object is exported on the
//! application's own bus name during D-Bus registration
//! (`application/file_dialogs.rs`); it stays inert until the user enables
//! "Open and Save dialogs" here, which makes the desktop portal route
//! `FileChooser` calls to the packaged `<app id>.portal` backend. The bus
//! name's existing D-Bus service file starts the app on the first call.
//! Each call opens a window in picker mode
//! ([`crate::window::BrowserWindow::begin_picking`]); the checks on the
//! caller and its options are ox-core's
//! ([`ox_core::integration::FileChooserBus`]).

use ox_core::integration::{
    DisabledFileDialogs, FileDialogRegistration, PortalRestart, GTK_PORTAL_VARIABLE, KDE_PORTAL_VARIABLE,
};

use super::changes::IntegrationError;
use super::DesktopIntegration;

/// Where Open and Save dialogs go, for the Settings page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileDialogsStatus {
    /// The opt-in can work here (not inside Flatpak).
    pub(crate) is_available: bool,
    /// The user's portal configuration prefers the app.
    pub(crate) is_enabled: bool,
    /// The backend the portal configuration names now, if any.
    pub(crate) backend: Option<String>,
    /// Where GTK 3 apps stand.
    pub(crate) gtk_apps: LoginApps,
    /// Where KDE's own apps stand.
    pub(crate) kde_apps: LoginApps,
}

/// Whether apps that need a login file to ask the portal (GTK 3 apps, and
/// KDE's own on KDE) send their dialogs to the app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LoginApps {
    /// Nothing to do: KDE's own apps off KDE.
    #[default]
    NotNeeded,
    /// The app's login file is not there.
    NotCovered,
    /// A file of the user's sits where the login file goes and is left
    /// alone.
    LeftAlone,
    /// The login file is in place; it applies at the next login.
    NextLogin,
    /// The login file is in place and this session started with it.
    Following,
}

impl FileDialogsStatus {
    /// Whether Enable has something to do: dialogs do not use the app
    /// yet, or (enabled with an earlier version) GTK or KDE apps are not
    /// covered yet.
    pub(crate) fn can_enable(&self) -> bool {
        !self.is_enabled || self.gtk_apps == LoginApps::NotCovered || self.kde_apps == LoginApps::NotCovered
    }

    /// The row's status line, translated sentence by sentence.
    pub(crate) fn text(&self) -> String {
        if !self.is_available {
            return ox_core::i18n::gettext_static(
                "Open and Save dialogs: available in the installed package, not the Flatpak.",
            )
            .to_owned();
        }
        if self.is_enabled {
            let mut sentences = vec![ox_core::i18n::gettext_static(
                "Open and Save dialogs: OpenXplorer. Apps that use the desktop portal show their file dialogs \
                 here after Apply now or the next login.",
            )];
            sentences.extend(match self.gtk_apps {
                LoginApps::Following => Some(ox_core::i18n::gettext_static("GTK 3 apps do too.")),
                LoginApps::NextLogin => Some(ox_core::i18n::gettext_static("GTK 3 apps follow after you log out and back in.")),
                LoginApps::LeftAlone => Some(ox_core::i18n::gettext_static(
                    "GTK 3 apps keep GTK's dialog: ~/.config/environment.d/60-openxplorer-file-dialogs.conf is \
                     not OpenXplorer's, so it was left alone.",
                )),
                LoginApps::NotCovered => Some(ox_core::i18n::gettext_static("Click Enable for GTK 3 apps too.")),
                LoginApps::NotNeeded => None,
            });
            sentences.extend(match self.kde_apps {
                LoginApps::Following => Some(ox_core::i18n::gettext_static("KDE apps do too.")),
                LoginApps::NextLogin => Some(ox_core::i18n::gettext_static("KDE apps follow after you log out and back in.")),
                LoginApps::LeftAlone => Some(ox_core::i18n::gettext_static(
                    "KDE apps keep KDE's dialog: ~/.config/plasma-workspace/env/openxplorer-file-dialogs.sh is \
                     not OpenXplorer's, so it was left alone.",
                )),
                LoginApps::NotNeeded | LoginApps::NotCovered => None,
            });
            return sentences.join(" ");
        }
        match self.backend.as_deref() {
            Some(backend) => ox_core::i18n::format_message(
                "Open and Save dialogs: the desktop's ({backend}).",
                &[("backend", backend)],
            ),
            None => ox_core::i18n::gettext_static("Open and Save dialogs: the desktop's.").to_owned(),
        }
    }
}

/// Whether this session started with `variable` set to 1, as a login
/// file sets it.
fn session_has(variable: &str) -> bool {
    std::env::var(variable).is_ok_and(|value| value == "1")
}

/// What is at a login file's path, for the status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginFileAt {
    /// The app's file.
    Ours,
    /// Nothing, or what cannot be read.
    Nothing,
    /// A file of the user's, or a symlink, left alone.
    Theirs,
}

impl LoginFileAt {
    fn of(covered: bool, someone_elses: bool) -> Self {
        if covered {
            Self::Ours
        } else if someone_elses {
            Self::Theirs
        } else {
            Self::Nothing
        }
    }
}

/// Where apps that need a login file stand: whether this session needs
/// it, what is at its path, and whether this session started with its
/// variable. A file of the user's there is left alone, but a session that
/// has the variable all the same follows.
fn login_apps(needed: bool, file: LoginFileAt, following: bool) -> LoginApps {
    match (needed, file, following) {
        (false, _, _) => LoginApps::NotNeeded,
        (true, LoginFileAt::Theirs, false) => LoginApps::LeftAlone,
        (true, LoginFileAt::Nothing, _) => LoginApps::NotCovered,
        (true, LoginFileAt::Ours, false) => LoginApps::NextLogin,
        (true, LoginFileAt::Ours | LoginFileAt::Theirs, true) => LoginApps::Following,
    }
}

/// Where GTK 3 apps stand for `registration`, in this session.
fn gtk_apps(registration: &FileDialogRegistration) -> LoginApps {
    login_apps(
        registration.reads_gtk_env_file(),
        LoginFileAt::of(
            registration.covers_gtk_apps(),
            registration.gtk_env_is_someone_elses(),
        ),
        session_has(GTK_PORTAL_VARIABLE),
    )
}

/// Where KDE's own apps stand for `registration`, in this session.
fn kde_apps(registration: &FileDialogRegistration) -> LoginApps {
    login_apps(
        registration.is_kde_session(),
        LoginFileAt::of(
            registration.covers_kde_apps(),
            registration.kde_script_is_someone_elses(),
        ),
        session_has(KDE_PORTAL_VARIABLE),
    )
}

/// Which apps follow an Enable only after a new login, for its toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AfterLogin {
    /// None: every login file this session needs is in place and was
    /// read at its start, or none could be written.
    Nothing,
    Gtk,
    Kde,
    GtkAndKde,
}

impl AfterLogin {
    /// The apps whose login file is in place for `registration` but not
    /// yet read by this session.
    pub(crate) fn of(registration: &FileDialogRegistration) -> Self {
        let gtk = gtk_apps(registration) == LoginApps::NextLogin;
        let kde = kde_apps(registration) == LoginApps::NextLogin;
        match (gtk, kde) {
            (false, false) => Self::Nothing,
            (true, false) => Self::Gtk,
            (false, true) => Self::Kde,
            (true, true) => Self::GtkAndKde,
        }
    }
}

impl DesktopIntegration {
    /// Sends Open and Save dialogs to the app from the portal's next start.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::FileDialogs`] when the configuration cannot be
    /// written, or inside Flatpak.
    pub(crate) async fn enable_file_dialogs(&self) -> Result<String, IntegrationError> {
        let after_login = self
            .file_dialogs()
            .run_in_background(|registration| registration.enable().map(|()| AfterLogin::of(registration)))
            .await?;
        self.notify_changed();
        Ok(ox_core::i18n::gettext_static(match after_login {
            AfterLogin::Nothing => crate::i18n::message_id("Open and Save dialogs will use OpenXplorer. Click Apply now, or log out and back in."),
            AfterLogin::Gtk => {
                crate::i18n::message_id("Open and Save dialogs will use OpenXplorer. Click Apply now, or log out and back in; GTK 3 apps \
                 follow after you log out and back in.")
            }
            AfterLogin::Kde => {
                crate::i18n::message_id("Open and Save dialogs will use OpenXplorer. Click Apply now, or log out and back in; KDE apps \
                 follow after you log out and back in.")
            }
            AfterLogin::GtkAndKde => {
                crate::i18n::message_id("Open and Save dialogs will use OpenXplorer. Click Apply now, or log out and back in; GTK 3 and \
                 KDE apps follow after you log out and back in.")
            }
        })
        .to_owned())
    }

    /// Gives Open and Save dialogs back to the desktop.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::FileDialogs`] when the configuration cannot be
    /// restored.
    pub(crate) async fn disable_file_dialogs(&self) -> Result<String, IntegrationError> {
        let outcome = self
            .file_dialogs()
            .run_in_background(FileDialogRegistration::disable)
            .await?;
        self.notify_changed();
        Ok(ox_core::i18n::gettext_static(match outcome {
            DisabledFileDialogs::Restored => crate::i18n::message_id(
                "Open and Save dialogs are back to the desktop's. Click Apply now, or log out and back in.",
            ),
            DisabledFileDialogs::LineRemoved => crate::i18n::message_id(
                "Removed OpenXplorer's line from your portal settings and kept your other edits. Click \
                 Apply now, or log out and back in.",
            ),
            DisabledFileDialogs::NotEnabled => {
                crate::i18n::message_id("Open and Save dialogs were not using OpenXplorer.")
            }
        })
        .to_owned())
    }

    /// Restarts the desktop portal so the choice applies now.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::FileDialogs`] when the portal could not be
    /// restarted.
    pub(crate) async fn apply_file_dialogs(&self) -> Result<String, IntegrationError> {
        let restart = self
            .file_dialogs()
            .run_in_background(FileDialogRegistration::restart_portal)
            .await?;
        self.notify_changed();
        Ok(ox_core::i18n::gettext_static(match restart {
            PortalRestart::Restarted => crate::i18n::message_id(
                "The desktop portal restarted. Open and Save dialogs follow the new choice.",
            ),
            PortalRestart::NotRunning => crate::i18n::message_id(
                "The desktop portal is not running as a service that can be restarted. The choice applies \
                 the next time you log in.",
            ),
        })
        .to_owned())
    }

    /// Where Open and Save dialogs go now, read off the main thread.
    pub(super) async fn file_dialogs_status(&self) -> FileDialogsStatus {
        self.file_dialogs()
            .run_in_background(|registration| FileDialogsStatus {
                is_available: registration.is_available(),
                is_enabled: registration.is_enabled(),
                gtk_apps: gtk_apps(registration),
                kde_apps: kde_apps(registration),
                backend: registration.current_backend(),
            })
            .await
    }

    fn file_dialogs(&self) -> &FileDialogRegistration {
        &self.services().file_dialogs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(kde_apps: LoginApps) -> FileDialogsStatus {
        FileDialogsStatus {
            is_available: true,
            is_enabled: true,
            backend: Some("io.winspace.Development".to_owned()),
            gtk_apps: LoginApps::Following,
            kde_apps,
        }
    }

    fn enabled_for_gtk(gtk_apps: LoginApps) -> FileDialogsStatus {
        FileDialogsStatus {
            gtk_apps,
            ..enabled(LoginApps::NotNeeded)
        }
    }

    /// On KDE the status says when KDE's own apps follow, and Enable stays
    /// available to add their login script for those who enabled the
    /// dialogs before.
    ///
    /// parity: INT-032
    #[test]
    fn the_status_says_when_kde_apps_follow() {
        assert!(enabled(LoginApps::NextLogin)
            .text()
            .ends_with("KDE apps follow after you log out and back in."));
        assert!(enabled(LoginApps::Following).text().ends_with("KDE apps do too."));
        assert!(!enabled(LoginApps::NotNeeded).text().contains("KDE"));
        assert!(enabled(LoginApps::NotCovered).can_enable());
        assert!(!enabled(LoginApps::NextLogin).can_enable());
        assert!(!enabled(LoginApps::NotNeeded).can_enable());
        assert!(enabled(LoginApps::LeftAlone)
            .text()
            .ends_with("so it was left alone."));
        assert!(!enabled(LoginApps::LeftAlone).can_enable());
        let off = FileDialogsStatus::default();
        assert!(off.can_enable());
    }

    /// A session that needs no login file says nothing about it; a file
    /// of the user's in its place reads as left alone unless the session
    /// has the variable all the same.
    ///
    /// parity: INT-032
    #[test]
    fn login_apps_follow_the_session_before_the_file() {
        use LoginFileAt::{Nothing, Ours, Theirs};
        assert_eq!(login_apps(false, Ours, true), LoginApps::NotNeeded);
        assert_eq!(login_apps(true, Theirs, false), LoginApps::LeftAlone);
        assert_eq!(login_apps(true, Theirs, true), LoginApps::Following);
        assert_eq!(login_apps(true, Nothing, true), LoginApps::NotCovered);
        assert_eq!(login_apps(true, Ours, false), LoginApps::NextLogin);
        assert_eq!(login_apps(true, Ours, true), LoginApps::Following);
    }

    /// The status says when GTK 3 apps follow, and Enable stays available
    /// to add their environment file for those who enabled the dialogs
    /// with an earlier version.
    ///
    /// parity: INT-032
    #[test]
    fn the_status_says_when_gtk_apps_follow() {
        assert!(enabled_for_gtk(LoginApps::NextLogin)
            .text()
            .ends_with("GTK 3 apps follow after you log out and back in."));
        assert!(enabled_for_gtk(LoginApps::Following)
            .text()
            .ends_with("GTK 3 apps do too."));
        let not_covered = enabled_for_gtk(LoginApps::NotCovered);
        assert!(not_covered.text().ends_with("Click Enable for GTK 3 apps too."));
        assert!(not_covered.can_enable());
        let left_alone = enabled_for_gtk(LoginApps::LeftAlone);
        assert!(left_alone.text().ends_with("so it was left alone."));
        assert!(!left_alone.can_enable());
        assert!(!enabled_for_gtk(LoginApps::NextLogin).can_enable());
        assert!(!enabled_for_gtk(LoginApps::Following).can_enable());
    }
}
