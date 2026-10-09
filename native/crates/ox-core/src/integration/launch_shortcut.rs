// SPDX-License-Identifier: AGPL-3.0-only
//! Super+E opens `OpenXplorer` on KDE Plasma (opt-in), as Win+E opens File
//! Explorer in Windows.
//!
//! On Plasma, Super+E (Meta+E in KDE's words) is a global shortcut that
//! launches Dolphin. Global shortcuts belong to KDE's shortcut service,
//! `kglobalacceld`, reached on the session bus as `org.kde.kglobalaccel`;
//! System Settings > Shortcuts changes them the same way. A desktop file is
//! a component named after it: its `_launch` action runs its command, and
//! each `[Desktop Action]` is an action of that name, which the service runs
//! when the keys are pressed.
//!
//! As Win+E always opens a new File Explorer window, Super+E runs the
//! desktop file's `NewWindow` action (`openxplorer --new-window`), which
//! opens a new window whether or not one is open; `_launch` would only show
//! the open window. Turning it on takes Super+E from the action that has it
//! (Dolphin's launch action, as Plasma ships), keeping that action's other
//! keys, and gives it to that `NewWindow` action. What it took is recorded in the settings
//! folder, so turning it off gives Super+E back to that action. The
//! service saves the change itself, so it lasts across logins. Nothing
//! changes until the user asks (INT-033), other desktops are left alone,
//! and the Flatpak, which cannot reach the service, does not offer it.

use std::path::{Path, PathBuf};

use gio::prelude::*;
use serde::{Deserialize, Serialize};

use super::private_file::write_private_file;
use super::sandbox::Sandbox;
use super::worker::on_worker;

/// The shortcut service's bus name.
pub const SHORTCUT_SERVICE: &str = "org.kde.kglobalaccel";

/// Its object path.
const SHORTCUT_PATH: &str = "/kglobalaccel";

/// Its interface.
const SHORTCUT_INTERFACE: &str = "org.kde.KGlobalAccel";

/// How long one call may take, in milliseconds.
const CALL_TIMEOUT_MS: i32 = 5_000;

/// The record of what turning it on took, in the settings folder.
const RECORD_FILE_NAME: &str = "launch-shortcut.json";

/// Qt's Meta modifier, which is Super.
const META: i32 = 0x1000_0000;

/// Qt's key code of E.
const KEY_E: i32 = 0x45;

/// The action of a desktop file that launches it.
const LAUNCH_ACTION: &str = "_launch";

/// The desktop file's action that opens a new window (`[Desktop Action
/// NewWindow]`, `openxplorer --new-window`), which Super+E runs.
pub const NEW_WINDOW_ACTION: &str = "NewWindow";

/// One key sequence as the service sends it: up to four key combinations,
/// unused ones 0 (Qt's `QKeySequence` on the bus, `(ai)`).
pub type KeySequence = [i32; 4];

/// Super+E.
pub const SUPER_E: KeySequence = [META | KEY_E, 0, 0, 0];

/// A global shortcut action: its component and action names, and the
/// names people see (`componentUnique`, `actionUnique`,
/// `componentFriendly`, `actionFriendly`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutAction {
    /// The component, for a launch action the desktop file's name.
    pub component: String,
    /// The action within the component.
    pub action: String,
    /// The component's name in System Settings.
    pub component_name: String,
    /// The action's name in System Settings.
    pub action_name: String,
}

impl ShortcutAction {
    /// The launch action of the desktop file `desktop_id`, called `name`.
    pub fn launch(desktop_id: &str, name: &str) -> Self {
        Self {
            component: desktop_id.to_owned(),
            action: LAUNCH_ACTION.to_owned(),
            component_name: name.to_owned(),
            action_name: name.to_owned(),
        }
    }

    /// The `[Desktop Action]` called `action` of the desktop file
    /// `desktop_id`, named `component_name` and `action_name` in System
    /// Settings.
    pub fn desktop_action(desktop_id: &str, action: &str, component_name: &str, action_name: &str) -> Self {
        Self {
            component: desktop_id.to_owned(),
            action: action.to_owned(),
            component_name: component_name.to_owned(),
            action_name: action_name.to_owned(),
        }
    }

    /// The action from the four names the service sends; `None` for
    /// anything else, such as the empty list of a free key.
    fn from_names(names: &[String]) -> Option<Self> {
        let [component, action, component_name, action_name] = names else {
            return None;
        };
        Some(Self {
            component: component.clone(),
            action: action.clone(),
            component_name: component_name.clone(),
            action_name: action_name.clone(),
        })
    }

    /// The four names, as the service takes them.
    fn names(&self) -> Vec<String> {
        vec![
            self.component.clone(),
            self.action.clone(),
            self.component_name.clone(),
            self.action_name.clone(),
        ]
    }

    /// True for the same action, whatever names people see.
    fn is(&self, other: &Self) -> bool {
        self.component == other.component && self.action == other.action
    }
}

/// Why the shortcut could not be read or changed. `Display` is the text
/// Settings shows.
#[derive(Debug, thiserror::Error)]
pub enum ShortcutError {
    /// The session has no KDE shortcut service.
    #[error("KDE's shortcut service did not answer: {0}")]
    Unreachable(String),
    /// The service did not give Super+E to `OpenXplorer`.
    #[error(
        "KDE kept Super+E for another shortcut. Change it in System Settings > Shortcuts, \
         then try again."
    )]
    NotGiven,
    /// Not on KDE Plasma, or inside the Flatpak.
    #[error("Super+E can be changed here only on KDE Plasma, with the installed package.")]
    Unsupported,
    /// The record could not be written.
    #[error("{error}: {path}")]
    Record {
        /// The record file.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
}

/// Reads and changes global shortcuts: KDE's service, or a table in tests.
pub trait GlobalShortcuts {
    /// The action Super+E runs now, if any.
    ///
    /// # Errors
    ///
    /// When the service cannot be reached.
    fn owner(&self, keys: KeySequence) -> Result<Option<ShortcutAction>, ShortcutError>;

    /// The keys of `action`.
    ///
    /// # Errors
    ///
    /// When the service cannot be reached.
    fn keys(&self, action: &ShortcutAction) -> Result<Vec<KeySequence>, ShortcutError>;

    /// Makes `action` known to the service, as an app does before its
    /// shortcut can be set.
    ///
    /// # Errors
    ///
    /// When the service cannot be reached.
    fn register(&self, action: &ShortcutAction) -> Result<(), ShortcutError>;

    /// Gives `action` exactly `keys`; the service skips a key another
    /// action has.
    ///
    /// # Errors
    ///
    /// When the service cannot be reached.
    fn set_keys(&self, action: &ShortcutAction, keys: &[KeySequence]) -> Result<(), ShortcutError>;
}

/// KDE's shortcut service on the session bus. Its calls block, so call it
/// on a worker thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct KdeShortcuts;

impl KdeShortcuts {
    /// Calls `method` with `arguments` and returns the reply.
    fn call(method: &str, arguments: &glib::Variant) -> Result<glib::Variant, ShortcutError> {
        let unreachable = |error: glib::Error| ShortcutError::Unreachable(error.message().to_owned());
        let connection =
            gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(unreachable)?;
        connection
            .call_sync(
                Some(SHORTCUT_SERVICE),
                SHORTCUT_PATH,
                SHORTCUT_INTERFACE,
                method,
                Some(arguments),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                CALL_TIMEOUT_MS,
                gio::Cancellable::NONE,
            )
            .map_err(unreachable)
    }
}

impl GlobalShortcuts for KdeShortcuts {
    fn owner(&self, keys: KeySequence) -> Result<Option<ShortcutAction>, ShortcutError> {
        let reply = Self::call(
            "actionList",
            &glib::Variant::tuple_from_iter([sequence_variant(keys)]),
        )?;
        let names = reply
            .get::<(Vec<String>,)>()
            .map(|(names,)| names)
            .unwrap_or_default();
        Ok(ShortcutAction::from_names(&names))
    }

    fn keys(&self, action: &ShortcutAction) -> Result<Vec<KeySequence>, ShortcutError> {
        let reply = Self::call("shortcutKeys", &(action.names(),).to_variant())?;
        Ok(sequences_of(&reply.child_value(0)))
    }

    fn register(&self, action: &ShortcutAction) -> Result<(), ShortcutError> {
        Self::call("doRegister", &(action.names(),).to_variant()).map(drop)
    }

    fn set_keys(&self, action: &ShortcutAction, keys: &[KeySequence]) -> Result<(), ShortcutError> {
        let arguments =
            glib::Variant::tuple_from_iter([action.names().to_variant(), sequences_variant(keys)]);
        Self::call("setForeignShortcutKeys", &arguments).map(drop)
    }
}

/// `keys` as the service takes one key sequence: `(ai)`.
fn sequence_variant(keys: KeySequence) -> glib::Variant {
    glib::Variant::tuple_from_iter([keys.to_vec().to_variant()])
}

/// `sequences` as the service takes a set of key sequences: `a(ai)`.
fn sequences_variant(sequences: &[KeySequence]) -> glib::Variant {
    let element = glib::VariantTy::new("(ai)").expect("a valid type");
    glib::Variant::array_from_iter_with_type(element, sequences.iter().map(|keys| sequence_variant(*keys)))
}

/// The key sequences of an `a(ai)` reply; anything else is none.
fn sequences_of(value: &glib::Variant) -> Vec<KeySequence> {
    if value.type_().as_str() != "a(ai)" {
        return Vec::new();
    }
    value
        .iter()
        .filter_map(|sequence| sequence.child_value(0).get::<Vec<i32>>())
        .map(|combinations| {
            let mut keys = [0; 4];
            for (key, combination) in keys.iter_mut().zip(combinations) {
                *key = combination;
            }
            keys
        })
        .filter(|keys| keys.iter().any(|key| *key != 0))
        .collect()
}

/// What Super+E does now, as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchShortcutStatus {
    /// Not on KDE Plasma, or inside the Flatpak.
    Unsupported,
    /// Super+E opens `OpenXplorer`.
    Ours,
    /// Super+E runs another action, named as System Settings names it.
    Other(String),
    /// Super+E is not used.
    Free,
    /// The shortcut service did not answer, for this reason.
    Unreachable(String),
}

/// What turning the shortcut off did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoredShortcut {
    /// Super+E went back to the action it was taken from.
    GivenBack,
    /// Super+E was taken from no action, so it is free again.
    Freed,
    /// `OpenXplorer` did not have Super+E, so nothing changed.
    NotOurs,
}

/// What turning it on took: the action that had Super+E and all its keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    /// The action Super+E was taken from.
    previous: ShortcutAction,
    /// That action's keys before, Super+E among them.
    keys: Vec<KeySequence>,
}

/// The opt-in that makes Super+E open `OpenXplorer` on KDE Plasma.
#[derive(Debug, Clone)]
pub struct LaunchShortcut<G> {
    shortcuts: G,
    settings: PathBuf,
    ours: ShortcutAction,
    desktops: Vec<String>,
    sandbox: Sandbox,
}

impl<G: GlobalShortcuts> LaunchShortcut<G> {
    /// The opt-in for the app whose desktop file is `desktop_id`, with its
    /// record in `settings`, in a session of `desktops`.
    pub fn new(
        shortcuts: G,
        settings: &Path,
        desktop_id: &str,
        desktops: Vec<String>,
        sandbox: Sandbox,
    ) -> Self {
        Self {
            shortcuts,
            settings: settings.to_owned(),
            ours: ShortcutAction::desktop_action(desktop_id, NEW_WINDOW_ACTION, "OpenXplorer", "New window"),
            desktops,
            sandbox,
        }
    }

    /// Where the shortcuts are read and changed.
    pub fn shortcuts(&self) -> &G {
        &self.shortcuts
    }

    /// True on KDE Plasma outside the Flatpak.
    pub fn is_available(&self) -> bool {
        !self.sandbox.is_flatpak() && self.desktops.iter().any(|desktop| desktop == "kde")
    }

    /// What Super+E does now. Reading changes nothing.
    pub fn status(&self) -> LaunchShortcutStatus {
        if !self.is_available() {
            return LaunchShortcutStatus::Unsupported;
        }
        match self.shortcuts.owner(SUPER_E) {
            Ok(Some(owner)) if owner.is(&self.ours) => LaunchShortcutStatus::Ours,
            Ok(Some(owner)) => LaunchShortcutStatus::Other(friendly_name(&owner)),
            Ok(None) => LaunchShortcutStatus::Free,
            Err(error) => LaunchShortcutStatus::Unreachable(error.to_string()),
        }
    }

    /// True for any action of `OpenXplorer`'s desktop file, such as the
    /// launch action an earlier version gave Super+E to.
    fn is_ours_in_any_action(&self, action: &ShortcutAction) -> bool {
        action.component == self.ours.component
    }

    /// Takes Super+E from the action that has it, keeping its other keys,
    /// and gives it to `OpenXplorer`'s new-window action. When the service
    /// does not give it, the other action gets its keys back. Super+E on
    /// another action of `OpenXplorer`'s (the launch action of an earlier
    /// version) moves to the new-window action, and the record of what was
    /// first taken is kept.
    ///
    /// # Errors
    ///
    /// [`ShortcutError::Unsupported`] off KDE Plasma and in the Flatpak,
    /// [`ShortcutError::NotGiven`] when the service kept Super+E for
    /// another action, and the service's or the record's failure.
    pub fn enable(&self) -> Result<(), ShortcutError> {
        if !self.is_available() {
            return Err(ShortcutError::Unsupported);
        }
        let owner = self.shortcuts.owner(SUPER_E)?;
        if owner.as_ref().is_some_and(|owner| owner.is(&self.ours)) {
            return Ok(());
        }
        let taken = match owner {
            Some(earlier) if self.is_ours_in_any_action(&earlier) => {
                let mut keys = self.shortcuts.keys(&earlier)?;
                keys.retain(|keys| *keys != SUPER_E);
                self.shortcuts.set_keys(&earlier, &keys)?;
                None
            }
            Some(previous) => {
                let keys = self.shortcuts.keys(&previous)?;
                let record = Record { previous, keys };
                self.write_record(&record)?;
                let kept: Vec<KeySequence> = record
                    .keys
                    .iter()
                    .copied()
                    .filter(|keys| *keys != SUPER_E)
                    .collect();
                self.shortcuts.set_keys(&record.previous, &kept)?;
                Some(record)
            }
            None => None,
        };
        self.shortcuts.register(&self.ours)?;
        self.shortcuts.set_keys(&self.ours, &[SUPER_E])?;
        let given = self
            .shortcuts
            .owner(SUPER_E)?
            .is_some_and(|owner| owner.is(&self.ours));
        if given {
            return Ok(());
        }
        if let Some(record) = taken {
            self.shortcuts.set_keys(&record.previous, &record.keys)?;
            self.remove_record();
        }
        Err(ShortcutError::NotGiven)
    }

    /// Takes Super+E from `OpenXplorer` and gives it back to the action it
    /// was taken from, with all that action's keys. A Super+E the user
    /// has given to something else since is left alone.
    ///
    /// # Errors
    ///
    /// [`ShortcutError::Unsupported`] off KDE Plasma and in the Flatpak,
    /// and the service's failure.
    pub fn restore(&self) -> Result<RestoredShortcut, ShortcutError> {
        if !self.is_available() {
            return Err(ShortcutError::Unsupported);
        }
        let record = self.read_record();
        let owner = self
            .shortcuts
            .owner(SUPER_E)?
            .filter(|owner| self.is_ours_in_any_action(owner));
        let Some(owner) = owner else {
            self.remove_record();
            return Ok(RestoredShortcut::NotOurs);
        };
        let mut keys = self.shortcuts.keys(&owner)?;
        keys.retain(|keys| *keys != SUPER_E);
        self.shortcuts.set_keys(&owner, &keys)?;
        let restored = match record {
            Some(record) => {
                self.shortcuts.set_keys(&record.previous, &record.keys)?;
                RestoredShortcut::GivenBack
            }
            None => RestoredShortcut::Freed,
        };
        self.remove_record();
        Ok(restored)
    }

    /// Runs `operation` on a worker thread, as the service's calls block.
    pub fn run_in_background<T, F>(&self, operation: F) -> impl std::future::Future<Output = T> + 'static
    where
        G: Clone + Send + 'static,
        T: Send + 'static,
        F: FnOnce(&Self) -> T + Send + 'static,
    {
        let shortcut = self.clone();
        on_worker(move || operation(&shortcut))
    }

    /// The record's path.
    fn record_path(&self) -> PathBuf {
        self.settings.join(RECORD_FILE_NAME)
    }

    /// The record, or `None` when there is none or it cannot be read.
    fn read_record(&self) -> Option<Record> {
        let text = std::fs::read_to_string(self.record_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Writes the record, privately.
    fn write_record(&self, record: &Record) -> Result<(), ShortcutError> {
        let path = self.record_path();
        let text = serde_json::to_string_pretty(record).expect("a record serialises");
        std::fs::create_dir_all(&self.settings)
            .and_then(|()| write_private_file(&path, ".winspace-", text.as_bytes()))
            .map_err(|error| ShortcutError::Record { path, error })
    }

    /// Removes the record; a missing one is fine.
    fn remove_record(&self) {
        let _ = std::fs::remove_file(self.record_path());
    }
}

/// The name System Settings gives `action`'s component, else its desktop
/// file without `.desktop`.
fn friendly_name(action: &ShortcutAction) -> String {
    if action.component_name.is_empty() {
        action.component.trim_end_matches(".desktop").to_owned()
    } else {
        action.component_name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: INT-033
    #[test]
    fn key_sequences_go_on_the_bus_as_kde_sends_them() {
        let one = sequence_variant(SUPER_E);
        assert_eq!(one.type_().as_str(), "(ai)");
        let several = sequences_variant(&[SUPER_E, [0x0400_0000 | 0x45, 0, 0, 0]]);
        assert_eq!(several.type_().as_str(), "a(ai)");

        assert_eq!(sequences_of(&several), [SUPER_E, [0x0400_0000 | 0x45, 0, 0, 0]]);
        assert_eq!(sequences_of(&sequences_variant(&[])), Vec::<KeySequence>::new());
        assert_eq!(sequences_of(&"text".to_variant()), Vec::<KeySequence>::new());
        assert_eq!(SUPER_E[0], 0x1000_0045, "Qt's Meta+E");
    }

    /// parity: INT-033
    #[test]
    fn only_four_names_are_an_action() {
        let names = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin").names();
        assert_eq!(
            names,
            ["org.kde.dolphin.desktop", "_launch", "Dolphin", "Dolphin"]
        );
        assert!(ShortcutAction::from_names(&names).is_some());
        assert_eq!(ShortcutAction::from_names(&[]), None);
    }
}
