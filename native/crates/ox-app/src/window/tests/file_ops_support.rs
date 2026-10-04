// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers the file-operation tests share: finding the open dialog,
//! answering it, selecting items by name, pressing the file keys, and the
//! guard of every test that uses the Recycle Bin.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::dialog::Dialog;
use crate::test_support::harness::{descendants, wait_until, TestWindow};

/// The dialog open over `test`'s window, once it shows.
///
/// # Panics
///
/// When none opens in time.
pub(in crate::window) fn open_dialog(test: &TestWindow) -> Dialog {
    wait_until("a dialog to open", || dialog_over(test).is_some());
    dialog_over(test).expect("wait_until returned only once a dialog showed")
}

/// The dialog over `test`'s window, if one shows.
pub(super) fn dialog_over(test: &TestWindow) -> Option<Dialog> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<Dialog>().ok())
        .find(|dialog| {
            dialog.is_visible() && dialog.transient_for().as_ref() == Some(test.window.upcast_ref())
        })
}

/// Waits until no dialog shows over `test`'s window.
pub(in crate::window) fn wait_for_no_dialog(test: &TestWindow) {
    wait_until("the dialog to close", || dialog_over(test).is_none());
}

/// The dialog's first text field.
pub(super) fn text_field(dialog: &Dialog) -> gtk::Entry {
    descendants::<gtk::Entry>(dialog)
        .into_iter()
        .next()
        .expect("the dialog has a text field")
}

/// The field that edits a name in place in `test`'s view, once it shows.
pub(super) fn name_editor(test: &TestWindow) -> gtk::Entry {
    let find = || {
        descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
            .into_iter()
            .find(|field| field.has_css_class("rename-field"))
    };
    wait_until("the name to become editable", || find().is_some());
    find().expect("wait_until returned only once the field showed")
}

/// Whether `test`'s view edits a name in place.
pub(super) fn is_renaming_in_place(test: &TestWindow) -> bool {
    descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
        .iter()
        .any(|field| field.has_css_class("rename-field"))
}

/// Selects the items called `names` in `test`'s folder view.
pub(super) fn select_names(test: &TestWindow, names: &[&str]) {
    let model = test.window.folder_model();
    model.select_none();
    for position in 0..model.n_items() {
        let is_wanted = model
            .name_at(position)
            .is_some_and(|name| names.contains(&name.as_str()));
        if is_wanted {
            model.selection().select_item(position, false);
        }
    }
    assert_eq!(test.selected_names().len(), names.len(), "every name is listed");
}

/// Whether the window action `name` is enabled.
pub(super) fn is_enabled(test: &TestWindow, name: &str) -> bool {
    test.window
        .lookup_action(name)
        .is_some_and(|action| action.is_enabled())
}

/// Presses `keyval` with exactly `modifiers` while the folder view has
/// focus, as far as the window's shortcut controllers go: the shortcut
/// with that key runs. GTK has no public way to synthesise key events.
///
/// # Panics
///
/// When no shortcut of the window has that key.
pub(super) fn press_shortcut(test: &TestWindow, keyval: gdk::Key, modifiers: gdk::ModifierType) {
    test.window.folder_pane().focus_view();
    press_shortcut_where_focused(test, keyval, modifiers);
}

/// Presses `keyval` with exactly `modifiers` wherever keyboard focus is
/// now, as far as the window's shortcut controllers go. Returns whether
/// the window took the key.
///
/// # Panics
///
/// When no shortcut of the window has that key.
pub(super) fn press_shortcut_where_focused(
    test: &TestWindow,
    keyval: gdk::Key,
    modifiers: gdk::ModifierType,
) -> bool {
    let shortcut = window_shortcuts(test)
        .into_iter()
        .find(|shortcut| {
            let trigger = shortcut.trigger();
            trigger.is_some_and(|trigger| is_triggered_by(&trigger, keyval, modifiers))
        })
        .expect("the window has a shortcut for the key");
    let action = shortcut.action().expect("every window shortcut has an action");
    action.activate(gtk::ShortcutActionFlags::empty(), &test.window, None)
}

/// Every shortcut of the window's own shortcut controllers.
pub(in crate::window) fn window_shortcuts(test: &TestWindow) -> Vec<gtk::Shortcut> {
    let window = test.window.upcast_ref::<gtk::Widget>();
    let mut shortcuts = shortcuts_of(window, gtk::PropagationPhase::Capture);
    shortcuts.extend(shortcuts_of(window, gtk::PropagationPhase::Bubble));
    shortcuts
}

/// Every shortcut of `widget`'s own shortcut controllers that run in
/// `phase`.
pub(super) fn shortcuts_of(widget: &gtk::Widget, phase: gtk::PropagationPhase) -> Vec<gtk::Shortcut> {
    let controllers: Vec<gtk::ShortcutController> = widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .filter(|controller| controller.propagation_phase() == phase)
        .collect();
    // A shortcut controller lists its shortcuts as plain objects.
    controllers
        .iter()
        .flat_map(|controller| controller.iter::<glib::Object>())
        .filter_map(Result::ok)
        .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
        .collect()
}

/// Whether `trigger`, or one of its alternatives, is `keyval` with exactly
/// `modifiers`.
pub(in crate::window) fn is_triggered_by(
    trigger: &gtk::ShortcutTrigger,
    keyval: gdk::Key,
    modifiers: gdk::ModifierType,
) -> bool {
    if let Some(alternatives) = trigger.downcast_ref::<gtk::AlternativeTrigger>() {
        return is_triggered_by(&alternatives.first(), keyval, modifiers)
            || is_triggered_by(&alternatives.second(), keyval, modifiers);
    }
    trigger
        .downcast_ref::<gtk::KeyvalTrigger>()
        .is_some_and(|key| key.keyval() == keyval && key.modifiers() == modifiers)
}

/// Panics unless the Recycle Bin is the private one of this test run, as
/// `native/tools/check.py` sets it up: a test must never fill or empty the
/// user's own. The same check as ox-core's Recycle Bin tests.
pub(super) fn require_private_trash() {
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let temp = std::env::temp_dir();
    let is_private = data_home.as_ref().is_some_and(|folder| folder.starts_with(&temp));
    assert!(
        is_private,
        "Recycle Bin tests only run with a private XDG_DATA_HOME below {}; found {data_home:?}",
        temp.display()
    );
    assert_eq!(
        Some(glib::user_data_dir()),
        data_home,
        "GLib uses the private data folder"
    );
    let schemes = gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "trash"),
        "this test needs GVfs with its Trash backend; found {schemes:?}"
    );
}
