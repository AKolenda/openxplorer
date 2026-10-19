// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's disk tools in the menus: Open in Disks and Format… for a
//! removable drive (DEV-012), Mount disk image for an `.iso` or `.img`
//! file (DEV-011) and Analyse disk usage for a folder (PROP-015).
//!
//! Dolphin offers these through KDE Partition Manager, image mounting and
//! Filelight; here GNOME Disks, GNOME's Disk Image Mounter and Disk Usage
//! Analyzer (or Filelight) do the work, and an item is only offered where
//! its tool is installed ([`is_installed`]). Format… is the app's own
//! dialog ([`crate::dialogs::format_drive_dialog`]), which formats through
//! UDisks, so it needs no tool.
//!
//! Dolphin's status bar also offers its disk-usage analyser from the menu
//! of its space indicator; here a secondary click on the status bar
//! offers Analyse disk usage for the folder shown.

use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::integration::{
    available_file_systems, drive_facts, format_drive, DiskTool, DriveFacts, ExecutableSearch, FileSystem,
    FormatError, FormatRequest, Formatted, Sandbox, UDISKS_NAME,
};
use ox_core::location::normalise;
use ox_core::network::WriteActivity;
use ox_core::transfer::Cancellation;

use super::actions::text_action;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::dialogs::{confirm_erase, format_drive_dialog};
use crate::icons::Icon;
use crate::window::Dialog;

/// Shown when a drive has no block device for Disks.
const NO_BLOCK_DEVICE: &str = crate::i18n::message_id("Disks cannot open this drive.");

/// Shown when a drive has no block device to format.
const NOT_FORMATTABLE: &str = crate::i18n::message_id("This drive cannot be formatted.");

/// Shown when the system cannot make any file system the dialog offers.
const NO_FILE_SYSTEMS: &str =
    crate::i18n::message_id("No program to make a file system is installed on this computer.");

/// Shown when Format… is chosen while files are written, as a write
/// could resume on the drive once it is formatted.
const FINISH_WRITING: &str = crate::i18n::message_id("Finish the current operation before formatting.");

/// Start's label while the drive is formatted.
const FORMATTING: &str = crate::i18n::message_id("Formatting…");

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

/// The mount at the canonical root `uri` among `mounts`.
fn mount_at<'a>(mounts: &'a [gio::Mount], uri: &str) -> Option<&'a gio::Mount> {
    mounts
        .iter()
        .find(|mount| normalise(&mount.root().uri()).as_deref() == Ok(uri))
}

/// The block device, such as `/dev/sdb1`, of the mount at the canonical
/// root `uri` among `mounts`.
fn block_device_of(mounts: &[gio::Mount], uri: &str) -> Option<String> {
    let device = mount_at(mounts, uri)?.volume()?.identifier("unix-device")?;
    Some(device.to_string())
}

/// The bus UDisks is on: the system bus, and in tests the private session
/// bus they run on, where no UDisks answers.
fn udisks_bus() -> gio::BusType {
    if cfg!(test) {
        gio::BusType::Session
    } else {
        gio::BusType::System
    }
}

/// What the Format dialog needs: the bus UDisks answers on, the drive,
/// and the file systems the system can make.
struct FormatTarget {
    connection: gio::DBusConnection,
    facts: DriveFacts,
    available: Vec<FileSystem>,
}

/// Asks UDisks about the block device `device` for the Format dialog.
async fn read_format_target(device: &str) -> Result<FormatTarget, FormatError> {
    let connection = gio::bus_get_future(udisks_bus())
        .await
        .map_err(FormatError::Unavailable)?;
    let facts = drive_facts(&connection, UDISKS_NAME, device).await?;
    let available = available_file_systems(&connection, UDISKS_NAME, facts.size).await;
    Ok(FormatTarget {
        connection,
        facts,
        available,
    })
}

impl BrowserWindow {
    /// Adds the disk-tool actions, each with a location as its target.
    pub(super) fn install_disk_tool_actions(&self) {
        self.add_action_entries([
            text_action(WindowAction::OpenInDisks, |window, uri| {
                window.open_drive_in_disks(uri, DiskTool::OpenInDisks);
            }),
            text_action(WindowAction::FormatDrive, BrowserWindow::open_format_dialog),
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

    /// Format…: asks UDisks about the drive mounted at `uri`, then shows
    /// the Format dialog for it. Refused while any window writes files,
    /// as Disconnect is, so no write goes on onto the formatted drive.
    fn open_format_dialog(&self, uri: &str) {
        if self.writes_in_any_window() == WriteActivity::Writing {
            self.show_message(ox_core::i18n::gettext_static(FINISH_WRITING));
            return;
        }
        let mounts = self.volume_monitor().mounts();
        let (Some(mount), Some(device)) = (mount_at(&mounts, uri), block_device_of(&mounts, uri)) else {
            self.show_message(ox_core::i18n::gettext_static(NOT_FORMATTABLE));
            return;
        };
        let name = mount.name().to_string();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let target = read_format_target(&device).await;
            let Some(window) = window.upgrade() else {
                return;
            };
            match target {
                Ok(target) => window.show_format_dialog(&name, target),
                Err(error) => window.show_message(&error.to_string()),
            }
        });
    }

    /// Shows the Format dialog for the drive `name` that `target`
    /// describes.
    fn show_format_dialog(&self, name: &str, target: FormatTarget) {
        if target.available.is_empty() {
            self.show_message(ox_core::i18n::gettext_static(NO_FILE_SYSTEMS));
            return;
        }
        let FormatTarget {
            connection,
            facts,
            available,
        } = target;
        let name = name.to_owned();
        let dialog = format_drive_dialog(
            self,
            &name,
            &facts,
            &available,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                facts,
                #[strong]
                name,
                move |dialog, request| window.start_format(dialog, &connection, &facts, &name, request)
            ),
        );
        dialog.open();
    }

    /// Start: once the user confirms, formats the drive `facts` describes
    /// as `request` asks, with "Formatting…" in `dialog` meanwhile, and
    /// reports the result there.
    ///
    /// UDisks cannot be stopped once it formats, so the buttons are
    /// disabled until it is done. Closing the dialog meanwhile leaves the
    /// format running, and the window reports how it ended.
    fn start_format(
        &self,
        dialog: &Dialog,
        connection: &gio::DBusConnection,
        facts: &DriveFacts,
        name: &str,
        request: FormatRequest,
    ) {
        let (window, form) = (self.downgrade(), dialog.downgrade());
        let (connection, facts) = (connection.clone(), facts.clone());
        let name = name.to_owned();
        glib::spawn_future_local(async move {
            let Some(dialog) = form.upgrade() else {
                return;
            };
            if !confirm_erase(&dialog).await {
                return;
            }
            let Some(holder) = window.upgrade() else {
                return;
            };
            // A write may have started while the dialog was open.
            if holder.writes_in_any_window() == WriteActivity::Writing {
                dialog.show_error(ox_core::i18n::gettext_static(FINISH_WRITING));
                return;
            }
            // From here until the drive is mounted again no window starts
            // writing, so no transfer goes on onto the formatted drive.
            let hold = holder.context().hold_writes_for_formatting();
            drop(holder);
            let (sender, receiver) = async_channel::bounded::<Result<Formatted, FormatError>>(1);
            let shown = (window.clone(), form.clone(), name.clone());
            dialog.run(ox_core::i18n::gettext_static(FORMATTING), async move {
                let Ok(formatted) = receiver.recv().await else {
                    return;
                };
                let (window, form, name) = shown;
                let (Some(window), Some(dialog)) = (window.upgrade(), form.upgrade()) else {
                    return;
                };
                dialog.set_busy(None);
                match formatted {
                    Ok(formatted) => {
                        dialog.finish();
                        window.show_message(&formatted_message(&name, &formatted));
                    }
                    Err(error) => dialog.show_error(&error.to_string()),
                }
            });
            dialog.set_busy(Some(&Cancellation::new()));
            drop(dialog);
            let formatted = format_drive(&connection, UDISKS_NAME, &facts, &request).await;
            drop(hold);
            // The dialog was closed: the window reports the result.
            if let Err(unheard) = sender.try_send(formatted) {
                let Some(window) = window.upgrade() else {
                    return;
                };
                match unheard.into_inner() {
                    Ok(formatted) => window.show_message(&formatted_message(&name, &formatted)),
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        });
    }

    /// Opens Disks for the drive mounted at `uri`.
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
    let program =
        installed_program(tool).ok_or_else(|| ox_core::i18n::gettext("The tool is not installed."))?;
    tool.launch(&program, target, sandbox).map_err(|error| {
        ox_core::i18n::format_message(
            "Could not start {display}: {error}",
            &[
                ("display", &program.display().to_string()),
                ("error", &error.to_string()),
            ],
        )
    })
}

/// What the window says once the drive `name` is `formatted`: that it is
/// formatted, and why it is not mounted again when it is not.
fn formatted_message(name: &str, formatted: &Formatted) -> String {
    match formatted {
        Formatted::Mounted => ox_core::i18n::format_message("{drive} is formatted.", &[("drive", name)]),
        Formatted::NotMounted(error) => ox_core::i18n::format_message(
            "{drive} is formatted, but could not be mounted again: {error}",
            &[("drive", name), ("error", error)],
        ),
    }
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

    /// A drive formatted but not mounted again says why it is not mounted,
    /// not only that it is formatted.
    ///
    /// parity: DEV-012
    #[test]
    fn a_drive_not_mounted_again_says_why() {
        assert_eq!(
            formatted_message("USB Drive", &Formatted::Mounted),
            "USB Drive is formatted."
        );
        assert_eq!(
            formatted_message("USB Drive", &Formatted::NotMounted("Not permitted".to_owned())),
            "USB Drive is formatted, but could not be mounted again: Not permitted"
        );
    }
}
