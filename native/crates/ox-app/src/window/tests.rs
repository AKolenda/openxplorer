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
mod compact_density;
mod context_menus;
mod details_pane;
mod details_preview;
mod details_tab;
mod devices;
mod dialog_keys;
mod disconnected;
mod drag_and_drop;
pub(super) mod empty_folder;
mod empty_folder_look;
mod environment;
mod expand_arrows;
mod explorer_keys;
mod file_name_extensions;
mod file_operations;
mod file_ops_captures;
pub(super) mod file_ops_support;
mod focus;
mod folder_location;
mod folder_options;
mod folder_tree;
mod geometry;
mod group_by;
mod history;
mod icons;
mod input;
mod item_check_boxes;
mod item_dialogs;
mod landing_pages;
mod late_replies;
mod listing;
mod look;
mod middle_click;
mod name_conflicts;
mod narrow_windows;
mod network;
mod network_shares;
mod new_folder;
mod new_text_document;
mod opening;
mod panes_layout;
mod pointer_feedback;
mod preview_pane;
mod properties_width;
mod recycle_bin;
mod renaming;
mod renaming_several;
mod right_drag;
mod search;
mod search_history;
mod search_options;
mod selection;
mod send_to;
mod settings;
mod share;
mod sidebar;
mod sidebar_layout;
mod sidebar_sections;
mod slow_locations;
mod split_view;
mod stopping;
mod support;
mod tab_commands;
mod tab_dragging;
mod tabs;
mod tiles_and_content;
pub(super) mod view_styles;
mod views;
mod worker_questions;
mod zip_folder;
