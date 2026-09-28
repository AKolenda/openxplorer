// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial transfer regressions against temporary files and simulated
//! devices. Each case file ports one group of Python tests and names them;
//! the shared doubles are in `transfer_support/`.
//!
//! | Case file | What it covers |
//! |---|---|
//! | `operations` | Copy, move and delete on local files |
//! | `containment` | Refusing to place a folder inside itself |
//! | `conflicts` | Skip, Keep both and moves into the item's own folder |
//! | `replace` | Replace: overwriting files and merging folders |
//! | `failures` | Failures injected at every step of a copy or replacement |
//! | `snapshots` | Previous-version (snapshot) protection |
//! | `modes` | Unix modes on backends without `chmod` |
//! | `devices` | Uploads to phones and moves on them |
//! | `same_device` | Copies within one phone |
//! | `device_replace` | Replace and publication races on phones |
//! | `device_cleanup` | Cleanup and verification after failed uploads |
//! | `mtp_adapter` | The production GIO adapter on a simulated phone |
//! | `gio_integration` | The engine over the production GIO adapter |
//! | `gio_engine` | File names that are not valid UTF-8 through GIO |
//! | `staging_cleanup` | Local staging cleanup through GIO |

mod transfer_support;

#[path = "transfer_cases/conflicts.rs"]
mod conflicts;
#[path = "transfer_cases/containment.rs"]
mod containment;
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
#[path = "transfer_cases/replace.rs"]
mod replace;
#[path = "transfer_cases/same_device.rs"]
mod same_device;
#[path = "transfer_cases/snapshots.rs"]
mod snapshots;
#[path = "transfer_cases/staging_cleanup.rs"]
mod staging_cleanup;
