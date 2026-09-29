// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's disk tools: GNOME Disks for a drive (Open in Disks and
//! Format…, DEV-012), the Disk Image Mounter for `.iso` and `.img` files
//! (DEV-011) and a disk-usage analyser for a folder (PROP-015).
//!
//! Dolphin offers the same through KDE Partition Manager, its own image
//! mounting and Filelight. Each tool gets its program path and one
//! checked target, never a shell, so a name that looks like an option or
//! code stays a name. A tool that is not installed is not offered.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;

use super::host_command::HostCommand;
use super::sandbox::Sandbox;
use super::terminal::ExecutableSearch;

/// GNOME Disks.
const DISKS: &str = "gnome-disks";

/// GNOME's Disk Image Mounter, which attaches an image read-only unless
/// it is asked for `--writable`.
const IMAGE_MOUNTER: &str = "gnome-disk-image-mounter";

/// The disk-usage analysers, in the order they are preferred: GNOME's
/// Disk Usage Analyzer, then KDE's Filelight.
const USAGE_ANALYSERS: [&str; 2] = ["baobab", "filelight"];

/// File name endings of the disk images the Disk Image Mounter attaches.
const DISK_IMAGE_SUFFIXES: [&str; 2] = [".iso", ".img"];

/// One of the desktop's disk tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskTool {
    /// GNOME Disks showing a drive's block device.
    OpenInDisks,
    /// GNOME Disks' Format dialog for a drive's block device.
    FormatDrive,
    /// Attaching a disk image read-only, so it shows among the drives.
    MountImage,
    /// A disk-usage analyser at a folder.
    AnalyseUsage,
}

impl DiskTool {
    /// The programs that can do it, in the order they are preferred.
    fn programs(self) -> &'static [&'static str] {
        match self {
            Self::OpenInDisks | Self::FormatDrive => &[DISKS],
            Self::MountImage => &[IMAGE_MOUNTER],
            Self::AnalyseUsage => &USAGE_ANALYSERS,
        }
    }

    /// The installed program for this tool, if any.
    pub fn find(self, search: &ExecutableSearch) -> Option<PathBuf> {
        self.programs().iter().find_map(|program| search.find(program))
    }

    /// The program and arguments that run the tool at `executable` on
    /// `target`: a block device such as `/dev/sdb1` for Disks, an image
    /// file or a folder otherwise.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] unless `target` is an absolute
    /// path, so it can never be read as an option.
    pub fn arguments(self, executable: &Path, target: &Path) -> io::Result<Vec<OsString>> {
        if !target.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the target must be an absolute path",
            ));
        }
        let mut arguments = vec![executable.as_os_str().to_owned()];
        if matches!(self, Self::OpenInDisks | Self::FormatDrive) {
            arguments.push("--block-device".into());
        }
        arguments.push(target.as_os_str().to_owned());
        if self == Self::FormatDrive {
            arguments.push("--format-device".into());
        }
        Ok(arguments)
    }

    /// Starts the tool at `executable` on `target` in a process group of
    /// its own, on the host inside Flatpak, and reaps it on a background
    /// thread once it exits.
    ///
    /// # Errors
    ///
    /// As [`DiskTool::arguments`], and the error of starting the program.
    pub fn launch(self, executable: &Path, target: &Path, sandbox: Sandbox) -> io::Result<()> {
        let mut arguments = self.arguments(executable, target)?.into_iter();
        let program = arguments.next().expect("the arguments start with the program");
        let command = arguments.fold(HostCommand::new(program), HostCommand::arg);
        let mut child = command
            .to_command(sandbox)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        // If no thread can be started, the tool still runs; it is reaped
        // when OpenXplorer exits.
        let _ = thread::Builder::new()
            .name("openxplorer-disk-tool".to_owned())
            .spawn(move || child.wait());
        Ok(())
    }
}

/// True for a file name the Disk Image Mounter attaches: `.iso` or
/// `.img`, in any case.
pub fn is_disk_image(name: &str) -> bool {
    let name = name.to_lowercase();
    DISK_IMAGE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A system root with the executables `names` in `/usr/bin`.
    fn system_with(names: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("a temporary root");
        let bin = root.path().join("usr/bin");
        fs::create_dir_all(&bin).expect("usr/bin");
        for name in names {
            let program = bin.join(name);
            fs::write(&program, "#!/bin/sh\n").expect("a program");
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        }
        root
    }

    /// parity: DEV-012
    #[test]
    fn disks_opens_or_formats_the_block_device() {
        let disks = Path::new("/usr/bin/gnome-disks");
        let device = Path::new("/dev/sdb1");

        let open = DiskTool::OpenInDisks.arguments(disks, device).expect("arguments");
        let format = DiskTool::FormatDrive.arguments(disks, device).expect("arguments");

        assert_eq!(open, ["/usr/bin/gnome-disks", "--block-device", "/dev/sdb1"]);
        assert_eq!(
            format,
            ["/usr/bin/gnome-disks", "--block-device", "/dev/sdb1", "--format-device"]
        );
        assert!(DiskTool::OpenInDisks.arguments(disks, Path::new("--help")).is_err());
    }

    /// parity: DEV-011
    #[test]
    fn iso_and_img_files_are_disk_images_mounted_by_path() {
        assert!(is_disk_image("Ubuntu 24.04.ISO"));
        assert!(is_disk_image("backup.img"));
        assert!(!is_disk_image("image.png"));
        let mounter = Path::new("/usr/bin/gnome-disk-image-mounter");
        let image = Path::new("/home/demo/-rf.iso");

        let arguments = DiskTool::MountImage.arguments(mounter, image).expect("arguments");

        assert_eq!(arguments, ["/usr/bin/gnome-disk-image-mounter", "/home/demo/-rf.iso"]);
    }

    /// parity: PROP-015, DEV-012
    #[test]
    fn a_tool_is_found_only_when_installed_and_baobab_comes_first() {
        let bare = system_with(&[]);
        let kde = system_with(&["filelight"]);
        let both = system_with(&["filelight", "baobab", "gnome-disks"]);
        let found = |tool: DiskTool, root: &tempfile::TempDir| tool.find(&ExecutableSearch::under(root.path()));

        assert_eq!(found(DiskTool::AnalyseUsage, &bare), None);
        assert_eq!(found(DiskTool::OpenInDisks, &bare), None);
        assert_eq!(
            found(DiskTool::AnalyseUsage, &kde),
            Some(PathBuf::from("/usr/bin/filelight"))
        );
        assert_eq!(
            found(DiskTool::AnalyseUsage, &both),
            Some(PathBuf::from("/usr/bin/baobab"))
        );
        assert_eq!(
            found(DiskTool::FormatDrive, &both),
            Some(PathBuf::from("/usr/bin/gnome-disks"))
        );
    }
}
