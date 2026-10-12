// SPDX-License-Identifier: AGPL-3.0-only
//! The new name of an item whose name conflict the user answers with
//! "Rename" (OPS-028).
//!
//! Dolphin's rename dialog has a field for the new name and "Suggest New
//! Name". The native dialog fills the field with the suggestion at once:
//! the first free `- Copy` name, the name Keep both would give. A typed
//! name must be valid ([`validate_name`]), differ from the item's own and
//! be free in the destination folder, and not one another item of the
//! same paste will take (an automatic `- Copy`, or an earlier new name);
//! the engine still refuses a name taken meanwhile rather than replacing
//! anything.

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;
use ox_core::location::{new_copy_name, validate_name, ItemKind};

/// How many `- Copy` names are tried for the suggestion.
const MAX_SUGGESTIONS: u32 = 100;

/// True when `folder` has a child called `name`, a link included.
async fn is_taken(folder: &gio::File, name: &str) -> bool {
    folder
        .child(name)
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .is_ok()
}

/// The first free `- Copy` name for the item at `uri` in
/// `destination_folder`, besides the names in `reserved` that other items
/// of the same paste will take, or the item's own name when none is found.
pub(crate) async fn suggested_name(uri: &str, destination_folder: &str, reserved: &[String]) -> String {
    let item = gio::File::for_uri(uri);
    let name = item
        .basename()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let is_folder = item
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .is_ok_and(|info| info.file_type() == gio::FileType::Directory);
    let kind = if is_folder {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let folder = gio::File::for_uri(destination_folder);
    for number in 1..MAX_SUGGESTIONS {
        let Ok(candidate) = new_copy_name(&name, number, kind) else {
            break;
        };
        if !reserved.contains(&candidate) && !is_taken(&folder, &candidate).await {
            return candidate;
        }
    }
    name
}

/// The name `typed` gives the item at `uri` in `destination_folder`, or
/// why it cannot; the names in `reserved` are taken by other items of the
/// same paste.
pub(super) async fn checked_new_name(
    typed: &str,
    uri: &str,
    destination_folder: &str,
    reserved: &[String],
) -> Result<String, String> {
    let name = typed.trim();
    validate_name(name).map_err(|error| error.to_string())?;
    let own_name = gio::File::for_uri(uri).basename();
    if own_name.is_some_and(|own| own.as_os_str() == name) {
        return Err(ox_core::i18n::gettext(
            "Enter a name that differs from the item's own name.",
        ));
    }
    let is_reserved = reserved.iter().any(|taken| taken == name);
    if is_reserved || is_taken(&gio::File::for_uri(destination_folder), name).await {
        return Err(ox_core::i18n::format_message(
            "“{name}” already exists here too. Enter another name.",
            &[("name", name)],
        ));
    }
    Ok(name.to_owned())
}
