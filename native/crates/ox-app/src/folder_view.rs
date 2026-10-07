// SPDX-License-Identifier: AGPL-3.0-only
//! The folder listing: loading, models and the two views that show it.
//!
//! Ports the file list of `v2.0.0:desktop/ui/app.js` (`load`, `filtered`,
//! `renderColumns` and `renderRows`), `enumerate_folder` in
//! `v2.0.0:desktop/gio_backend.py` and `watch` in `v2.0.0:desktop/winspace.py`. Data
//! flows one way:
//!
//! - [`loader`] lists a folder in batches and [`watch`] reports changes;
//!   [`reconcile`] merges a reload into a tab's store of [`item`]s.
//! - [`model`] filters ([`filter`]) and sorts ([`sorting`]) the active
//!   tab's store and holds the selection both views share.
//! - [`details`] (with its [`column_titles`], [`column_keys`] and
//!   [`column_widths`]) and [`grid`] show the model through the
//!   [`cells`] they share.

pub(crate) mod cells;
pub(crate) mod column_keys;
pub(crate) mod column_titles;
pub(crate) mod column_widths;
pub(crate) mod details;
pub(crate) mod filter;
pub(crate) mod grid;
pub(crate) mod groups;
pub(crate) mod icon_size;
pub(crate) mod item;
pub(crate) mod keep_top;
pub(crate) mod loader;
pub(crate) mod model;
pub(crate) mod recent_locations;
pub(crate) mod reconcile;
pub(crate) mod sort_roles;
pub(crate) mod sorting;
pub(crate) mod tree;
pub(crate) mod watch;

pub(crate) use cells::CUSTOM_ICON;
