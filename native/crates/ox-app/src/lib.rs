// SPDX-License-Identifier: AGPL-3.0-only
//! The native desktop application: its widgets, windows and application
//! lifetime.
//!
//! Only [`application::run`] is public; `main.rs` calls it. Every other
//! module is private to the crate, so the compiler reports anything the
//! app no longer uses. The GTK tests of the window live beside it in
//! `window::tests` and reach its internals directly.

pub mod application;
mod config;
mod folder_view;
mod history;
mod icons;
mod locations;
mod places;
mod settings_store;
mod shared;
mod text_size;
mod theme;
mod typeahead;
mod volumes;
mod window;

#[cfg(test)]
mod test_support;
