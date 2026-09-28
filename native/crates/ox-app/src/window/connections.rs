// SPDX-License-Identifier: AGPL-3.0-only
//! What the window connects to its widgets when it is created, and what it
//! lets go of when it goes away.
//!
//! Ports the event wiring of `setup` in `desktop/ui/app.js`. The window
//! follows its widgets' own calls (the selection, the search box, the
//! address, activating an item, the skin, the history buttons) for as long
//! as it lives. Handlers it registers on objects that outlive it, the skin
//! every window shares, the application's signals and the volume monitor,
//! are kept in [`ExternalHandlers`] and disconnected in `dispose`, so a
//! closed window leaves nothing connected behind.

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
}

impl BrowserWindow {
    /// Follows the window's own widgets.
    pub(super) fn connect_signals(&self) {
        self.follow_selection();
        self.connect_filter();
        self.connect_address_entry();
        self.connect_view_activation();
        self.follow_skin();
        gestures::connect_history_buttons(
            self,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |direction| window.go_history(direction)
            ),
        );
    }

    /// Filters the folder as the user types in the search box.
    fn connect_filter(&self) {
        self.search_box().connect_query_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |query| {
                window.folder_pane().model().set_query(query);
                window.update_content();
            }
        ));
    }

    /// Lets go of what the window registered on objects that outlive it:
    /// the skin, places and volume handlers and the type-to-select timer.
    pub(super) fn disconnect_external_handlers(&self) {
        let handlers = self.imp().handlers.take();
        for handler in handlers.skin {
            self.skin().disconnect(handler);
        }
        for handler in [handlers.places, handlers.layout].into_iter().flatten() {
            self.context().disconnect(handler);
        }
        for handler in handlers.volumes {
            self.volume_monitor().disconnect(handler);
        }
        if let Some(timer) = self.imp().typeahead.borrow_mut().timer.take() {
            timer.remove();
        }
    }
}
