// SPDX-License-Identifier: AGPL-3.0-only
//! Which terminal emulator opens, and how it is told the folder.
//!
//! Ports `TERMINALS`, `SYSTEM_PATH`, `Terminal` and `find_terminal` in
//! `v2.0.0:desktop/terminal_integration.py` (OPEN-018, OPEN-020), with the
//! desktop's configured terminal first (OPEN-019). kitty, Ptyxis, Ghostty,
//! Alacritty, `WezTerm`, foot, Tilix and Terminator are native additions to
//! the ported list, and `xdg-terminal-exec` starts a configured terminal
//! the app does not know.

use std::fs;
use std::path::{Path, PathBuf};

use rustix::fs::Access;

use super::preference::PreferredTerminal;
use super::TerminalError;
use crate::integration::sandbox::Sandbox;

/// The only folders searched for a terminal.
///
/// Safety rule "only system-installed terminals" (`SYSTEM_PATH` in
/// `terminal_integration.py`): never the selected folder, a share, `$PATH`
/// or `$TERMINAL`, any of which could hold a program named like a
/// terminal.
pub const SYSTEM_PATH: [&str; 3] = ["/usr/bin", "/bin", "/usr/local/bin"];

/// Debian's alternative that points to the chosen terminal.
const DEBIAN_ALTERNATIVE: &str = "x-terminal-emulator";

/// The suffix of Debian's option-translating wrapper scripts.
const WRAPPER_SUFFIX: &str = ".wrapper";

/// How many symbolic links are followed before giving up.
const MAX_LINK_HOPS: usize = 40;

/// A terminal emulator the app knows how to start in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    /// GNOME Terminal (`gnome-terminal`), Zorin's terminal.
    GnomeTerminal,
    /// GNOME Console (`kgx`).
    Console,
    /// Ptyxis (`ptyxis`), GNOME's terminal on Fedora and Ubuntu.
    Ptyxis,
    /// Xfce Terminal (`xfce4-terminal`).
    XfceTerminal,
    /// Konsole (`konsole`).
    Konsole,
    /// kitty (`kitty`).
    Kitty,
    /// Ghostty (`ghostty`).
    Ghostty,
    /// Alacritty (`alacritty`).
    Alacritty,
    /// `WezTerm` (`wezterm`).
    WezTerm,
    /// foot (`foot`), a Wayland terminal.
    Foot,
    /// Tilix (`tilix`).
    Tilix,
    /// Terminator (`terminator`).
    Terminator,
    /// `XTerm` (`xterm`).
    XTerm,
    /// `XTerm` with Unicode (`uxterm`).
    UXTerm,
    /// `xdg-terminal-exec`, which starts the terminal the desktop's
    /// `xdg-terminals.list` names. Not in [`TerminalKind::ALL`]: it is
    /// used for a configured terminal the app does not know, and when no
    /// known terminal is installed.
    XdgTerminalExec,
}

/// How a terminal is told the program to run in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandStyle {
    /// The program and its arguments follow this option, each its own
    /// argument.
    After(&'static str),
    /// The program and its arguments are the terminal's trailing
    /// arguments.
    Trailing,
    /// The program and its arguments follow this option as one string,
    /// each quoted, which the terminal splits again with
    /// `g_shell_parse_argv` (Tilix joins several arguments itself, without
    /// quoting them all).
    QuotedAfter(&'static str),
    /// The options come first; then the program and its arguments, each
    /// quoted for `/bin/sh` and joined with spaces, are the value of the
    /// option ending in `=`, which the terminal runs with `/bin/sh -c`.
    ///
    /// Ghostty before 1.2 joins the arguments after `-e` with spaces and
    /// runs the result with `/bin/sh -c`, unquoted, while 1.2 and later
    /// run them as they are; its `--initial-command=` value without a
    /// `direct:` or `shell:` prefix is a `/bin/sh -c` command in both. The
    /// options stand in for what `-e` implies: a Ghostty of its own,
    /// never a running one, that quits with its window, and no shell
    /// integration rewriting the command.
    ShellCommand {
        /// Options that come before the command.
        options: &'static [&'static str],
        /// The option the quoted command is joined to.
        option: &'static str,
    },
}

impl TerminalKind {
    /// Every terminal, in the order they are preferred.
    pub const ALL: [Self; 14] = [
        Self::GnomeTerminal,
        Self::Console,
        Self::Ptyxis,
        Self::XfceTerminal,
        Self::Konsole,
        Self::Kitty,
        Self::Ghostty,
        Self::Alacritty,
        Self::WezTerm,
        Self::Foot,
        Self::Tilix,
        Self::Terminator,
        Self::XTerm,
        Self::UXTerm,
    ];

    /// The program's file name.
    pub fn program_name(self) -> &'static str {
        match self {
            Self::GnomeTerminal => "gnome-terminal",
            Self::Console => "kgx",
            Self::Ptyxis => "ptyxis",
            Self::XfceTerminal => "xfce4-terminal",
            Self::Konsole => "konsole",
            Self::Kitty => "kitty",
            Self::Ghostty => "ghostty",
            Self::Alacritty => "alacritty",
            Self::WezTerm => "wezterm",
            Self::Foot => "foot",
            Self::Tilix => "tilix",
            Self::Terminator => "terminator",
            Self::XTerm => "xterm",
            Self::UXTerm => "uxterm",
            Self::XdgTerminalExec => "xdg-terminal-exec",
        }
    }

    /// The name messages use.
    pub fn label(self) -> &'static str {
        match self {
            Self::GnomeTerminal => crate::i18n::gettext_static("GNOME Terminal"),
            Self::Console => crate::i18n::gettext_static("Console"),
            Self::Ptyxis => crate::i18n::gettext_static("Ptyxis"),
            Self::XfceTerminal => crate::i18n::gettext_static("Xfce Terminal"),
            Self::Konsole => crate::i18n::gettext_static("Konsole"),
            Self::Kitty => crate::i18n::gettext_static("kitty"),
            Self::Ghostty => crate::i18n::gettext_static("Ghostty"),
            Self::Alacritty => crate::i18n::gettext_static("Alacritty"),
            Self::WezTerm => crate::i18n::gettext_static("WezTerm"),
            Self::Foot => crate::i18n::gettext_static("foot"),
            Self::Tilix => crate::i18n::gettext_static("Tilix"),
            Self::Terminator => crate::i18n::gettext_static("Terminator"),
            Self::XTerm => crate::i18n::gettext_static("XTerm"),
            Self::UXTerm => crate::i18n::gettext_static("UXTerm"),
            // The terminal it starts is the desktop's choice.
            Self::XdgTerminalExec => crate::i18n::gettext_static("Terminal"),
        }
    }

    /// Options that come first: they make the terminal open a new window
    /// of its own, whatever already runs.
    ///
    /// `WezTerm` takes its `start` subcommand. Ptyxis ignores a starting
    /// folder unless asked for a new window. Terminator without D-Bus
    /// runs a program as given instead of handing it to a running
    /// Terminator, which would join its arguments into a shell command.
    pub fn leading_options(self) -> &'static [&'static str] {
        match self {
            Self::WezTerm => &["start"],
            Self::Ptyxis => &["--new-window"],
            Self::Terminator => &["--no-dbus"],
            _ => &[],
        }
    }

    /// The option that names the starting folder, or `None` for the
    /// `XTerm` family, which starts in its working directory. The folder
    /// is joined to an option ending in `=` and follows any other as the
    /// next argument, as in `xdg-terminal-exec`'s `X-TerminalArgDir`.
    pub fn working_directory_option(self) -> Option<&'static str> {
        match self {
            Self::GnomeTerminal
            | Self::Console
            | Self::Ptyxis
            | Self::XfceTerminal
            | Self::Ghostty
            | Self::Alacritty
            | Self::Foot
            | Self::Tilix
            | Self::Terminator => Some("--working-directory="),
            Self::Konsole => Some("--workdir="),
            Self::Kitty => Some("--directory="),
            Self::WezTerm => Some("--cwd"),
            Self::XdgTerminalExec => Some("--dir="),
            Self::XTerm | Self::UXTerm => None,
        }
    }

    /// How the terminal is told the program to run in it.
    pub(super) fn command_style(self) -> CommandStyle {
        match self {
            Self::GnomeTerminal | Self::Console | Self::Ptyxis | Self::WezTerm | Self::XdgTerminalExec => {
                CommandStyle::After("--")
            }
            Self::XfceTerminal | Self::Terminator => CommandStyle::After("-x"),
            Self::Konsole | Self::Alacritty | Self::XTerm | Self::UXTerm => CommandStyle::After("-e"),
            Self::Kitty | Self::Foot => CommandStyle::Trailing,
            Self::Tilix => CommandStyle::QuotedAfter("-e"),
            Self::Ghostty => CommandStyle::ShellCommand {
                options: &[
                    "--gtk-single-instance=false",
                    "--quit-after-last-window-closed=true",
                    "--shell-integration=none",
                ],
                option: "--initial-command=",
            },
        }
    }

    /// The terminal whose program is named `name`, if it is a known one.
    pub fn from_program_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.program_name() == name)
    }
}

/// An installed terminal: its program and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    executable: PathBuf,
    kind: TerminalKind,
}

impl Terminal {
    /// The terminal of `kind` installed at `executable`.
    ///
    /// # Errors
    ///
    /// [`TerminalError::UnsupportedTerminal`] unless `executable` is an
    /// absolute path, so no search path is ever consulted.
    pub fn new(executable: PathBuf, kind: TerminalKind) -> Result<Self, TerminalError> {
        if !executable.is_absolute() {
            return Err(TerminalError::UnsupportedTerminal);
        }
        Ok(Self { executable, kind })
    }

    /// The program, as a path on the host.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Which terminal it is.
    pub fn kind(&self) -> TerminalKind {
        self.kind
    }

    /// The name messages use.
    pub fn label(&self) -> &'static str {
        self.kind.label()
    }
}

/// Where the host's programs are looked for: the [`SYSTEM_PATH`] folders
/// under a root, which is `/` on the host and `/run/host` inside Flatpak.
/// Symbolic links are followed inside the root, so an absolute link such
/// as Debian's alternatives resolves to the host's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableSearch {
    root: PathBuf,
    /// The terminals the desktop is configured to use, tried first.
    preferred: Vec<PreferredTerminal>,
}

impl ExecutableSearch {
    /// The host's programs as seen from `sandbox`.
    pub fn for_sandbox(sandbox: Sandbox) -> Self {
        Self::under(&sandbox.host_root())
    }

    /// The programs of a system whose `/` is at `root`.
    pub fn under(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            preferred: Vec::new(),
        }
    }

    /// Tries `preferred`, the desktop's configured terminals in order
    /// (see [`super::desktop_terminals`]), before the Debian alternative and
    /// the preference order.
    #[must_use]
    pub fn preferring(mut self, preferred: Vec<PreferredTerminal>) -> Self {
        self.preferred = preferred;
        self
    }

    /// The host path of the first executable named `name` in
    /// [`SYSTEM_PATH`], like `shutil.which(name, path=SYSTEM_PATH)`.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        SYSTEM_PATH
            .iter()
            .map(|folder| Path::new(folder).join(name))
            .find(|candidate| self.is_executable(candidate))
    }

    /// True if `host_path` leads to an executable regular file.
    fn is_executable(&self, host_path: &Path) -> bool {
        let Some(target) = self.resolve(host_path) else {
            return false;
        };
        let file = self.on_this_system(&target);
        let is_file = fs::metadata(&file).is_ok_and(|metadata| metadata.is_file());
        is_file && rustix::fs::access(&file, Access::EXEC_OK).is_ok()
    }

    /// The host path `host_path` finally leads to, following symbolic
    /// links inside the root; `None` for a link loop.
    fn resolve(&self, host_path: &Path) -> Option<PathBuf> {
        let mut current = host_path.to_owned();
        for _ in 0..MAX_LINK_HOPS {
            let Ok(target) = fs::read_link(self.on_this_system(&current)) else {
                return Some(current);
            };
            // Joining an absolute target replaces the path, so both
            // absolute and relative links resolve against the host.
            current = current.parent()?.join(target);
        }
        None
    }

    /// Where the host's `host_path` is on this system.
    fn on_this_system(&self, host_path: &Path) -> PathBuf {
        let relative = host_path.strip_prefix("/").unwrap_or(host_path);
        self.root.join(relative)
    }
}

/// The terminal to open (OPEN-018, OPEN-019): the desktop's configured
/// terminal when it is installed, then Debian's `x-terminal-emulator`
/// alternative when it points to a known terminal, otherwise the first
/// installed of [`TerminalKind::ALL`], and last `xdg-terminal-exec`.
///
/// The configured terminals are tried in order. One the app knows and
/// finds in the system folders is started directly. At the first
/// `xdg-terminals.list` entry it cannot start itself, because it does not
/// know the terminal or does not find it, `xdg-terminal-exec` takes over
/// when it is installed: it reads the same lists and starts the first
/// terminal of them that is installed. Every program, `xdg-terminal-exec`
/// included, is looked for in [`SYSTEM_PATH`] only; `$PATH` and
/// `$TERMINAL` are never read.
///
/// # Errors
///
/// [`TerminalError::NoTerminal`] when no known terminal is installed.
pub fn find_terminal(search: &ExecutableSearch) -> Result<Terminal, TerminalError> {
    let installed = |kind: TerminalKind| {
        let executable = search.find(kind.program_name())?;
        Some(Terminal { executable, kind })
    };
    for preferred in &search.preferred {
        let (kind, listed) = match *preferred {
            PreferredTerminal::Listed(kind) => (kind, true),
            PreferredTerminal::Gnome(kind) => (Some(kind), false),
        };
        if let Some(terminal) = kind.and_then(installed) {
            return Ok(terminal);
        }
        if listed {
            if let Some(terminal) = installed(TerminalKind::XdgTerminalExec) {
                return Ok(terminal);
            }
        }
    }
    if let Some(terminal) = debian_alternative(search) {
        return Ok(terminal);
    }
    TerminalKind::ALL
        .into_iter()
        .chain([TerminalKind::XdgTerminalExec])
        .find_map(installed)
        .ok_or(TerminalError::NoTerminal)
}

/// The terminal Debian's alternative points to, if it is a known one.
fn debian_alternative(search: &ExecutableSearch) -> Option<Terminal> {
    let alternative = search.find(DEBIAN_ALTERNATIVE)?;
    let target_name = file_name(&search.resolve(&alternative)?)?;
    // gnome-terminal.wrapper is the Debian alternative's option
    // translator; call the real program so --working-directory cannot be
    // misinterpreted.
    let program_name = target_name.strip_suffix(WRAPPER_SUFFIX).unwrap_or(&target_name);
    let kind = TerminalKind::from_program_name(program_name)?;
    let executable = search.find(kind.program_name())?;
    Some(Terminal { executable, kind })
}

/// The final component of `path` as text.
fn file_name(path: &Path) -> Option<String> {
    path.file_name()?.to_str().map(str::to_owned)
}
