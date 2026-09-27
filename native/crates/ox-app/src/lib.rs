// SPDX-License-Identifier: AGPL-3.0-only
//! Native OpenXplorer widgets and the desktop application.
//!
//! Keeping the widgets in a library makes the restored components part of
//! every build and gives integration tests the same entry points as the app.

pub mod application;
mod config;
pub mod folder_view;
pub mod history;
pub mod icons;
pub mod locations;
pub mod text_size;
pub mod theme;
pub mod typeahead;
pub mod volumes;
pub mod window;
