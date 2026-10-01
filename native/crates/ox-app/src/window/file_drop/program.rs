// SPDX-License-Identifier: AGPL-3.0-only
//! Dropping items onto a program or script runs it with their paths
//! (DND-026), as Windows Explorer does when a file is dropped onto an
//! `.exe`, and onto a `.desktop` launcher starts its application with
//! them ([`super::launcher`], DND-020).
//!
//! New in the native app, promised by the owner. A file under a drag is a
//! program when GIO says it is a regular file the user may execute and its
//! content type is a binary's or a script's; the answer is looked up once per file while the drag hovers
//! ([`ProgramChecks`]) and checked again before anything runs. A binary
//! or an `AppImage` runs directly; a script (a text file) runs in the
//! user's terminal, which stays open after it ends so its output can be
//! read.
//!
//! Safety rules:
//! - "Nothing runs from a file that is not executable": the execute
//!   permission is checked again when the drop arrives.
//! - "Ask before running a program from elsewhere": a program on a
//!   network share or a removable drive runs only after the user
//!   confirms.
//! - "Names are never code": the program gets the items' paths as
//!   separate arguments, never through a command line a shell parses. The
//!   terminal's shell runs one fixed script that calls the program with
//!   its arguments as they are.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::integration::{find_terminal, ExecutableSearch, Sandbox, Terminal};
use ox_core::network::local_path;

use super::launcher;
use crate::integration::process::{in_terminal, spawn_command};
use crate::window::dialog::{ButtonStyle, Dialog};
use crate::window::BrowserWindow;

/// What GIO is asked about a file under a drag.
const PROGRAM_ATTRIBUTES: &str = "standard::type,standard::content-type,access::can-execute";

/// The content type every script is a kind of.
const TEXT_CONTENT_TYPE: &str = "text/plain";

/// The content types of programs: binaries, and the scripts that
/// shared-mime-info declares or older versions only name. A file of any
/// other type is never run, whatever its execute bit says, since on FAT,
/// NTFS and SMB mounts every file has it. This is the list Dolphin offers
/// "Execute" for, less `.desktop` launchers, which [`launcher`] handles.
const PROGRAM_CONTENT_TYPES: [&str; 8] = [
    "application/x-executable",
    "application/x-sharedlib",
    "application/x-pie-executable",
    "application/x-shellscript",
    "application/x-perl",
    "application/x-ruby",
    "text/x-python",
    "text/x-python3",
];

/// The name the hold script runs under (`$0`).
const HOLD_SCRIPT_NAME: &str = "openxplorer-drop";

/// Why a program cannot run.
const NO_LOCAL_PATH: &str = "This program has no local path. Mount its share before dropping files on it.";

/// How a program runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgramKind {
    /// A binary or an `AppImage`: runs directly.
    Binary,
    /// A script: runs in the terminal.
    Script,
    /// A desktop launcher: its application starts with the items.
    Launcher,
}

/// An executable file that dropped items can be given to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgramTarget {
    /// Where the program is.
    pub(crate) uri: String,
    /// Its name, for "Open with <name>".
    pub(crate) name: String,
    /// Whether it runs directly or in the terminal.
    pub(crate) kind: ProgramKind,
}

/// What is known about whether one file under a drag is a program.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProgramCheck {
    /// GIO has been asked and has not answered yet.
    Asked,
    /// It is not a program, or GIO could not tell.
    NotProgram,
    /// It is this program.
    Program(ProgramTarget),
}

/// What is known about whether each file under a drag is a program.
#[derive(Debug, Default)]
pub(crate) struct ProgramChecks {
    answers: HashMap<String, ProgramCheck>,
}

/// The program `entry` is, going by GIO's answer `info` about it.
fn program_from_info(entry: &Entry, info: &gio::FileInfo) -> Option<ProgramTarget> {
    let is_regular = info.file_type() == gio::FileType::Regular;
    let may_execute = info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE);
    let content_type = info.content_type()?;
    let is_program_type = PROGRAM_CONTENT_TYPES
        .iter()
        .any(|program_type| gio::content_type_is_a(&content_type, program_type));
    if !is_regular || !may_execute || !is_program_type {
        return None;
    }
    let is_script = gio::content_type_is_a(&content_type, TEXT_CONTENT_TYPE);
    let kind = if is_script {
        ProgramKind::Script
    } else {
        ProgramKind::Binary
    };
    Some(ProgramTarget {
        uri: entry.uri.clone(),
        name: entry.name.clone(),
        kind,
    })
}

/// The command that gives `items` to the program at `program`: the
/// program itself for a binary, or `terminal` running it through the hold
/// script for a script. Each item stays one argument, whatever its name.
pub(super) fn program_command(
    program: &Path,
    kind: ProgramKind,
    items: &[OsString],
    terminal: Option<&Terminal>,
) -> Vec<OsString> {
    let mut command = vec![program.as_os_str().to_owned()];
    command.extend(items.iter().cloned());
    match (kind, terminal) {
        (ProgramKind::Script, Some(terminal)) => in_terminal(terminal, Some(HOLD_SCRIPT_NAME), command),
        _ => command,
    }
}

/// The argument that names the dropped item at `uri`: its local path, or
/// its address when it has none.
fn item_argument(uri: &str) -> OsString {
    local_path(uri).map_or_else(|| OsString::from(uri), PathBuf::into_os_string)
}

/// True when a program at `uri` asks before it runs: it is not on this
/// computer's own disks, but on a network share (`is_network`) or a drive
/// that can be removed (`is_removable`).
fn needs_run_confirmation(uri: &str, is_network: bool, is_removable: bool) -> bool {
    !uri.starts_with("file:") || is_network || is_removable
}

impl BrowserWindow {
    /// The program `entry` is, as far as known; asks GIO when nobody has
    /// yet, so a later motion of the drag finds the answer.
    pub(super) fn program_under_drag(&self, entry: &Entry) -> Option<ProgramTarget> {
        let known = self
            .imp()
            .program_checks
            .borrow()
            .answers
            .get(&entry.uri)
            .cloned();
        match known {
            Some(ProgramCheck::Program(program)) => return Some(program),
            Some(ProgramCheck::Asked | ProgramCheck::NotProgram) => return None,
            None => {}
        }
        self.record_program_check(&entry.uri, ProgramCheck::Asked);
        let entry = entry.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let answer = match query_program(&entry).await {
                    Some(program) => ProgramCheck::Program(program),
                    None => ProgramCheck::NotProgram,
                };
                window.record_program_check(&entry.uri, answer);
            }
        ));
        None
    }

    /// Records what is known about whether the file at `uri` is a program.
    fn record_program_check(&self, uri: &str, check: ProgramCheck) {
        let mut checks = self.imp().program_checks.borrow_mut();
        checks.answers.insert(uri.to_owned(), check);
    }

    /// Forgets which files were programs, when a drag ends.
    pub(super) fn forget_program_checks(&self) {
        self.imp().program_checks.borrow_mut().answers.clear();
    }

    /// Gives `items` to `program` once the drop handler has returned:
    /// checked again, confirmed when it is not on this computer's own
    /// disk, then started.
    pub(super) fn open_with_program(&self, program: ProgramTarget, items: Vec<String>) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                if let Err(message) = window.run_program(&program, &items).await {
                    window.show_message(&message);
                }
            }
        ));
    }

    /// Checks, confirms and starts `program` with `items`; the message to
    /// show when it does not start.
    pub(in crate::window) async fn run_program(
        &self,
        program: &ProgramTarget,
        items: &[String],
    ) -> Result<(), String> {
        let file = gio::File::for_uri(&program.uri);
        let info = file
            .query_info_future(
                PROGRAM_ATTRIBUTES,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
            .map_err(|error| error.to_string())?;
        let path = local_path(&program.uri).ok_or_else(|| NO_LOCAL_PATH.to_owned())?;
        let may_run = match program.kind {
            ProgramKind::Launcher => launcher::is_trusted(&path, &info),
            ProgramKind::Binary | ProgramKind::Script => info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE),
        };
        if !may_run {
            return Err(format!("“{}” is not a program you can run.", program.name));
        }
        // Safety rule "ask before running a program from elsewhere".
        if self.is_from_elsewhere(program).await && !self.confirm_run(program).await {
            return Ok(());
        }
        self.start_program(program, &path, items)
    }

    /// True for a program on a network share or a removable drive.
    async fn is_from_elsewhere(&self, program: &ProgramTarget) -> bool {
        let is_network = self.imp().locations.borrow().is_network_location(&program.uri);
        let uri = program.uri.clone();
        // GIO has no asynchronous form of this lookup, and it may wait for
        // the volume monitor, so it runs on a worker. A failed lookup asks.
        let is_removable = gio::spawn_blocking(move || is_on_removable_drive(&uri))
            .await
            .unwrap_or(true);
        needs_run_confirmation(&program.uri, is_network, is_removable)
    }

    /// Asks before running `program`; true when the user agreed.
    async fn confirm_run(&self, program: &ProgramTarget) -> bool {
        let message = format!(
            "“{}” is on a network share or a removable drive. Run it only if you trust where it came from.",
            program.name
        );
        let dialog = Dialog::new(self, "Run this program?", &message);
        dialog.add_cancel_button();
        dialog.add_button("Run", ButtonStyle::Primary);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        answer.is_some()
    }

    /// Starts `program`, at `path`, with the local paths of `items`.
    fn start_program(&self, program: &ProgramTarget, path: &Path, items: &[String]) -> Result<(), String> {
        // Test safety: tests record the run instead of starting a program
        // or a terminal on the developer's desktop.
        #[cfg(test)]
        if self.context().record_run(&program.uri) {
            return Ok(());
        }
        let sandbox = Sandbox::detect();
        let arguments: Vec<OsString> = items.iter().map(|uri| item_argument(uri)).collect();
        let (command, folder) = match program.kind {
            ProgramKind::Launcher => (
                launcher::launch_command(path, &arguments),
                launcher::launch_folder(),
            ),
            ProgramKind::Binary | ProgramKind::Script => {
                let terminal = match program.kind {
                    ProgramKind::Script => Some(
                        find_terminal(&ExecutableSearch::for_sandbox(sandbox))
                            .map_err(|error| error.to_string())?,
                    ),
                    _ => None,
                };
                let command = program_command(path, program.kind, &arguments, terminal.as_ref());
                (command, path.parent().unwrap_or(Path::new("/")).to_path_buf())
            }
        };
        spawn_command(&command, &folder, sandbox).map_err(|error| error.to_string())?;
        self.show_message(&format!("Opened {} item(s) with {}.", items.len(), program.name));
        Ok(())
    }
}

/// True when the local file at `uri` is on a drive that can be ejected or
/// removed, such as a USB stick or an SD card. Blocks; run it on a worker.
fn is_on_removable_drive(uri: &str) -> bool {
    let file = gio::File::for_uri(uri);
    let Ok(mount) = file.find_enclosing_mount(None::<&gio::Cancellable>) else {
        return false;
    };
    let is_removable_drive = mount.drive().is_some_and(|drive| drive.is_removable());
    mount.can_eject() || is_removable_drive
}

/// Asks GIO whether `entry` is a program or a launcher; `None` for a file
/// that is neither or cannot be read.
pub(in crate::window) async fn query_program(entry: &Entry) -> Option<ProgramTarget> {
    let file = gio::File::for_uri(&entry.uri);
    let info = file
        .query_info_future(
            PROGRAM_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    // A desktop entry is text, but never a script to run in a shell.
    if launcher::is_desktop_entry(&info) {
        return launcher::query_launcher(entry, &info).await;
    }
    program_from_info(entry, &info)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use ox_core::integration::TerminalKind;

    use super::*;
    use crate::integration::process::HOLD_SCRIPT;
    use crate::test_support::file_entry;

    fn info(file_type: gio::FileType, content_type: &str, can_execute: bool) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_file_type(file_type);
        info.set_content_type(content_type);
        info.set_attribute_boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_EXECUTE, can_execute);
        info
    }

    /// parity: DND-026
    #[test]
    fn only_executable_programs_of_a_program_type_are_programs() {
        let tool = file_entry("convert");
        let binary = program_from_info(
            &tool,
            &info(gio::FileType::Regular, "application/x-executable", true),
        );
        let script = program_from_info(
            &tool,
            &info(gio::FileType::Regular, "application/x-shellscript", true),
        );
        let not_executable = program_from_info(
            &tool,
            &info(gio::FileType::Regular, "application/x-executable", false),
        );
        let folder = program_from_info(&tool, &info(gio::FileType::Directory, "inode/directory", true));
        // On FAT, NTFS and SMB mounts every file may be executed.
        let text = program_from_info(&tool, &info(gio::FileType::Regular, "text/plain", true));
        let photo = program_from_info(&tool, &info(gio::FileType::Regular, "image/jpeg", true));

        assert_eq!(binary.map(|program| program.kind), Some(ProgramKind::Binary));
        assert_eq!(script.map(|program| program.kind), Some(ProgramKind::Script));
        assert_eq!(not_executable, None);
        assert_eq!(folder, None);
        assert_eq!(text, None, "a text file is not a script");
        assert_eq!(photo, None, "a photo is not a program");
    }

    /// parity: DND-026
    #[test]
    fn a_binary_gets_each_path_as_one_argument_whatever_its_name() {
        let items = [
            OsString::from("/home/ada/My files/it's \"quoted\".txt"),
            OsString::from("/home/ada/a;b&c $(rm).txt"),
        ];

        let command = program_command(Path::new("/opt/tool/convert"), ProgramKind::Binary, &items, None);

        assert_eq!(
            command,
            [
                OsString::from("/opt/tool/convert"),
                items[0].clone(),
                items[1].clone(),
            ]
        );
    }

    /// parity: DND-026
    #[test]
    fn a_script_runs_in_the_terminal_through_the_fixed_hold_script() {
        let terminal = Terminal::new(
            PathBuf::from("/usr/bin/gnome-terminal"),
            TerminalKind::GnomeTerminal,
        )
        .expect("an absolute terminal path");
        let items = [OsString::from("/home/ada/Photo 1.jpg")];

        let command = program_command(
            Path::new("/home/ada/resize.sh"),
            ProgramKind::Script,
            &items,
            Some(&terminal),
        );

        let expected: Vec<OsString> = [
            "/usr/bin/gnome-terminal",
            "--",
            "/bin/sh",
            "-c",
            HOLD_SCRIPT,
            HOLD_SCRIPT_NAME,
            "/home/ada/resize.sh",
            "/home/ada/Photo 1.jpg",
        ]
        .map(OsString::from)
        .to_vec();
        assert_eq!(command, expected);
    }

    /// One program's place and whether it asks before running.
    struct ConfirmationCase {
        uri: &'static str,
        is_network: bool,
        is_removable: bool,
        asks: bool,
    }

    /// parity: DND-026
    #[test]
    fn only_programs_on_this_computers_own_disks_run_without_asking() {
        let cases = [
            ConfirmationCase {
                uri: "file:///home/ada/bin/convert",
                is_network: false,
                is_removable: false,
                asks: false,
            },
            ConfirmationCase {
                uri: "file:///media/ada/USB/convert",
                is_network: false,
                is_removable: true,
                asks: true,
            },
            ConfirmationCase {
                uri: "file:///mnt/nas/convert",
                is_network: true,
                is_removable: false,
                asks: true,
            },
            ConfirmationCase {
                uri: "smb://nas/tools/convert",
                is_network: true,
                is_removable: false,
                asks: true,
            },
        ];

        for case in cases {
            let asks = needs_run_confirmation(case.uri, case.is_network, case.is_removable);
            assert_eq!(asks, case.asks, "{}", case.uri);
        }
    }

    /// parity: DND-026
    #[test]
    fn the_hold_script_passes_names_through_as_arguments() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let odd_name = "it's \"odd\" $(touch pwned) ; name.txt";
        let mut shell = Command::new("/bin/sh")
            .args([
                "-c",
                HOLD_SCRIPT,
                HOLD_SCRIPT_NAME,
                "/usr/bin/printf",
                "%s\\n",
                odd_name,
            ])
            .current_dir(folder.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("sh runs");

        let mut enter = shell.stdin.take().expect("stdin is piped");
        enter.write_all(b"\n").expect("Enter reaches the script");
        drop(enter);
        let output = shell.wait_with_output().expect("the script ends");

        assert!(output.status.success());
        let printed = String::from_utf8(output.stdout).expect("text output");
        assert!(printed.starts_with(&format!("{odd_name}\n")), "{printed:?}");
        assert!(!folder.path().join("pwned").exists());
    }
}
