// SPDX-License-Identifier: AGPL-3.0-only
//! GTK tests of the browsing window.
//!
//! Each test opens a real window on GTK's test thread (`#[gtk::test]`),
//! through the shared [`harness`](crate::test_support::harness), and drives
//! it through its actions and methods. `native/tools/check.py` runs
//! them on a private X display and D-Bus session with a disposable home, so
//! they never touch the user's desktop, files or settings.

mod address_bar;
mod archives;
mod captures;
mod chrome;
mod clipboard;
mod clipboard_interop;
mod command_bar;
mod context_menus;
mod drag_and_drop;
mod environment;
mod file_operations;
mod file_ops_captures;
mod file_ops_support;
mod geometry;
mod icons;
mod input;
mod item_dialogs;
mod landing_pages;
mod listing;
mod narrow_windows;
mod network;
mod opening;
mod panes_layout;
mod recycle_bin;
mod search;
mod settings;
mod sidebar_layout;
mod support;
mod tabs;
mod views;
mod worker_questions;
