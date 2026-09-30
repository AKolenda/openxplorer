// SPDX-License-Identifier: AGPL-3.0-only
//! The terminal the desktop is configured to use (OPEN-019), as Dolphin
//! follows KDE's configured terminal.
//!
//! Two settings name one: the `xdg-terminals.list` files of
//! `xdg-terminal-exec` (a desktop's own list, such as
//! `gnome-xdg-terminals.list`, before the shared one, in the user's
//! configuration folder before the system's), and GNOME's
//! `org.gnome.desktop.default-applications.terminal` `exec` key when the
//! user set it. Only a terminal the app knows is taken, and
//! [`super::find_terminal`] still looks for it in the system folders only.

use std::fs;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::TerminalKind;

/// The file `xdg-terminal-exec` reads the preferred terminals from.
const TERMINALS_LIST: &str = "xdg-terminals.list";

/// GNOME's terminal setting.
const GNOME_TERMINAL_SCHEMA: &str = "org.gnome.desktop.default-applications.terminal";

/// The largest list read; a real one names a few desktop entries.
const LIST_SIZE_LIMIT: u64 = 64 * 1024;

/// Where the desktop's terminal settings are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTerminalConfig {
    /// The configuration folders, the user's first.
    pub config_dirs: Vec<PathBuf>,
    /// The desktops of `XDG_CURRENT_DESKTOP`, such as `zorin` and `gnome`.
    pub desktops: Vec<String>,
}

impl DesktopTerminalConfig {
    /// The configuration of this session.
    pub fn of_session() -> Self {
        let mut config_dirs = vec![glib::user_config_dir()];
        config_dirs.extend(glib::system_config_dirs());
        let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self {
            config_dirs,
            desktops: desktops
                .split(':')
                .filter(|desktop| !desktop.is_empty())
                .map(str::to_lowercase)
                .collect(),
        }
    }
}

/// The known terminal the desktop is configured to use, if any: the
/// `xdg-terminals.list` choice first, then GNOME's.
pub fn desktop_terminal(config: &DesktopTerminalConfig) -> Option<TerminalKind> {
    listed_terminal(config).or_else(gnome_terminal)
}

/// The first known terminal of the first `xdg-terminals.list` found.
fn listed_terminal(config: &DesktopTerminalConfig) -> Option<TerminalKind> {
    let names = config
        .desktops
        .iter()
        .map(|desktop| format!("{desktop}-{TERMINALS_LIST}"))
        .chain(std::iter::once(TERMINALS_LIST.to_owned()));
    let names: Vec<String> = names.collect();
    config
        .config_dirs
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find_map(|path| read_list(&path))
        .and_then(|entries| entries.iter().find_map(|entry| kind_of_desktop_entry(entry)))
}

/// The entries of the list at `path`, or `None` when there is none.
fn read_list(path: &Path) -> Option<Vec<String>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > LIST_SIZE_LIMIT {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let entries = text
        .lines()
        .map(str::trim)
        // A leading + or - marks an entry as added or excluded; `/` starts
        // an action, such as `org.gnome.Terminal.desktop:new-window`.
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('-'))
        .map(|line| line.trim_start_matches('+'))
        .map(|line| line.split(':').next().unwrap_or(line).to_owned());
    Some(entries.collect())
}

/// The terminal a desktop entry ID names, such as
/// `org.gnome.Terminal.desktop`.
fn kind_of_desktop_entry(entry: &str) -> Option<TerminalKind> {
    let id = entry.strip_suffix(".desktop").unwrap_or(entry);
    match id {
        "org.gnome.Terminal" | "gnome-terminal" => Some(TerminalKind::GnomeTerminal),
        "org.gnome.Console" | "kgx" => Some(TerminalKind::Console),
        "xfce4-terminal" | "org.xfce.terminal" => Some(TerminalKind::XfceTerminal),
        "org.kde.konsole" | "konsole" => Some(TerminalKind::Konsole),
        "xterm" | "debian-xterm" => Some(TerminalKind::XTerm),
        "uxterm" | "debian-uxterm" => Some(TerminalKind::UXTerm),
        _ => None,
    }
}

/// The terminal of GNOME's `exec` key, when the user set it.
fn gnome_terminal() -> Option<TerminalKind> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(GNOME_TERMINAL_SCHEMA, true)?;
    if !schema.has_key("exec") {
        return None;
    }
    let settings = gio::Settings::new(GNOME_TERMINAL_SCHEMA);
    let program = settings.user_value("exec")?.get::<String>()?;
    let name = Path::new(&program).file_name()?.to_str()?;
    TerminalKind::from_program_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(folder: &Path, desktops: &[&str]) -> DesktopTerminalConfig {
        DesktopTerminalConfig {
            config_dirs: vec![folder.join("user"), folder.join("system")],
            desktops: desktops.iter().map(|desktop| (*desktop).to_owned()).collect(),
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
        fs::write(path, text).expect("list");
    }

    /// parity: OPEN-019
    #[test]
    fn the_desktops_own_list_then_the_shared_one_names_the_terminal() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        write(
            &folder.path().join("system/xdg-terminals.list"),
            "# shared\norg.gnome.Terminal.desktop\n",
        );
        write(
            &folder.path().join("user/gnome-xdg-terminals.list"),
            "-org.kde.konsole.desktop\nkitty.desktop\n+org.gnome.Console.desktop:new-window\n",
        );

        let gnome = config(folder.path(), &["zorin", "gnome"]);
        let other = config(folder.path(), &["kde"]);

        assert_eq!(listed_terminal(&gnome), Some(TerminalKind::Console));
        assert_eq!(listed_terminal(&other), Some(TerminalKind::GnomeTerminal));
        assert_eq!(listed_terminal(&config(&folder.path().join("none"), &[])), None);
    }
}
