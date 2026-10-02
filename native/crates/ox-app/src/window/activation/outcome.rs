// SPDX-License-Identifier: AGPL-3.0-only
//! Where an activation started and what becomes of its result: the result
//! belongs to the tab that asked, a folder opens there even when another
//! tab is in front by then, and a failure is reported with an offer to
//! find an application in Software (OPEN-001, OPEN-004, OPEN-010).

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;

use super::Resolved;
use crate::dialog::Dialog;
use crate::window::session::TabId;
use crate::window::software_search::{self, FIND_IN_SOFTWARE};
use crate::window::{BrowserWindow, ButtonStyle};

/// The title of the dialog that says why an item did not open.
const OPEN_FAILED: &str = "Could not open the item";

/// Where an activation started: its tab, and how often that tab had moved
/// to another location by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActivationOrigin {
    tab: TabId,
    moves: u64,
}

impl BrowserWindow {
    /// Marks the active tab as opening an item; `None` while it already
    /// is, or before the window has a tab.
    pub(super) fn begin_item_activation(&self) -> Option<ActivationOrigin> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.active_mut()?;
        if tab.is_activating {
            return None;
        }
        tab.is_activating = true;
        Some(ActivationOrigin {
            tab: tab.id,
            moves: tab.history.moves(),
        })
    }

    /// Ends the activation `origin` started; true when its tab is still
    /// open and still at the location it was activated in.
    pub(super) fn end_item_activation(&self, origin: ActivationOrigin) -> bool {
        let mut session = self.imp().session.borrow_mut();
        let Some(tab) = session.tab_mut(origin.tab) else {
            return false;
        };
        tab.is_activating = false;
        tab.history.moves() == origin.moves
    }

    /// Shows the result of an activation in the tab `origin` names.
    pub(super) fn show_item_activation(
        &self,
        origin: ActivationOrigin,
        entry: &Entry,
        outcome: Result<Resolved, String>,
    ) {
        let is_active = self.imp().session.borrow().is_active(origin.tab);
        match outcome {
            Ok(Resolved::Folder(uri)) if is_active => self.navigate_or_report(&uri),
            Ok(Resolved::Folder(uri)) => self.navigate_background_tab(origin.tab, &uri),
            Ok(Resolved::Archive(archive)) if is_active => self.open_archive_or_file(&archive),
            Ok(Resolved::Archive(_) | Resolved::Opened) => {}
            Err(reason) if is_active => self.report_open_failure(&reason, entry),
            Err(reason) => self.show_message(&format!("Could not open {}: {reason}", entry.name)),
        }
    }

    /// Moves the background tab `id` to the folder `uri`; it is listed
    /// when it is next shown.
    fn navigate_background_tab(&self, id: TabId, uri: &str) {
        let Ok(uri) = self.resolve_address(uri) else {
            return;
        };
        let stale = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else {
                return;
            };
            tab.history.push(&uri);
            tab.forget_location_state();
            tab.mark_stale()
        };
        stale.remove_all();
        self.render_tabs();
    }

    /// Says why `entry` could not be opened in a dialog, as
    /// `showMessage('Could not open the item', …)` does. When no
    /// application opens its type, the dialog offers to find one in
    /// Software (OPEN-010).
    pub(in crate::window) fn report_open_failure(&self, reason: &str, entry: &Entry) {
        let reason = reason.to_owned();
        let unhandled = software_search::unhandled_type(&reason, entry.content_type.as_deref())
            .filter(|_| software_search::is_available());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let dialog = Dialog::new(&window, OPEN_FAILED, &reason);
                let find = unhandled
                    .as_ref()
                    .map(|_| dialog.add_button(FIND_IN_SOFTWARE, ButtonStyle::Bordered));
                dialog.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent);
                dialog.open();
                let answer = dialog.next_response().await;
                dialog.finish();
                let (Some(content_type), true) = (unhandled, answer.is_some() && answer == find) else {
                    return;
                };
                if let Err(error) = software_search::search_software(&content_type).await {
                    window.show_message(&error.to_string());
                }
            }
        ));
    }
}
