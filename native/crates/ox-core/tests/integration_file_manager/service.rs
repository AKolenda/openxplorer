// SPDX-License-Identifier: AGPL-3.0-only
//! Owning the name, serving the object and answering calls. Ports
//! `BusTests` of `desktop/tests/test_v07.py`.

use ox_core::integration::{FileManagerMethod, RequestNotOpened, BUS_NAME};

use super::{remote_error_name, Fixture};

/// Ported from `desktop/tests/test_v07.py::BusTests::test_registration_paths_and_flags`
/// parity: INT-013
#[test]
fn enabling_owns_the_standard_name_and_serves_the_standard_path() {
    let mut fixture = Fixture::new(Ok(()));

    fixture.enable_and_wait();

    let service_name = fixture.service_connection_name();
    assert_eq!(fixture.owner_of(BUS_NAME), Some(service_name));
    let reply = fixture.call("ShowFolders", &["file:///tmp"], "");
    assert!(reply.is_ok(), "{reply:?}");
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_registration_paths_and_flags`
/// parity: INT-013
#[test]
fn enabling_takes_the_name_over_from_an_owner_that_allows_it() {
    let mut fixture = Fixture::new(Ok(()));
    let other_manager = fixture.bus.connect();
    let other_owner = fixture
        .context
        .with_thread_default(|| {
            gio::bus_own_name_on_connection(
                &other_manager,
                BUS_NAME,
                gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
                |_, _| {},
                |_, _| {},
            )
        })
        .expect("the context is free");
    let other_name = other_manager.unique_name().expect("a unique name").to_string();
    fixture.run_until(|fixture| fixture.owner_of(BUS_NAME).as_deref() == Some(other_name.as_str()));

    fixture.enable_and_wait();

    assert_eq!(
        fixture.owner_of(BUS_NAME),
        Some(fixture.service_connection_name())
    );
    gio::bus_unown_name(other_owner);
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_enable_idempotent`
/// parity: INT-013
#[test]
fn enabling_twice_registers_once() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    let context = fixture.context.clone();
    let second = context
        .with_thread_default(|| fixture.service.enable())
        .expect("the context is free");

    assert!(second.is_ok(), "{second:?}");
    assert!(fixture.call("ShowItems", &["file:///tmp/report.pdf"], "").is_ok());
    assert_eq!(fixture.received().len(), 1);
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_owner_status`
/// parity: INT-013, INT-016
#[test]
fn acquiring_and_releasing_the_name_is_reported() {
    let mut fixture = Fixture::new(Ok(()));
    assert!(!fixture.service.is_owned());

    fixture.enable_and_wait();
    let changes_after_enabling = fixture.ownership_changes.get();
    fixture.service.disable();

    assert!(changes_after_enabling >= 1);
    assert!(!fixture.service.is_owned());
    assert!(fixture.ownership_changes.get() > changes_after_enabling);
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_disable_releases`
/// parity: INT-013
#[test]
fn disabling_releases_the_name_and_the_object() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    fixture.service.disable();
    fixture.run_until(|fixture| fixture.owner_of(BUS_NAME).is_none());

    let reply = fixture.call("ShowItems", &["file:///tmp/report.pdf"], "");
    assert!(reply.is_err());
    assert!(fixture.received().is_empty());
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_showitems_dispatch`
/// parity: INT-013, INT-014, INT-023
#[test]
fn show_items_reaches_the_app_with_the_startup_id() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    let reply = fixture.call("ShowItems", &["file:///tmp/report.pdf"], "startup");

    assert!(reply.is_ok(), "{reply:?}");
    let received = fixture.received();
    assert_eq!(received.len(), 1);
    let (request, startup_id) = &received[0];
    assert_eq!(request.method(), FileManagerMethod::ShowItems);
    assert_eq!(request.uris(), ["file:///tmp/report.pdf"]);
    assert_eq!(startup_id, "startup");
}

/// parity: INT-023
#[test]
fn a_long_startup_id_is_cut_to_4096_characters() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();
    let startup_id = "é".repeat(5000);

    fixture
        .call("ShowFolders", &["file:///tmp"], &startup_id)
        .expect("reply");

    let received = fixture.received();
    assert_eq!(received[0].1.chars().count(), 4096);
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_bad_scheme_error_not_launch`
/// parity: INT-013
#[test]
fn an_unsupported_location_is_refused_without_reaching_the_app() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    let refused = fixture.call("ShowFolders", &["https://example.test"], "x");

    let error = refused.expect_err("refused");
    assert_eq!(
        remote_error_name(&error).as_deref(),
        Some("org.freedesktop.DBus.Error.InvalidArgs")
    );
    assert!(fixture.received().is_empty());
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_bad_method_error`
/// parity: INT-013
#[test]
fn an_unknown_method_is_refused() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    let refused = fixture.call("Run", &["/tmp/x"], "");

    assert!(refused.is_err());
    assert!(fixture.received().is_empty());
}

/// Ported from `desktop/tests/test_v07.py::BusTests::test_no_request_executes_shell`
/// parity: INT-013
#[test]
fn a_path_that_looks_like_a_command_stays_an_escaped_location() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    fixture
        .call("ShowItems", &["/tmp/a; touch test"], "")
        .expect("reply");

    let received = fixture.received();
    let uri = &received[0].0.uris()[0];
    assert_eq!(uri, "file:///tmp/a%3B%20touch%20test");
}

/// parity: INT-013
#[test]
fn a_request_the_app_cannot_open_is_reported_as_failed() {
    let mut fixture = Fixture::new(Err(RequestNotOpened));
    fixture.enable_and_wait();

    let failed = fixture.call("ShowFolders", &["file:///tmp"], "");

    let error = failed.expect_err("failed");
    assert_eq!(
        remote_error_name(&error).as_deref(),
        Some("org.freedesktop.DBus.Error.Failed")
    );
    assert!(error
        .message()
        .contains("OpenXplorer could not open the requested location."));
}
