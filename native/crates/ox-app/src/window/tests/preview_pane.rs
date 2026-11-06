// SPDX-License-Identifier: AGPL-3.0-only
//! The Preview pane (VIEW-070): Windows 11's View > Preview pane, which
//! shows the selection's preview alone in the details pane's place, and
//! which the Details pane replaces, as in Windows.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::geometry::laid_out_with_details_pane;
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::menu_popover::MenuPopover;
use crate::window::window_action::WindowAction;

/// Whether `action`, a pane toggle, is on.
fn is_on(test: &TestWindow, action: WindowAction) -> bool {
    test.window
        .window_action_state(action)
        .and_then(|state| state.get::<bool>())
        .unwrap_or_default()
}

/// The labels the View menu marks now.
fn view_marks(test: &TestWindow) -> Vec<String> {
    let button = super::support::menu_button_with_class(test, "view-command");
    let menu = button
        .popover()
        .and_downcast::<MenuPopover>()
        .expect("View's menu");
    menu.popup();
    wait_until("the View menu", || menu.is_visible());
    let marked = menu.checked_labels();
    menu.popdown();
    wait_until("the View menu to close", || !menu.is_visible());
    marked
}

/// View > Preview pane shows the preview alone, in the pane the Details
/// pane uses, and closes the Details pane; View > Details pane closes it
/// again. Each has the dot while it is open, and the choice is saved.
///
/// parity: VIEW-070
#[gtk::test]
fn the_preview_pane_and_the_details_pane_take_turns_in_one_pane() {
    let fixture = Fixture::standard();
    let test = laid_out_with_details_pane(&fixture.uri());
    test.select_named("Notes 2.txt");
    let pane = test.window.imp().details_pane.get();
    assert!(pane.is_visible());
    assert!(pane.shows_properties(), "the Details pane lists properties");
    assert!(view_marks(&test).contains(&"Details pane".to_owned()));

    test.activate("preview-pane", None);

    wait_for_frames(&test.window, 2);
    assert!(is_on(&test, WindowAction::PreviewPane));
    assert!(!is_on(&test, WindowAction::DetailsPane));
    assert!(pane.is_visible(), "the same pane");
    assert!(pane.shows_preview(), "with the preview");
    assert!(!pane.shows_properties(), "and nothing else");
    assert_eq!(pane.title_text(), "Preview");
    assert_eq!(pane.close_tooltip(), "Close preview pane");
    let marks = view_marks(&test);
    assert!(marks.contains(&"Preview pane".to_owned()), "{marks:?}");
    assert!(!marks.contains(&"Details pane".to_owned()), "{marks:?}");
    wait_until("the choice to be saved", || {
        let saved = test.context.settings_data().preferences;
        saved.preview_pane && !saved.show_details_pane
    });

    test.activate("details-pane", None);

    assert!(is_on(&test, WindowAction::DetailsPane));
    assert!(!is_on(&test, WindowAction::PreviewPane));
    assert!(pane.shows_properties());
    assert_eq!(pane.title_text(), "Details");
    assert_eq!(pane.close_tooltip(), "Close details pane");

    test.activate("details-pane", None);
    assert!(!pane.is_visible(), "closing the open pane closes it");
    test.activate("preview-pane", None);
    test.activate("preview-pane", None);
    assert!(!pane.is_visible());
    assert!(!is_on(&test, WindowAction::DetailsPane) && !is_on(&test, WindowAction::PreviewPane));
}

/// A window opens with the pane the user chose last: the Preview pane
/// when it was saved open.
///
/// parity: VIEW-070
#[gtk::test]
fn a_new_window_opens_the_preview_pane_when_it_was_open() {
    let fixture = Fixture::standard();
    let first = TestWindow::open(&fixture.uri());
    first.activate("preview-pane", None);
    wait_until("the choice to be saved", || {
        first.context.settings_data().preferences.preview_pane
    });
    let second = first.open_beside(&fixture.uri());
    assert!(is_on(&second, WindowAction::PreviewPane));
    assert!(!is_on(&second, WindowAction::DetailsPane));
    assert!(!second.window.imp().details_pane.shows_properties());
}
