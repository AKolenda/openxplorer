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

/// The installed code editors, one per visible name, read on a worker
/// thread: listing the applications reads every desktop file.
pub(crate) async fn editor_shortcuts_in_background() -> Vec<EditorShortcut> {
    let listed = gio::spawn_blocking(|| editor_shortcuts(gio::AppInfo::all())).await;
    listed.unwrap_or_default()
}
