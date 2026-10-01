// SPDX-License-Identifier: AGPL-3.0-only
//! What the window connects to its widgets when it is created, and what it
//! lets go of when it goes away.
//!
//! Ports the event wiring of `setup` in `v2.0.0:desktop/ui/app.js`. The window
//! follows its widgets' own calls (the selection, the search box, the
//! address, activating an item, the skin, the history buttons) for as long
//! as it lives. Handlers it registers on objects that outlive it (the skin
//! every window shares, the application's signals, the display's clipboard,
//! the volume monitor and the search cache) are kept in [`ExternalHandlers`]
//! and disconnected in `dispose`, so a closed window leaves nothing
//! connected behind.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::gestures;
use super::BrowserWindow;

/// Handlers this window registered on objects that outlive it.
#[derive(Debug, Default)]
pub(super) struct ExternalHandlers {
    /// On the skin shared by every window: its appearance and text size
    /// signals.
    pub(super) skin: Vec<glib::SignalHandlerId>,
    /// On the application's `places-changed` signal.
    pub(super) places: Option<glib::SignalHandlerId>,
    /// On the application's `layout-reset` signal.
    pub(super) layout: Option<glib::SignalHandlerId>,
    /// On the volume monitor's mount and volume signals.
    pub(super) volumes: Vec<glib::SignalHandlerId>,
    /// On the application's `journal-changed` signal, which relabels Undo
    /// and Redo.
    pub(super) journal: Option<glib::SignalHandlerId>,
    /// On the display clipboard's `changed` signal, which enables Paste.
    pub(super) clipboard: Option<glib::SignalHandlerId>,
    /// On the shared search cache's status and contents signals.
    pub(super) search_cache: Vec<glib::SignalHandlerId>,
    /// On the application's updates, which the status bar shows.
    pub(super) updates: Option<glib::SignalHandlerId>,
}

impl BrowserWindow {
    /// Follows the window's own widgets.
    pub(super) fn connect_signals(&self) {
        self.follow_selection();
        self.connect_search();
        let search_cache = self.follow_search_cache();
        self.imp().handlers.borrow_mut().search_cache = search_cache;
        self.connect_address_entry();
        self.connect_view_activation();
        self.connect_drag_and_drop();
        self.connect_tab_drag_and_drop();
        self.follow_skin();
        self.install_view_zoom();
        self.folder_pane().connect_loading_line_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                window.update_status();
                window.show_stop_or_refresh();
            }
        ));
        gestures::connect_history_buttons(
            self,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |direction| window.go_history_from_mouse(direction)
            ),
        );
    }

    /// Lets go of what the window registered on objects that outlive it:
    /// the skin, places and volume handlers and the type-to-select timer.
    pub(super) fn disconnect_external_handlers(&self) {
        let handlers = self.imp().handlers.take();
        for handler in handlers.skin {
            self.skin().disconnect(handler);
        }
        for handler in [handlers.places, handlers.layout, handlers.journal]
            .into_iter()
            .flatten()
        {
            self.context().disconnect(handler);
        }
        if let Some(handler) = handlers.clipboard {
            self.clipboard().disconnect(handler);
        }
        for handler in handlers.volumes {
            self.volume_monitor().disconnect(handler);
        }
        for handler in handlers.search_cache {
            self.context().search_cache().disconnect(handler);
        }
        if let Some(handler) = handlers.updates {
            self.context().updates().disconnect(handler);
        }
        self.imp().typeahead.borrow_mut().stop_timer();
    }
}
