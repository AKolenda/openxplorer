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
    /// The calls that fail, as when the service restarts: `register`,
    /// `set_keys` of `OpenXplorer`'s actions, or `give_back`, a `set_keys`
    /// that adds Super+E to any action but the New window action.
    failing: RefCell<Vec<&'static str>>,
    /// Runs before each `set_keys`, with its action, as another process
    /// could act at that moment.
    before_set_keys: RefCell<Option<Hook>>,
    /// The `set_keys` calls made so far, and the first one that fails (with
    /// every later one), as when KDE's service goes away partway.
    set_keys_calls: std::cell::Cell<usize>,
    fail_from_set_keys: std::cell::Cell<Option<usize>>,
}

/// What runs before a `set_keys`.
struct Hook(Box<dyn Fn(&ShortcutAction)>);

impl std::fmt::Debug for Hook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Hook")
    }
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
            failing: RefCell::new(Vec::new()),
            before_set_keys: RefCell::new(None),
            set_keys_calls: std::cell::Cell::new(0),
            fail_from_set_keys: std::cell::Cell::new(None),
        }
    }

    /// Fails `call` from now on.
    fn fail(&self, call: &'static str) {
        self.failing.borrow_mut().push(call);
    }

    /// The service's error for a failing `call`.
    fn check(&self, call: &'static str) -> Result<(), ShortcutError> {
        if self.failing.borrow().contains(&call) {
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
        if let Some(Hook(hook)) = self.before_set_keys.borrow().as_ref() {
            hook(action);
        }
        let call = self.set_keys_calls.get();
        self.set_keys_calls.set(call + 1);
        if self.fail_from_set_keys.get().is_some_and(|first| call >= first) {
            return Err(ShortcutError::Unreachable("the service went away".to_owned()));
        }
        if action.component == OURS {
            self.check("set_keys")?;
        }
        if keys.contains(&SUPER_E) && !(action.component == OURS && action.action == NEW_WINDOW_ACTION) {
            self.check("give_back")?;
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

    shortcuts.failing.borrow_mut().clear();
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

/// Super+E the user cleared in System Settings after `OpenXplorer` took
/// it from Dolphin is free: turning the switch on and off again leaves it
/// free, instead of giving it to Dolphin by the old record.
///
/// parity: INT-033
#[test]
fn turning_it_on_from_a_free_key_forgets_an_old_record() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    shortcuts
        .set_keys(
            &ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "", ""),
            &[],
        )
        .unwrap();
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);

    opt_in.enable().unwrap();
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::Freed);

    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);
    assert_eq!(shortcuts.keys_of("org.kde.dolphin.desktop"), [CTRL_ALT_D]);
    assert!(!settings.path().join(RECORD).exists());
}

/// The record both packages kept, under one name, before each kept its
/// own.
const EARLIER: &str = "launch-shortcut.json";

/// Moves `package`'s record to the one name every package shared before.
fn make_it_an_earlier_record(settings: &std::path::Path, package: &str) {
    let record = settings.join(format!("launch-shortcut-{package}"));
    std::fs::rename(&record, settings.join(EARLIER)).expect("the record moves");
}

/// A preview build from before the records were split kept its record
/// under the shared name. While the preview has Super+E, that record is
/// its own: turning the preview off gives Super+E back to Dolphin.
///
/// parity: INT-033
#[test]
fn the_preview_reads_the_earlier_record_while_it_has_super_e() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let preview = preview_shortcut(&shortcuts, settings.path());
    preview.enable().unwrap();
    make_it_an_earlier_record(settings.path(), &format!("{PREVIEW}.json"));

    assert_eq!(preview.restore().unwrap(), RestoredShortcut::GivenBack);

    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
    assert!(!settings.path().join(EARLIER).exists(), "the record is used up");
}

/// The shared record belongs to whichever package has Super+E: turning
/// the other one off leaves it alone.
///
/// parity: INT-033
#[test]
fn the_earlier_record_stays_with_the_package_that_has_super_e() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    stable.enable().unwrap();
    make_it_an_earlier_record(settings.path(), &format!("{OURS}.json"));

    assert_eq!(preview.restore().unwrap(), RestoredShortcut::NotOurs);
    assert!(
        settings.path().join(EARLIER).exists(),
        "the stable package's record stays"
    );
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::GivenBack);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}

/// While Super+E is free, the shared record is nobody's: turning either
/// package on forgets it, so turning it off later leaves Super+E free.
///
/// parity: INT-033
#[test]
fn a_free_super_e_forgets_the_earlier_record_too() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    stable.enable().unwrap();
    make_it_an_earlier_record(settings.path(), &format!("{OURS}.json"));
    shortcuts
        .set_keys(
            &ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "", ""),
            &[],
        )
        .unwrap();

    preview.enable().unwrap();

    assert!(
        !settings.path().join(EARLIER).exists(),
        "the old record is forgotten"
    );
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::Freed);
    stable.enable().unwrap();
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::Freed);
    assert_eq!(shortcuts.keys_of("org.kde.dolphin.desktop"), [CTRL_ALT_D]);
}

/// True when another process could take the lock both packages use for
/// the shortcut in `settings` now.
fn shortcut_lock_is_free(settings: &std::path::Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .open(settings.join("launch-shortcut.lock"))
    else {
        return true;
    };
    let taken = rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive);
    taken.is_ok()
}

/// The stable and the preview package run in two processes and may be
/// turned on or off at the same moment. Reading the owner, taking
/// Super+E and giving it on happen under one lock in the settings
/// folder, so the other package never sees Super+E free in between.
///
/// parity: INT-033
#[test]
fn turning_it_on_or_off_holds_a_lock_the_other_package_waits_for() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let seen = std::rc::Rc::new(RefCell::new(Vec::new()));
    let (folder, record) = (settings.path().to_owned(), seen.clone());
    *shortcuts.before_set_keys.borrow_mut() = Some(Hook(Box::new(move |_| {
        record.borrow_mut().push(shortcut_lock_is_free(&folder));
    })));

    stable.enable().unwrap();
    stable.restore().unwrap();

    let seen = seen.borrow().clone();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|free| !free),
        "every change is made under the lock: {seen:?}"
    );
    assert!(
        shortcut_lock_is_free(settings.path()),
        "and the lock is let go after"
    );
}

/// Turning it off when the action Super+E was taken from is gone, as when
/// Dolphin was uninstalled since, leaves Super+E free and says so,
/// instead of saying it was given back.
///
/// parity: INT-033
#[test]
fn turning_it_off_says_super_e_is_free_when_its_old_action_is_gone() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    shortcuts
        .actions
        .borrow_mut()
        .retain(|(action, _)| action.component != "org.kde.dolphin.desktop");

    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::Freed);

    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);
    assert!(!settings.path().join(RECORD).exists());
}

/// The shared old record can point back at this package while Super+E
/// has since gone to another app's launch action, such as Konsole's.
/// That app is no package of `OpenXplorer`: turning the switch on records
/// it, and turning it off gives Super+E back to it.
///
/// parity: INT-033
#[test]
fn another_apps_launch_action_is_never_taken_for_the_other_package() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
    shortcuts.set_keys(&dolphin, &[CTRL_ALT_D]).unwrap();
    let konsole = ShortcutAction::launch("org.kde.konsole.desktop", "Konsole");
    shortcuts.actions.borrow_mut().push((konsole, vec![SUPER_E]));
    let pointing_here = format!(
        "{{\"previous\": {{\"component\": \"{OURS}\", \"action\": \"{NEW_WINDOW_ACTION}\", \
         \"component_name\": \"OpenXplorer\", \"action_name\": \"New window\"}}}}"
    );
    std::fs::write(settings.path().join(EARLIER), pointing_here).unwrap();
    let opt_in = launch_shortcut(&shortcuts, settings.path());

    opt_in.enable().unwrap();
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Ours);
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);

    assert_eq!(
        shortcuts.keys_of("org.kde.konsole.desktop"),
        [SUPER_E],
        "Konsole has it back"
    );
}

/// Super+E an earlier version gave to `OpenXplorer`'s launch action goes
/// back to that action when turning the switch on fails, instead of being
/// left on no action.
///
/// parity: INT-033
#[test]
fn a_failure_while_moving_super_e_from_the_earlier_launch_action_gives_it_back() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    shortcuts
        .set_keys(
            &ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "", ""),
            &[],
        )
        .unwrap();
    let earlier = ShortcutAction::launch(OURS, "OpenXplorer");
    shortcuts.actions.borrow_mut().push((earlier, vec![SUPER_E]));
    shortcuts.fail("register");

    assert!(opt_in.enable().is_err());

    assert_eq!(
        shortcuts.keys_of_action(OURS, "_launch"),
        [SUPER_E],
        "the earlier launch action has Super+E again"
    );
    shortcuts.failing.borrow_mut().clear();
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D, SUPER_E]
    );
}

/// When turning it on fails and giving Super+E back to Dolphin fails too,
/// the record of Dolphin is kept: turning the switch on and off again,
/// once the service works, gives Super+E back to Dolphin.
///
/// parity: INT-033
#[test]
fn a_failed_give_back_keeps_the_way_back_to_dolphin() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    shortcuts.fail("register");
    shortcuts.fail("give_back");

    assert!(opt_in.enable().is_err());
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);

    shortcuts.failing.borrow_mut().clear();
    opt_in.enable().unwrap();
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D, SUPER_E]
    );
}

/// The same after an upgrade: Super+E was on the earlier launch action,
/// and both moving it and giving it back fail. The record of Dolphin
/// still leads Super+E back there.
///
/// parity: INT-033
#[test]
fn a_failed_give_back_to_the_earlier_launch_action_keeps_the_way_back_to_dolphin() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    shortcuts
        .set_keys(
            &ShortcutAction::desktop_action(OURS, NEW_WINDOW_ACTION, "", ""),
            &[],
        )
        .unwrap();
    let earlier = ShortcutAction::launch(OURS, "OpenXplorer");
    shortcuts.actions.borrow_mut().push((earlier, vec![SUPER_E]));
    make_it_an_earlier_record(settings.path(), &format!("{OURS}.json"));
    shortcuts.fail("register");
    shortcuts.fail("give_back");

    assert!(opt_in.enable().is_err());
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);

    shortcuts.failing.borrow_mut().clear();
    opt_in.enable().unwrap();
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}

/// The preview package has Super+E with only the record every package
/// shared earlier, of Dolphin. The stable package takes Super+E and gives
/// it back: that record is the preview's again, so it stays, and turning
/// the preview off still gives Super+E to Dolphin.
///
/// parity: INT-033
#[test]
fn giving_super_e_back_to_the_preview_keeps_its_earlier_record() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let stable = launch_shortcut(&shortcuts, settings.path());
    let preview = preview_shortcut(&shortcuts, settings.path());
    preview.enable().unwrap();
    make_it_an_earlier_record(settings.path(), &format!("{PREVIEW}.json"));

    stable.enable().unwrap();
    assert_eq!(stable.restore().unwrap(), RestoredShortcut::GivenBack);

    assert_eq!(preview.status(), LaunchShortcutStatus::Ours);
    assert!(
        settings.path().join(EARLIER).exists(),
        "the preview's record stays"
    );
    assert_eq!(preview.restore().unwrap(), RestoredShortcut::GivenBack);
    assert!(shortcuts.keys_of("org.kde.dolphin.desktop").contains(&SUPER_E));
}

/// Turning it off takes Super+E from `OpenXplorer`, then KDE's service
/// goes away: giving it to Dolphin fails, and so does putting it back on
/// `OpenXplorer`. Super+E is left free; the record of Dolphin is kept for
/// the next turning on, so turning on and off again gives it to Dolphin.
///
/// parity: INT-033
#[test]
fn a_failed_turning_off_whose_undo_fails_keeps_the_way_back_to_dolphin() {
    let settings = tempfile::tempdir().unwrap();
    let shortcuts = Shortcuts::plasma();
    let opt_in = launch_shortcut(&shortcuts, settings.path());
    opt_in.enable().unwrap();
    shortcuts
        .fail_from_set_keys
        .set(Some(shortcuts.set_keys_calls.get() + 1));

    assert!(opt_in.restore().is_err());
    assert_eq!(opt_in.status(), LaunchShortcutStatus::Free);

    shortcuts.fail_from_set_keys.set(None);
    opt_in.enable().unwrap();
    assert_eq!(opt_in.restore().unwrap(), RestoredShortcut::GivenBack);
    assert_eq!(
        shortcuts.keys_of("org.kde.dolphin.desktop"),
        [CTRL_ALT_D, SUPER_E]
    );
}
