// SPDX-License-Identifier: AGPL-3.0-only
//! "Install updates without asking for a password" on Settings › About:
//! on by default (the package's postinst installs the rule), shown only
//! where the package installed the update helper, and on only while the
//! polkit rule is there.
//!
//! The setting runs on a fake system in a temporary folder, and a script
//! stands in for `pkexec` and the helper, so nothing asks for a password
//! or becomes root. The helper's own writes are tested in ox-core.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use ox_core::update::{Installation, PasswordlessUpdates};

use super::super::pages::{Category, SettingsView};
use super::super::row::SettingRow;
use super::super::SettingsPage;
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

const TITLE: &str = "Install updates without asking for a password";

const RULE: &str =
    "polkit.addRule(function (action, subject) {\n    return polkit.Result.NOT_HANDLED;\n});\n";

/// Stands in for `pkexec <helper> <flag>`: does what the helper does on
/// the fake system, or, while `cancel` exists, exits as pkexec does when
/// the prompt is dismissed. Each run is counted in `runs`.
const FAKE_PKEXEC: &str = r#"#!/bin/sh
root="$(dirname "$0")"
echo "$2" >> "$root/runs"
[ -e "$root/cancel" ] && exit 126
[ -e "$root/fail" ] && exit 1
rule="$root/etc/polkit-1/rules.d/60-io.winspace.Development.install-update.rules"
record="$root/var/lib/openxplorer/install-updates-without-password"
case "$2" in
    --enable-passwordless)
        mkdir -p "$(dirname "$record")"
        cp "$root/usr/share/openxplorer/install-update.rules" "$rule"
        echo on > "$record" ;;
    --disable-passwordless)
        mkdir -p "$(dirname "$record")"
        rm -f "$rule"
        echo off > "$record" ;;
    *) exit 2 ;;
esac
"#;

/// A fake stable Debian installation with the helper and the template.
struct FakeSystem {
    root: tempfile::TempDir,
}

impl FakeSystem {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a temporary folder");
        let write = |path: &str, text: &str, mode: u32| {
            let path = root.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        write(
            "opt/openxplorer/libexec/openxplorer-install-update",
            "fixture",
            0o755,
        );
        write("usr/share/openxplorer/install-update.rules", RULE, 0o644);
        write("pkexec", FAKE_PKEXEC, 0o755);
        std::fs::create_dir_all(root.path().join("etc/polkit-1/rules.d")).unwrap();
        Self { root }
    }

    fn setting(&self) -> PasswordlessUpdates {
        PasswordlessUpdates::detect(Installation::DebianPackage, self.root.path())
            .expect("the fake system has the setting")
            .with_launcher(self.path("pkexec"))
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn rule(&self) -> PathBuf {
        self.path("etc/polkit-1/rules.d/60-io.winspace.Development.install-update.rules")
    }

    fn runs(&self) -> Vec<String> {
        std::fs::read_to_string(self.path("runs"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What the package's postinst does by default: the rule in place,
    /// recorded on.
    fn install_rule(&self) {
        std::fs::copy(
            self.path("usr/share/openxplorer/install-update.rules"),
            self.rule(),
        )
        .unwrap();
        let record = self.path("var/lib/openxplorer/install-updates-without-password");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(record, "on\n").unwrap();
    }

    fn touch(&self, name: &str) {
        std::fs::write(self.path(name), "").unwrap();
    }
}

/// A window with Settings › About open, whose setting is `setting`.
struct AboutTest {
    test: TestWindow,
    page: SettingsPage,
    _fixture: Fixture,
}

impl AboutTest {
    fn open(setting: Option<PasswordlessUpdates>) -> Self {
        let fixture = Fixture::standard();
        let test = TestWindow::open_prepared(&fixture.uri(), |context| {
            context.use_passwordless_updates(setting);
        });
        test.window
            .open_settings(Some(SettingsView::Category(Category::About)));
        let page = descendants::<SettingsPage>(&test.window)
            .into_iter()
            .next()
            .expect("the window has a Settings page");
        Self {
            test,
            page,
            _fixture: fixture,
        }
    }

    fn row(&self) -> Option<SettingRow> {
        let rows = self.page.category_section(Category::About).rows();
        rows.into_iter().find(|row| row.text().title == TITLE)
    }

    fn switch(&self) -> gtk::Switch {
        let row = self.row().expect("About has the setting");
        descendants::<gtk::Switch>(&row)
            .into_iter()
            .next()
            .expect("the row has a switch")
    }

    /// Flips the switch as the user does, and waits for the helper.
    fn flip(&self, on: bool) {
        let switch = self.switch();
        switch.set_active(on);
        wait_until("the helper to finish", || switch.is_sensitive());
    }

    /// Goes to a folder tab and back to Settings, as the user does: the
    /// page reads what may have changed whenever it is shown.
    fn reopen(&self) {
        self.test.activate("new-tab", None);
        self.test.wait_for_listing("a folder tab");
        assert!(!self.page.is_mapped());
        self.test
            .window
            .open_settings(Some(SettingsView::Category(Category::About)));
        wait_until("Settings to show again", || self.page.is_mapped());
    }
}

/// A build without the update helper (RPM, Arch, Flatpak, the preview or a
/// source build, as these tests are) does not show the setting.
/// parity: UPD-003
#[gtk::test]
fn the_setting_is_hidden_where_the_build_has_no_update_helper() {
    let about = AboutTest::open(None);

    assert!(about.row().is_none());
    assert!(descendants::<SettingRow>(&about.page)
        .iter()
        .all(|row| row.text().title != TITLE));
}

/// It sits with the other update settings, and is on where the package's
/// postinst installed the rule, as it does by default.
/// parity: UPD-003
#[gtk::test]
fn the_setting_is_on_by_default_beside_the_update_settings() {
    let system = FakeSystem::new();
    system.install_rule();
    let about = AboutTest::open(Some(system.setting()));

    let row = about.row().expect("the stable package shows the setting");
    assert!(about.switch().is_active());
    assert!(about.switch().is_sensitive());
    let titles: Vec<&str> = about
        .page
        .category_section(Category::About)
        .rows()
        .iter()
        .map(|row| row.text().title)
        .collect();
    let position = |title| titles.iter().position(|shown| *shown == title).unwrap();
    assert_eq!(position(TITLE), position("Check for updates automatically") + 1);
    assert!(row.is_visible());
    assert!(system.runs().is_empty(), "showing the setting runs nothing");
}

/// Turning it off runs the helper once and shows off once the rule is
/// gone; turning it on again puts the rule back.
/// parity: UPD-003
#[gtk::test]
fn the_switch_removes_and_adds_the_rule() {
    let system = FakeSystem::new();
    system.install_rule();
    let about = AboutTest::open(Some(system.setting()));

    about.flip(false);

    assert!(!system.rule().exists());
    assert!(!about.switch().is_active());
    assert_eq!(system.runs(), ["--disable-passwordless"]);

    about.flip(true);

    assert_eq!(std::fs::read_to_string(system.rule()).unwrap(), RULE);
    assert!(about.switch().is_active());
    assert_eq!(system.runs(), ["--disable-passwordless", "--enable-passwordless"]);
}

/// A cancelled prompt puts the switch back and changes nothing.
/// parity: UPD-003
#[gtk::test]
fn a_cancelled_prompt_puts_the_switch_back() {
    let system = FakeSystem::new();
    system.install_rule();
    system.touch("cancel");
    let about = AboutTest::open(Some(system.setting()));

    about.flip(false);

    assert!(about.switch().is_active());
    assert!(system.rule().exists());
    assert_eq!(system.runs(), ["--disable-passwordless"]);
}

/// A helper that fails leaves the switch as the rule is, and says so.
#[gtk::test]
fn a_failed_change_shows_the_rule_as_it_is() {
    let system = FakeSystem::new();
    let about = AboutTest::open(Some(system.setting()));
    about.flip(true);
    system.touch("fail");

    about.flip(false);

    assert!(about.switch().is_active(), "the rule is still there");
    assert!(system.rule().exists());
}

/// The switch follows the file, not a remembered preference: a rule
/// added or removed elsewhere shows when Settings opens again.
/// parity: UPD-003
#[gtk::test]
fn the_switch_follows_the_rule_file() {
    let system = FakeSystem::new();
    std::fs::write(system.rule(), RULE).unwrap();
    let about = AboutTest::open(Some(system.setting()));
    assert!(about.switch().is_active());

    std::fs::remove_file(system.rule()).unwrap();
    about.reopen();
    assert!(!about.switch().is_active());

    std::fs::write(system.rule(), RULE).unwrap();
    about.reopen();
    assert!(about.switch().is_active());
    assert!(system.runs().is_empty(), "showing the rule changes nothing");
}

#[gtk::test]
fn the_helper_path_is_the_packages() {
    let system = FakeSystem::new();
    let command = system.setting().command(true);

    assert_eq!(
        Path::new(&command[1]),
        system.path("opt/openxplorer/libexec/openxplorer-install-update")
    );
}

/// A keyboard user keeps their place: after the helper, the switch still
/// has the keyboard and shows its focus ring.
#[gtk::test]
fn the_switch_keeps_its_focus_ring_after_a_change() {
    let system = FakeSystem::new();
    let about = AboutTest::open(Some(system.setting()));
    let switch = about.switch();
    switch.grab_focus();
    about.test.window.set_focus_visible(true);
    assert!(switch.state_flags().contains(gtk::StateFlags::FOCUS_VISIBLE));

    about.flip(true);

    assert!(switch.has_focus());
    assert!(about.test.window.gets_focus_visible());
    assert!(switch.state_flags().contains(gtk::StateFlags::FOCUS_VISIBLE));
}
