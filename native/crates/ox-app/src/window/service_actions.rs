// SPDX-License-Identifier: AGPL-3.0-only
//! Installed service actions appear only after explicit per-action opt-in.
use super::{
    actions::{plain_action, text_action},
    dialog::{self, Dialog},
    menu_popover::{MenuEntry, MenuItem},
    window_action::WindowAction,
    BrowserWindow, ButtonStyle,
};
use crate::icons::Icon;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::service_actions::{self, ServiceAction};
use ox_core::settings::PreferencesUpdate;

impl BrowserWindow {
    pub(super) fn install_service_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::ManageServiceActions, |window| {
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    window,
                    async move {
                        window.manage_service_actions().await;
                    }
                ));
            }),
            text_action(WindowAction::RunServiceAction, |window, id| {
                let id = id.to_owned();
                let selection = window.service_selection();
                let folder = window.current_uri().unwrap_or_default();
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    window,
                    async move {
                        window.run_service_action(&id, &selection, &folder).await;
                    }
                ));
            }),
        ]);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window
                    .imp()
                    .service_actions
                    .replace(service_actions::discover().await);
            }
        ));
    }

    fn service_selection(&self) -> Vec<(String, String)> {
        self.folder_pane()
            .model()
            .selected_items()
            .iter()
            .filter_map(|item| {
                let entry = item.entry();
                if !entry.can_operate || entry.is_virtual {
                    return None;
                }
                let mime = if entry.is_dir {
                    "inode/directory".into()
                } else {
                    entry
                        .content_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into())
                };
                Some((entry.uri.clone(), mime))
            })
            .collect()
    }

    pub(super) fn append_service_actions(&self, entries: &mut Vec<MenuEntry>) {
        let enabled = self.context().settings_data().preferences.enabled_service_actions;
        let selection = self.service_selection();
        let actions = self.imp().service_actions.borrow();
        let available: Vec<MenuEntry> = actions
            .iter()
            .filter(|action| enabled.contains(&action.id) && action.accepts(&selection))
            .map(|action| {
                MenuItem::with_text_target(
                    &action.name,
                    Icon::Apps,
                    WindowAction::RunServiceAction,
                    &action.id,
                )
                .into()
            })
            .collect();
        if !available.is_empty() {
            entries.push(
                MenuItem::submenu(
                    "Service actions",
                    Icon::Apps,
                    WindowAction::ManageServiceActions,
                    available,
                )
                .into(),
            );
        }
        entries.push(
            MenuItem::new(
                "Configure service actions…",
                Icon::Settings,
                WindowAction::ManageServiceActions,
            )
            .into(),
        );
    }

    async fn manage_service_actions(&self) {
        let actions = service_actions::discover().await;
        self.imp().service_actions.replace(actions.clone());
        let enabled = self.context().settings_data().preferences.enabled_service_actions;
        let prompt = Dialog::new(self, "Service actions", "Enable only actions you trust. Enabled programs run with your permissions when chosen from an item’s menu. Edited definitions must be enabled again.");
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let checks: Vec<(ServiceAction, gtk::CheckButton)> = actions
            .into_iter()
            .map(|action| {
                let check = gtk::CheckButton::with_label(&action.name);
                check.set_active(enabled.contains(&action.id));
                check.set_tooltip_text(Some(&action.path.display().to_string()));
                rows.append(&check);
                (action, check)
            })
            .collect();
        if checks.is_empty() {
            prompt.add_note("No supported actions are installed. Add KDE .desktop service menus to your XDG data folder’s kio/servicemenus directory, or executable scripts to nautilus/scripts. Embedded shell substitutions and terminal actions are not supported.");
        } else {
            let scroll = gtk::ScrolledWindow::builder()
                .child(&rows)
                .max_content_height(320)
                .propagate_natural_height(true)
                .build();
            prompt.add_labelled("Installed actions", &scroll);
        }
        prompt.add_cancel_button();
        let save = prompt.add_button("Save", ButtonStyle::Accent);
        prompt.open();
        let answer = prompt.next_response().await;
        prompt.finish();
        if answer == Some(save) {
            let enabled_service_actions = checks
                .into_iter()
                .filter_map(|(action, check)| check.is_active().then_some(action.id))
                .collect();
            self.context().update_preferences(
                PreferencesUpdate {
                    enabled_service_actions: Some(enabled_service_actions),
                    ..PreferencesUpdate::default()
                },
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |result| {
                        if let Err(error) = result {
                            window.show_message(&format!("Could not save service actions: {error}"));
                        }
                    }
                ),
            );
        }
    }

    async fn run_service_action(&self, id: &str, selection: &[(String, String)], folder: &str) {
        if !self
            .context()
            .settings_data()
            .preferences
            .enabled_service_actions
            .iter()
            .any(|enabled| enabled == id)
        {
            return;
        }
        // Discover again: replacing or editing an enabled definition revokes its opt-in.
        let actions = service_actions::discover().await;
        let Some(action) = actions
            .into_iter()
            .find(|action| action.id == id && action.accepts(selection))
        else {
            self.show_message(
                "This service action changed or no longer matches the selection. Configure it again.",
            );
            return;
        };
        let uris: Vec<String> = selection.iter().map(|(uri, _)| uri.clone()).collect();
        let result = async {
            let command = action.command(&uris, folder)?;
            let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
            if let Some(directory) = command.directory {
                launcher.set_cwd(directory);
            }
            for (key, value) in command.environment {
                launcher.setenv(key, value, true);
            }
            let argv: Vec<&std::ffi::OsStr> = command.argv.iter().map(AsRef::as_ref).collect();
            let child = launcher.spawn(&argv).map_err(|error| error.to_string())?;
            child.wait_check_future().await.map_err(|error| error.to_string())
        }
        .await;
        if let Err(error) = result {
            dialog::show_message(self, "Service action failed", &error).await;
        }
    }
}
