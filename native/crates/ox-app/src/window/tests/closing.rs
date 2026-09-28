// SPDX-License-Identifier: AGPL-3.0-only
//! Closing a window: the caption's Close and the last tab ask first, and
//! no close cuts off a running write (`askClose` in `desktop/ui/app.js`,
//! `on_delete` in `desktop/winspace.py`).

use gtk::prelude::*;

use super::item_dialogs::{press, texts};
use crate::test_support::harness::{descendants, settle, Fixture, TestWindow};

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
    let dialog = test.wait_for_dialog("the refusal");
    assert_eq!(dialog.title(), "A file operation is running");
    assert!(texts(&dialog)
        .iter()
        .any(|text| text == "Cancel the operation and wait for its result before closing OpenXplorer."));
    assert!(test.window.is_visible(), "the window stays open");
    press(&dialog, "OK");
    test.window.end_operation();
}

/// Alt+F4, the dock and the shell close through the window manager, which
/// the window refuses with a toast while a write runs.
///
/// parity: TAB-049
#[gtk::test]
fn a_window_manager_close_is_refused_while_a_file_operation_runs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let operation = test.window.begin_operation("Preparing copy…");

    test.window.close();
    settle();

    assert!(operation.is_some());
    assert!(test.window.is_visible(), "the window stays open");
    assert_eq!(
        test.window.shown_message_text(),
        "A file operation is still finishing. Wait or cancel it before closing."
    );
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
    let dialog = test.wait_for_dialog("the refusal");
    assert_eq!(dialog.title(), "A file operation is running");
    assert_eq!(test.window.tab_count(), 1);
    press(&dialog, "OK");
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
