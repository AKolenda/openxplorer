// SPDX-License-Identifier: AGPL-3.0-only
//! Drives, volumes and phones coming and going: every volume monitor
//! signal redraws every window, as the `mounts` events of
//! `v2.0.0:desktop/winspace.py` refresh the environment in `v2.0.0:desktop/ui/app.js`.
//!
//! A test cannot plug in a drive, so each signal is emitted on the
//! desktop's volume monitor without a drive, volume or mount; the window
//! reads the monitor itself and does not look at what the signal names.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::{BookmarkRequest, Settings};

use crate::test_support::harness::{wait_until, Fixture, TestWindow};
use crate::window::environment::VOLUME_MONITOR_SIGNALS;

/// No drive, volume or mount, as the value `signal` carries.
fn no_object_for(signal: &str) -> glib::Value {
    if signal.starts_with("mount-") {
        None::<gio::Mount>.to_value()
    } else if signal.starts_with("volume-") {
        None::<gio::Volume>.to_value()
    } else {
        None::<gio::Drive>.to_value()
    }
}

/// Every mount, volume and drive signal refreshes the sidebar of every
/// window: a pin another process saved meanwhile appears after each one,
/// as the environment is read again with the devices.
///
/// parity: DEV-001, DEV-002
#[gtk::test]
fn every_volume_monitor_signal_refreshes_every_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let beside = test.open_beside(&fixture.uri());
    let mut python_app = Settings::open(test.settings_directory());
    let monitor = gio::VolumeMonitor::get();

    for signal in VOLUME_MONITOR_SIGNALS {
        let label = format!("Pinned before {signal}");
        std::fs::create_dir(fixture.path(signal)).expect("a new folder");
        let pin = BookmarkRequest::new(fixture.uri_of(signal), &label);
        python_app
            .pin_many(&[pin], None, None)
            .expect("the settings file takes a pin");

        monitor.emit_by_name_with_values(signal, &[no_object_for(signal)]);

        for shown in [&test, &beside] {
            wait_until(&label, || shown.window.sidebar().labels().contains(&label));
        }
    }
}
