// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers the file-operation tests share: finding the open dialog,
//! answering it, selecting items by name, pressing the file keys, and the
//! guard of every test that uses the Recycle Bin, and the check that a
//! finished operation says nothing.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use ox_core::settings::{PreferencesUpdate, Settings};

use crate::dialog::Dialog;
use crate::test_support::harness::{descendants, settle, wait_until, TestWindow};

/// Turns on "Moving items to the Recycle Bin" under Settings > Confirmations
/// (Windows' "Display delete confirmation dialog"), off by default as in
/// Windows 11, so Delete asks first.
pub(in crate::window) fn ask_before_recycling(test: &TestWindow) {
    let update = PreferencesUpdate {
        confirm_trash: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context.settings_data().preferences.confirm_trash
    });
}

/// The question Windows Explorer asks before a rename changes a file's
/// extension.
pub(in crate::window) const EXTENSION_QUESTION: &str =
    "If you change a file name extension, the file might become unusable.\nAre you sure you want to change it?";

/// Whether `dialog` is the question asked before a rename changes a
/// file's extension. Its title, "Rename", is also the Rename dialog's, so
/// the question is told apart by its words.
pub(in crate::window) fn is_extension_question(dialog: &Dialog) -> bool {
    dialog
        .message_text()
        .starts_with("If you change a file name extension")
}

/// Checks that `dialog` is the extension question drawn as Windows 11
/// draws it: a message box titled "Rename" with the warning icon, the two
/// lines of its question and Yes, focused and pressed by Enter, then No;
/// Alt+Y and Alt+N press them, and the box is sized to its words.
pub(in crate::window) fn assert_windows_extension_question(dialog: &Dialog) {
    assert_eq!(dialog.title_text(), "Rename");
    assert_eq!(dialog.message_text(), EXTENSION_QUESTION);
    assert_eq!(dialog.button_labels(), ["Yes", "No"]);
    assert_windows_message_box(dialog.upcast_ref(), "Yes");
    assert_eq!(
        focused_button(dialog).as_deref(),
        Some("Yes"),
        "Yes has the keyboard"
    );
    let default = dialog.default_widget().and_downcast::<gtk::Button>();
    assert_eq!(
        default.and_then(|button| button.label()).as_deref(),
        Some("Yes"),
        "Enter presses Yes"
    );
    wait_until("the box to be drawn", || dialog.is_mapped() && dialog.width() > 0);
    assert!(
        dialog.width() < 480,
        "a compact box sized to its words, not a 510-pixel dialog: {}",
        dialog.width()
    );
    assert!(dialog.height() < 240, "no empty room: {}", dialog.height());
}

/// Checks the parts every Windows message box of the app shares, inside
/// `widget`: its caption with the title, the warning icon beside the
/// words, and Alt with the first letter of each button pressing it.
/// `default` is the label of the button drawn as the one Enter presses.
pub(in crate::window) fn assert_windows_message_box(widget: &gtk::Widget, default: &str) {
    let icon = descendants::<gtk::Image>(widget)
        .into_iter()
        .find(|image| image.icon_name().as_deref() == Some("ox-warning-32-color") && image.is_visible());
    assert!(icon.is_some(), "the warning icon shows");
    let frame = descendants::<crate::dialog::DialogFrame>(widget)
        .into_iter()
        .chain(widget.clone().downcast::<crate::dialog::DialogFrame>())
        .next()
        .expect("the box is drawn by a dialog frame");
    assert!(frame.has_css_class("message-box"), "the shared message-box style");
    let accent: Vec<String> = descendants::<gtk::Button>(widget)
        .into_iter()
        .filter(|button| button.has_css_class("accent") && button.is_visible())
        .filter_map(|button| button.label().map(String::from))
        .collect();
    assert_eq!(accent, [default], "the default button is drawn as the default");
    let triggers: Vec<String> = frame
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok())
        .flat_map(|controller| {
            controller
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
                .filter_map(|shortcut| shortcut.trigger())
                .map(|trigger| trigger.to_str().to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    for key in ["<Alt>y", "<Alt>n"] {
        assert!(
            triggers.iter().any(|trigger| trigger == key),
            "{key} presses its button: {triggers:?}"
        );
    }
}

/// The label of the button that has focus in `dialog`, the one Enter
/// presses.
pub(in crate::window) fn focused_button(dialog: &Dialog) -> Option<String> {
    gtk::prelude::GtkWindowExt::focus(dialog)
        .and_downcast::<gtk::Button>()
        .and_then(|button| button.label())
        .map(String::from)
}

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
pub(in crate::window) fn dialog_over(test: &TestWindow) -> Option<Dialog> {
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

/// Waits until `test`'s window writes no files, then checks that the
/// toast shows nothing: as in Windows Explorer, an operation that
/// succeeds ends without a message or an Undo button.
///
/// # Panics
///
/// When the operation does not end in time or a message shows.
pub(in crate::window) fn assert_ended_without_a_message(test: &TestWindow) {
    wait_until("the operation to end", || !test.window.is_writing_files());
    settle();
    assert_eq!(test.window.shown_message(), "", "no message after the operation");
}

/// The dialog's first text field.
pub(super) fn text_field(dialog: &Dialog) -> gtk::Entry {
    descendants::<gtk::Entry>(dialog)
        .into_iter()
        .next()
        .expect("the dialog has a text field")
}

/// The field that edits a name in place in `test`'s view, once it shows.
pub(in crate::window) fn name_editor(test: &TestWindow) -> gtk::Entry {
    let find = || {
        descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
            .into_iter()
            .find(|field| field.has_css_class("rename-field"))
    };
    wait_until("the name to become editable", || find().is_some());
    find().expect("wait_until returned only once the field showed")
}

/// Whether `test`'s view edits a name in place.
pub(in crate::window) fn is_renaming_in_place(test: &TestWindow) -> bool {
    descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
        .iter()
        .any(|field| field.has_css_class("rename-field"))
}

/// Selects the items called `names` in `test`'s folder view.
pub(in crate::window) fn select_names(test: &TestWindow, names: &[&str]) {
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
        // GTK 4.24 parses "a|b|c" into one trigger that lists all three,
        // whose first and second reach only two; older GTK nests pairs.
        if let Some(list) = alternatives.dynamic_cast_ref::<gio::ListModel>() {
            return list
                .iter::<gtk::ShortcutTrigger>()
                .filter_map(Result::ok)
                .any(|alternative| is_triggered_by(&alternative, keyval, modifiers));
        }
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

/// How a test answers the "Replace or Skip Files" question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum ConflictAnswer {
    /// "Replace the file(s) in the destination".
    Replace,
    /// "Skip this file" / "Skip these files".
    Skip,
    /// Both versions checked in the File Conflict window for every file.
    KeepBoth,
}

/// The progress window of `test`'s application, once an action asks
/// "Replace or Skip Files".
pub(in crate::window) fn asking_window(test: &TestWindow) -> crate::window::progress_window::ProgressWindow {
    wait_until("the Replace or Skip Files question", || {
        test.context.progress_window().is_some_and(|window| {
            window.is_visible()
                && window
                    .actions()
                    .iter()
                    .any(crate::window::transfer_panel::TransferPanel::is_asking)
        })
    });
    test.context.progress_window().expect("it asks")
}

/// Waits for an action to ask "Replace or Skip Files", then presses that
/// action's Cancel.
pub(in crate::window) fn cancel_conflicts(test: &TestWindow) {
    let window = asking_window(test);
    let action = window
        .actions()
        .into_iter()
        .find(crate::window::transfer_panel::TransferPanel::is_asking)
        .expect("the asking action");
    action.press_cancel();
}

/// Waits for the progress window of `test`'s application to ask "Replace
/// or Skip Files", and answers with `answer`.
pub(in crate::window) fn answer_conflicts(test: &TestWindow, answer: ConflictAnswer) {
    let window = asking_window(test);
    let texts: &[&str] = match answer {
        ConflictAnswer::Replace => &[
            "Replace the file in the destination",
            "Replace the files in the destination",
        ],
        ConflictAnswer::Skip => &["Skip this file", "Skip these files"],
        ConflictAnswer::KeepBoth => &["Compare info for both files", "Let me decide for each file"],
    };
    let row = descendants::<gtk::Button>(&window)
        .into_iter()
        .find(|button| {
            button.get_visible()
                && descendants::<gtk::Label>(button)
                    .iter()
                    .any(|label| texts.contains(&label.text().as_str()))
        })
        .expect("the question's row");
    row.emit_clicked();
    if answer != ConflictAnswer::KeepBoth {
        return;
    }
    wait_until("the File Conflict window", || window.conflict_window().is_some());
    let conflicts = window.conflict_window().expect("it shows");
    let (incoming, existing) = conflicts.header_checks();
    incoming.set_active(true);
    existing.set_active(true);
    descendants::<gtk::Button>(&conflicts)
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Continue"))
        .expect("Continue")
        .emit_clicked();
}

/// The progress window's action that asks about a failed item, once one
/// of the operations of `test` asks.
pub(super) fn failure_question(test: &TestWindow) -> crate::window::transfer_panel::TransferPanel {
    let window = crate::window::progress_window::ProgressWindow::of(&test.context);
    let asking = || {
        window
            .actions()
            .into_iter()
            .find(crate::window::transfer_panel::TransferPanel::asks_about_failure)
    };
    wait_until("the action to ask about a failed item", || asking().is_some());
    asking().expect("the asking action")
}
