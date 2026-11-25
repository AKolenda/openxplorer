// SPDX-License-Identifier: AGPL-3.0-only
//! The terminal the desktop is configured to use (OPEN-019), as Dolphin
//! follows KDE's configured terminal.
//!
//! Two settings name one: the `xdg-terminals.list` files of
//! `xdg-terminal-exec` (a desktop's own list, such as
//! `gnome-xdg-terminals.list`, before the shared one, in the user's
//! configuration folder before the system's, and the distribution's lists
//! in the data folders last), and GNOME's
//! `org.gnome.desktop.default-applications.terminal` `exec` key when the
//! user set it, which comes before the distribution's lists, since those
//! are defaults the user did not choose. [`super::find_terminal`] starts a listed terminal the app
//! knows itself, and hands one it does not know to `xdg-terminal-exec`;
//! either is looked for in the system folders only.

use std::fs;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::TerminalKind;
use crate::integration::sandbox::Sandbox;

/// The file `xdg-terminal-exec` reads the preferred terminals from.
const TERMINALS_LIST: &str = "xdg-terminals.list";

/// The subfolder of each system data folder that holds the
/// distribution's lists.
const DATA_SUBFOLDER: &str = "xdg-terminal-exec";

/// The variable Flatpak sets to the host's `XDG_CONFIG_HOME`, when the
/// host sets that.
const HOST_CONFIG_HOME_VARIABLE: &str = "HOST_XDG_CONFIG_HOME";

/// GNOME's terminal setting.
const GNOME_TERMINAL_SCHEMA: &str = "org.gnome.desktop.default-applications.terminal";

/// The largest list read; a real one names a few desktop entries.
const LIST_SIZE_LIMIT: u64 = 64 * 1024;

/// Where the desktop's terminal settings are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTerminalConfig {
    /// The configuration folders, the user's first.
    pub config_dirs: Vec<PathBuf>,
    /// The system data folders, most preferred first; the lists in their
    /// `xdg-terminal-exec` subfolders come after every configuration
    /// folder's.
    pub data_dirs: Vec<PathBuf>,
    /// The desktops of `XDG_CURRENT_DESKTOP`, such as `zorin` and `gnome`.
    pub desktops: Vec<String>,
}

impl DesktopTerminalConfig {
    /// The configuration of this session, as the host's
    /// `xdg-terminal-exec` reads it.
    ///
    /// Inside Flatpak the XDG folders are the sandbox's own, so the host's
    /// are read where `--filesystem=host` shows them: its configuration
    /// home (`HOST_XDG_CONFIG_HOME`, otherwise `~/.config`) at the same
    /// path, and `/etc/xdg` and the default data folders under
    /// `/run/host`. The host's `XDG_CONFIG_DIRS` and `XDG_DATA_DIRS` do not
    /// reach the sandbox, so their defaults stand in; `XDG_CURRENT_DESKTOP`
    /// does.
    pub fn of_session(sandbox: Sandbox) -> Self {
        let (config_dirs, data_dirs) = match sandbox {
            Sandbox::Host => {
                let mut config_dirs = vec![glib::user_config_dir()];
                config_dirs.extend(glib::system_config_dirs());
                (config_dirs, glib::system_data_dirs())
            }
            Sandbox::Flatpak => {
                let root = sandbox.host_root();
                let user = std::env::var_os(HOST_CONFIG_HOME_VARIABLE)
                    .map(PathBuf::from)
                    .filter(|folder| folder.is_absolute())
                    .unwrap_or_else(|| glib::home_dir().join(".config"));
                (
                    vec![user, root.join("etc/xdg")],
                    vec![root.join("usr/local/share"), root.join("usr/share")],
                )
            }
        };
        let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self {
            config_dirs,
            data_dirs,
            desktops: desktops
                .split(':')
                .filter(|desktop| !desktop.is_empty())
                .map(str::to_lowercase)
                .collect(),
        }
    }
}

/// A terminal the desktop is configured to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferredTerminal {
    /// An entry of an `xdg-terminals.list`, which `xdg-terminal-exec` can
    /// start: the terminal it names, or `None` for one the app does not
    /// know.
    Listed(Option<TerminalKind>),
    /// GNOME's `exec` key, naming a terminal the app knows.
    Gnome(TerminalKind),
}

/// The terminals the desktop is configured to use, most preferred first:
/// the entries of the `xdg-terminals.list` files in the configuration
/// folders, in the order `xdg-terminal-exec` tries them, then GNOME's
/// choice when the user set it, then the entries of the distribution's
/// lists in the data folders.
pub fn desktop_terminals(config: &DesktopTerminalConfig) -> Vec<PreferredTerminal> {
    ordered_terminals(config, gnome_terminal())
}

/// [`desktop_terminals`], with `gnome` as GNOME's choice.
fn ordered_terminals(config: &DesktopTerminalConfig, gnome: Option<TerminalKind>) -> Vec<PreferredTerminal> {
    let data_folders: Vec<PathBuf> = config
        .data_dirs
        .iter()
        .map(|folder| folder.join(DATA_SUBFOLDER))
        .collect();
    let listed = |folders: &[PathBuf]| {
        listed_entries(config, folders)
            .into_iter()
            .map(|entry| PreferredTerminal::Listed(kind_of_desktop_entry(&entry)))
    };
    listed(&config.config_dirs)
        .chain(gnome.map(PreferredTerminal::Gnome))
        .chain(listed(&data_folders))
        .collect()
}

/// The desktop entry IDs of the `xdg-terminals.list` files in `folders`,
/// in the order `xdg-terminal-exec` tries them: folder by folder, and in
/// each folder the desktops' own lists before the shared one.
fn listed_entries(config: &DesktopTerminalConfig, folders: &[PathBuf]) -> Vec<String> {
    let names = config
        .desktops
        .iter()
        .map(|desktop| format!("{desktop}-{TERMINALS_LIST}"))
        .chain(std::iter::once(TERMINALS_LIST.to_owned()));
    let names: Vec<String> = names.collect();
    folders
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .filter_map(|path| read_list(&path))
        .flatten()
        .collect()
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
        // A leading - excludes an entry from the fallback search and a
        // leading + protects it from that exclusion; neither prefers it.
        // `/` starts a directive, such as `/execarg_default:…`, and `:` an
        // action, such as `org.gnome.Terminal.desktop:new-window`.
        .filter(|line| !line.is_empty() && !line.starts_with(['#', '-', '+', '/']))
        .map(|line| line.split(':').next().unwrap_or(line).to_owned())
        .filter(|entry| !entry.is_empty());
    Some(entries.collect())
}

/// The terminal a desktop entry ID names, such as
/// `org.gnome.Terminal.desktop`.
fn kind_of_desktop_entry(entry: &str) -> Option<TerminalKind> {
    let id = entry.strip_suffix(".desktop").unwrap_or(entry);
    match id {
        "org.gnome.Terminal" | "gnome-terminal" => Some(TerminalKind::GnomeTerminal),
        "org.gnome.Console" | "kgx" => Some(TerminalKind::Console),
        "org.gnome.Ptyxis" | "ptyxis" => Some(TerminalKind::Ptyxis),
        "xfce4-terminal" | "org.xfce.terminal" => Some(TerminalKind::XfceTerminal),
        "org.kde.konsole" | "konsole" => Some(TerminalKind::Konsole),
        "kitty" => Some(TerminalKind::Kitty),
        "com.mitchellh.ghostty" | "ghostty" => Some(TerminalKind::Ghostty),
        "Alacritty" | "alacritty" | "org.alacritty.Alacritty" => Some(TerminalKind::Alacritty),
        "org.wezfurlong.wezterm" | "wezterm" => Some(TerminalKind::WezTerm),
        "org.codeberg.dnkl.foot" | "foot" => Some(TerminalKind::Foot),
        "com.gexperts.Tilix" | "tilix" => Some(TerminalKind::Tilix),
        "terminator" => Some(TerminalKind::Terminator),
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
            data_dirs: vec![folder.join("local-data"), folder.join("data")],
            desktops: desktops.iter().map(|desktop| (*desktop).to_owned()).collect(),
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
        fs::write(path, text).expect("list");
    }

    fn listed(config: &DesktopTerminalConfig) -> Vec<Option<TerminalKind>> {
        ordered_terminals(config, None)
            .into_iter()
            .map(|terminal| match terminal {
                PreferredTerminal::Listed(kind) => kind,
                PreferredTerminal::Gnome(kind) => panic!("no GNOME choice was given: {kind:?}"),
            })
            .collect()
    }

    /// parity: OPEN-019
    #[test]
    fn the_desktops_own_list_then_the_shared_one_names_the_terminals_in_order() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        write(
            &folder.path().join("system/xdg-terminals.list"),
            "# shared\norg.gnome.Terminal.desktop\n",
        );
        write(
            &folder.path().join("user/gnome-xdg-terminals.list"),
            "-org.kde.konsole.desktop\n/execarg_default:foot.desktop:--\nrio.desktop\n\
             +org.gnome.Console.desktop:new-window\n",
        );

        let gnome = config(folder.path(), &["zorin", "gnome"]);
        let other = config(folder.path(), &["kde"]);

        assert_eq!(
            listed(&gnome),
            [None, Some(TerminalKind::GnomeTerminal)],
            "excluded and protected entries and directives are skipped; an unknown entry keeps its place"
        );
        assert_eq!(listed(&other), [Some(TerminalKind::GnomeTerminal)]);
        assert!(listed(&config(&folder.path().join("none"), &[])).is_empty());

        write(&folder.path().join("user/xdg-terminals.list"), "kitty.desktop\n");
        assert_eq!(
            listed(&other),
            [Some(TerminalKind::Kitty), Some(TerminalKind::GnomeTerminal)],
            "the user's folder comes before the system's"
        );
    }

    /// The distribution's lists in the data folders' `xdg-terminal-exec`
    /// subfolders come after every configuration folder, in the same
    /// order: folder by folder, the desktop's own list first.
    ///
    /// parity: OPEN-019
    #[test]
    fn the_distributions_lists_in_the_data_folders_come_last() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        write(
            &folder.path().join("data/xdg-terminal-exec/xdg-terminals.list"),
            "xterm.desktop\n",
        );
        write(
            &folder
                .path()
                .join("data/xdg-terminal-exec/gnome-xdg-terminals.list"),
            "org.gnome.Ptyxis.desktop\n",
        );
        write(
            &folder
                .path()
                .join("local-data/xdg-terminal-exec/xdg-terminals.list"),
            "foot.desktop\n",
        );
        write(&folder.path().join("data/xdg-terminals.list"), "kitty.desktop\n");
        write(
            &folder.path().join("system/xdg-terminals.list"),
            "org.gnome.Terminal.desktop\n",
        );

        assert_eq!(
            listed(&config(folder.path(), &["gnome"])),
            [
                Some(TerminalKind::GnomeTerminal),
                Some(TerminalKind::Foot),
                Some(TerminalKind::Ptyxis),
                Some(TerminalKind::XTerm),
            ],
            "a list straight in a data folder is not read"
        );
    }

    /// The terminal the user set in GNOME comes after the lists in the
    /// configuration folders and before the distribution's lists in the
    /// data folders, which are defaults the user did not choose.
    ///
    /// parity: OPEN-019
    #[test]
    fn the_users_gnome_choice_comes_before_the_distributions_lists() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        write(
            &folder.path().join("data/xdg-terminal-exec/xdg-terminals.list"),
            "org.gnome.Ptyxis.desktop\n",
        );
        let config = config(folder.path(), &["gnome"]);

        assert_eq!(
            ordered_terminals(&config, Some(TerminalKind::Alacritty)),
            [
                PreferredTerminal::Gnome(TerminalKind::Alacritty),
                PreferredTerminal::Listed(Some(TerminalKind::Ptyxis)),
            ]
        );

        write(
            &folder.path().join("system/xdg-terminals.list"),
            "kitty.desktop\n",
        );
        assert_eq!(
            ordered_terminals(&config, Some(TerminalKind::Alacritty)),
            [
                PreferredTerminal::Listed(Some(TerminalKind::Kitty)),
                PreferredTerminal::Gnome(TerminalKind::Alacritty),
                PreferredTerminal::Listed(Some(TerminalKind::Ptyxis)),
            ],
            "a configuration folder's list still comes first"
        );
    }

    /// The desktop entry IDs of the terminals the app knows, as their
    /// packages install them.
    ///
    /// parity: OPEN-019
    #[test]
    fn modern_terminals_are_known_by_their_desktop_entry_ids() {
        let cases = [
            ("com.mitchellh.ghostty.desktop", TerminalKind::Ghostty),
            ("org.gnome.Ptyxis.desktop", TerminalKind::Ptyxis),
            ("Alacritty.desktop", TerminalKind::Alacritty),
            ("org.wezfurlong.wezterm.desktop", TerminalKind::WezTerm),
            ("foot.desktop", TerminalKind::Foot),
            ("org.codeberg.dnkl.foot.desktop", TerminalKind::Foot),
            ("com.gexperts.Tilix.desktop", TerminalKind::Tilix),
            ("terminator.desktop", TerminalKind::Terminator),
            ("kitty.desktop", TerminalKind::Kitty),
        ];

        for (entry, kind) in cases {
            assert_eq!(kind_of_desktop_entry(entry), Some(kind), "{entry}");
        }
        // footclient needs a running foot server, so xdg-terminal-exec
        // starts it.
        assert_eq!(kind_of_desktop_entry("footclient.desktop"), None);
    }
}
