// SPDX-License-Identifier: AGPL-3.0-only
//! Restore from the Recycle Bin onto a name taken again in the original
//! folder, as Windows 11 asks it: the progress window's "Replace or Skip
//! Files" question, under "Moving 1 item to <folder>", never the
//! "Operation result" dialog (OPS-041, OPS-026, OPS-028). Every file is a
//! fictional one of the fixture, in the test run's private Recycle Bin.

use std::fs;
use std::path::Path;
use std::time::Duration;

use gtk::gio;
use gtk::prelude::*;
use ox_core::location::TRASH_URI;
use ox_core::ops::{delete_from_recycle_bin, list_recycle_bin};
use ox_core::settings::PreferencesUpdate;
use ox_core::transfer::Cancellation;

use super::file_ops_support::{dialog_over, require_private_trash, select_names};
use super::name_conflicts::{button, question_window, texts};
use crate::test_support::harness::{
    capture, capture_dialog, wait_for, wait_until, Fixture, TestWindow, ThemeGuard,
};

/// What a fixture file holds; the file trashed and restored.
const TRASHED: &[u8] = b"Synthetic test data\n";
/// What the newer file of the same name holds.
const NEWER: &[u8] = b"A newer fictional file\n";

/// Moves `path` to the Trash, as another file manager would.
fn trash(path: &Path) {
    gio::File::for_path(path)
        .trash(None::<&gio::Cancellable>)
        .expect("the private Trash takes the file");
}

/// Writes each of `names` into the fixture, trashes it, and writes a newer
/// file under its name; then opens the Recycle Bin listing them.
fn trash_and_take_names(fixture: &Fixture, names: &[&str]) -> TestWindow {
    for name in names {
        fixture.write(name);
        trash(&fixture.path(name));
        fs::write(fixture.path(name), NEWER).expect("the newer file is written");
    }
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed files to be listed", || {
        let shown = test.names();
        names.iter().all(|name| shown.contains(&(*name).to_owned()))
    });
    test
}

/// What Tab focuses in the question `window`, from no focus, `presses`
/// times: each focused widget, or the button it is in.
fn focus_after_tabs(window: &impl IsA<gtk::Window>, presses: usize) -> Vec<gtk::Widget> {
    let window = window.upcast_ref::<gtk::Window>();
    GtkWindowExt::set_focus(window, None::<&gtk::Widget>);
    (0..presses)
        .map(|_| {
            window.child_focus(gtk::DirectionType::TabForward);
            let focused = GtkWindowExt::focus(window).expect("Tab focuses a widget");
            focused.ancestor(gtk::Button::static_type()).unwrap_or(focused)
        })
        .collect()
}

/// Deletes the Recycle Bin items that came from `fixture`, so the next
/// pass finds only its own.
fn forget_trashed_items_of(fixture: &Fixture) {
    let context = gtk::glib::MainContext::default();
    let items = context
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    let uris: Vec<String> = items
        .into_iter()
        .filter(|item| {
            item.original_path
                .as_ref()
                .is_some_and(|path| path.starts_with(fixture.root()))
        })
        .map(|item| item.uri)
        .collect();
    context
        .block_on(delete_from_recycle_bin(&uris, &Cancellation::new()))
        .expect("the test's own items are deleted");
}

/// Waits for the action to end, then checks that nothing was said over
/// the browsing window.
fn ends_without_a_report(test: &TestWindow) {
    wait_until("the action to end", || test.context.progress_window().is_none());
    wait_for(Duration::from_millis(300));
    assert!(dialog_over(test).is_none(), "no Operation result dialog");
}

/// Restore onto a taken name asks in the progress window, as a move
/// into the original folder; Skip leaves the item in the Recycle Bin and
/// says nothing; Restore again and Replace puts it back in place.
///
/// parity: OPS-041, OPS-026, OPS-028
#[gtk::test]
fn restoring_onto_a_taken_name_asks_replace_or_skip_files() {
    require_private_trash();
    let fixture = Fixture::standard();
    let name = "Quarterly report.txt";
    let test = trash_and_take_names(&fixture, &[name]);
    select_names(&test, &[name]);

    test.activate("restore", None);
    let window = question_window(&test);
    let shown = texts(&window);

    assert!(
        shown.contains(&"Moving 1 item to Example projects".to_owned()),
        "{shown:?}"
    );
    assert!(
        shown.contains(&format!("The destination already has a file named \"{name}\"")),
        "{shown:?}"
    );
    for row in [
        "Replace the file in the destination",
        "Skip this file",
        "Compare info for both files",
    ] {
        let row = button(&window, row);
        assert!(row.is_focusable() && row.can_focus(), "keyboard reachable");
    }
    assert!(
        dialog_over(&test).is_none(),
        "nothing is asked over the browsing window"
    );
    // Tab goes to the destination's link first, then down the answers.
    let order = focus_after_tabs(&window, 4);
    assert!(order[0].has_css_class("link"), "{order:?}");
    let answers = [
        "Replace the file in the destination",
        "Skip this file",
        "Compare info for both files",
    ]
    .map(|text| button(&window, text).upcast::<gtk::Widget>());
    assert_eq!(order[1..], answers);

    button(&window, "Skip this file").emit_clicked();
    ends_without_a_report(&test);
    assert_eq!(fs::read(fixture.path(name)).unwrap(), NEWER);
    test.window.refresh();
    wait_until("the skipped item to be listed still", || {
        !test.window.is_loading() && test.names().contains(&name.to_owned())
    });

    select_names(&test, &[name]);
    test.activate("restore", None);
    let window = question_window(&test);
    button(&window, "Replace the file in the destination").emit_clicked();
    ends_without_a_report(&test);
    assert_eq!(fs::read(fixture.path(name)).unwrap(), TRASHED);
}

/// Restore all items with two taken names asks once, in the plural, and
/// Replace the files in the destination puts both back.
///
/// parity: OPS-041, OPS-026, OPS-028
#[gtk::test]
fn restore_all_items_asks_once_about_every_taken_name() {
    require_private_trash();
    let fixture = Fixture::standard();
    let names = ["Budget plan.txt", "Meeting notes.txt"];
    let test = trash_and_take_names(&fixture, &names);

    test.activate("restore-all-items", None);
    let window = question_window(&test);
    let shown = texts(&window);

    assert!(
        shown.contains(&"The destination has 2 files with the same names".to_owned()),
        "{shown:?}"
    );
    button(&window, "Skip these files");
    button(&window, "Let me decide for each file");
    button(&window, "Replace the files in the destination").emit_clicked();
    ends_without_a_report(&test);
    for name in names {
        assert_eq!(fs::read(fixture.path(name)).unwrap(), TRASHED);
    }
}

/// The question, the File Conflict window and the result, in both
/// themes, for review against Windows 11: one taken name, then Restore
/// all items with two. Saves `native-restore-question-*.png`,
/// `native-restore-file-conflict-*.png`, `native-restore-result-*.png`,
/// `native-restore-all-question-*.png` and
/// `native-restore-all-result-*.png` into `$OX_NATIVE_CAPTURE_DIR`.
#[gtk::test]
fn restore_conflicts_are_captured_light_and_dark() {
    require_private_trash();
    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        let fixture = Fixture::standard();
        let one = "Report.txt - Shortcut";
        let test = trash_and_take_names(&fixture, &[one]);
        test.activate("theme", Some(theme));
        select_names(&test, &[one]);
        test.activate("restore", None);
        let window = question_window(&test);
        capture_dialog(&window, &format!("native-restore-question-{theme}.png"));
        if theme == "light" {
            focus_after_tabs(&window, 2);
            window.set_focus_visible(true);
            capture_dialog(&window, "native-restore-question-focus-light.png");
        }
        button(&window, "Compare info for both files").emit_clicked();
        let conflicts = super::name_conflicts::conflict_window(&window);
        wait_for(Duration::from_millis(1500));
        capture_dialog(&conflicts, &format!("native-restore-file-conflict-{theme}.png"));
        conflicts.close();
        ends_without_a_report(&test);
        test.window.refresh();
        wait_until("the cancelled item to be listed still", || {
            !test.window.is_loading() && test.names().contains(&one.to_owned())
        });
        select_names(&test, &[one]);
        test.activate("restore", None);
        button(&question_window(&test), "Replace the file in the destination").emit_clicked();
        ends_without_a_report(&test);
        test.activate("go-to", Some(&fixture.uri()));
        test.wait_for_listing("the original folder");
        select_names(&test, &[one]);
        capture(&test.window, &format!("native-restore-result-{theme}.png"));
        drop(test);
        // The replaced file is in the Recycle Bin now, and its name taken.
        forget_trashed_items_of(&fixture);

        let names = ["Budget plan.txt", "Meeting notes.txt"];
        let test = trash_and_take_names(&fixture, &names);
        test.activate("theme", Some(theme));
        test.activate("restore-all-items", None);
        let window = question_window(&test);
        capture_dialog(&window, &format!("native-restore-all-question-{theme}.png"));
        button(&window, "Skip these files").emit_clicked();
        ends_without_a_report(&test);
        test.window.refresh();
        wait_until("the skipped items to be listed still", || {
            let shown = test.names();
            !test.window.is_loading() && names.iter().all(|name| shown.contains(&(*name).to_owned()))
        });
        capture(&test.window, &format!("native-restore-all-result-{theme}.png"));
        test.activate("restore-all-items", None);
        button(&question_window(&test), "Replace the files in the destination").emit_clicked();
        ends_without_a_report(&test);
        drop(test);
        forget_trashed_items_of(&fixture);
    }
}

/// Whether every one of `wanted` is a shown text of `window`.
fn shows_all(window: &impl IsA<gtk::Widget>, wanted: &[&str]) -> bool {
    let shown = texts(window);
    wanted.iter().all(|text| shown.iter().any(|shown| shown == text))
}

/// "More details" stays as it was left, as in Windows: a question opened
/// with the details remembered open reads "Fewer details" and compares
/// both files (name, folders, dates and sizes) under it, never an empty
/// body; folded, it reads "More details" and shows none. Saves
/// `native-restore-question-expanded-*.png` and
/// `native-restore-question-collapsed-*.png`.
///
/// parity: OPS-021, OPS-026
#[gtk::test]
fn the_question_keeps_the_remembered_details_and_compares_both_files_in_them() {
    require_private_trash();
    let _theme = ThemeGuard::keep();
    for theme in ["light", "dark"] {
        let fixture = Fixture::standard();
        let name = "Report.txt - Shortcut";
        let test = trash_and_take_names(&fixture, &[name]);
        test.activate("theme", Some(theme));
        test.context.remember_preferences(PreferencesUpdate {
            progress_details: Some(true),
            ..PreferencesUpdate::default()
        });
        wait_until("the details to be remembered as open", || {
            test.context.settings_data().preferences.progress_details
        });
        select_names(&test, &[name]);
        let compared = [
            format!("Name: {name}"),
            "From Recycle Bin".to_owned(),
            "Already in Example projects".to_owned(),
            "20 bytes".to_owned(),
            "23 bytes".to_owned(),
        ];
        let compared: Vec<&str> = compared.iter().map(String::as_str).collect();

        test.activate("restore", None);
        let window = question_window(&test);

        assert!(window.shows_details(), "the remembered choice is kept");
        assert_eq!(window.details_text(), "Fewer details");
        assert!(shows_all(&window, &compared), "{:?}", texts(&window));
        button(&window, "Skip this file");
        capture_dialog(&window, &format!("native-restore-question-expanded-{theme}.png"));

        window.press_details();

        assert_eq!(window.details_text(), "More details");
        let folded = texts(&window);
        assert!(
            !folded.iter().any(|text| compared.contains(&text.as_str())),
            "{folded:?}"
        );
        capture_dialog(&window, &format!("native-restore-question-collapsed-{theme}.png"));

        window.press_details();
        assert_eq!(window.details_text(), "Fewer details");
        assert!(shows_all(&window, &compared), "{:?}", texts(&window));
        button(&window, "Skip this file").emit_clicked();
        ends_without_a_report(&test);
        drop(test);
        forget_trashed_items_of(&fixture);
    }
}

/// The user's steps: in a folder, Twin.txt is created and deleted, then
/// created again and deleted; in the Recycle Bin both Twin.txt items are
/// selected and restored. The second is asked about, and:
///
/// - Replace restores it over the first, which goes back to the Recycle
///   Bin, with no Operation result report;
/// - Skip leaves it in the Recycle Bin, with the first restored;
/// - Compare info, both kept, restores it as "Twin (2).txt".
///
/// parity: OPS-041, OPS-026, OPS-028
#[gtk::test]
fn replace_restores_the_second_of_two_twins_over_the_first() {
    require_private_trash();
    let name = "Twin.txt";
    let first: &[u8] = b"The first fictional twin\n";
    let second: &[u8] = b"The second fictional twin, longer\n";
    for answer in ["Replace", "Skip", "Keep both"] {
        let fixture = Fixture::standard();
        for contents in [first, second] {
            fs::write(fixture.path(name), contents).expect("Twin.txt is written");
            trash(&fixture.path(name));
        }
        let test = TestWindow::open(TRASH_URI);
        wait_until("both twins to be listed", || {
            test.names().iter().filter(|shown| *shown == name).count() == 2
        });
        let model = test.window.folder_model();
        model.select_none();
        for position in 0..model.n_items() {
            if model.name_at(position).as_deref() == Some(name) {
                model.selection().select_item(position, false);
            }
        }
        assert_eq!(test.selected_names(), [name, name]);

        test.activate("restore", None);
        let window = question_window(&test);
        let shown = texts(&window);
        assert!(
            shown.contains(&format!("The destination already has a file named \"{name}\"")),
            "{shown:?}"
        );
        // The first twin went back before the question, as Windows
        // restores the free names first; the second meets it.
        let first_back = fs::read(fixture.path(name)).expect("the first twin is back");
        assert!(first_back == first || first_back == second);
        let other = if first_back == first { second } else { first };
        let kept_both = fixture.path("Twin (2).txt");
        match answer {
            "Replace" => button(&window, "Replace the file in the destination").emit_clicked(),
            "Skip" => button(&window, "Skip this file").emit_clicked(),
            _ => {
                button(&window, "Compare info for both files").emit_clicked();
                let conflicts = super::name_conflicts::conflict_window(&window);
                let (incoming, existing) = conflicts.row_checks(0);
                incoming.set_active(true);
                existing.set_active(true);
                button(&conflicts, "Continue").emit_clicked();
                wait_until("both versions", || kept_both.is_file());
            }
        }
        ends_without_a_report(&test);

        let restored = fs::read(fixture.path(name)).expect("Twin.txt is restored");
        let in_bin = |count: usize| {
            let context = gtk::glib::MainContext::default();
            context
                .block_on(list_recycle_bin(&Cancellation::new()))
                .expect("a readable Recycle Bin")
                .into_iter()
                .filter(|item| item.original_path.as_deref() == Some(fixture.path(name).as_path()))
                .count()
                == count
        };
        match answer {
            "Replace" => {
                assert_eq!(restored, other, "the second replaced the first");
                assert!(!kept_both.exists());
                wait_until("the first twin to be in the Recycle Bin", || in_bin(1));
            }
            "Skip" => {
                assert_eq!(restored, first_back, "the first stays restored");
                wait_until("the second twin to stay in the Recycle Bin", || in_bin(1));
            }
            _ => {
                assert_eq!(restored, first_back);
                assert_eq!(fs::read(&kept_both).unwrap(), other, "the second is numbered");
                wait_until("the Recycle Bin to hold neither twin", || in_bin(0));
            }
        }
        drop(test);
        forget_trashed_items_of(&fixture);
    }
}
