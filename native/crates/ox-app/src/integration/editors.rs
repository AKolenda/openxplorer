// SPDX-License-Identifier: AGPL-3.0-only
//! The "Open in <editor>" shortcuts of the item menu: Visual Studio Code,
//! Code Insiders and `VSCodium` when they are installed.
//!
//! Ports the `editors` of `environment` in `v2.0.0:desktop/winspace.py` and
//! `uniqueEditors` in `v2.0.0:desktop/ui/app.js` (OPEN-015); the rules are
//! ox-core's [`editor_shortcuts`]. Choosing one opens the item through
//! Open with ([`super::applications::launch`]), so the same checks apply.

use gtk::gio;
use ox_core::integration::editor_shortcuts;

pub(crate) use ox_core::integration::EditorShortcut;

/// The default text editor for a file of `content_type`, which the item
/// menu offers as "Edit in <editor>", as Windows offers "Edit in Notepad"
/// for a text file whatever opens it: for a type that is plain text (a
/// script or Markdown too), the default application for the type or for
/// plain text when it is a text editor, else the first text editor
/// registered for plain text ([`ox_core::integration::default_text_editor`]).
/// `None` for any other type.
pub(crate) fn text_editor_for(content_type: Option<&str>) -> Option<EditorShortcut> {
    const PLAIN_TEXT: &str = "text/plain";
    let content_type = content_type?;
    if !gio::content_type_is_a(content_type, PLAIN_TEXT) {
        return None;
    }
    let defaults = [content_type, PLAIN_TEXT]
        .into_iter()
        .filter_map(|kind| gio::AppInfo::default_for_type(kind, false));
    let others = gio::AppInfo::recommended_for_type(PLAIN_TEXT)
        .into_iter()
        .chain(gio::AppInfo::all_for_type(PLAIN_TEXT));
    ox_core::integration::default_text_editor(None, defaults.chain(others))
}

/// The installed code editors, one per visible name, read on a worker
/// thread: listing the applications reads every desktop file.
pub(crate) async fn editor_shortcuts_in_background() -> Vec<EditorShortcut> {
    let listed = gio::spawn_blocking(|| editor_shortcuts(gio::AppInfo::all())).await;
    listed.unwrap_or_default()
}
