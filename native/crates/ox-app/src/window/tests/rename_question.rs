// SPDX-License-Identifier: AGPL-3.0-only
//! The "Rename" question a changed extension asks, answered with check.py's
//! display's real keyboard and pointer, as Windows 11 File Explorer answers
//! it: No, Escape, the close button, N and Alt+N end the rename in place
//! and the item keeps its old name, with the name box closed; Yes, Y, Alt+Y
//! and Enter rename it (OPS-049). F2 renames again afterwards.

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::{
    is_renaming_in_place, name_editor, open_dialog, select_names, wait_for_no_dialog,
};
use crate::dialog::Dialog;
use crate::test_support::harness::{
    descendants, skin, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::test_support::pointer::{screen_point, Button as PointerButton, Pointer};
use ox_core::settings::Theme;

/// One way of answering the question.
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// A click on the No button.
    NoButton,
    /// Escape.
    Escape,
    /// A click on the box's own close button, in its caption.
    CloseButton,
    /// The desktop's close request, as a window manager sends it.
    CloseRequest,
    /// N without a modifier.
    N,
    /// Alt+N.
    AltN,
    /// A click on the Yes button.
    YesButton,
    /// Y without a modifier.
    Y,
    /// Alt+Y.
    AltY,
    /// Enter, which presses Yes, the focused button.
    Enter,
}

impl Answer {
    /// Whether the answer changes the extension.
    const fn renames(self) -> bool {
        matches!(self, Self::YesButton | Self::Y | Self::AltY | Self::Enter)
    }
}

/// Clicks the middle of `widget` with the real pointer.
fn click(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>) {
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    let point = screen_point(pointer, widget, middle, (720, 500));
    pointer.move_to(point.0, point.1);
    pointer.press(PointerButton::Primary);
    pointer.release(PointerButton::Primary);
}

/// Puts the real pointer inside `window`, where its keyboard input and a
/// window manager's close request go.
fn point_into(pointer: &mut Pointer, window: &impl IsA<gtk::Widget>) {
    let inside = screen_point(pointer, window, (40.0, 40.0), (720, 500));
    pointer.move_to(inside.0, inside.1);
}

/// Taps `key` with Alt held.
fn alt(pointer: &mut Pointer, key: gdk::Key) {
    pointer.hold_key(gdk::Key::Alt_L);
    pointer.tap_key(key);
    pointer.let_go_key(gdk::Key::Alt_L);
}

/// The button of `dialog` labelled `label`.
fn button(dialog: &Dialog, label: &str) -> gtk::Button {
    descendants::<gtk::Button>(dialog)
        .into_iter()
        .find(|button| button.is_visible() && button.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("the question has {label}"))
}

/// The close button in the caption of the message box `dialog`.
fn close_button(dialog: &Dialog) -> gtk::Button {
    descendants::<gtk::Button>(dialog)
        .into_iter()
        .find(|button| button.is_visible() && button.has_css_class("close"))
        .expect("the message box has a close button")
}

/// Answers the question `dialog` the way `answer` says.
fn answer_with(pointer: &mut Pointer, dialog: &Dialog, answer: Answer) {
    wait_for_frames(dialog, 3);
    point_into(pointer, dialog);
    wait_for_frames(dialog, 2);
    match answer {
        Answer::NoButton => click(pointer, &button(dialog, "No")),
        Answer::YesButton => click(pointer, &button(dialog, "Yes")),
        Answer::CloseButton => click(pointer, &close_button(dialog)),
        Answer::CloseRequest => pointer.close_window_under_pointer(),
        Answer::Escape => pointer.tap_key(gdk::Key::Escape),
        Answer::N => pointer.tap_key(gdk::Key::n),
        Answer::Y => pointer.tap_key(gdk::Key::y),
        Answer::AltN => alt(pointer, gdk::Key::n),
        Answer::AltY => alt(pointer, gdk::Key::y),
        Answer::Enter => pointer.tap_key(gdk::Key::Return),
    }
}

/// Whether the keyboard is in `field`.
fn has_keyboard(test: &TestWindow, field: &gtk::Entry) -> bool {
    GtkWindowExt::focus(&test.window).is_some_and(|focus| focus.is_ancestor(field) || focus == *field)
}

/// Whether the view shows a name `name` as text, outside a name box.
fn shows_name(test: &TestWindow, name: &str) -> bool {
    descendants::<gtk::Label>(&test.window.folder_pane().view_widget())
        .iter()
        .any(|label| label.is_mapped() && label.text() == name)
}

/// [`answer_in`] the light and the dark theme.
fn answer_the_question(answer: Answer) {
    let _theme = ThemeGuard::keep();
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        skin().set_theme(theme);
        answer_in(answer, name);
    }
}

/// Renames "Notes 2.txt" in place to "Notes 2.md" with F2 and Enter on the
/// real keyboard, answers the question with `answer` and checks the
/// result, then checks that F2 starts a new rename of the item; `theme`
/// names the theme for the messages.
fn answer_in(answer: Answer, theme: &str) {
    let case = format!("{theme}: {answer:?}");
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    test.window.folder_pane().focus_view();
    wait_for_frames(&test.window, 3);
    point_into(&mut pointer, &test.window);
    pointer.tap_key(gdk::Key::F2);
    let field = name_editor(&test);
    wait_until(&format!("{case}: the field takes the keyboard"), || {
        has_keyboard(&test, &field)
    });
    field.set_text("Notes 2.md");
    pointer.tap_key(gdk::Key::Return);
    let question = open_dialog(&test);
    assert!(
        super::file_ops_support::is_extension_question(&question),
        "{case}: the extension question"
    );

    answer_with(&mut pointer, &question, answer);
    wait_for_no_dialog(&test);
    point_into(&mut pointer, &test.window);

    if answer.renames() {
        wait_until(&format!("{case}: the rename"), || {
            fixture.path("Notes 2.md").is_file()
        });
        assert!(
            !fixture.path("Notes 2.txt").exists(),
            "{case}: the old name is gone"
        );
        wait_until(&format!("{case}: the new name shows"), || {
            !is_renaming_in_place(&test) && shows_name(&test, "Notes 2.md")
        });
        return;
    }
    wait_until(&format!("{case}: the name box to close"), || {
        !is_renaming_in_place(&test)
    });
    wait_for_frames(&test.window, 3);
    assert!(!is_renaming_in_place(&test), "{case}: the name box stays closed");
    assert!(shows_name(&test, "Notes 2.txt"), "{case}: the old name shows");
    assert!(
        fixture.path("Notes 2.txt").is_file(),
        "{case}: the old name stays"
    );
    assert!(!fixture.path("Notes 2.md").exists(), "{case}: nothing renamed");
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt"],
        "{case}: the item stays selected"
    );
    assert!(
        test.window.folder_pane().view_has_focus(),
        "{case}: the keyboard is back on the list"
    );

    // F2 again renames the item, from its old name.
    pointer.tap_key(gdk::Key::F2);
    let field = name_editor(&test);
    assert_eq!(
        field.text(),
        "Notes 2.txt",
        "{case}: F2 again starts from the old name"
    );
    wait_until(&format!("{case}: the new field takes the keyboard"), || {
        has_keyboard(&test, &field)
    });
    pointer.tap_key(gdk::Key::Escape);
    wait_until(&format!("{case}: Escape ends it"), || {
        !is_renaming_in_place(&test)
    });
}

/// parity: OPS-049
#[gtk::test]
fn no_ends_the_rename_with_the_old_name() {
    answer_the_question(Answer::NoButton);
}

/// parity: OPS-049
#[gtk::test]
fn escape_ends_the_rename_with_the_old_name() {
    answer_the_question(Answer::Escape);
}

/// parity: OPS-049
#[gtk::test]
fn the_close_button_ends_the_rename_with_the_old_name() {
    answer_the_question(Answer::CloseButton);
    answer_the_question(Answer::CloseRequest);
}

/// parity: OPS-049
#[gtk::test]
fn n_and_alt_n_end_the_rename_with_the_old_name() {
    answer_the_question(Answer::N);
    answer_the_question(Answer::AltN);
}

/// parity: OPS-049
#[gtk::test]
fn yes_y_alt_y_and_enter_change_the_extension() {
    for answer in [Answer::YesButton, Answer::Y, Answer::AltY, Answer::Enter] {
        answer_the_question(answer);
    }
}
