// SPDX-License-Identifier: AGPL-3.0-only
//! Send to ▸ Bluetooth device (CMD-038): the desktop's own Bluetooth file
//! sender, as Windows hands the files to its Bluetooth File Transfer
//! wizard (`fsquirt.exe`).
//!
//! The entry is offered only where the computer has a Bluetooth adapter
//! (an `hci` device under `/sys/class/bluetooth`) and the desktop has a
//! sender: GNOME's `bluetooth-sendto`, Blueman's `blueman-sendto` or KDE's
//! `bluedevil-sendfile`, in that order. Each asks which device to send to
//! and shows the transfer itself. The files are passed as absolute paths
//! in an argument list, never through a shell, and after `--` (or as the
//! value of `--files=`), so a name that looks like an option stays a name.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;

use super::host_command::HostCommand;
use super::sandbox::Sandbox;
use super::terminal::ExecutableSearch;

/// Where the kernel lists Bluetooth adapters, under the system root.
const ADAPTERS: &str = "sys/class/bluetooth";

/// A desktop's Bluetooth file sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BluetoothSender {
    /// GNOME Bluetooth's `bluetooth-sendto [--device=…] [FILE…]`.
    Gnome,
    /// Blueman's `blueman-sendto [-d DEVICE] [FILE…]`.
    Blueman,
    /// KDE `BlueDevil`'s `bluedevil-sendfile [-u UBI] -f FILE…`.
    BlueDevil,
}

impl BluetoothSender {
    /// Every sender, in the order they are preferred.
    pub const ALL: [BluetoothSender; 3] = [Self::Gnome, Self::Blueman, Self::BlueDevil];

    /// The sender's program name.
    pub const fn program(self) -> &'static str {
        match self {
            Self::Gnome => "bluetooth-sendto",
            Self::Blueman => "blueman-sendto",
            Self::BlueDevil => "bluedevil-sendfile",
        }
    }

    /// The first installed sender and its path, when the system under
    /// `system_root` (`/` outside tests) has a Bluetooth adapter.
    pub fn find(search: &ExecutableSearch, system_root: &Path) -> Option<(Self, PathBuf)> {
        if !has_adapter(system_root) {
            return None;
        }
        Self::ALL
            .into_iter()
            .find_map(|sender| search.find(sender.program()).map(|path| (sender, path)))
    }

    /// The program and arguments that send `files` with the sender at
    /// `executable`, which asks for the device.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] without files, or unless every file
    /// is an absolute path.
    pub fn arguments(self, executable: &Path, files: &[PathBuf]) -> io::Result<Vec<OsString>> {
        if files.is_empty() || files.iter().any(|file| !file.is_absolute()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the files must be absolute paths",
            ));
        }
        let mut arguments = vec![executable.as_os_str().to_owned()];
        match self {
            Self::Gnome | Self::Blueman => {
                arguments.push("--".into());
                arguments.extend(files.iter().map(|file| file.as_os_str().to_owned()));
            }
            Self::BlueDevil => arguments.extend(files.iter().map(|file| {
                let mut option = OsString::from("--files=");
                option.push(file.as_os_str());
                option
            })),
        }
        Ok(arguments)
    }

    /// Starts the sender at `executable` with `files` in a process group
    /// of its own, on the host inside Flatpak, and reaps it on a
    /// background thread once it exits.
    ///
    /// # Errors
    ///
    /// As [`BluetoothSender::arguments`], and the error of starting it.
    pub fn launch(self, executable: &Path, files: &[PathBuf], sandbox: Sandbox) -> io::Result<()> {
        let arguments = self.arguments(executable, files)?;
        let command = arguments[1..]
            .iter()
            .fold(HostCommand::new(executable), HostCommand::arg);
        let mut child = command
            .to_command(sandbox)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let _ = thread::Builder::new()
            .name("openxplorer-bluetooth".to_owned())
            .spawn(move || child.wait());
        Ok(())
    }
}

/// Whether the system under `system_root` lists a Bluetooth adapter.
fn has_adapter(system_root: &Path) -> bool {
    std::fs::read_dir(system_root.join(ADAPTERS)).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("hci"))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A system root with the executables `names` in `/usr/bin`, and a
    /// Bluetooth adapter when `adapter` holds.
    fn system_with(names: &[&str], adapter: bool) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("a temporary root");
        let bin = root.path().join("usr/bin");
        fs::create_dir_all(&bin).expect("usr/bin");
        for name in names {
            let program = bin.join(name);
            fs::write(&program, "#!/bin/sh\n").expect("a program");
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        }
        let adapters = root.path().join(ADAPTERS);
        fs::create_dir_all(&adapters).expect("the adapter list");
        if adapter {
            fs::create_dir(adapters.join("hci0")).expect("an adapter");
        }
        root
    }

    fn found(root: &tempfile::TempDir) -> Option<(BluetoothSender, PathBuf)> {
        BluetoothSender::find(&ExecutableSearch::under(root.path()), root.path())
    }

    /// parity: CMD-038
    #[test]
    fn a_sender_is_offered_only_with_an_adapter_and_gnome_comes_first() {
        let no_adapter = system_with(&["bluetooth-sendto"], false);
        let no_sender = system_with(&[], true);
        let kde = system_with(&["bluedevil-sendfile"], true);
        let both = system_with(&["blueman-sendto", "bluetooth-sendto"], true);

        assert_eq!(found(&no_adapter), None);
        assert_eq!(found(&no_sender), None);
        assert_eq!(
            found(&kde),
            Some((
                BluetoothSender::BlueDevil,
                PathBuf::from("/usr/bin/bluedevil-sendfile")
            ))
        );
        assert_eq!(
            found(&both),
            Some((BluetoothSender::Gnome, PathBuf::from("/usr/bin/bluetooth-sendto")))
        );
    }

    /// parity: CMD-038
    #[test]
    fn files_are_passed_as_paths_that_cannot_be_read_as_options() {
        let files = [
            PathBuf::from("/home/demo/-rf.txt"),
            PathBuf::from("/home/demo/plan.pdf"),
        ];
        let gnome = BluetoothSender::Gnome.arguments(Path::new("/usr/bin/bluetooth-sendto"), &files);
        let kde = BluetoothSender::BlueDevil.arguments(Path::new("/usr/bin/bluedevil-sendfile"), &files);

        assert_eq!(
            gnome.expect("arguments"),
            [
                "/usr/bin/bluetooth-sendto",
                "--",
                "/home/demo/-rf.txt",
                "/home/demo/plan.pdf"
            ]
        );
        assert_eq!(
            kde.expect("arguments"),
            [
                "/usr/bin/bluedevil-sendfile",
                "--files=/home/demo/-rf.txt",
                "--files=/home/demo/plan.pdf"
            ]
        );
        let relative = [PathBuf::from("plan.pdf")];
        assert!(BluetoothSender::Blueman
            .arguments(Path::new("/usr/bin/blueman-sendto"), &relative)
            .is_err());
        assert!(BluetoothSender::Blueman
            .arguments(Path::new("/usr/bin/blueman-sendto"), &[])
            .is_err());
    }
}
