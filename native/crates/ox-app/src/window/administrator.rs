// SPDX-License-Identifier: AGPL-3.0-only
//! Opt-in `GVfs` admin access. `GVfs` owns authentication; the app stays unprivileged.
use super::{actions::plain_action, window_action::WindowAction, BrowserWindow, ButtonStyle};
use crate::dialog::{self, Dialog};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

impl BrowserWindow {
    pub(super) fn administrator_target(&self) -> Option<String> {
        let items = self.folder_pane().model().selected_items();
        let uri = match items.as_slice() {
            [] => self.current_uri()?,
            [item] if item.entry().is_dir => item.entry().uri.clone(),
            _ => return None,
        };
        if self.imp().locations.borrow().is_snapshot_location(&uri) {
            return None;
        }
        let local = uri.strip_prefix("file:///")?;
        Some(format!("admin:///{local}"))
    }

    pub(super) fn install_administrator_action(&self) {
        self.add_action_entries([plain_action(WindowAction::OpenAsAdministrator, |window| {
            let Some(uri) = window.administrator_target() else { return; };
            glib::spawn_future_local(glib::clone!(#[weak] window, async move {
                if !gio::Vfs::default().supported_uri_schemes().iter().any(|scheme| scheme == "admin") {
                    dialog::show_message(&window, "Administrator access unavailable", "Install your distribution’s `GVfs` administrator backend to use admin:// locations. OpenXplorer itself always runs as your user.").await;
                    return;
                }
                let prompt = Dialog::new(&window, "Open as administrator?", "This folder will use administrator permissions. Your desktop may ask you to authenticate. Changes here can affect every user.");
                prompt.add_cancel_button();
                let open = prompt.add_button("Open as administrator", ButtonStyle::Accent);
                prompt.open();
                let answer = prompt.next_response().await;
                prompt.finish();
                if answer == Some(open) { window.navigate_or_report(&uri); }
            }));
        })]);
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::harness::{Fixture, TestWindow};
    /// parity: OPS-039
    #[gtk::test]
    fn administrator_action_only_maps_local_folders_without_changing_the_current_location() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let target = test.window.administrator_target().unwrap();
        assert_eq!(target, fixture.uri().replacen("file:", "admin:", 1));
        assert_eq!(test.window.current_uri().as_deref(), Some(fixture.uri().as_str()));
    }
}
