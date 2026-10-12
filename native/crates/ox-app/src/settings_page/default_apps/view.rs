// SPDX-License-Identifier: AGPL-3.0-only
//! Showing the Default apps status, and running the page's changes.
//!
//! Ports `updateDefaultStatus`, `renderDefaultStatus`, `changeDefault` and
//! `changeZipDefault` in `v2.0.0:desktop/ui/app.js`. "Make `OpenXplorer` default"
//! is enabled again after every status read, so it can re-apply after
//! another application took a route; Restore previous and Restore ZIP
//! handler are enabled only when a handler is recorded, and "Use
//! `OpenXplorer` for ZIPs" only while another app opens ZIP files (INT-012,
//! INT-030). While a change runs, Make default and Restore previous are
//! disabled; its outcome is toasted and the status read again.

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{MimeType, ZipAssociation};

use super::super::row::SettingRow;
use super::super::status_card::StatusCard;
use super::super::SettingsPage;
use super::Controls;
use crate::integration::{DesktopIntegration, IntegrationError, IntegrationStatus, MakeDefaultChoice};

/// The status card's title once the defaults are read.
fn status_title(is_default: bool) -> &'static str {
    if is_default {
        ox_core::i18n::gettext_static("OpenXplorer is your default file explorer")
    } else {
        ox_core::i18n::gettext_static("OpenXplorer isn't your default file explorer yet")
    }
}

/// The page's controls, held weakly: the page owns them.
#[derive(Debug, Clone)]
pub(super) struct DefaultAppsView {
    page: glib::WeakRef<SettingsPage>,
    card: glib::WeakRef<StatusCard>,
    make_default: glib::WeakRef<gtk::Button>,
    include_show_in_folder: glib::WeakRef<gtk::Switch>,
    include_zip: glib::WeakRef<gtk::Switch>,
    folders: glib::WeakRef<gtk::Label>,
    smb_links: glib::WeakRef<gtk::Label>,
    zip_files: glib::WeakRef<gtk::Label>,
    zip_row: glib::WeakRef<SettingRow>,
    use_for_zips: glib::WeakRef<gtk::Button>,
    show_in_folder_row: glib::WeakRef<SettingRow>,
    restore_previous: glib::WeakRef<gtk::Button>,
    restore_zip: glib::WeakRef<gtk::Button>,
    file_dialogs_row: glib::WeakRef<SettingRow>,
    enable_file_dialogs: glib::WeakRef<gtk::Button>,
    apply_file_dialogs: glib::WeakRef<gtk::Button>,
    restore_file_dialogs: glib::WeakRef<gtk::Button>,
    super_e_row: glib::WeakRef<SettingRow>,
    super_e: glib::WeakRef<gtk::Switch>,
    /// True while the page sets the switch to what KDE says, so that is
    /// not taken for the user turning it on or off.
    showing_super_e: Rc<Cell<bool>>,
    /// Counts status reads and changes: a read shows its status only when
    /// nothing was read or changed since it started, so a late read never
    /// turns a control back while a change runs or after a newer read.
    status_turn: Rc<Cell<u64>>,
    /// True while a change runs; reads that end meanwhile are not shown.
    is_changing: Rc<Cell<bool>>,
}

impl DefaultAppsView {
    /// The view of `controls` on `page`, with every button connected.
    pub(super) fn new(page: &SettingsPage, controls: &Controls) -> Self {
        let view = Self {
            page: page.downgrade(),
            card: controls.card.downgrade(),
            make_default: controls.make_default.downgrade(),
            include_show_in_folder: controls.include_show_in_folder.downgrade(),
            include_zip: controls.include_zip.downgrade(),
            folders: controls.folders.downgrade(),
            smb_links: controls.smb_links.downgrade(),
            zip_files: controls.zip_files.downgrade(),
            zip_row: controls.zip_row.downgrade(),
            use_for_zips: controls.use_for_zips.downgrade(),
            show_in_folder_row: controls.show_in_folder_row.downgrade(),
            restore_previous: controls.restore_previous.downgrade(),
            restore_zip: controls.restore_zip.downgrade(),
            file_dialogs_row: controls.file_dialogs_row.downgrade(),
            enable_file_dialogs: controls.enable_file_dialogs.downgrade(),
            apply_file_dialogs: controls.apply_file_dialogs.downgrade(),
            restore_file_dialogs: controls.restore_file_dialogs.downgrade(),
            super_e_row: controls.super_e_row.downgrade(),
            super_e: controls.super_e.downgrade(),
            showing_super_e: Rc::new(Cell::new(false)),
            status_turn: Rc::new(Cell::new(0)),
            is_changing: Rc::new(Cell::new(false)),
        };
        view.connect_changes(controls);
        view.connect_show_in_folder(controls);
        view.connect_file_dialogs(controls);
        view.connect_super_e(controls);
        view
    }

    /// Make default, Restore previous and the ZIP buttons.
    fn connect_changes(&self, controls: &Controls) {
        let view = self.clone();
        controls.make_default.connect_clicked(move |_| {
            let choice = view.make_default_choice();
            view.run_change(move |integration| async move {
                integration
                    .make_default(choice)
                    .await
                    .map(|outcome| Some(outcome.message()))
            });
        });
        let view = self.clone();
        controls.restore_previous.connect_clicked(move |_| {
            view.run_change(|integration| async move {
                integration
                    .restore_previous()
                    .await
                    .map(|outcome| Some(outcome.message()))
            });
        });
        let view = self.clone();
        controls.use_for_zips.connect_clicked(move |_| {
            view.run_change(|integration| async move {
                integration
                    .make_zip_default()
                    .await
                    .map(|outcome| Some(outcome.message()))
            });
        });
        let view = self.clone();
        controls.restore_zip.connect_clicked(move |_| {
            view.run_change(|integration| async move {
                integration
                    .restore_zip()
                    .await
                    .map(|outcome| Some(outcome.message()))
            });
        });
    }

    /// Test, Enable and Disable Show in folder. Enabling and disabling say
    /// nothing when they work, since the status line shows the result,
    /// except when the Flatpak answers only while it runs.
    fn connect_show_in_folder(&self, controls: &Controls) {
        let view = self.clone();
        controls.test_show_in_folder.connect_clicked(move |_| {
            view.run_change(|integration| async move {
                integration
                    .test_show_in_folder()
                    .await
                    .map(|outcome| Some(outcome.message()))
            });
        });
        let view = self.clone();
        controls.enable_show_in_folder.connect_clicked(move |_| {
            view.run_change(|integration| async move {
                integration
                    .enable_show_in_folder()
                    .await
                    .map(|reach| reach.message())
            });
        });
        let view = self.clone();
        controls.disable_show_in_folder.connect_clicked(move |_| {
            view.run_change(
                |integration| async move { integration.disable_show_in_folder().await.map(|_| None) },
            );
        });
    }

    /// Enable, Apply now and Restore for Open and Save dialogs (INT-032).
    fn connect_file_dialogs(&self, controls: &Controls) {
        let view = self.clone();
        controls.enable_file_dialogs.connect_clicked(move |_| {
            view.run_change(|integration| async move { integration.enable_file_dialogs().await.map(Some) });
        });
        let view = self.clone();
        controls.apply_file_dialogs.connect_clicked(move |_| {
            view.run_change(|integration| async move { integration.apply_file_dialogs().await.map(Some) });
        });
        let view = self.clone();
        controls.restore_file_dialogs.connect_clicked(move |_| {
            view.run_change(|integration| async move { integration.disable_file_dialogs().await.map(Some) });
        });
    }

    /// The Super+E switch (INT-033): on takes Super+E for `OpenXplorer`,
    /// off gives it back. The status read after the change sets the switch
    /// to what KDE then says, so a refused change turns it back.
    fn connect_super_e(&self, controls: &Controls) {
        let view = self.clone();
        controls.super_e.connect_active_notify(move |switch| {
            if view.showing_super_e.get() {
                return;
            }
            if switch.is_active() {
                view.run_change(
                    |integration| async move { integration.enable_launch_shortcut().await.map(Some) },
                );
            } else {
                view.run_change(|integration| async move {
                    integration.restore_launch_shortcut().await.map(Some)
                });
            }
        });
    }

    /// The options of Make `OpenXplorer` default, as the switches show them.
    fn make_default_choice(&self) -> MakeDefaultChoice {
        let includes_zip = self
            .include_zip
            .upgrade()
            .is_some_and(|switch| switch.is_active());
        MakeDefaultChoice {
            zip: if includes_zip {
                ZipAssociation::Included
            } else {
                ZipAssociation::Unchanged
            },
            show_in_folder: self
                .include_show_in_folder
                .upgrade()
                .is_some_and(|switch| switch.is_active()),
        }
    }

    /// The application's desktop integration, while the page exists.
    fn integration(&self) -> Option<DesktopIntegration> {
        let page = self.page.upgrade()?;
        Some(page.context().desktop_integration().clone())
    }

    /// Reads the status off the main thread and shows it.
    pub(super) fn read_status(&self) {
        let Some(integration) = self.integration() else {
            return;
        };
        let turn = self.next_status_turn();
        let view = self.clone();
        glib::spawn_future_local(async move {
            let status = integration.status().await;
            if view.status_turn.get() == turn && !view.is_changing.get() {
                view.show(&status);
            }
        });
    }

    /// Starts a new status turn, which outdates every read still running.
    fn next_status_turn(&self) -> u64 {
        let turn = self.status_turn.get().wrapping_add(1);
        self.status_turn.set(turn);
        turn
    }

    /// Runs `change` with Make default, Restore previous and the Open and
    /// Save dialogs' Enable, Apply now and Restore disabled, toasts its
    /// outcome and reads the status again (`changeDefault`), which enables
    /// what applies then.
    fn run_change<F, Change>(&self, change: F)
    where
        F: FnOnce(DesktopIntegration) -> Change + 'static,
        Change: Future<Output = Result<Option<String>, IntegrationError>> + 'static,
    {
        let Some(integration) = self.integration() else {
            return;
        };
        self.set_requests_enabled(false);
        self.next_status_turn();
        self.is_changing.set(true);
        let view = self.clone();
        glib::spawn_future_local(async move {
            let outcome = change(integration).await;
            let message = outcome.unwrap_or_else(|error| Some(error.to_string()));
            if let (Some(message), Some(page)) = (message, view.page.upgrade()) {
                page.report(&message);
            }
            view.is_changing.set(false);
            view.read_status();
        });
    }

    fn set_requests_enabled(&self, enabled: bool) {
        for button in [
            &self.make_default,
            &self.restore_previous,
            &self.enable_file_dialogs,
            &self.apply_file_dialogs,
            &self.restore_file_dialogs,
        ] {
            if let Some(button) = button.upgrade() {
                button.set_sensitive(enabled);
            }
        }
        if let Some(switch) = self.super_e.upgrade() {
            switch.set_sensitive(enabled);
        }
    }

    /// Shows `status` in the controls still on screen.
    fn show(&self, status: &IntegrationStatus) {
        if let Some(row) = self.show_in_folder_row.upgrade() {
            row.set_description(&status.show_in_folder.text());
        }
        let dialogs = &status.file_dialogs;
        if let Some(row) = self.file_dialogs_row.upgrade() {
            row.set_description(&dialogs.text());
        }
        set_sensitive(
            &self.enable_file_dialogs,
            dialogs.is_available && dialogs.can_enable(),
        );
        set_sensitive(&self.apply_file_dialogs, dialogs.is_available);
        set_sensitive(&self.restore_file_dialogs, dialogs.is_enabled);
        let shortcut = &status.launch_shortcut;
        if let Some(row) = self.super_e_row.upgrade() {
            row.set_description(&shortcut.text());
        }
        if let Some(switch) = self.super_e.upgrade() {
            self.showing_super_e.set(true);
            switch.set_active(shortcut.is_enabled());
            self.showing_super_e.set(false);
            switch.set_sensitive(shortcut.is_available());
        }
        if let Some(button) = self.make_default.upgrade() {
            button.set_sensitive(true);
        }
        match &status.defaults {
            Ok(report) => self.show_defaults(report),
            Err(message) => self.show_status_error(message),
        }
    }

    fn show_defaults(&self, report: &crate::integration::DefaultsReport) {
        let defaults = &report.status;
        set_text(&self.folders, &report.handler_label(MimeType::Directory));
        set_text(&self.smb_links, &report.handler_label(MimeType::SmbLink));
        set_text(&self.zip_files, &report.handler_label(MimeType::Zip));
        if let Some(card) = self.card.upgrade() {
            card.set_title(status_title(defaults.is_default_for_folder_types()));
        }
        if let Some(row) = self.zip_row.upgrade() {
            row.set_description(&report.zip_text());
        }
        set_sensitive(&self.use_for_zips, !defaults.is_zip_default());
        set_sensitive(&self.restore_previous, defaults.can_restore);
        set_sensitive(&self.restore_zip, defaults.can_restore_zip);
    }

    /// A status that could not be read replaces the routes with its
    /// message (INT-030).
    fn show_status_error(&self, message: &str) {
        for value in [&self.folders, &self.smb_links, &self.zip_files] {
            set_text(value, "Unknown");
        }
        if let Some(card) = self.card.upgrade() {
            card.set_title(message);
        }
    }
}

fn set_text(label: &glib::WeakRef<gtk::Label>, text: &str) {
    if let Some(label) = label.upgrade() {
        label.set_text(text);
    }
}

fn set_sensitive(button: &glib::WeakRef<gtk::Button>, sensitive: bool) {
    if let Some(button) = button.upgrade() {
        button.set_sensitive(sensitive);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_states_whether_openxplorer_is_the_default() {
        assert_eq!(status_title(true), "OpenXplorer is your default file explorer");
        assert_eq!(
            status_title(false),
            "OpenXplorer isn't your default file explorer yet"
        );
    }
}
