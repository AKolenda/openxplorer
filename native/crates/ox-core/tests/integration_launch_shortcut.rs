// SPDX-License-Identifier: AGPL-3.0-only
//! Super+E opens `OpenXplorer` on KDE Plasma (INT-033): turning it on takes
//! the key from the action that has it and turning it off gives it back,
//! against a table that behaves as KDE's shortcut service does.

use std::cell::RefCell;

use ox_core::integration::{
    GlobalShortcuts, KeySequence, LaunchShortcut, LaunchShortcutStatus, RestoredShortcut, Sandbox,
    ShortcutAction, ShortcutError, SUPER_E,
};

/// `OpenXplorer`'s desktop file.
const OURS: &str = "io.winspace.Development.desktop";

/// Ctrl+Alt+D, a second key of Dolphin's in these tests.
const CTRL_ALT_D: KeySequence = [0x0400_0000 | 0x0800_0000 | 0x44, 0, 0, 0];

/// Global shortcuts in memory: each action and its keys. As KDE's service
/// does, a key another action has is skipped, and an action must be
/// registered (or known from the start) before its keys can be set.
#[derive(Debug, Default)]
struct Shortcuts {
    actions: RefCell<Vec<(ShortcutAction, Vec<KeySequence>)>>,
    /// The desktop files the service can launch.
    launchable: Vec<String>,
}

impl Shortcuts {
    /// Plasma as it ships: Super+E launches Dolphin.
    fn plasma() -> Self {
        let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
        Self {
            actions: RefCell::new(vec![(dolphin, vec![SUPER_E, CTRL_ALT_D])]),
            launchable: vec!["org.kde.dolphin.desktop".to_owned(), OURS.to_owned()],
        }
    }

    /// The keys of the action of `component` now.
    fn keys_of(&self, component: &str) -> Vec<KeySequence> {
        self.actions
            .borrow()
            .iter()
            .find(|(action, _)| action.component == component)
            .map(|(_, keys)| keys.clone())
            .unwrap_or_default()
    }
}

impl GlobalShortcuts for Shortcuts {
    fn owner(&self, keys: KeySequence) -> Result<Option<ShortcutAction>, ShortcutError> {
        Ok(self
            .actions
            .borrow()
            .iter()
            .find(|(_, owned)| owned.contains(&keys))
            .map(|(action, _)| action.clone()))
    }

    fn keys(&self, action: &ShortcutAction) -> Result<Vec<KeySequence>, ShortcutError> {
        Ok(self.keys_of(&action.component))
    }

    fn register(&self, action: &ShortcutAction) -> Result<(), ShortcutError> {
        let mut actions = self.actions.borrow_mut();
        let known = actions
            .iter()
            .any(|(known, _)| known.component == action.component);
        if !known && self.launchable.contains(&action.component) {
            actions.push((action.clone(), Vec::new()));
        }
        Ok(())
    }

    fn set_keys(&self, action: &ShortcutAction, keys: &[KeySequence]) -> Result<(), ShortcutError> {
        let mut actions = self.actions.borrow_mut();
        let free: Vec<KeySequence> = keys
            .iter()
            .copied()
            .filter(|key| {
                !actions
                    .iter()
                    .any(|(other, owned)| other.component != action.component && owned.contains(key))
            })
            .collect();
        if let Some((_, owned)) = actions
            .iter_mut()
            .find(|(known, _)| known.component == action.component)
        {
            *owned = free;
        }
        Ok(())
    }
}

/// The opt-in over `shortcuts` on KDE, with its record in `settings`.
fn launch_shortcut<'a>(
    shortcuts: &'a Shortcuts,
    settings: &std::path::Path,
) -> LaunchShortcut<&'a Shortcuts> {
    LaunchShortcut::new(shortcuts, settings, OURS, vec!["kde".to_owned()], Sandbox::Host)
}

impl GlobalShortcuts for &Shortcuts {
    fn owner(&self, keys: KeySequence) -> Result<Option<ShortcutAction>, ShortcutError> {
        (*self).owner(keys)
    }
    fn keys(&self, action: &ShortcutAction) -> Result<Vec<KeySequence>, ShortcutError> {
        (*self).keys(action)
    }
    fn register(&self, action: &ShortcutAction) -> Result<(), ShortcutError> {
        (*self).register(action)
    }
    fn set_keys(&self, action: &ShortcutAction, keys: &[KeySequence]) -> Result<(), ShortcutError> {
        (*self).set_keys(action, keys)
    }
}

/// parity: INT-033
#[test]
fn super_e_moves_from_dolphin_to_openxplorer_and_back() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Other("Dolphin".to_owned()));

    opt_in.enable().unwrap();

    assert_eq!(opt_in.status(), LaunchShortcutStatus::Ours);
    assert_eq!(shortcuts.keys_of(OURS), [SUPER_E]);
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D],
        "Dolphin keeps its other keys"
    );
    assert!(settings.path().join("launch-shortcut.json").is_file());
    opt_in.enable().unwrap();
    assert_eq!(
        shortcuts.keys_of(OURS),
        [SUPER_E],
        "turning it on twice changes nothing"
    );

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);

    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [SUPER_E, CTRL_ALT_D]
    );
    assert_eq!(shortcuts.keys_of(OURS), Vec::<KeySequence>::new());
    assert!(!settings.path().join("launch-shortcut.json").exists());
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Other("Dolphin".to_owned()));
}

/// parity: INT-033
#[test]
fn a_free_super_e_is_taken_and_freed_again() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts {
        launchable: vec![OURS.to_owned()],
        ..Shortcuts::default()
    };
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);

    opt_in.enable().unwrap();
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Ours);

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::Freed);
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);
}

/// When the service does not take `OpenXplorer`'s launch action (no
/// desktop file it can launch), Dolphin gets Super+E back.
///
/// parity: INT-033
#[test]
fn super_e_goes_back_when_the_service_does_not_give_it() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts {
        launchable: vec!["org.kde.dolphin.desktop".to_owned()],
        ..Shortcuts::plasma()
    };
    let opt_in = launch_shortcut(&shortcuts, settings.path());

    let refusal = opt_in.enable();

    assert!(matches!(refusal, Err(ShortcutError::NotGiven)), "{refusal:?}");
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [SUPER_E, CTRL_ALT_D]
    );
    assert!(!settings.path().join("launch-shortcut.json").exists());
}

/// A Super+E the user gave to another app after turning it on is left
/// alone by turning it off.
///
/// parity: INT-033
#[test]
fn turning_it_off_leaves_a_super_e_the_user_moved_since() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    let terminal = ShortcutAction::launch("org.kde.konsole.desktop", "Konsole");
    shortcuts
        .actions
        .borrow_mut()
        .retain(|(action, _)| action.component != OURS);
    shortcuts.actions.borrow_mut().push((terminal, vec![SUPER_E]));

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::NotOurs);

    assert_eq!(shortcuts.keys_of("org.kde.konsole.desktop"), [SUPER_E]);
    assert_eq!(shortcuts.keys_of("org.kde.dolphin.desktop"), [CTRL_ALT_D]);
}

/// Other desktops and the Flatpak never change a shortcut.
///
/// parity: INT-033
#[test]
fn other_desktops_and_the_flatpak_are_left_alone() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let gnome = LaunchShortcut::new(
        &shortcuts,
        settings.path(),
        OURS,
        vec!["gnome".to_owned()],
        Sandbox::Host,
    );
    let flatpak = LaunchShortcut::new(
        &shortcuts,
        settings.path(),
        OURS,
        vec!["kde".to_owned()],
        Sandbox::Flatpak,
    );

    for opt_in in [gnome, flatpak] {
        assert_eq!(opt_in.status(), LaunchShortcutStatus::Unsupported);
        assert!(matches!(opt_in.enable(), Err(ShortcutError::Unsupported)));
        assert!(matches!(opt_in.restore(), Err(ShortcutError::Unsupported)));
    }
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [SUPER_E, CTRL_ALT_D]
    );
}
