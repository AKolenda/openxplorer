// SPDX-License-Identifier: AGPL-3.0-only
//! Commands a navigation-pane menu runs on its row's folder rather than on
//! the file list's selection, as Windows 11's menu of a pinned folder
//! offers them: Copy as path, Compress to (ZIP File, Additional options)
//! and New. New opens the folder, then creates the item there, so its name
//! can be typed where it shows.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::actions::text_action;
use super::copy_path::{copied_path, CopiedPath};
use super::menu_popover::{MenuEntry, MenuItem};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What separates the parts of a New-in target: the folder, the action's
/// name and its own target.
const PART: char = '\u{1f}';

/// How long New waits for the folder to be listed before giving up.
const LISTING_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The folder `uri` is in and the name it has there.
fn parent_and_name(uri: &str) -> Option<(String, String)> {
    let file = gio::File::for_uri(uri);
    let parent = file.parent()?.uri().to_string();
    let name = file.basename()?.to_string_lossy().into_owned();
    Some((parent, name))
}

/// The command bar's New menu `entries`, each item rewritten to create its
/// item in the folder at `uri` (`win.new-in`).
pub(in crate::window) fn new_menu_in(uri: &str, entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    entries
        .into_iter()
        .map(|entry| match entry {
            MenuEntry::Item(item) => {
                let inner = item
                    .target
                    .as_ref()
                    .and_then(|target| target.str().map(str::to_owned))
                    .unwrap_or_default();
                let name = item.action.detailed_name();
                let target = format!("{uri}{PART}{name}{PART}{inner}");
                MenuEntry::Item(MenuItem {
                    target: Some(target.to_variant()),
                    submenu: new_menu_in(uri, item.submenu),
                    ..MenuItem::with_text_target(&item.label, item.glyph, WindowAction::NewIn, &target)
                        .with_application_icon(item.application_icon.as_deref())
                })
            }
            MenuEntry::Divider => MenuEntry::Divider,
        })
        .collect()
}

impl BrowserWindow {
    /// Adds Copy as path, Compress to and New for a folder named by its
    /// location.
    pub(super) fn install_place_commands(&self) {
        self.add_action_entries([
            text_action(WindowAction::CopyPathOf, |window, uri| {
                if let CopiedPath::Address(address) = window.copied_path_of(uri) {
                    window.clipboard().set_text(&address);
                }
            }),
            text_action(WindowAction::CompressToZipOf, |window, uri| {
                if let Some((folder, name)) = parent_and_name(uri) {
                    window.compress_items(vec![uri.to_owned()], folder, name);
                }
            }),
            text_action(WindowAction::CompressToOf, |window, uri| {
                if let Some((folder, name)) = parent_and_name(uri) {
                    window.ask_compress(vec![uri.to_owned()], folder, &name);
                }
            }),
            text_action(WindowAction::NewIn, BrowserWindow::new_in),
        ]);
    }

    /// What Copy as path copies for the folder at `uri`.
    fn copied_path_of(&self, uri: &str) -> CopiedPath {
        let locations = self.imp().locations.borrow();
        copied_path(&[uri.to_owned()], None, &locations)
    }

    /// New in a folder: opens the folder, then once it is listed runs the
    /// New menu's action there.
    fn new_in(&self, target: &str) {
        let mut parts = target.splitn(3, PART);
        let (Some(uri), Some(action), Some(inner)) = (parts.next(), parts.next(), parts.next()) else {
            return;
        };
        if self.current_uri().as_deref() != Some(uri) {
            if let Err(error) = self.navigate(uri) {
                self.show_message(&error.to_string());
                return;
            }
        }
        let (uri, action, inner) = (uri.to_owned(), action.to_owned(), inner.to_owned());
        let started = std::time::Instant::now();
        glib::timeout_add_local(
            std::time::Duration::from_millis(50),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    if started.elapsed() > LISTING_WAIT
                        || window.current_uri().as_deref() != Some(uri.as_str())
                    {
                        return glib::ControlFlow::Break;
                    }
                    if !window.is_listed() {
                        return glib::ControlFlow::Continue;
                    }
                    let target = (!inner.is_empty()).then(|| inner.to_variant());
                    let _ = WidgetExt::activate_action(&window, &action, target.as_ref());
                    glib::ControlFlow::Break
                }
            ),
        );
    }
}
