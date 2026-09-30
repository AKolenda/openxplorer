// SPDX-License-Identifier: AGPL-3.0-only
//! Which terminal emulator opens, and how it is told the folder.
//!
//! Ports `TERMINALS`, `SYSTEM_PATH`, `Terminal` and `find_terminal` in
//! `desktop/terminal_integration.py` (OPEN-018, OPEN-020).

use std::fs;
use std::path::{Path, PathBuf};

use rustix::fs::Access;

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
    /// Xfce Terminal (`xfce4-terminal`).
    XfceTerminal,
    /// Konsole (`konsole`).
    Konsole,
    /// `XTerm` (`xterm`).
    XTerm,
    /// `XTerm` with Unicode (`uxterm`).
    UXTerm,
}

impl TerminalKind {
    /// Every terminal, in the order they are preferred.
    pub const ALL: [Self; 6] = [
        Self::GnomeTerminal,
        Self::Console,
        Self::XfceTerminal,
        Self::Konsole,
        Self::XTerm,
        Self::UXTerm,
    ];

    /// The program's file name.
    pub fn program_name(self) -> &'static str {
        match self {
            Self::GnomeTerminal => "gnome-terminal",
            Self::Console => "kgx",
            Self::XfceTerminal => "xfce4-terminal",
            Self::Konsole => "konsole",
            Self::XTerm => "xterm",
            Self::UXTerm => "uxterm",
        }
    }

    /// The name messages use.
    pub fn label(self) -> &'static str {
        match self {
            Self::GnomeTerminal => "GNOME Terminal",
            Self::Console => "Console",
            Self::XfceTerminal => "Xfce Terminal",
            Self::Konsole => "Konsole",
            Self::XTerm => "XTerm",
            Self::UXTerm => "UXTerm",
        }
    }

    /// The option that names the starting folder, or `None` for the
    /// `XTerm` family, which starts in its working directory.
    pub fn working_directory_option(self) -> Option<&'static str> {
        match self {
            Self::GnomeTerminal | Self::Console | Self::XfceTerminal => Some("--working-directory="),
            Self::Konsole => Some("--workdir="),
            Self::XTerm | Self::UXTerm => None,
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
        }
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

/// The terminal to open (OPEN-018): Debian's `x-terminal-emulator`
/// alternative when it points to a known terminal, otherwise the first
/// installed of [`TerminalKind::ALL`]. `$PATH` and `$TERMINAL` are never
/// read.
///
/// # Errors
///
/// [`TerminalError::NoTerminal`] when no known terminal is installed.
pub fn find_terminal(search: &ExecutableSearch) -> Result<Terminal, TerminalError> {
    if let Some(terminal) = debian_alternative(search) {
        return Ok(terminal);
    }
    TerminalKind::ALL
        .into_iter()
        .find_map(|kind| {
            let executable = search.find(kind.program_name())?;
            Some(Terminal { executable, kind })
        })
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
