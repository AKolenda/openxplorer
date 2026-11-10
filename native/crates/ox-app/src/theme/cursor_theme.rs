// SPDX-License-Identifier: AGPL-3.0-only
//! Keeps the pointer's resize, text and hand cursors on Wayland when the
//! desktop names a cursor theme that is not installed.
//!
//! A compositor without the cursor-shape protocol, such as Mutter 46, has
//! GTK 4.14 draw every cursor itself, from the theme in
//! `gtk-cursor-theme-name` (GNOME's `cursor-theme`). GTK looks for that
//! theme's `cursors` folder only in `icons` of the user's data folder, in
//! `~/.icons` and in `icons` of each system data folder, and last in
//! `/usr/share/icons/default/cursors` (`get_cursor_theme` in
//! gdk/wayland/gdkdisplay-wayland.c). Unlike the compositor and other
//! toolkits it never follows a theme's `Inherits`. So when the named theme
//! is missing and `default` only names another theme in its
//! `index.theme`, as on Ubuntu and Zorin, no named cursor loads and GTK
//! shows its built-in arrow everywhere: over the window's edges, text and
//! links alike, while the rest of the desktop looks right.
//!
//! [`follow`] checks the theme GTK was given when the app starts and
//! whenever it changes, with a few `stat` calls. A theme GTK can load, or a
//! `/usr/share/icons/default/cursors` folder, is left alone. Otherwise the
//! app's own GTK settings name the first theme with cursors among those
//! the missing theme's `index.theme` inherits, those `default` inherits,
//! and Adwaita, as the desktop itself would resolve them. GNOME's setting
//! is never written, and a new choice there is followed again.

use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

/// The GTK setting that names the cursor theme.
const SETTING: &str = "gtk-cursor-theme-name";

/// The cursors GTK on Wayland loads when the named theme has none.
const GTK_LAST_RESORT: &str = "/usr/share/icons/default/cursors";

/// The theme of last resort, GTK's own default for the setting.
const ADWAITA: &str = "Adwaita";

/// The most themes followed through `Inherits`, against loops.
const MOST_THEMES: usize = 32;

/// Makes sure GTK on `display` draws cursors from an installed theme, now
/// and whenever the desktop names another. Only Wayland needs it: on X11
/// libXcursor follows `Inherits` itself.
pub(crate) fn follow(display: &gdk::Display) {
    if !display.backend().is_wayland() {
        return;
    }
    let settings = gtk::Settings::for_display(display);
    let replaced = Rc::new(Cell::new(false));
    repair(&settings, &replaced);
    settings.connect_notify_local(Some(SETTING), {
        let replaced = Rc::clone(&replaced);
        move |settings, _| repair(settings, &replaced)
    });
    // A theme the app set hides the desktop's later choices from GTK's
    // settings, so a new choice there is taken up again.
    display.connect_setting_changed(move |_, name| {
        if name == SETTING && replaced.replace(false) {
            settings.reset_property(SETTING);
        }
    });
}

/// Names an installed theme in `settings` when the one named there has
/// no cursors GTK can load.
fn repair(settings: &gtk::Settings, replaced: &Cell<bool>) {
    let Some(name) = settings.gtk_cursor_theme_name() else {
        return;
    };
    if let Some(theme) = replacement(&name, &theme_folders(), Path::new(GTK_LAST_RESORT)) {
        glib::g_info!(
            ox_core::LOG_DOMAIN,
            "The cursor theme {name} has no cursors GTK can load; using {theme}"
        );
        replaced.set(true);
        settings.set_gtk_cursor_theme_name(Some(&theme));
    }
}

/// The folders GTK 4.14 on Wayland looks for cursor themes in, in its
/// order.
fn theme_folders() -> Vec<PathBuf> {
    let mut folders = vec![
        glib::user_data_dir().join("icons"),
        glib::home_dir().join(".icons"),
    ];
    folders.extend(
        glib::system_data_dirs()
            .into_iter()
            .map(|folder| folder.join("icons")),
    );
    folders
}

/// The theme to draw cursors from instead of `name`, looking for themes in
/// `folders`: `None` when GTK loads `name`'s cursors or those in
/// `last_resort`, or when no theme with cursors is found.
pub(crate) fn replacement(name: &str, folders: &[PathBuf], last_resort: &Path) -> Option<String> {
    if has_cursors(name, folders) || last_resort.is_dir() {
        return None;
    }
    let mut candidates = inherited(name, folders);
    candidates.push("default".to_owned());
    candidates.extend(inherited("default", folders));
    candidates.push(ADWAITA.to_owned());
    candidates
        .into_iter()
        .find(|theme| theme != name && has_cursors(theme, folders))
}

/// Whether a theme called `name` in one of `folders` has cursors.
fn has_cursors(name: &str, folders: &[PathBuf]) -> bool {
    is_theme_name(name)
        && folders
            .iter()
            .any(|folder| folder.join(name).join("cursors").is_dir())
}

/// Whether `name` names a theme folder, not a path.
fn is_theme_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
}

/// The themes `name` inherits, nearest first, each once: the `Inherits`
/// of its `index.theme`, then theirs.
fn inherited(name: &str, folders: &[PathBuf]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut queue = VecDeque::from([name.to_owned()]);
    while let Some(theme) = queue.pop_front() {
        for parent in parents(&theme, folders) {
            if parent != name && !found.contains(&parent) && found.len() < MOST_THEMES {
                found.push(parent.clone());
                queue.push_back(parent);
            }
        }
    }
    found
}

/// The themes the first `index.theme` of `theme` in `folders` inherits.
fn parents(theme: &str, folders: &[PathBuf]) -> Vec<String> {
    if !is_theme_name(theme) {
        return Vec::new();
    }
    folders
        .iter()
        .find_map(|folder| std::fs::read_to_string(folder.join(theme).join("index.theme")).ok())
        .map(|index| inherits(&index))
        .unwrap_or_default()
}

/// The `Inherits` list of the `[Icon Theme]` group of an `index.theme`.
fn inherits(index: &str) -> Vec<String> {
    let mut in_theme_group = false;
    for line in index.lines().map(str::trim) {
        if line.starts_with('[') {
            in_theme_group = line == "[Icon Theme]";
        } else if in_theme_group {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "Inherits" {
                    return value
                        .split(',')
                        .map(str::trim)
                        .filter(|theme| is_theme_name(theme))
                        .map(str::to_owned)
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Theme folders in a temporary folder, as a system's `icons`.
    struct Icons {
        root: tempfile::TempDir,
    }

    impl Icons {
        fn new() -> Self {
            Self {
                root: tempfile::tempdir().expect("a temporary folder"),
            }
        }

        fn folders(&self) -> Vec<PathBuf> {
            vec![self.root.path().join("user"), self.root.path().join("system")]
        }

        /// A theme in the system folder, with cursors or not, inheriting
        /// `inherits` when given.
        fn theme(&self, name: &str, cursors: bool, inherits: Option<&str>) -> &Self {
            let folder = self.root.path().join("system").join(name);
            std::fs::create_dir_all(&folder).expect("a theme folder");
            if cursors {
                std::fs::create_dir_all(folder.join("cursors")).expect("a cursors folder");
            }
            if let Some(inherits) = inherits {
                let index =
                    format!("[Icon Theme]\nName={name}\nInherits={inherits}\n\n[16x16/apps]\nSize=16\n");
                std::fs::write(folder.join("index.theme"), index).expect("an index.theme");
            }
            self
        }

        /// The cursors folder GTK falls back to, absent.
        fn no_last_resort(&self) -> PathBuf {
            self.root.path().join("default-cursors")
        }

        fn replacement(&self, name: &str) -> Option<String> {
            replacement(name, &self.folders(), &self.no_last_resort())
        }
    }

    /// The owner's desktop: GNOME names DMZ-Black, which is not
    /// installed, and `default` only inherits Adwaita, so GTK would find
    /// no cursors; Adwaita's are drawn instead.
    #[test]
    fn a_missing_theme_falls_back_through_default_to_adwaita() {
        let icons = Icons::new();
        icons
            .theme("default", false, Some("Adwaita"))
            .theme("Adwaita", true, Some("hicolor"))
            .theme("hicolor", false, None);
        assert_eq!(icons.replacement("DMZ-Black").as_deref(), Some("Adwaita"));
    }

    /// A theme without cursors of its own, such as an icon theme picked as
    /// the cursor theme, gives way to the nearest theme it inherits that
    /// has some.
    #[test]
    fn a_theme_without_cursors_gives_way_to_the_theme_it_inherits() {
        let icons = Icons::new();
        icons
            .theme("Zorin-Dark", false, Some("Zorin, hicolor"))
            .theme("Zorin", false, Some("Yaru"))
            .theme("Yaru", true, Some("hicolor"))
            .theme("Adwaita", true, None);
        assert_eq!(icons.replacement("Zorin-Dark").as_deref(), Some("Yaru"));
    }

    /// A theme GTK loads, or a cursors folder GTK falls back to anyway,
    /// is left alone, and so is a desktop with no cursors to use.
    #[test]
    fn working_themes_are_left_alone() {
        let icons = Icons::new();
        icons.theme("DMZ-Black", true, None).theme("Adwaita", true, None);
        assert_eq!(icons.replacement("DMZ-Black"), None);

        let last_resort = icons.root.path().join("default").join("cursors");
        std::fs::create_dir_all(&last_resort).expect("the last-resort cursors");
        assert_eq!(replacement("Missing", &icons.folders(), &last_resort), None);

        let bare = Icons::new();
        bare.theme("default", false, Some("Adwaita"));
        assert_eq!(bare.replacement("Missing"), None);
    }

    /// Themes that inherit each other end the search instead of looping.
    #[test]
    fn inheritance_loops_end() {
        let icons = Icons::new();
        icons
            .theme("Left", false, Some("Right"))
            .theme("Right", false, Some("Left"))
            .theme("Adwaita", true, None);
        assert_eq!(icons.replacement("Left").as_deref(), Some("Adwaita"));
    }

    /// Only the `[Icon Theme]` group's `Inherits` counts, and paths are no
    /// theme names.
    #[test]
    fn inherits_is_read_from_the_icon_theme_group() {
        let index = "[Other]\nInherits=Wrong\n[Icon Theme]\nName=X\nInherits = One,Two , ../Up\n";
        assert_eq!(inherits(index), ["One", "Two"]);
        assert!(inherits("[Icon Theme]\nName=X\n").is_empty());
    }
}
