// SPDX-License-Identifier: AGPL-3.0-only
//! Super+E opens `OpenXplorer` on KDE Plasma (INT-033): turning it on takes
//! the key from the action that has it and turning it off gives it back,
//! against a table that behaves as KDE's shortcut service does.

use std::cell::RefCell;

use ox_core::integration::{
    GlobalShortcuts, KeySequence, LaunchShortcut, LaunchShortcutStatus, RestoredShortcut, Sandbox,
    ShortcutAction, ShortcutError, NEW_WINDOW_ACTION, SUPER_E,
};

/// `OpenXplorer`'s desktop file.
const OURS: &str = "io.winspace.Development.desktop";

/// The record the stable package keeps of what it took.
const RECORD: &str = "launch-shortcut-io.winspace.Development.desktop.json";

/// The preview package's desktop file, which shares the settings folder.
const PREVIEW: &str = "io.winspace.Development.Native.desktop";

/// Ctrl+Alt+F, a key the user gives Dolphin while Super+E is
/// `OpenXplorer`'s.
const CTRL_ALT_F: KeySequence = [0x0400_0000 | 0x0800_0000 | 0x46, 0, 0, 0];

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
    /// A call that fails, as when the service restarts: `register`, or
    /// `set_keys` of `OpenXplorer`'s actions.
    failing: RefCell<Option<&'static str>>,
}

impl Shortcuts {
    /// Plasma as it ships: Super+E launches Dolphin.
    fn plasma() -> Self {
        let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
        Self {
            actions: RefCell::new(vec![(dolphin, vec![SUPER_E, CTRL_ALT_D])]),
            launchable: vec![
                "org.kde.dolphin.desktop".to_owned(),
                OURS.to_owned(),
                PREVIEW.to_owned(),
            ],
            failing: RefCell::new(None),
        }
    }

    /// Fails `call` from now on.
    fn fail(&self, call: &'static str) {
        *self.failing.borrow_mut() = Some(call);
    }

    /// The service's error for a failing `call`.
    fn check(&self, call: &'static str) -> Result<(), ShortcutError> {
        if *self.failing.borrow() == Some(call) {
            return Err(ShortcutError::Unreachable("the service restarted".to_owned()));
        }
        Ok(())
    }

    /// The keys of `component`'s launch action, or of `OpenXplorer`'s
    /// new-window action for [`OURS`], now.
    fn keys_of(&self, component: &str) -> Vec<KeySequence> {
        let action = if component == OURS {
            NEW_WINDOW_ACTION
        } else {
            "_launch"
        };
        self.keys_of_action(component, action)
    }

    /// The keys of `component`'s `action` now.
    fn keys_of_action(&self, component: &str, action: &str) -> Vec<KeySequence> {
        self.actions
            .borrow()
            .iter()
            .find(|(known, _)| known.component == component && known.action == action)
            .map(|(_, keys)| keys.clone())
            .unwrap_or_default()
    }
}

/// True for the same action of the same component.
fn same(one: &ShortcutAction, other: &ShortcutAction) -> bool {
    one.component == other.component && one.action == other.action
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
        Ok(self.keys_of_action(&action.component, &action.action))
    }

    fn register(&self, action: &ShortcutAction) -> Result<(), ShortcutError> {
        self.check("register")?;
        let mut actions = self.actions.borrow_mut();
        let known = actions.iter().any(|(known, _)| same(known, action));
        if !known && self.launchable.contains(&action.component) {
            actions.push((action.clone(), Vec::new()));
        }
        Ok(())
    }

    fn set_keys(&self, action: &ShortcutAction, keys: &[KeySequence]) -> Result<(), ShortcutError> {
        if action.component == OURS {
            self.check("set_keys")?;
        }
        let mut actions = self.actions.borrow_mut();
        let free: Vec<KeySequence> = keys
            .iter()
            .copied()
            .filter(|key| {
                !actions
                    .iter()
                    .any(|(other, owned)| !same(other, action) && owned.contains(key))
            })
            .collect();
        if let Some((_, owned)) = actions.iter_mut().find(|(known, _)| same(known, action)) {
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
    assert!(settings.path().join(RECORD).is_file());
    opt_in.enable().unwrap();
    assert_eq!(
        shortcuts.keys_of(OURS),
        [SUPER_E],
        "turning it on twice changes nothing"
    );

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);

    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D, SUPER_E]
    );
    assert_eq!(shortcuts.keys_of(OURS), Vec::<KeySequence>::new());
    assert!(!settings.path().join(RECORD).exists());
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
        [CTRL_ALT_D, SUPER_E]
    );
    assert!(!settings.path().join(RECORD).exists());
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

/// As Win+E always opens a new File Explorer window, Super+E goes to the
/// desktop file's New window action (`openxplorer --new-window`), not to
/// its launch action, which only shows the open window.
///
/// parity: INT-033
#[test]
fn super_e_runs_the_new_window_action() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());

    opt_in.enable().unwrap();

    let owner = shortcuts.owner(SUPER_E).unwrap().expect("Super+E has an owner");
    assert_eq!(
        (owner.component.as_str(), owner.action.as_str()),
        (OURS, "NewWindow")
    );
    assert_eq!(
        shortcuts.keys_of_action(OURS, "_launch"),
        Vec::<KeySequence>::new()
    );
}

/// Super+E that an earlier version gave to `OpenXplorer`'s launch action
/// moves to the New window action when the switch is turned on again, and
/// turning it off still gives it back to Dolphin.
///
/// parity: INT-033
#[test]
fn super_e_on_the_earlier_launch_action_moves_to_new_window_and_back() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    let earlier = ShortcutAction::launch(OURS, "OpenXplorer");
    shortcuts
        .set_keys(
            &ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "", ""),
            &[],
        )
        .unwrap();
    shortcuts.actions.borrow_mut().push((earlier, vec![SUPER_E]));
    assert_ne!(
        opt_in.status(),
        LaunchShortcutStatus::Ours,
        "the switch shows off"
    );

    opt_in.enable().unwrap();

    assert_eq!(opt_in.status(), LaunchShortcutStatus::Ours);
    assert_eq!(shortcuts.keys_of(OURS), [SUPER_E]);
    assert_eq!(
        shortcuts.keys_of_action(OURS, "_launch"),
        Vec::<KeySequence>::new()
    );
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D, SUPER_E]
    );

    // Turning it off while the earlier launch action has Super+E gives it
    // back to Dolphin too.
    opt_in.enable().unwrap();
    shortcuts
        .set_keys(
            &ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "", ""),
            &[],
        )
        .unwrap();
    shortcuts
        .set_keys(&ShortcutAction::launch(OURS, "OpenXplorer"), &[SUPER_E])
        .unwrap();
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D, SUPER_E]
    );
    assert_eq!(
        shortcuts.keys_of_action(OURS, "_launch"),
        Vec::<KeySequence>::new()
    );
}

/// The New window action Super+E runs is in the desktop file the packages
/// install, and opens a new window.
///
/// parity: INT-033
#[test]
fn the_desktop_file_has_the_new_window_action() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/data/io.winspace.Development.desktop");
    let entry = std::fs::read_to_string(path).expect("the desktop file is in the tree");
    let actions = entry
        .lines()
        .find_map(|line| line.strip_prefix("Actions="))
        .expect("the desktop file lists its actions");
    assert!(actions.split(';').any(|action| action == NEW_WINDOW_ACTION));
    let section = entry
        .split("[Desktop Action NewWindow]")
        .nth(1)
        .expect("the New window action has a section");
    let exec = section
        .lines()
        .find_map(|line| line.strip_prefix("Exec="))
        .expect("the action has a command");
    assert_eq!(exec, "openxplorer --new-window");
}

/// Turning it off gives Super+E back to Dolphin and keeps the keys the user
/// changed on Dolphin since it was turned on.
///
/// parity: INT-033
#[test]
fn turning_it_off_adds_super_e_back_and_keeps_later_changes() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
    shortcuts.set_keys(&dolphin, &[CTRL_ALT_F]).unwrap();

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);

    let mut keys = shortcuts.keys_of("org.kde.dolphin.desktop");
    keys.sort_unstable();
    let mut expected = vec![CTRL_ALT_F, SUPER_E];
    expected.sort_unstable();
    assert_eq!(
        keys, expected,
        "Ctrl+Alt+D was removed by the user and stays removed"
    );
}

/// A failure partway through turning it on, as when KDE's service
/// restarts, gives Super+E back to Dolphin.
///
/// parity: INT-033
#[test]
fn a_failure_while_turning_it_on_gives_super_e_back() {
    for call in ["register", "set_keys"] {
        let settings = tempfile::tempdir().unwrap();
        let shortcuts = Shortcuts::plasma();
        let opt_in = launch_shortcut(&shortcuts, settings.path());
        shortcuts.fail(call);

        assert!(opt_in.enable().is_err(), "{call} failed");

        assert_eq!(
            shortcuts.keys_of("org.kde.dolphin.desktop"),
            [CTRL_ALT_D, SUPER_E],
            "{call}: Dolphin has Super+E again"
        );
        assert!(!settings.path().join(RECORD).exists());
    }
}

/// The stable and the preview package share the settings folder, and each
/// keeps its own record of what it took, so turning both off in any order
/// gives Super+E back to Dolphin.
///
/// parity: INT-033
#[test]
fn the_stable_and_preview_packages_keep_their_own_records() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = LaunchShortcut::new(
        &shortcuts,
        settings.path(),
        PREVIEW,
        vec!["kde".to_owned()],
        Sandbox::Host,
    );

    stable.enable().unwrap();
    preview.enable().unwrap();
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        stable.status(),
        LaunchShortcutStatus::Ours,
        "the preview gave it back to the stable app"
    );
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::GivenBack);

    let keys = shortcuts.keys_of("org.kde.dolphin.desktop");
    assert!(keys.contains(&SUPER_E), "Dolphin has Super+E again: {keys:?}");
}

/// A record an earlier test build of the stable package kept, under the
/// one name every package shared, still gives Super+E back to Dolphin.
///
/// parity: INT-033
#[test]
fn an_earlier_record_still_gives_super_e_back() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    let record = settings.path().join(RECORD);
    let earlier = settings.path().join("launch-shortcut.json");
    let text = std::fs::read_to_string(&record).unwrap();
    let with_keys = text.replacen('{', "{\"keys\": [[268435525, 0, 0, 0]],", 1);
    std::fs::write(&earlier, with_keys).unwrap();
    std::fs::remove_file(&record).unwrap();

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);

    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
    assert!(!earlier.exists(), "the earlier record is gone");
}

/// Ctrl+Alt+N, a key the user gave `OpenXplorer`'s New window action.
const CTRL_ALT_N: KeySequence = [0x0400_0000 | 0x0800_0000 | 0x4e, 0, 0, 0];

/// The preview package's opt-in over `shortcuts`, with its record in
/// `settings`.
fn preview_shortcut<'a>(
    shortcuts: &'a Shortcuts,
    settings: &std::path::Path,
) -> LaunchShortcut<&'a Shortcuts> {
    LaunchShortcut::new(
        shortcuts,
        settings,
        PREVIEW,
        vec!["kde".to_owned()],
        Sandbox::Host,
    )
}

/// Turning it on adds Super+E to the keys the New window action already
/// has, and turning it off takes only Super+E away.
///
/// parity: INT-033
#[test]
fn turning_it_on_keeps_the_new_window_actions_other_keys() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let new_window = ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "OpenXplorer", "New window");
    shortcuts
        .actions
        .borrow_mut()
        .push((new_window, vec![CTRL_ALT_N]));
    let opt_in = launch_shortcut(&shortcuts, settings.path());

    opt_in.enable().unwrap();

    assert_eq!(opt_in.status(), LaunchShortcutStatus::Ours);
    assert_eq!(shortcuts.keys_of(OURS), [CTRL_ALT_N, SUPER_E]);
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(shortcuts.keys_of(OURS), [CTRL_ALT_N]);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}

/// Super+E the user gave another action of `OpenXplorer`'s own, such as
/// its Settings action, goes back to that action when the switch is
/// turned off; only the launch action of an earlier version is moved
/// without a record. With the switch off, that Super+E is left alone.
///
/// parity: INT-033
#[test]
fn super_e_on_another_openxplorer_action_goes_back_to_it() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
    shortcuts.set_keys(&dolphin, &[CTRL_ALT_D]).unwrap();
    let settings_action = ShortcutAction::desktop_action(OURS, "Settings", "OpenXplorer", "Settings");
    shortcuts
        .actions
        .borrow_mut()
        .push((settings_action, vec![SUPER_E]));
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    assert_ne!(opt_in.status(), LaunchShortcutStatus::Ours);
    assert_eq!(
        opt_in.restore().unwrap(),
        RestoredShortcut::NotOurs,
        "with the switch off, the user's own Super+E stays"
    );
    assert_eq!(shortcuts.keys_of_action(OURS, "Settings"), [SUPER_E]);

    opt_in.enable().unwrap();

    assert_eq!(opt_in.status(), LaunchShortcutStatus::Ours);
    assert_eq!(
        shortcuts.keys_of_action(OURS, "Settings"),
        Vec::<KeySequence>::new()
    );
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(shortcuts.keys_of_action(OURS, "Settings"), [SUPER_E]);
    assert_eq!(shortcuts.keys_of(OURS), Vec::<KeySequence>::new());
}

/// With both packages' Settings open, the stable package's switch can be
/// turned off after the preview took Super+E from it. That opt-out hands
/// its record on, so turning the preview off gives Super+E to Dolphin,
/// not back to the stable package.
///
/// parity: INT-033
#[test]
fn a_stale_opt_out_hands_its_record_to_the_package_that_took_super_e() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    stable.enable().unwrap();
    preview.enable().unwrap();

    assert_eq!(stable.restore().unwrap(), RestoredShortcut::NotOurs);
    assert_eq!(
        preview.status(),
        LaunchShortcutStatus::Ours,
        "the preview keeps it"
    );
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::GivenBack);

    assert_ne!(stable.status(), LaunchShortcutStatus::Ours, "the opt-out holds");
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::NotOurs);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}

/// A stale opt-out of a package that took a free Super+E leaves the key
/// free when the other package lets it go.
///
/// parity: INT-033
#[test]
fn a_stale_opt_out_of_a_free_super_e_leaves_it_free_later() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
    shortcuts.set_keys(&dolphin, &[CTRL_ALT_D]).unwrap();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    stable.enable().unwrap();
    preview.enable().unwrap();

    assert_eq!(stable.restore().unwrap(), RestoredShortcut::NotOurs);
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::Freed);

    assert_eq!(stable.status(), LaunchShortcutStatus::Free);
}

/// Taking Super+E back from the package that took it keeps the record of
/// Dolphin, so turning it off gives Super+E to Dolphin.
///
/// parity: INT-033
#[test]
fn taking_super_e_back_from_the_other_package_keeps_the_record_of_dolphin() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    stable.enable().unwrap();
    preview.enable().unwrap();

    stable.enable().unwrap();

    assert_eq!(stable.status(), LaunchShortcutStatus::Ours);
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::GivenBack);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::NotOurs);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}

/// When taking Super+E back from the other package fails, the other
/// package keeps Super+E and its record.
///
/// parity: INT-033
#[test]
fn a_failure_while_taking_super_e_back_leaves_the_other_package_as_it_was() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    stable.enable().unwrap();
    preview.enable().unwrap();
    shortcuts.fail("register");

    assert!(stable.enable().is_err());

    *shortcuts.failing.borrow_mut() = None;
    assert_eq!(preview.status(), LaunchShortcutStatus::Ours);
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        stable.status(),
        LaunchShortcutStatus::Ours,
        "back to the stable package"
    );
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::GivenBack);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}
