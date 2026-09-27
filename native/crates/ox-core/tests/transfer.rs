// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial transfer regressions against temporary files and simulated devices.

mod transfer_support;

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
#[path = "transfer_cases/modes.rs"]
mod modes;
#[path = "transfer_cases/operations.rs"]
mod operations;
