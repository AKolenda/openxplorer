// SPDX-License-Identifier: AGPL-3.0-only
//! Icons read at run time from Windows icon files the user chooses
//! (LOOK-030).
//!
//! `OpenXplorer` ships no Windows artwork. When the user turns on "Icons from
//! Windows icon files" in Settings and picks files they already have, such
//! as `imageres.dll.mun` and `shell32.dll.mun` from a Windows installation,
//! this module reads the icon groups those files hold ([`pe`]), decodes the
//! images ([`picture`]) and keeps them as PNG files in the user's cache
//! folder ([`cache`]), from where the app draws them in place of its own
//! icons for the places and items [`IconRole`] names. Only the paths are
//! saved in the settings file.
//!
//! Every file is untrusted input: each offset is checked against the file
//! before it is read, every count and size is capped, and a file that
//! breaks a rule gives no icon rather than an error the user must handle.
//! The app then shows its own icon.

mod cache;
mod pe;
mod picture;
mod roles;
pub mod synthetic;

#[cfg(test)]
mod tests;

pub use cache::{cache_folder, load_icons, CachedImage, LoadedIcons, MAX_FILES};
pub use pe::{IconFile, IconFileError, IconImage};
pub use picture::{decode, encode_png, Decoded, Rgba};
pub use roles::{IconLibrary, IconRole, Provenance};
