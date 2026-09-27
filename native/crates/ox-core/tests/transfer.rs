// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial transfer regressions against temporary files and simulated devices.

mod transfer_support;

#[path = "transfer_cases/conflicts.rs"]
mod conflicts;
#[path = "transfer_cases/device_cleanup.rs"]
mod device_cleanup;
#[path = "transfer_cases/device_replace.rs"]
mod device_replace;
#[path = "transfer_cases/devices.rs"]
mod devices;
#[path = "transfer_cases/failures.rs"]
mod failures;
#[path = "transfer_cases/gio_engine.rs"]
mod gio_engine;
#[path = "transfer_cases/gio_integration.rs"]
mod gio_integration;
#[path = "transfer_cases/modes.rs"]
mod modes;
#[path = "transfer_cases/mtp_adapter.rs"]
mod mtp_adapter;
#[path = "transfer_cases/operations.rs"]
mod operations;
#[path = "transfer_cases/snapshots.rs"]
mod snapshots;
#[path = "transfer_cases/staging_cleanup.rs"]
mod staging_cleanup;
