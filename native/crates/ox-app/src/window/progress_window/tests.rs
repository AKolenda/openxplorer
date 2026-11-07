// SPDX-License-Identifier: AGPL-3.0-only
//! The progress window with real browsing windows on simulated
//! operations: no file is written.

use std::time::Duration;

use gtk::prelude::*;
use ox_core::transfer::{Cancellation, Progress, ProgressScope};

use super::ProgressWindow;
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
use crate::window::transfer_panel::{Action, TransferKind, TransferPanel, Verb};
use crate::window::BrowserWindow;

/// A report of the whole batch at `fraction`.
fn batch(fraction: f64) -> Progress {
    Progress {
        label: "Copy: Notes 2.txt (1/3)".to_owned(),
        fraction,
        scope: ProgressScope::Batch,
        bytes: None,
        items: None,
        name: None,
    }
}

/// A new action of `test`'s window, started and described as a copy of
/// `count` items of the fixture into its Documents folder.
fn start_copy(test: &TestWindow, fixture: &Fixture, count: usize) -> (TransferPanel, Cancellation) {
    let action = TransferPanel::for_window(&test.window);
    let cancel = Cancellation::new();
    action.start(TransferKind::Files, "Preparing copy…", cancel.clone());
    let uris: Vec<String> = (0..count)
        .map(|index| format!("{}/Item {index}.txt", fixture.uri()))
        .collect();
    let documents = format!("{}/Documents", fixture.uri());
    action.describe(Action::of(Verb::Copying, &uris, Some(&documents)).unwrap());
    (action, cancel)
}

/// The progress window of `test`'s application, made now, showing as
/// soon as an action starts.
fn window_of(test: &TestWindow) -> ProgressWindow {
    let window = ProgressWindow::of(&test.context);
    window.set_show_delay(Duration::ZERO);
    window
}

/// One action titles the window with its percentage; two stack in the
/// same window, titled "2 Running Actions", each with its own Cancel.
///
/// parity: OPS-019, OPS-025
#[gtk::test]
fn actions_stack_in_one_window_titled_by_their_progress() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);

    let (first, first_cancel) = start_copy(&test, &fixture, 1718);
    first.show_progress(&batch(0.44));
    let single = window.title().map(|title| title.to_string());
    let (second, second_cancel) = start_copy(&test, &fixture, 10);
    let several = window.title().map(|title| title.to_string());
    let actions = window.actions();
    first.press_cancel();

    assert_eq!(single.as_deref(), Some("44% complete"));
    assert_eq!(several.as_deref(), Some("2 Running Actions"));
    assert_eq!(actions, [first.clone(), second.clone()]);
    assert!(first_cancel.is_cancelled() && !second_cancel.is_cancelled());
    assert_eq!(
        first.header_text(),
        "Copying 1,718 items from Example projects to Documents"
    );
    first.finish();
    second.finish();
}

/// The window waits a moment before it shows, so a quick operation shows
/// nothing; it stays while an action runs and closes with the last one.
///
/// parity: OPS-019
#[gtk::test]
fn the_window_shows_after_a_moment_and_closes_with_its_last_action() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = ProgressWindow::of(&test.context);
    window.set_show_delay(Duration::from_millis(300));

    let (quick, _) = start_copy(&test, &fixture, 1);
    quick.finish();
    wait_for(Duration::from_millis(500));
    let quick_showed = window.is_visible();
    let window = ProgressWindow::of(&test.context);
    window.set_show_delay(Duration::from_millis(300));
    let (first, _) = start_copy(&test, &fixture, 1);
    let at_once = window.is_visible();
    wait_until("the window to show", || window.is_visible());
    let (second, _) = start_copy(&test, &fixture, 2);
    first.finish();
    let after_first = window.is_visible();
    second.finish();

    assert!(!quick_showed, "a quick operation shows nothing");
    assert!(!at_once, "the window waits a moment");
    assert!(after_first, "it stays while an action runs");
    assert!(
        test.context.progress_window().is_none(),
        "it closes with its last action"
    );
    assert!(!gtk::Window::list_toplevels().contains(window.upcast_ref()));
}

/// "More details" shows every action's details, becomes "Fewer details",
/// and is kept for the next window, across sessions in the settings.
///
/// parity: OPS-021
#[gtk::test]
fn more_details_applies_to_every_action_and_is_remembered() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (first, _) = start_copy(&test, &fixture, 1);
    let (second, _) = start_copy(&test, &fixture, 2);
    let before = (window.shows_details(), window.details_text());

    window.press_details();
    let after = (window.shows_details(), window.details_text());
    wait_until("the choice to be saved", || {
        test.context.settings_data().preferences.progress_details
    });
    first.finish();
    second.finish();
    let next = ProgressWindow::of(&test.context);
    let (third, _) = start_copy(&test, &fixture, 1);

    assert_eq!(before, (false, "More details".to_owned()));
    assert_eq!(after, (true, "Fewer details".to_owned()));
    assert!(next.shows_details(), "the next window opens with its details");
    third.finish();
}

/// Closing the window cancels every action, which then end and take the
/// window with them.
///
/// parity: OPS-022
#[gtk::test]
fn closing_the_window_cancels_every_action() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (first, first_cancel) = start_copy(&test, &fixture, 1);
    let (second, second_cancel) = start_copy(&test, &fixture, 2);
    wait_until("the window to show", || window.is_mapped());

    window.close();

    assert!(first_cancel.is_cancelled() && second_cancel.is_cancelled());
    assert_eq!(window.title().as_deref(), Some("2 Running Actions"));
    first.finish();
    second.finish();
    assert!(test.context.progress_window().is_none());
}

/// The window belongs to no browsing window and is not modal: the
/// desktop places it, as Wayland requires, and browsing goes on.
#[gtk::test]
fn the_window_has_no_parent_and_is_not_modal() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (action, _) = start_copy(&test, &fixture, 1);
    wait_until("the window to show", || window.is_visible());

    assert!(window.transient_for().is_none());
    assert!(!window.is_modal());
    assert!(
        window.application().is_none(),
        "Quit and the window list leave it out"
    );
    action.finish();
}

/// Clicking a folder of an action opens it in a new browsing window, as
/// in Windows.
///
/// parity: OPS-019
#[gtk::test]
fn a_folder_of_an_action_opens_in_a_new_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let _window = window_of(&test);
    let (action, _) = start_copy(&test, &fixture, 1);
    let documents = format!("{}/Documents", fixture.uri());
    let app = test
        .window
        .application()
        .expect("the test window has its application");

    action.follow_link(&documents);
    let shows_documents = || {
        app.windows()
            .into_iter()
            .filter_map(|window| window.downcast::<BrowserWindow>().ok())
            .find(|window| window != &test.window && window.current_uri().as_deref() == Some(&documents))
    };
    wait_until("a window on Documents", || shows_documents().is_some());

    action.finish();
    if let Some(opened) = shows_documents() {
        opened.close();
    }
}

/// A browsing window's operations are actions of the application's
/// progress window, not panels over its folder pane.
///
/// parity: OPS-019
#[gtk::test]
fn a_running_operation_is_an_action_of_the_progress_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let running = test.window.begin_operation("Preparing copy…");
    let window = test.context.progress_window();
    let panel = test.window.transfer_panel().clone();
    let shown_in = panel.root().and_downcast::<ProgressWindow>();
    test.window.end_operation();

    assert!(running.is_some());
    assert!(window.is_some());
    assert_eq!(shown_in, window);
    assert!(test.context.progress_window().is_none());
}

/// The window cannot be resized, as the Windows one cannot: it takes its
/// natural height whenever More details opens or closes and whenever an
/// action comes or goes, so the graph is never squashed.
///
/// parity: OPS-021
#[gtk::test]
fn the_window_takes_its_natural_height_for_its_details_and_actions() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (first, _) = start_copy(&test, &fixture, 1);
    wait_until("the window to show", || window.is_mapped());
    let settled = || {
        crate::test_support::harness::wait_for_frames(&window, 6);
        window.height()
    };
    let collapsed = settled();

    window.press_details();
    wait_until("the window to grow for the details", || {
        window.height() > collapsed + 60
    });
    let expanded = settled();
    window.press_details();
    wait_until("the window to shrink back", || window.height() == collapsed);
    let (second, _) = start_copy(&test, &fixture, 2);
    wait_until("the window to grow for a second action", || {
        window.height() > collapsed + 20
    });
    second.finish();
    wait_until("the window to shrink to one action", || {
        window.height() == collapsed
    });

    assert!(!window.is_resizable(), "the window keeps its own size");
    assert!(expanded > collapsed + 60, "{expanded} after {collapsed}");
    first.finish();
}

/// The window opens with no focus ring and takes no widget's focus, as
/// Windows shows none; Tab still reaches the folder links, whose ring the
/// keyboard shows.
///
/// parity: OPS-019
#[gtk::test]
fn the_window_opens_without_a_focus_ring_and_tab_reaches_the_links() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (action, _) = start_copy(&test, &fixture, 1);
    wait_until("the window to show", || window.is_mapped());
    crate::test_support::harness::wait_for_frames(&window, 4);

    let opened = (
        GtkWindowExt::focus(&window),
        window.property::<bool>("focus-visible"),
    );
    window.child_focus(gtk::DirectionType::TabForward);
    let tabbed = GtkWindowExt::focus(&window).and_downcast::<gtk::Label>();

    assert_eq!(opened, (None, false), "nothing has focus, no ring shows");
    let links = tabbed.expect("Tab reaches the description's links");
    assert!(links.label().contains("<a href="));
    action.finish();
}

/// More details and Fewer details are remembered without telling the
/// browsing windows that their places or settings changed, which made
/// every one of them redraw its sidebar and folder, so they flashed.
///
/// parity: OPS-021
#[gtk::test]
fn toggling_the_details_leaves_the_browsing_windows_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (action, _) = start_copy(&test, &fixture, 1);
    let announced = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = std::rc::Rc::clone(&announced);
    let handler = test
        .context
        .connect_places_changed(move || counter.set(counter.get() + 1));

    window.press_details();
    wait_until("the choice to be saved", || {
        ox_core::settings::Settings::open(&test.context.settings_directory())
            .data()
            .preferences
            .progress_details
    });
    wait_for(Duration::from_millis(200));

    assert_eq!(announced.get(), 0, "no window hears that its settings changed");
    assert!(test.context.settings_data().preferences.progress_details);
    test.context.disconnect(handler);
    action.finish();
}

/// A question about the taken name "Report.txt" in Documents.
fn taken_name(fixture: &Fixture) -> ox_core::ops::ConflictScan {
    use ox_core::ops::{ConflictScan, ItemFacts, NameConflict};
    ConflictScan {
        mode: ox_core::transfer::TransferMode::Copy,
        destination_folder: format!("{}/Documents", fixture.uri()),
        selection: Vec::new(),
        items: Vec::new(),
        conflicts: vec![NameConflict {
            incoming_uri: format!("{}/Report.txt", fixture.uri()),
            existing_uri: format!("{}/Documents/Report.txt", fixture.uri()),
            incoming: ItemFacts::default(),
            existing: ItemFacts::default(),
        }],
        merged_folders: Vec::new(),
    }
}

/// The shown button of `window` labelled `text`.
fn row(window: &ProgressWindow, text: &str) -> gtk::Button {
    crate::window::widget_tree::descendants::<gtk::Button>(window)
        .into_iter()
        .find(|button| {
            button.is_visible()
                && crate::window::widget_tree::descendants::<gtk::Label>(button)
                    .iter()
                    .any(|label| label.text() == text)
        })
        .unwrap_or_else(|| panic!("a button {text:?}"))
}

/// While the one action asks Replace or Skip Files, "More details" shows
/// greyed, as in Windows 11: worded "More details" even when remembered
/// open, out of Tab's reach, and the details stay hidden. Answered, the
/// toggle and the remembered choice come back with the progress.
///
/// parity: OPS-021, OPS-026
#[gtk::test]
fn replace_or_skip_greys_more_details_until_it_is_answered() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (action, _) = start_copy(&test, &fixture, 1);
    window.press_details();
    let scan = taken_name(&fixture);
    let asking = action.clone();
    let answered = std::rc::Rc::new(std::cell::Cell::new(false));
    let done = std::rc::Rc::clone(&answered);
    gtk::glib::MainContext::default().spawn_local(async move {
        asking.ask_about_conflicts(&scan).await;
        done.set(true);
    });
    wait_until("the question", || {
        window.title().as_deref() == Some("Replace or Skip Files")
    });

    let asked = (window.offers_details(), window.details_text());
    let details_hidden = !action.shows_details_now();
    let mut reached = Vec::new();
    GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
    for _ in 0..12 {
        window.child_focus(gtk::DirectionType::TabForward);
        if let Some(focused) = GtkWindowExt::focus(&window) {
            reached.push(focused);
        }
    }
    let toggle_reached = reached.iter().any(|widget| {
        widget
            .ancestor(gtk::Button::static_type())
            .unwrap_or_else(|| widget.clone())
            .has_css_class("progress-details-toggle")
    });
    row(&window, "Skip this file").emit_clicked();
    wait_until("the answer", || answered.get());

    assert_eq!(asked, (false, "More details".to_owned()), "greyed while asking");
    assert!(details_hidden, "no comparison under the question");
    assert!(!toggle_reached, "Tab passes the greyed toggle");
    assert!(window.offers_details(), "offered again with the progress");
    assert_eq!(window.details_text(), "Fewer details", "the remembered choice");
    assert!(action.shows_details_now(), "the details show again");
    action.finish();
}

/// Only Replace or Skip Files greys the toggle: the question about a full
/// drive leaves it as it was.
///
/// parity: OPS-021
#[gtk::test]
fn a_full_drive_leaves_more_details_offered() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let window = window_of(&test);
    let (action, _) = start_copy(&test, &fixture, 1);
    let texts = crate::window::transfer_panel::SpaceTexts {
        summary: "There is not enough space on USB Drive.".to_owned(),
        needed: "4.4 GB is needed to copy this item.".to_owned(),
        drive: Vec::new(),
    };
    let asking = action.clone();
    gtk::glib::MainContext::default().spawn_local(async move {
        asking.ask_about_space(&texts).await;
    });
    wait_until("the question", || action.asks_about_space());

    let offered = window.offers_details();
    action.press_space_answer(false);

    assert!(offered);
    action.finish();
}
