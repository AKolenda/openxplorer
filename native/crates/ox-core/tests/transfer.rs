// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial transfer regressions against temporary files and simulated devices.

mod transfer_support;

#[path = "transfer_cases/devices.rs"]
mod devices;
#[path = "transfer_cases/failures.rs"]
mod failures;
#[path = "transfer_cases/operations.rs"]
mod operations;
