// SPDX-License-Identifier: AGPL-3.0-only
//! What Copy and Cut put on the desktop's clipboard outlives the last
//! window (CLIP-010).
//!
//! Windows Explorer's process outlives its windows, so files copied in a
//! window that was closed afterwards still paste in any application. An X11
//! or Wayland clipboard is served by the process that owns it and empties
//! when that process ends, unless a clipboard manager keeps a copy: KDE's
//! Klipper keeps a file list, but GNOME Shell keeps only text and pictures,
//! and Weston or a plain X11 session keep nothing. GTK asks a manager to
//! store the content when the application quits (`gtk_main_sync`), which
//! needs one that speaks X11's `SAVE_TARGETS`.
//!
//! So while this application owns the clipboard, the [`Keeper`] holds the
//! application ([`gio::ApplicationHoldGuard`]): closing the last window
//! leaves it running, without a window, to answer pastes. It lets go as
//! soon as another application takes the clipboard, or this one empties it
//! (a cut pasted in full, CLIP-008), and the application then ends as it
//! would have when the window closed. Quit (`app.quit`, `--quit`) still ends
//! it at once.
//!
//! The context also remembers the items the pastes of the latest cut moved
//! ([`MovedCut`]), so a cut a clipboard manager puts back after the paste
//! emptied the clipboard is recognized as one already done (CLIP-008).

use std::collections::HashSet;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::clipboard::{ClipboardFiles, ClipboardMode};

use super::AppContext;

/// The hold on the application while it owns `clipboard`, and the handler
/// that ends it when the owner changes.
#[derive(Debug)]
pub(crate) struct Keeper {
    /// Keeps the application running without a window.
    _hold: gio::ApplicationHoldGuard,
    /// The display's clipboard this application owns.
    clipboard: gdk::Clipboard,
    /// Watches it for another owner.
    changed: Option<glib::SignalHandlerId>,
}

impl Drop for Keeper {
    fn drop(&mut self) {
        if let Some(handler) = self.changed.take() {
            self.clipboard.disconnect(handler);
        }
    }
}

/// The items the pastes of one cut moved, by that cut's token.
#[derive(Debug, Default)]
pub(crate) struct MovedCut {
    /// The token of the cut the items were moved from.
    token: String,
    /// Their addresses before the move.
    uris: HashSet<String>,
}

impl AppContext {
    /// Keeps `application` running while it owns `clipboard`, which a
    /// window has just filled. Nothing changes when it already does.
    pub(crate) fn serve_clipboard(&self, application: &gtk::Application, clipboard: &gdk::Clipboard) {
        if !clipboard.is_local() {
            return;
        }
        if self.imp().clipboard_keeper.borrow().is_some() {
            return;
        }
        let changed = clipboard.connect_changed(glib::clone!(
            #[weak(rename_to = context)]
            self,
            move |clipboard| {
                // A change of this application's own content keeps it
                // serving; another owner, or an empty clipboard, ends it.
                if !clipboard.is_local() {
                    context.stop_serving_clipboard();
                }
            }
        ));
        let keeper = Keeper {
            _hold: application.hold(),
            clipboard: clipboard.clone(),
            changed: Some(changed),
        };
        self.imp().clipboard_keeper.replace(Some(keeper));
    }

    /// Lets the application end once it has no window, as the clipboard
    /// no longer needs it.
    pub(crate) fn stop_serving_clipboard(&self) {
        // Taken out before it drops, so its handler is disconnected and its
        // hold released with no borrow of the context left open.
        let keeper = self.imp().clipboard_keeper.take();
        drop(keeper);
    }

    /// Whether the application keeps running to serve the clipboard.
    #[cfg(test)]
    pub(crate) fn serves_clipboard(&self) -> bool {
        self.imp().clipboard_keeper.borrow().is_some()
    }

    /// Notes that a paste of the cut with `token` moved `moved`, which
    /// the paste has just taken off the clipboard. The pastes of one cut
    /// add up; a new cut starts afresh.
    pub(crate) fn remember_moved_cut(&self, token: &str, moved: &[String]) {
        let mut moved_cut = self.imp().moved_cut.borrow_mut();
        if moved_cut.token != token {
            token.clone_into(&mut moved_cut.token);
            moved_cut.uris.clear();
        }
        moved_cut.uris.extend(moved.iter().cloned());
    }

    /// Whether `files` is a cut of nothing but items the pastes of the
    /// latest cut moved away: the cut put back by a clipboard manager.
    pub(crate) fn is_moved_cut(&self, files: &ClipboardFiles) -> bool {
        let moved_cut = self.imp().moved_cut.borrow();
        files.mode() == ClipboardMode::Cut
            && !moved_cut.uris.is_empty()
            && files.uris().iter().all(|uri| moved_cut.uris.contains(uri))
    }

    /// Forgets the moved items once the clipboard holds something new.
    pub(crate) fn forget_moved_cut(&self) {
        self.imp().moved_cut.replace(MovedCut::default());
    }
}
