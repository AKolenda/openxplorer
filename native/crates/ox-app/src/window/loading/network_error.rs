// SPDX-License-Identifier: AGPL-3.0-only
//! A share that cannot be reached, gone to from a folder: as Windows 11
//! does (checked on a Windows 11 VM with `\\192.168.1.250\Projects`), the
//! tab stays on the folder it showed and "Network Error" says
//! "`OpenXplorer` cannot access \\…", what to check, and, under "See details" ("Hide
//! details" while open), Windows' error code and words. Windows offers
//! Diagnose, its network troubleshooter; here Diagnose opens the desktop's
//! network settings when an app for them is installed, and is left out,
//! with the sentence that names it, when none is. A tab with nowhere to
//! stay (a new tab or window, Back or Forward, a reload) keeps the
//! unavailable page.

use gtk::glib;
use gtk::prelude::*;
use ox_core::entry::EntryError;
use ox_core::i18n::{format_message, gettext};
use ox_core::system_error::SystemError;

use super::LoadMode;
use crate::dialog::Dialog;
use crate::window::session::TabId;
use crate::window::{BrowserWindow, ButtonStyle};

/// The desktop's network settings, in the order they are looked for:
/// GNOME's Network panel, KDE's Connections and `NetworkManager`'s editor.
const NETWORK_SETTINGS: [&str; 3] = [
    "gnome-network-panel.desktop",
    "kcm_networkmanagement.desktop",
    "nm-connection-editor.desktop",
];

/// The installed app that shows the desktop's network settings, if any.
fn network_settings() -> Option<gtk::gio::AppInfo> {
    NETWORK_SETTINGS
        .into_iter()
        .find_map(crate::integration::installed_application)
}

/// The error Windows reports for the share's failure `error`, when it is
/// one of the network's: a share or server that cannot be reached, or one
/// that went away. `None` for anything else, which keeps its page.
fn network_failure(error: &EntryError) -> Option<SystemError> {
    let (EntryError::Failed(said) | EntryError::NotMounted(said) | EntryError::NotFound(said)) = error else {
        return None;
    };
    let sentence = said
        .rsplit_once(": ")
        .map_or(said.as_str(), |(_, sentence)| sentence);
    let failure = SystemError::from_sentence(sentence)?.on_network();
    let network = [
        ox_core::system_error::BAD_NET_NAME,
        ox_core::system_error::BAD_NETPATH,
        ox_core::system_error::NETNAME_DELETED,
        ox_core::system_error::UNEXPECTED_NETWORK_ERROR,
        ox_core::system_error::DEVICE_NOT_CONNECTED,
    ];
    network.contains(&failure).then_some(failure)
}

/// "Error code: 0x80070043", then the error's words, as Windows' "See
/// details" shows them.
fn details_text(failure: SystemError) -> String {
    format_message(
        "Error code: {code}\n{message}",
        &[
            ("code", &format!("0x{:08X}", failure.hresult())),
            ("message", &failure.message()),
        ],
    )
}

impl BrowserWindow {
    /// Takes tab `id`, whose share at `uri` could not be mounted with
    /// `error`, back to the folder it came from and says "Network Error",
    /// as Windows does; false, changing nothing, when the error is not the
    /// network's or the tab has no folder to stay in.
    pub(super) fn leave_unreachable_share(
        &self,
        id: TabId,
        uri: &str,
        mode: LoadMode,
        error: &EntryError,
    ) -> bool {
        let Some(failure) = network_failure(error) else {
            return false;
        };
        if !ox_core::location::is_server_location(uri) || !self.may_step_back(id, uri, mode) {
            return false;
        }
        if !self.step_back(id) {
            return false;
        }
        glib::g_debug!(ox_core::LOG_DOMAIN, "{uri} cannot be reached: {error}");
        let location = ox_core::search::display_path(uri);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.say_network_error(&location, failure).await }
        ));
        true
    }

    /// Windows' "Network Error" about `location`, with `failure` under
    /// "See details".
    async fn say_network_error(&self, location: &str, failure: SystemError) {
        let settings = network_settings();
        let heading = format_message("OpenXplorer cannot access {location}", &[("location", location)]);
        let advice = if settings.is_some() {
            gettext(
                "Check the spelling of the name. Otherwise, there might be a problem with your network. To try to identify and resolve network problems, click Diagnose.",
            )
        } else {
            gettext("Check the spelling of the name. Otherwise, there might be a problem with your network.")
        };
        let dialog = Dialog::with_caption(self, &gettext("Network Error"), &heading, &advice);
        let details = gtk::Label::new(Some(&details_text(failure)));
        details.set_xalign(0.0);
        details.set_wrap(true);
        details.set_selectable(true);
        let expander = gtk::Expander::new(Some(&gettext("See details")));
        expander.set_child(Some(&details));
        expander.connect_expanded_notify(|expander| {
            let label = if expander.is_expanded() {
                gettext("Hide details")
            } else {
                gettext("See details")
            };
            expander.set_label(Some(&label));
        });
        dialog.add_field(&expander);
        let diagnose = settings
            .as_ref()
            .map(|_| dialog.add_button(&gettext("Diagnose"), ButtonStyle::Accent));
        let cancel_style = if diagnose.is_some() {
            ButtonStyle::Bordered
        } else {
            ButtonStyle::Accent
        };
        dialog.add_button(&gettext("Cancel"), cancel_style);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        if let (Some(settings), true) = (settings, answer.is_some() && answer == diagnose) {
            let context = WidgetExt::display(self).app_launch_context();
            if let Err(error) = settings.launch(&[], Some(&context)) {
                self.show_message(&ox_core::system_error::message_of(&error));
            }
        }
    }
}
