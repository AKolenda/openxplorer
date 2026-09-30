// SPDX-License-Identifier: AGPL-3.0-only
//! The native desktop application: its widgets, windows and application
//! lifetime.
//!
//! Only [`application::run`] is public; `main.rs` calls it. Every other
//! module is private to the crate, so the compiler reports anything the
//! app no longer uses. The GTK tests of the window live beside it in
//! `window::tests` and reach its internals directly.
//!
//! Since no other crate can reach an item here, a plain `pub` would claim
//! a visibility no item has; `unreachable_pub` makes the compiler ask for
//! `pub(crate)` or narrower on every item, so the declared visibility is
//! the real one.
#![warn(unreachable_pub)]

mod app_context;
pub mod application;
mod archive_view;
mod config;
mod devices;
mod dialog_layer;
mod dialogs;
mod folder_view;
mod history;
mod icons;
mod integration;
mod launcher_progress;
mod locations;
mod modal;
mod network;
mod operation_session;
mod places;
mod properties;
mod search;
mod settings_page;
mod settings_store;
mod snapshot;
mod text_size;
mod theme;
mod typeahead;
mod update;
mod volumes;
mod window;
mod write_inhibitor;

#[cfg(test)]
mod test_support;
