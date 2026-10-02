// SPDX-License-Identifier: AGPL-3.0-only
//! Sign out of server: disconnect a server's mounts and forget its
//! credentials.
//!
//! Ports `signOut` in `v2.0.0:desktop/ui/app.js` and the window's part of
//! `sign_out` in `v2.0.0:desktop/winspace.py` around ox-core's
//! [`begin_sign_out`] and [`finish_sign_out`]:
//!
//! 1. The dialog asks what to forget ([`sign_out_dialog`]).
//! 2. ox-core checks the preconditions and marks the server, so no window
//!    lists or mounts it meanwhile (NET-023).
//! 3. Every window stops listing and watching the server, and the server
//!    leaves the session's Network list.
//! 4. ox-core disconnects the server's mounts and deletes its credentials
//!    (NET-021).
//! 5. The tabs on the server are listed again when next shown, the window
//!    shows Network, and a message says what was forgotten (NET-020).
//!
//! The search cache pauses the server's indexing when the sign-out starts,
//! clears its cached file names at the end when the user asked to, and
//! resumes after the next successful mount of the server (NET-022,
//! [`AppContext::announce_server_signed_out`]).
//!
//! [`AppContext::announce_server_signed_out`]: crate::app_context::AppContext::announce_server_signed_out

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::network::{
    begin_sign_out, finish_sign_out, ForgetScope, NetworkError, ServerKey, SignOutReport, SignOutRequest,
    WriteActivity,
};

use crate::dialogs::{sign_out_dialog, SignOutChoice};
use crate::locations::Page;

use super::BrowserWindow;

/// The heading of the message when signing out failed.
const SIGN_OUT_FAILED: &str = "Sign-out did not fully finish";

/// The lower-case host of an SMB location, as Sign out compares servers
/// (`new URL(uri).hostname`); `None` for any other location.
fn smb_host_of(uri: &str) -> Option<String> {
    ServerKey::for_location(uri).map(|server| server.host().to_owned())
}

/// What the window says once the server is signed out.
fn signed_out_message(report: &SignOutReport) -> &'static str {
    match report.forget {
        ForgetScope::AllScopes => "Signed out. Matching saved credentials were cleared or none were present.",
        ForgetScope::SessionOnly => "Disconnected. Saved credentials are retained.",
    }
}

impl BrowserWindow {
    /// Sign out of server…: asks what to forget, then signs out of the
    /// server of `uri`.
    pub(super) fn sign_out_of_server(&self, uri: &str) {
        if self.write_activity() == WriteActivity::Writing {
            self.show_message(&ox_core::i18n::gettext(
                "Finish the current file operation before signing out.",
            ));
            return;
        }
        let Some(host) = smb_host_of(uri) else {
            self.show_message(&NetworkError::NotAnSmbLocation.to_string());
            return;
        };
        let uri = uri.to_owned();
        let dialog = sign_out_dialog(
            self,
            &host,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |dialog, choice| {
                    dialog.finish();
                    window.sign_out(&uri, choice);
                }
            ),
        );
        dialog.open();
    }

    /// Signs out of the server of `uri` as `choice` asks.
    fn sign_out(&self, uri: &str, choice: SignOutChoice) {
        let prompts = self.network().prompts().clone();
        let signing_out = self.context().network().sign_out_registry();
        let mounts = self.volume_monitor().mounts();
        let request_uri = uri.to_owned();
        let writes = self.writes_in_any_window();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let request = SignOutRequest {
                uri: &request_uri,
                forget: choice.forget,
                writes,
            };
            let signed_out = match begin_sign_out(&prompts, &signing_out, request) {
                Ok(started) => {
                    if let Some(window) = window.upgrade() {
                        window.leave_server(started.host());
                    }
                    finish_sign_out(&started, &mounts).await
                }
                Err(refusal) => Err(refusal),
            };
            if let Some(window) = window.upgrade() {
                window.finish_sign_out(signed_out, choice);
            }
        });
    }

    /// Whether any window of the app writes files, which Sign out waits
    /// for.
    fn writes_in_any_window(&self) -> WriteActivity {
        let windows = self.browser_windows();
        let writing = windows
            .iter()
            .any(|window| window.write_activity() == WriteActivity::Writing);
        if writing {
            WriteActivity::Writing
        } else {
            WriteActivity::Idle
        }
    }

    /// Every browser window of the app, this one included.
    fn browser_windows(&self) -> Vec<BrowserWindow> {
        let Some(app) = self.application() else {
            return vec![self.clone()];
        };
        let windows = app.windows().into_iter();
        windows
            .filter_map(|window| window.downcast::<BrowserWindow>().ok())
            .collect()
    }

    /// The server `host` is being signed out: every window stops listing
    /// and watching it, it leaves the session's Network list and its
    /// indexing pauses (`serverSigningOut` in winspace.py).
    fn leave_server(&self, host: &str) {
        for window in self.browser_windows() {
            window.stop_reading_host(host);
        }
        self.context().forget_network_host(host);
    }

    /// Stops the listings and folder watches of the tabs on `host`.
    fn stop_reading_host(&self, host: &str) {
        self.imp().session.borrow_mut().change_panes(|tab| {
            if smb_host_of(tab.uri()).as_deref() == Some(host) {
                tab.stop_reading();
            }
        });
    }

    /// Shows what Sign out did: on success the tabs on the server are
    /// listed again when next shown, the window goes to Network and the
    /// search cache hears whether to clear the server's names.
    fn finish_sign_out(&self, signed_out: Result<SignOutReport, NetworkError>, choice: SignOutChoice) {
        self.context().reload_settings();
        let report = match signed_out {
            Ok(report) => report,
            Err(error) => {
                self.render_places();
                self.show_failure(SIGN_OUT_FAILED, &error.to_string());
                return;
            }
        };
        self.mark_host_stale(&report.host);
        self.navigate_or_report(Page::Network.uri());
        self.render_places();
        self.context()
            .announce_server_signed_out(&report.host, choice.search_cache);
        self.show_message(signed_out_message(&report));
    }

    /// Makes every tab on `host` list again when it is next shown.
    fn mark_host_stale(&self, host: &str) {
        self.mark_tabs_stale(|uri| smb_host_of(uri).as_deref() == Some(host));
    }

    /// Shows a message box headed `title` saying `detail`, with OK
    /// (`showMessage` in app.js).
    pub(super) fn show_failure(&self, title: &str, detail: &str) {
        let dialog = gtk::AlertDialog::builder()
            .message(title)
            .detail(detail)
            .buttons(["OK"])
            .default_button(0)
            .cancel_button(0)
            .modal(true)
            .build();
        dialog.show(Some(self));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NET-020
    #[test]
    fn the_message_says_whether_saved_credentials_were_kept() {
        let report = |forget| SignOutReport {
            host: "nas".into(),
            disconnected: 1,
            credentials_removed: true,
            forget,
        };
        assert_eq!(
            signed_out_message(&report(ForgetScope::AllScopes)),
            "Signed out. Matching saved credentials were cleared or none were present."
        );
        assert_eq!(
            signed_out_message(&report(ForgetScope::SessionOnly)),
            "Disconnected. Saved credentials are retained."
        );
    }

    #[test]
    fn only_smb_locations_have_a_server_to_sign_out_of() {
        assert_eq!(smb_host_of("smb://NAS/share/folder").as_deref(), Some("nas"));
        assert_eq!(smb_host_of("file:///home/demo"), None);
    }
}
