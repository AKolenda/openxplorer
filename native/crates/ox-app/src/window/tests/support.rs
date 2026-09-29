// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers the window tests share: driving tabs as their widgets do,
//! saving a network location as the Python app would, finding the menus of
//! the command bar and the title bar, and reading the art on screen.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, Settings};

use crate::icons::{Art, ArtImage};
use crate::test_support::harness::{descendants, TestWindow};
use crate::window::menu_popover::MenuPopover;
use crate::window::session::TabId;
use crate::window::window_action::WindowAction;

impl TestWindow {
    /// The id of the tab in front, if any.
    pub(super) fn active_tab(&self) -> Option<TabId> {
        self.window.imp().session.borrow().active_id()
    }

    /// Shows tab `id`, as clicking it does.
    pub(super) fn activate_tab(&self, id: TabId) {
        self.run_tab_action(WindowAction::SelectTab, id);
    }

    /// Closes tab `id`, as its close button does.
    pub(super) fn activate_tab_close(&self, id: TabId) {
        self.run_tab_action(WindowAction::CloseTabById, id);
    }

    /// Each tab's location and whether it is listed again when shown.
    pub(super) fn tab_listing_needs(&self) -> Vec<(String, bool)> {
        let session = self.window.imp().session.borrow();
        let tabs = session.tabs().iter();
        tabs.map(|tab| (tab.uri().to_owned(), tab.listing_state.needs_listing()))
            .collect()
    }

    /// Saves `uri` as a network location called `label`, as the Python
    /// app would, and has the window read the settings again.
    pub(super) fn save_share(&self, uri: &str, label: &str) {
        let mut python_app = Settings::open(self.settings_directory());
        python_app
            .bookmark(
                BookmarkAction::Add,
                BookmarkKind::Share,
                &BookmarkRequest::new(uri, label),
            )
            .expect("the settings file takes a share");
        self.activate("refresh", None);
    }

    fn run_tab_action(&self, action: WindowAction, id: TabId) {
        let target = id.to_variant();
        WidgetExt::activate_action(&self.window, &action.detailed_name(), Some(&target))
            .expect("the window has the tab actions");
    }
}

/// The click gesture of `widget` that listens to `button`.
///
/// # Panics
///
/// When `widget` has none.
pub(super) fn click_gesture(widget: &impl IsA<gtk::Widget>, button: u32) -> gtk::GestureClick {
    let controllers = widget.observe_controllers();
    let gesture = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .find(|gesture| gesture.button() == button);
    gesture.unwrap_or_else(|| panic!("the widget has a gesture for button {button}"))
}

/// What `widget`'s gesture for `button` does for a press and, when
/// `release` says so, a release at (`x`, `y`). GTK has no public way to
/// synthesise pointer events, so this emits the gesture's signals, with
/// no modifier held.
pub(super) fn click_at(widget: &impl IsA<gtk::Widget>, button: u32, point: (f64, f64), release: Release) {
    let gesture = click_gesture(widget, button);
    let (x, y) = point;
    gesture.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
    if release == Release::Released {
        gesture.emit_by_name::<()>("released", &[&1_i32, &x, &y]);
    }
}

/// Whether a click in [`click_at`] lets go of the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Release {
    /// The button goes down and up: a click.
    Released,
    /// The button stays down.
    Held,
}

/// Middle-clicks `widget` at `point`, in its own coordinates.
pub(super) fn middle_click_at(widget: &impl IsA<gtk::Widget>, point: (f64, f64)) {
    click_at(widget, gdk::BUTTON_MIDDLE, point, Release::Released);
}

/// The middle of `widget` in the coordinates of `ancestor`.
///
/// # Panics
///
/// When `widget` is not laid out inside `ancestor`.
pub(super) fn middle_of(widget: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let bounds = widget
        .compute_bounds(ancestor)
        .expect("the widget is laid out inside its ancestor");
    let centre = bounds.center();
    (f64::from(centre.x()), f64::from(centre.y()))
}

/// The menu button in `test`'s window that has the CSS class `class`.
pub(super) fn menu_button_with_class(test: &TestWindow, class: &str) -> gtk::MenuButton {
    descendants::<gtk::MenuButton>(&test.window)
        .into_iter()
        .find(|button| button.has_css_class(class))
        .unwrap_or_else(|| panic!("the window has a .{class} menu button"))
}

/// The app menu `button` opens.
pub(super) fn app_menu(button: &gtk::MenuButton) -> MenuPopover {
    button
        .popover()
        .and_downcast::<MenuPopover>()
        .expect("the button opens an app menu")
}

/// What every [`ArtImage`] in `widget` shows.
pub(super) fn arts_in(widget: &impl IsA<gtk::Widget>) -> Vec<Art> {
    descendants::<ArtImage>(widget)
        .iter()
        .filter_map(ArtImage::art)
        .collect()
}

/// The [`ArtImage`] in `widget` that shows `art`, if any.
pub(super) fn art_image_showing(widget: &impl IsA<gtk::Widget>, art: Art) -> Option<ArtImage> {
    descendants::<ArtImage>(widget)
        .into_iter()
        .find(|image| image.art() == Some(art))
}
