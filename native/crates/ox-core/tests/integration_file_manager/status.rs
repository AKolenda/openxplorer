// SPDX-License-Identifier: AGPL-3.0-only
//! Who owns `org.freedesktop.FileManager1`, as the Settings card shows it.
//! Ports `FileManagerBus.status` of `v2.0.0:desktop/filemanager_bus.py`.

use std::fs;

use ox_core::integration::BUS_NAME;

use super::Fixture;

/// parity: INT-016
#[test]
fn the_status_names_openxplorer_while_it_owns_the_name() {
    let mut fixture = Fixture::new(Ok(()));
    fixture.enable_and_wait();

    let status = fixture.context.block_on(fixture.service.status());

    assert!(status.is_owned_by_openxplorer);
    assert_eq!(status.owner_label, "OpenXplorer");
    assert_eq!(status.owner, Some(fixture.service_connection_name()));
}

/// parity: INT-016
#[test]
fn the_status_names_another_owner_by_its_process() {
    let fixture = Fixture::new(Ok(()));
    let other_manager = fixture.bus.connect();
    let other_owner = fixture
        .context
        .with_thread_default(|| {
            gio::bus_own_name_on_connection(
                &other_manager,
                BUS_NAME,
                gio::BusNameOwnerFlags::NONE,
                |_, _| {},
                |_, _| {},
            )
        })
        .expect("the context is free");
    fixture.run_until(|fixture| fixture.owner_of(BUS_NAME).is_some());

    let status = fixture.context.block_on(fixture.service.status());

    assert!(!status.is_owned_by_openxplorer);
    let own_process_name = fs::read_to_string("/proc/self/comm").expect("comm");
    assert_eq!(status.owner_label, own_process_name.trim());
    gio::bus_unown_name(other_owner);
}

/// parity: INT-016
#[test]
fn the_status_is_empty_while_nobody_owns_the_name() {
    let fixture = Fixture::new(Ok(()));

    let status = fixture.context.block_on(fixture.service.status());

    assert_eq!(status.owner, None);
    assert_eq!(status.owner_label, "");
    assert!(!status.is_owned_by_openxplorer);
}
