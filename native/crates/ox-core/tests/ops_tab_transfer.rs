// SPDX-License-Identifier: AGPL-3.0-only
//! Moving tabs between windows: every `TabTransferTests` case of
//! `v2.0.0:desktop/tests/test_rc3.py` that the typed broker can express.
//!
//! Two Python cases need no port: `test_non_integer_destination_rejected`
//! is ruled out by [`WindowId`]'s type, and `test_invalid_location_rejected`
//! checks `tab_snapshot` in `v2.0.0:desktop/window_state.py`, which validates the
//! tab's state when the app builds it, not when the broker moves it.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ox_core::ops::{
    Acceptance, Delivery, KeptReason, TabMessage, TabMoveOutcome, TabTransferError, TabTransferToken,
    TabTransfers, WindowId, MAX_PENDING_TAB_TRANSFERS,
};

/// The state a moving tab carries in these tests, like the Python
/// snapshot: its location, history position and selection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TabState {
    uri: String,
    history_index: usize,
    selection: Vec<String>,
}

/// Every message delivered, with the window it went to, oldest first.
type Delivered = Rc<RefCell<Vec<(WindowId, TabMessage<TabState>)>>>;

const SOURCE: WindowId = WindowId(1);
const DESTINATION: WindowId = WindowId(2);
const OTHER: WindowId = WindowId(3);

/// A broker over three ready windows, a clock the test moves and a record
/// of every message, like the `setUp` of the Python suite.
struct Hub {
    transfers: TabTransfers<TabState>,
    messages: Delivered,
    ready_windows: Rc<RefCell<HashSet<WindowId>>>,
    now: Rc<Cell<Instant>>,
}

impl Hub {
    fn new() -> Self {
        let messages = Rc::new(RefCell::new(Vec::new()));
        let ready_windows = Rc::new(RefCell::new(HashSet::from([SOURCE, DESTINATION, OTHER])));
        let now = Rc::new(Cell::new(Instant::now()));
        let delivered = Rc::clone(&messages);
        let ready = Rc::clone(&ready_windows);
        let clock = Rc::clone(&now);
        let transfers = TabTransfers::new(
            move |window| ready.borrow().contains(&window),
            move |window, message| {
                delivered.borrow_mut().push((window, message));
                Delivery::Delivered
            },
        )
        .with_clock(move || clock.get());
        Self {
            transfers,
            messages,
            ready_windows,
            now,
        }
    }

    /// Offers the tab `tab-1` of the source window.
    fn offer(&mut self) -> TabTransferToken {
        self.transfers
            .offer(SOURCE, "tab-1", snapshot())
            .expect("an offer")
    }

    /// Moves the clock forward by `seconds`.
    fn wait(&self, seconds: u64) {
        self.now.set(self.now.get() + Duration::from_secs(seconds));
    }

    /// The last message delivered.
    fn last_message(&self) -> (WindowId, TabMessage<TabState>) {
        self.messages
            .borrow()
            .last()
            .cloned()
            .expect("a delivered message")
    }

    /// The message delivered before the last.
    fn second_last_message(&self) -> (WindowId, TabMessage<TabState>) {
        let messages = self.messages.borrow();
        messages[messages.len() - 2].clone()
    }

    /// How the move ended for the source window, from its last message.
    fn source_outcome(&self) -> TabMoveOutcome {
        match self.last_message() {
            (SOURCE, TabMessage::Done { outcome, .. }) => outcome,
            other => panic!("the source was told how the move ended: {other:?}"),
        }
    }
}

/// The moving tab's state.
fn snapshot() -> TabState {
    TabState {
        uri: "smb://studio-nas/Projects".into(),
        history_index: 1,
        selection: vec!["smb://studio-nas/Projects/Readme.md".into()],
    }
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_capability_is_random_and_never_contains_a_uri`.
#[test]
fn the_capability_is_random_and_never_contains_a_location() {
    let mut hub = Hub::new();

    let first = hub.offer();
    hub.transfers.cancel(&first, KeptReason::Cancelled);
    let second = hub.offer();

    assert_ne!(first, second);
    assert_eq!(first.as_str().len(), 64);
    assert!(!first.as_str().contains("smb:"));
    assert_eq!(first.as_str().parse::<TabTransferToken>(), Ok(first.clone()));
    assert_eq!(
        "smb://nas".parse::<TabTransferToken>(),
        Err(TabTransferError::NotPending)
    );
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_claim_does_not_remove_source`.
#[test]
fn a_claim_sends_the_tab_but_does_not_remove_it_from_the_source() {
    let mut hub = Hub::new();
    let token = hub.offer();

    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    let messages = hub.messages.borrow();
    assert_eq!(messages.len(), 1);
    assert!(matches!(messages[0], (DESTINATION, TabMessage::Receive { .. })));
    assert!(hub.transfers.is_busy(SOURCE));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_ack_commits_once_to_original_source`.
#[test]
fn the_acknowledgement_commits_once_to_the_original_source() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers
        .claim(&token, DESTINATION, Some("existing-tab"))
        .unwrap();

    let outcome = hub
        .transfers
        .acknowledge(&token, DESTINATION, Acceptance::Accepted);

    assert_eq!(outcome, TabMoveOutcome::Committed);
    assert!(!hub.transfers.has_pending());
    assert_eq!(hub.source_outcome(), TabMoveOutcome::Committed);
    let settled = TabMessage::Settled {
        token,
        outcome: TabMoveOutcome::Committed,
    };
    assert_eq!(hub.second_last_message(), (DESTINATION, settled));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_receives_snapshot_position_and_selection`.
#[test]
fn the_destination_receives_the_tab_state_and_its_position() {
    let mut hub = Hub::new();
    let token = hub.offer();

    hub.transfers.claim(&token, DESTINATION, Some("first")).unwrap();

    let receive = TabMessage::Receive {
        token,
        tab: snapshot(),
        before_tab_id: Some("first".into()),
    };
    assert_eq!(hub.last_message(), (DESTINATION, receive));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_spoofed_ack_cannot_close_source`.
#[test]
fn an_acknowledgement_from_another_window_cannot_close_the_source_tab() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    let outcome = hub.transfers.acknowledge(&token, OTHER, Acceptance::Accepted);

    assert_eq!(outcome, TabMoveOutcome::Kept(KeptReason::NotPending));
    assert!(hub.transfers.is_busy(DESTINATION));
    assert_eq!(hub.messages.borrow().len(), 1);
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_unknown_token_rejected`.
#[test]
fn an_unknown_capability_is_refused() {
    let mut hub = Hub::new();
    let unknown: TabTransferToken = "0".repeat(64).parse().unwrap();

    let claimed = hub.transfers.claim(&unknown, DESTINATION, None);

    assert_eq!(claimed, Err(TabTransferError::NotPending));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_replayed_token_rejected`.
#[test]
fn a_used_capability_cannot_be_replayed() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();
    hub.transfers
        .acknowledge(&token, DESTINATION, Acceptance::Accepted);

    let replayed = hub.transfers.claim(&token, OTHER, None);

    assert_eq!(replayed, Err(TabTransferError::NotPending));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_claim_cannot_be_retargeted`.
#[test]
fn a_claimed_move_cannot_be_retargeted() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    let retargeted = hub.transfers.claim(&token, OTHER, None);

    assert_eq!(retargeted, Err(TabTransferError::NotPending));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_expiration_rolls_back_destination_and_keeps_source`.
#[test]
fn expiry_rolls_back_the_destination_and_keeps_the_source_tab() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    hub.wait(40);
    hub.transfers.expire();

    assert!(!hub.transfers.has_pending());
    assert_eq!(hub.source_outcome(), TabMoveOutcome::Kept(KeptReason::TimedOut));
    assert!(matches!(
        hub.second_last_message(),
        (DESTINATION, TabMessage::Settled { .. })
    ));
    assert_eq!(
        KeptReason::TimedOut.message(),
        "The tab move timed out. The original tab was kept."
    );
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_late_ack_after_timeout_never_commits`.
#[test]
fn an_acknowledgement_after_the_timeout_never_commits() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    hub.wait(40);
    let outcome = hub
        .transfers
        .acknowledge(&token, DESTINATION, Acceptance::Accepted);

    assert_eq!(outcome, TabMoveOutcome::Kept(KeptReason::NotPending));
    assert_eq!(hub.source_outcome(), TabMoveOutcome::Kept(KeptReason::TimedOut));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_negative_ack_keeps_source`.
#[test]
fn a_refusal_keeps_the_source_tab() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    let outcome = hub
        .transfers
        .acknowledge(&token, DESTINATION, Acceptance::Refused);

    assert_eq!(outcome, TabMoveOutcome::Kept(KeptReason::DestinationBusy));
    assert_eq!(
        hub.source_outcome(),
        TabMoveOutcome::Kept(KeptReason::DestinationBusy)
    );
    assert!(!hub.transfers.has_pending());
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_destination_close_rolls_back`.
#[test]
fn closing_the_destination_rolls_the_move_back() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    hub.transfers.window_closed(DESTINATION);

    assert!(!hub.transfers.has_pending());
    assert_eq!(
        hub.source_outcome(),
        TabMoveOutcome::Kept(KeptReason::WindowClosed)
    );
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_source_close_rolls_back`.
#[test]
fn closing_the_source_rolls_the_move_back() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    hub.transfers.window_closed(SOURCE);

    assert!(!hub.transfers.has_pending());
    assert!(!hub.transfers.is_busy(DESTINATION));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_unready_destination_not_claimed`.
#[test]
fn a_destination_that_is_not_ready_does_not_claim_the_move() {
    let mut hub = Hub::new();
    let token = hub.offer();
    hub.ready_windows.borrow_mut().remove(&DESTINATION);

    let claimed = hub.transfers.claim(&token, DESTINATION, None);

    assert_eq!(claimed, Err(TabTransferError::DestinationNotReady));
    assert!(!hub.transfers.is_busy(DESTINATION));
    hub.ready_windows.borrow_mut().insert(DESTINATION);
    assert_eq!(hub.transfers.claim(&token, DESTINATION, None), Ok(()));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_same_window_rejected_by_cross_window_hub`.
#[test]
fn a_tab_cannot_move_to_its_own_window() {
    let mut hub = Hub::new();
    let token = hub.offer();

    let claimed = hub.transfers.claim(&token, SOURCE, None);

    assert_eq!(claimed, Err(TabTransferError::DestinationNotReady));
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_concurrent_transfer_of_same_tab_rejected`.
#[test]
fn the_same_tab_cannot_move_twice_at_once() {
    let mut hub = Hub::new();
    hub.offer();

    let again = hub.transfers.offer(SOURCE, "tab-1", snapshot());

    assert_eq!(again, Err(TabTransferError::AlreadyMoving));
    assert_eq!(
        TabTransferError::AlreadyMoving.to_string(),
        "That tab is already moving. Wait for it to finish."
    );
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_no_credentials_or_arbitrary_js_forwarded`,
/// as far as the broker goes: it hands the destination exactly the tab
/// state it was offered. The Python broker also filtered that state
/// (`tab_snapshot` in `v2.0.0:desktop/window_state.py`); dropping passwords and
/// other fields is now the rule of the app's tab-state type, which must
/// carry this test's other half.
#[test]
fn only_the_tab_state_reaches_the_destination() {
    let mut hub = Hub::new();
    let token = hub.offer();

    hub.transfers.claim(&token, DESTINATION, None).unwrap();

    let (_, TabMessage::Receive { tab, .. }) = hub.last_message() else {
        panic!("the destination receives the tab");
    };
    assert_eq!(tab, snapshot());
}

/// Ported from `v2.0.0:desktop/tests/test_rc3.py::TabTransferTests::test_global_cap_is_bounded`.
#[test]
fn at_most_sixty_four_moves_wait_at_once() {
    let mut hub = Hub::new();
    for number in 0..MAX_PENDING_TAB_TRANSFERS {
        hub.transfers
            .offer(SOURCE, &number.to_string(), snapshot())
            .expect("room for another move");
    }

    let one_more = hub.transfers.offer(SOURCE, "more", snapshot());

    assert_eq!(one_more, Err(TabTransferError::AlreadyMoving));
}

#[test]
fn tab_identifiers_and_positions_are_bounded_and_the_source_must_be_ready() {
    let mut hub = Hub::new();
    let long_id = "t".repeat(81);

    let empty_id = hub.transfers.offer(SOURCE, "", snapshot());
    let too_long_id = hub.transfers.offer(SOURCE, &long_id, snapshot());
    hub.ready_windows.borrow_mut().remove(&SOURCE);
    let not_ready = hub.transfers.offer(SOURCE, "tab-1", snapshot());
    hub.ready_windows.borrow_mut().insert(SOURCE);
    let token = hub.offer();
    let bad_position = hub.transfers.claim(&token, DESTINATION, Some(&long_id));

    assert_eq!(empty_id, Err(TabTransferError::InvalidTabId));
    assert_eq!(too_long_id, Err(TabTransferError::InvalidTabId));
    assert_eq!(not_ready, Err(TabTransferError::SourceNotReady));
    assert_eq!(bad_position, Err(TabTransferError::InvalidPosition));
    assert!(!hub.transfers.is_busy(DESTINATION));
}

#[test]
fn a_destination_that_cannot_be_reached_rolls_the_move_back() {
    let sent_to_source = Rc::new(RefCell::new(Vec::new()));
    let source_messages = Rc::clone(&sent_to_source);
    let mut transfers = TabTransfers::new(
        |_| true,
        move |window, message: TabMessage<TabState>| {
            if window != SOURCE {
                return Delivery::WindowGone;
            }
            source_messages.borrow_mut().push(message);
            Delivery::Delivered
        },
    );
    let token = transfers.offer(SOURCE, "tab-1", snapshot()).unwrap();

    let claimed = transfers.claim(&token, DESTINATION, None);

    assert_eq!(claimed, Err(TabTransferError::DestinationUnreachable));
    assert!(!transfers.has_pending());
    let done = TabMessage::Done {
        token,
        tab_id: "tab-1".into(),
        outcome: TabMoveOutcome::Kept(KeptReason::DestinationUnreachable),
    };
    assert_eq!(*sent_to_source.borrow(), [done]);
}
