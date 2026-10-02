// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's disk tools in the menus: Open in Disks and Format… for a
//! removable drive (DEV-012), Mount disk image for an `.iso` or `.img`
//! file (DEV-011) and Analyse disk usage for a folder (PROP-015).
//!
//! Dolphin offers these through KDE Partition Manager, image mounting and
//! Filelight; here GNOME Disks, GNOME's Disk Image Mounter and Disk Usage
//! Analyzer (or Filelight) do the work, and an item is only offered where
//! its tool is installed ([`is_installed`]).
//!
//! Dolphin's status bar also offers its disk-usage analyser from the menu
//! of its space indicator; here a secondary click on the status bar
//! offers Analyse disk usage for the folder shown.

use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::integration::{DiskTool, ExecutableSearch, Sandbox};
use ox_core::location::normalise;

use super::actions::text_action;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::icons::Icon;

/// Shown when a drive has no block device for Disks.
const NO_BLOCK_DEVICE: &str = crate::i18n::message_id("Disks cannot open this drive.");

/// Shown for a location without a local path.
const NO_LOCAL_PATH: &str = crate::i18n::message_id("This tool needs a location on this computer.");

/// Whether the program for `tool` is installed, on the host inside
/// Flatpak.
pub(crate) fn is_installed(tool: DiskTool) -> bool {
    installed_program(tool).is_some()
}

/// The installed program for `tool`.
fn installed_program(tool: DiskTool) -> Option<PathBuf> {
    tool.find(&ExecutableSearch::for_sandbox(Sandbox::detect()))
}

/// The block device, such as `/dev/sdb1`, of the mount at the canonical
/// root `uri` among `mounts`.
fn block_device_of(mounts: &[gio::Mount], uri: &str) -> Option<String> {
    let mount = mounts
        .iter()
        .find(|mount| normalise(&mount.root().uri()).as_deref() == Ok(uri))?;
    let device = mount.volume()?.identifier("unix-device")?;
    Some(device.to_string())
}

impl BrowserWindow {
    /// Adds the disk-tool actions, each with a location as its target.
    pub(super) fn install_disk_tool_actions(&self) {
        self.add_action_entries([
            text_action(WindowAction::OpenInDisks, |window, uri| {
                window.open_drive_in_disks(uri, DiskTool::OpenInDisks);
            }),
            text_action(WindowAction::FormatDrive, |window, uri| {
                window.open_drive_in_disks(uri, DiskTool::FormatDrive);
            }),
            text_action(WindowAction::MountDiskImage, |window, uri| {
                window.run_disk_tool_at(DiskTool::MountImage, uri);
            }),
            text_action(WindowAction::AnalyseDiskUsage, |window, uri| {
                window.run_disk_tool_at(DiskTool::AnalyseUsage, uri);
            }),
        ]);
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, x, y| {
                window.show_status_bar_menu(x, y);
            }
        ));
        self.status_bar().add_controller(click);
        // Dolphin's space indicator offers the tools on a click too.
        let free_space = self.status_bar().free_space_widget();
        let primary = gtk::GestureClick::new();
        primary.connect_released(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            free_space,
            move |_, _, x, y| {
                #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
                let point = free_space.compute_point(
                    window.status_bar(),
                    &gtk::graphene::Point::new(x as f32, y as f32),
                );
                if let Some(point) = point {
                    window.show_status_bar_menu(f64::from(point.x()), f64::from(point.y()));
                }
            }
        ));
        free_space.add_controller(primary);
    }

    /// The status bar's menu at (`x`, `y`), when it has something to
    /// offer for the folder shown.
    pub(super) fn show_status_bar_menu(&self, x: f64, y: f64) -> Option<MenuPopover> {
        let folder = self.current_uri()?;
        let entries = status_bar_entries(&folder, is_installed(DiskTool::AnalyseUsage));
        if entries.is_empty() {
            return None;
        }
        let popover = MenuPopover::new(entries);
        popover.set_parent(self.status_bar());
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let closed = popover.clone();
            glib::idle_add_local_once(move || closed.unparent());
        });
        popover.popup();
        Some(popover)
    }

    /// Opens Disks, or its Format dialog, for the drive mounted at `uri`.
    fn open_drive_in_disks(&self, uri: &str, tool: DiskTool) {
        match block_device_of(&self.volume_monitor().mounts(), uri) {
            Some(device) => self.run_disk_tool(tool, PathBuf::from(device)),
            None => self.show_message(ox_core::i18n::gettext_static(NO_BLOCK_DEVICE)),
        }
    }

    /// Runs `tool` on the local path of the file or folder at `uri`.
    fn run_disk_tool_at(&self, tool: DiskTool, uri: &str) {
        match gio::File::for_uri(uri).path() {
            Some(path) => self.run_disk_tool(tool, path),
            None => self.show_message(ox_core::i18n::gettext_static(NO_LOCAL_PATH)),
        }
    }

    /// Starts `tool` on `target` off the main thread and reports a
    /// failure.
    fn run_disk_tool(&self, tool: DiskTool, target: PathBuf) {
        // Test safety: tests record the tool instead of starting it.
        #[cfg(test)]
        if self.context().record_tool_launch(tool, &target) {
            return;
        }
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let started = gio::spawn_blocking(move || start(tool, &target)).await;
            let failure = match started {
                Ok(Ok(())) => return,
                Ok(Err(message)) => message,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            if let Some(window) = window.upgrade() {
                window.show_message(&failure);
            }
        });
    }
}

/// The status bar's menu for the folder at `folder`: Analyse disk usage
/// when an analyser is installed and the folder is on this computer.
fn status_bar_entries(folder: &str, has_analyser: bool) -> Vec<MenuEntry> {
    let is_local = folder.starts_with("file:") && gio::File::for_uri(folder).path().is_some();
    if !(has_analyser && is_local) {
        return Vec::new();
    }
    let analyse = MenuItem::with_text_target(
        &ox_core::i18n::gettext("Analyse disk usage"),
        Icon::HardDrive,
        WindowAction::AnalyseDiskUsage,
        folder,
    );
    vec![analyse.into()]
}

/// Finds and starts `tool` on `target`; the message to show when it
/// cannot.
fn start(tool: DiskTool, target: &Path) -> Result<(), String> {
    let sandbox = Sandbox::detect();
    let program = installed_program(tool).ok_or_else(|| "The tool is not installed.".to_owned())?;
    tool.launch(&program, target, sandbox).map_err(|error| {
        ox_core::i18n::format_message(
            "Could not start {display}: {error}",
            &[
                ("display", &(program.display()).to_string()),
                ("error", &(error).to_string()),
            ],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status bar offers the analyser for a folder on this computer,
    /// and only where one is installed.
    ///
    /// parity: PROP-015
    #[test]
    fn the_status_bar_offers_the_analyser_for_a_local_folder() {
        let entries = status_bar_entries("file:///srv/media", true);
        let [MenuEntry::Item(item)] = entries.as_slice() else {
            panic!("one item: {entries:?}");
        };
        assert_eq!(item.label, "Analyse disk usage");
        assert_eq!(item.action, WindowAction::AnalyseDiskUsage.into());
        assert_eq!(item.target, Some("file:///srv/media".to_variant()));
        assert!(status_bar_entries("file:///srv/media", false).is_empty());
        assert!(status_bar_entries("smb://nas/share", true).is_empty());
        assert!(status_bar_entries("ox:this-pc", true).is_empty());
    }
}
