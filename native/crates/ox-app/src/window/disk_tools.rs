// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's disk tools in the menus: Open in Disks and Format… for a
//! removable drive (DEV-012), Mount disk image for an `.iso` or `.img`
//! file (DEV-011) and Analyse disk usage for a folder (PROP-015).
//!
//! Dolphin offers these through KDE Partition Manager, image mounting and
//! Filelight; here GNOME Disks, GNOME's Disk Image Mounter and Disk Usage
//! Analyzer (or Filelight) do the work, and an item is only offered where
//! its tool is installed ([`is_installed`]).

use std::path::{Path, PathBuf};

use gio::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{DiskTool, ExecutableSearch, Sandbox};
use ox_core::location::normalise;

use super::actions::text_action;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// Shown when a drive has no block device for Disks.
const NO_BLOCK_DEVICE: &str = "Disks cannot open this drive.";

/// Shown for a location without a local path.
const NO_LOCAL_PATH: &str = "This tool needs a location on this computer.";

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
    }

    /// Opens Disks, or its Format dialog, for the drive mounted at `uri`.
    fn open_drive_in_disks(&self, uri: &str, tool: DiskTool) {
        match block_device_of(&self.volume_monitor().mounts(), uri) {
            Some(device) => self.run_disk_tool(tool, PathBuf::from(device)),
            None => self.show_message(NO_BLOCK_DEVICE),
        }
    }

    /// Runs `tool` on the local path of the file or folder at `uri`.
    fn run_disk_tool_at(&self, tool: DiskTool, uri: &str) {
        match gio::File::for_uri(uri).path() {
            Some(path) => self.run_disk_tool(tool, path),
            None => self.show_message(NO_LOCAL_PATH),
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

/// Finds and starts `tool` on `target`; the message to show when it
/// cannot.
fn start(tool: DiskTool, target: &Path) -> Result<(), String> {
    let sandbox = Sandbox::detect();
    let program = installed_program(tool).ok_or_else(|| "The tool is not installed.".to_owned())?;
    tool.launch(&program, target, sandbox)
        .map_err(|error| format!("Could not start {}: {error}", program.display()))
}
