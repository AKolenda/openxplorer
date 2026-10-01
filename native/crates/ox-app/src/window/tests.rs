// SPDX-License-Identifier: AGPL-3.0-only
//! GTK tests of the browsing window.
//!
//! Each test opens a real window on GTK's test thread (`#[gtk::test]`),
//! through the shared [`harness`](crate::test_support::harness), and drives
//! it through its actions and methods. `native/tools/check.py` runs
//! them on a private X display and D-Bus session with a disposable home, so
//! they never touch the user's desktop, files or settings.

mod accessibility;
mod address_bar;
mod address_input;
mod archives;
mod captures;
mod chrome;
mod clipboard;
mod clipboard_interop;
mod closing;
mod command_bar;
mod context_menus;
mod details_preview;
mod devices;
mod drag_and_drop;
mod environment;
mod file_operations;
mod file_ops_captures;
mod file_ops_support;
mod folder_location;
mod folder_tree;
mod geometry;
mod history;
mod icons;
mod input;
mod item_dialogs;
mod landing_pages;
mod late_replies;
mod listing;
mod look;
mod middle_click;
mod narrow_windows;
mod network;
mod network_shares;
mod opening;
mod panes_layout;
mod recycle_bin;
mod renaming;
mod search;
mod search_options;
mod selection;
mod settings;
mod sidebar;
mod sidebar_layout;
mod support;
mod tab_commands;
mod tabs;
mod views;
mod worker_questions;
