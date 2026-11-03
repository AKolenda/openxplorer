// SPDX-License-Identifier: AGPL-3.0-only
//! Stopping a slow listing from the button before the address bar.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{Fixture, TestWindow};
use crate::window::listing_state::ListingState;

/// The action the last button before the address bar runs.
fn last_button_action(test: &TestWindow) -> Option<String> {
    let row = &*test.window.imp().navigation_buttons;
    let button = row.last_child().and_downcast::<gtk::Button>()?;
    button.action_name().map(|name| name.to_string())
}

/// While a listing runs, Refresh is Stop from the first moment, with no
/// line over the items (VIEW-068); Stop ends the listing, says so, and
/// the button is Refresh again.
///
/// parity: VIEW-049
#[gtk::test]
fn stop_ends_a_slow_listing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .imp()
        .session
        .borrow_mut()
        .active_mut()
        .expect("an active tab")
        .listing_state = ListingState::Listing {
        listed_before: true,
        reload_pending: false,
    };
    test.window.update_content();
    assert_eq!(last_button_action(&test).as_deref(), Some("win.stop"));

    test.activate("stop", None);

    assert!(!test.window.is_loading());
    assert_eq!(last_button_action(&test).as_deref(), Some("win.refresh"));
    assert!(test.window.shown_message_text().starts_with("Stopped."));
}
