// SPDX-License-Identifier: AGPL-3.0-only
//! Closing a window: every close asks first while a write runs, so no
//! close cuts off a running write (`askClose` in `v2.0.0:desktop/ui/app.js`,
//! `on_delete` in `v2.0.0:desktop/winspace.py`).

use gtk::prelude::*;

use super::file_ops_support::{open_dialog, wait_for_no_dialog};
use crate::test_support::harness::{descendants, settle, wait_until, Fixture, TestWindow};

/// The caption Close button, when the desktop's layout shows one.
fn caption_close(test: &TestWindow) -> Option<gtk::Button> {
    descendants::<gtk::Button>(&test.window)
        .into_iter()
        .find(|button| button.has_css_class("caption") && button.has_css_class("close"))
}

/// parity: TAB-047, TAB-049
#[gtk::test]
fn the_close_button_asks_while_a_file_operation_runs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");

    match caption_close(&test) {
        Some(close) => close.emit_clicked(),
        None => test.activate("close-window", None),
    }

    assert!(operation.is_some());
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "A file operation is running");
    assert_eq!(
        dialog.message_text(),
        "Cancel it and close this window once it has stopped? Items already finished stay where they are."
    );
    assert!(test.window.is_visible(), "the window stays open");
    dialog.press("Keep open");
    wait_for_no_dialog(&test);
    test.window.end_operation();
}

/// Alt+F4, the dock and the shell close through the window manager, which
/// asks the same question while a write runs.
///
/// parity: TAB-049
#[gtk::test]
fn a_window_manager_close_asks_while_a_file_operation_runs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");

    test.window.close();
    let dialog = open_dialog(&test);

    assert!(operation.is_some());
    assert_eq!(dialog.title_text(), "A file operation is running");
    assert!(test.window.is_visible(), "the window stays open");
    dialog.press("Keep open");
    wait_for_no_dialog(&test);
    test.window.end_operation();
    test.window.close();
    settle();
    assert!(!test.window.is_visible(), "an idle window closes");
}

/// parity: TAB-002, TAB-049
#[gtk::test]
fn closing_the_only_tab_closes_the_window_through_the_same_question() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");

    test.activate("close-tab", None);

    assert!(operation.is_some());
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "A file operation is running");
    assert_eq!(test.window.tab_count(), 1);
    dialog.press("Keep open");
    wait_for_no_dialog(&test);
    test.window.end_operation();
    test.activate("close-tab", None);
    settle();
    assert!(!test.window.is_visible(), "the last tab closed the window");
}

/// parity: TAB-047
#[gtk::test]
fn the_close_button_closes_an_idle_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let close = caption_close(&test);
    if let Some(close) = &close {
        assert_eq!(close.tooltip_text().as_deref(), Some("Close window"));
        close.emit_clicked();
    } else {
        test.activate("close-window", None);
    }
    settle();
    assert!(!test.window.is_visible());
}

/// The folder-watch threads of this process that run now.
fn folder_watch_threads() -> usize {
    let tasks = std::fs::read_dir("/proc/self/task").expect("Linux lists a process's threads");
    let names = tasks
        .filter_map(Result::ok)
        .filter_map(|task| std::fs::read_to_string(task.path().join("comm")).ok());
    names.filter(|name| name.trim() == "folder-watch").count()
}

/// Closing a window stops its running listings and its folder watches,
/// even while something still holds the window. Its sign-ins end too: see
/// `closing_the_window_aborts_its_sign_ins` in network.rs.
///
/// parity: TAB-050
#[gtk::test]
fn closing_a_window_stops_its_listings_and_folder_watches() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    wait_until("both folder watches", || folder_watch_threads() >= 2);
    let watching = folder_watch_threads();
    test.window.refresh();
    assert!(test.window.is_loading(), "a listing runs");

    test.window.close();
    settle();

    assert!(!test.window.is_loading(), "the listing stopped");
    wait_until("the window's folder watches to stop", || {
        folder_watch_threads() <= watching - 2
    });
}
