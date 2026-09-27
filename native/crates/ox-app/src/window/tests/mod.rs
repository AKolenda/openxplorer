// SPDX-License-Identifier: AGPL-3.0-only
//! GTK tests of the browsing window.
//!
//! Each test opens a real window on GTK's test thread (`#[gtk::test]`),
//! through the shared [`harness`](crate::test_support::harness), and drives
//! it through its actions and methods. `native/tools/check.py` runs
//! them on a private X display and D-Bus session with a disposable home, so
//! they never touch the user's desktop, files or settings.

mod address_bar;
mod captures;
mod chrome;
mod command_bar;
mod environment;
mod geometry;
mod input;
mod landing_pages;
mod listing;
mod narrow_windows;
mod opening;
mod panes_layout;
mod sidebar_layout;
mod tabs;
mod views;
