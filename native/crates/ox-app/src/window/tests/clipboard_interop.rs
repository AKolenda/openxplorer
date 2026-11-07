// SPDX-License-Identifier: AGPL-3.0-only
//! What other applications put on the clipboard: a plain URI list, a
//! picture, nothing at all, empty file lists, a conversion that fails, an
//! owner that changes during a read and payloads over the size the app
//! accepts, and what a reader that takes text only gets from what this app
//! copied. Dolphin's cut and plain text are in the `clipboard` tests.
//!
//! Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py`, against the
//! display's clipboard instead of its fake GTK clipboard. GitHub issue #20 (1.1.4
//! crashed on KDE Plasma when GTK returned selection data of length -1) is
//! the reason every foreign clipboard here must leave the window running.
//! A foreign clipboard with files enables Paste for those files, a picture
//! is pasted as one new file (CLIP-015), and one with neither is no data,
//! which leaves Paste disabled (CLIP-016).

use std::fs;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{GNOME, MAX_BYTES, URI_LIST};

use super::file_ops_support::{is_enabled, select_names};
use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};

use scripted_owner::{Script, ScriptedOwner};

/// A clipboard owner in another application, played by a content
/// provider whose answers the test scripts.
mod scripted_owner {
    use std::cell::{Cell, OnceCell};
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::time::Duration;

    use gtk::gdk::subclass::prelude::*;
    use gtk::prelude::*;
    use gtk::{gdk, gio, glib};

    /// How a [`ScriptedOwner`] answers a read.
    #[derive(Debug, Clone)]
    pub(super) enum Script {
        /// The conversion fails, as when an X11 owner refuses a target
        /// (the length -1 selection data of GitHub issue #20).
        Fail,
        /// The payload arrives after `delay`, like a slow application's.
        Late {
            /// How long the owner takes to answer.
            delay: Duration,
            /// What it answers.
            payload: Vec<u8>,
        },
    }

    /// What a [`ScriptedOwner`] has done, for the test to wait on.
    #[derive(Debug, Default)]
    pub(super) struct OwnerProgress {
        /// A reader asked for the payload.
        pub(super) was_asked: Cell<bool>,
        /// The owner has answered, or failed as scripted.
        pub(super) has_answered: Cell<bool>,
    }

    mod imp {
        use super::*;

        /// Private state of [`super::ScriptedOwner`].
        #[derive(Debug, Default)]
        pub(crate) struct ScriptedOwner {
            /// The one format the owner offers.
            pub(super) mime_type: OnceCell<&'static str>,
            /// How it answers.
            pub(super) script: OnceCell<Script>,
            /// What it has done so far.
            pub(super) progress: Rc<OwnerProgress>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for ScriptedOwner {
            const NAME: &'static str = "OxTestScriptedOwner";
            type Type = super::ScriptedOwner;
            type ParentType = gdk::ContentProvider;
        }

        impl ObjectImpl for ScriptedOwner {}

        impl ContentProviderImpl for ScriptedOwner {
            fn formats(&self) -> gdk::ContentFormats {
                let mime_type = self.mime_type.get().expect("ScriptedOwner::new sets the format");
                gdk::ContentFormats::new(&[mime_type])
            }

            fn write_mime_type_future(
                &self,
                _mime_type: &str,
                stream: &gio::OutputStream,
                io_priority: glib::Priority,
            ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'static>> {
                let script = self
                    .script
                    .get()
                    .expect("ScriptedOwner::new sets the script")
                    .clone();
                let progress = Rc::clone(&self.progress);
                let stream = stream.clone();
                progress.was_asked.set(true);
                Box::pin(async move {
                    let answer = answer(script, &stream, io_priority).await;
                    progress.has_answered.set(true);
                    answer
                })
            }
        }

        /// Writes what `script` says to `stream`.
        async fn answer(
            script: Script,
            stream: &gio::OutputStream,
            io_priority: glib::Priority,
        ) -> Result<(), glib::Error> {
            match script {
                Script::Fail => Err(glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "the owner refused the target",
                )),
                Script::Late { delay, payload } => {
                    glib::timeout_future(delay).await;
                    stream
                        .write_all_future(payload, io_priority)
                        .await
                        .map_err(|(_, error)| error)?;
                    Ok(())
                }
            }
        }
    }

    glib::wrapper! {
        /// A clipboard owner offering one format and answering reads of it
        /// as scripted.
        pub(crate) struct ScriptedOwner(ObjectSubclass<imp::ScriptedOwner>)
            @extends gdk::ContentProvider;
    }

    impl ScriptedOwner {
        /// An owner offering `mime_type` and answering as `script` says.
        pub(super) fn new(mime_type: &'static str, script: Script) -> Self {
            let owner: Self = glib::Object::new();
            let imp = owner.imp();
            imp.mime_type
                .set(mime_type)
                .expect("a new owner has no format yet");
            imp.script.set(script).expect("a new owner has no script yet");
            owner
        }

        /// What the owner has done so far.
        pub(super) fn progress(&self) -> Rc<OwnerProgress> {
            Rc::clone(&self.imp().progress)
        }
    }
}

/// Makes `content` the clipboard of `test`'s window, as another
/// application taking the clipboard does.
fn take_clipboard(test: &TestWindow, content: &impl IsA<gdk::ContentProvider>) {
    test.window
        .clipboard()
        .set_content(Some(content))
        .expect("the test owns the clipboard");
}

/// A provider of `payload` as `mime_type`.
fn offering(mime_type: &str, payload: &[u8]) -> gdk::ContentProvider {
    gdk::ContentProvider::for_bytes(mime_type, &glib::Bytes::from(payload))
}

/// A window on the fixture's Documents folder with the fixture's
/// "Notes 2.txt" copied, so Paste is enabled until the clipboard changes.
fn window_with_a_file_clipboard(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let uri_list = format!("{}\r\n", fixture.uri_of("Notes 2.txt"));
    take_clipboard(&test, &offering(URI_LIST, uri_list.as_bytes()));
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test
}

/// The names of the items in `folder` of `fixture` that end in
/// `.extension`, sorted.
fn names_ending(fixture: &Fixture, folder: &str, extension: &str) -> Vec<String> {
    let ending = format!(".{extension}");
    let mut names: Vec<String> = fs::read_dir(fixture.path(folder))
        .expect("a readable folder")
        .filter_map(|entry| entry.expect("a folder entry").file_name().into_string().ok())
        .filter(|name| name.ends_with(&ending))
        .collect();
    names.sort();
    names
}

/// Waits until `folder` of `fixture` holds `count` items with
/// `extension`, and returns their names.
///
/// # Panics
///
/// When they do not appear in time.
fn pasted_names(fixture: &Fixture, folder: &str, extension: &str, count: usize) -> Vec<String> {
    wait_until("the pasted file", || {
        names_ending(fixture, folder, extension).len() == count
    });
    names_ending(fixture, folder, extension)
}

/// True when `name` is the name a paste gives its file: the local time,
/// `2026-02-05 14-30-00`, and `extension`; or the same with the ` (2)`
/// group a name that was taken gets.
fn is_pasted_name(name: &str, extension: &str) -> bool {
    let Some(stem) = name.strip_suffix(&format!(".{extension}")) else {
        return false;
    };
    let stem = match stem.rsplit_once(" (") {
        Some((stem, group)) => {
            let digits = group.strip_suffix(')').unwrap_or_default();
            if digits.is_empty() || !digits.chars().all(|digit| digit.is_ascii_digit()) {
                return false;
            }
            stem
        }
        None => stem,
    };
    stem.char_indices().all(|(index, character)| match index {
        4 | 7 | 13 | 16 => character == '-',
        10 => character == ' ',
        _ => character.is_ascii_digit(),
    }) && stem.len() == 19
}

/// Ported from `test_uri_list_without_exact_kde_cut_marker_remains_copy`.
///
/// parity: CLIP-006, CLIP-016
#[gtk::test]
fn a_uri_list_without_a_kde_marker_pastes_as_a_copy() {
    let fixture = Fixture::standard();
    let test = window_with_a_file_clipboard(&fixture);

    test.activate("paste", None);

    wait_until("the item to be copied", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
    assert!(
        fixture.path("Notes 2.txt").is_file(),
        "a plain URI list is a copy"
    );
}

/// parity: CLIP-016
#[gtk::test]
fn an_empty_clipboard_disables_paste() {
    let fixture = Fixture::standard();
    let test = window_with_a_file_clipboard(&fixture);

    test.window
        .clipboard()
        .set_content(None::<&gdk::ContentProvider>)
        .expect("the test owns the clipboard");

    wait_until("Paste to be disabled", || !is_enabled(&test, "paste"));
}

/// A picture on the clipboard is not a file list, but it is a paste: the
/// owner's PNG bytes become one file named after the local time.
///
/// parity: CLIP-015, CLIP-016
#[gtk::test]
fn a_picture_on_the_clipboard_pastes_as_an_image_file() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let one_blue_pixel = glib::Bytes::from_static(&[0, 120, 212, 255]);
    let image = gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &one_blue_pixel, 4);

    test.window.clipboard().set_texture(&image);

    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    let names = pasted_names(&fixture, "Documents", "png", 1);
    let written = fs::read(fixture.path(&format!("Documents/{}", names[0]))).expect("the pasted picture");

    assert!(is_pasted_name(&names[0], "png"), "{}", names[0]);
    assert!(
        written.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "the picture is written as an image, not as text"
    );
    assert!(
        names_ending(&fixture, "Documents", "txt").is_empty(),
        "a picture is no text"
    );
    wait_until("the pasted file to be selected", || {
        test.selected_names() == names
    });
}

/// A picture offered in a format the app keeps is written from the owner's
/// own bytes under that format's extension, never re-encoded.
///
/// parity: CLIP-015
#[gtk::test]
fn a_jpeg_on_the_clipboard_is_written_as_it_was_copied() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let jpeg = [0xff, 0xd8, 0xff, 0xe0, b'\r', b'\n', 0x00, 0xff, 0xd9];

    take_clipboard(&test, &offering("image/jpeg", &jpeg));
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.window.paste_from_keyboard();
    let names = pasted_names(&fixture, "Documents", "jpg", 1);

    assert!(is_pasted_name(&names[0], "jpg"), "{}", names[0]);
    assert_eq!(
        fs::read(fixture.path(&format!("Documents/{}", names[0]))).expect("the pasted picture"),
        jpeg,
        "the owner's bytes, as they were copied"
    );
}

/// A selection whose copy offers its paths as text beside the file formats
/// is still a selection: the file list is read first and the text is never
/// one, so Ctrl+C then Ctrl+V copies the files and writes nothing else.
///
/// parity: CLIP-004, CLIP-015
#[gtk::test]
fn a_file_list_wins_over_the_text_paths_copied_beside_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let item = fixture.uri_of("Notes 2.txt");
    let uri_list = format!("{item}\r\n");
    let both = gdk::ContentProvider::new_union(&[
        offering(URI_LIST, uri_list.as_bytes()),
        offering("text/plain;charset=utf-8", item.as_bytes()),
    ]);

    take_clipboard(&test, &both);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);

    wait_until("the item to be copied", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
    assert!(
        names_ending(&fixture, "Documents", "txt")
            .iter()
            .all(|name| name == "Notes 2.txt"),
        "no timestamped .txt is written beside the copied file"
    );
    assert_eq!(
        fs::read(fixture.path("Documents/Notes 2.txt")).expect("the copy"),
        fs::read(fixture.path("Notes 2.txt")).expect("the original")
    );
}

/// A picture over the size the app accepts is no data: Paste stays off,
/// and Ctrl+V says why instead of writing half of it.
///
/// parity: CLIP-015
#[gtk::test]
fn a_picture_over_the_size_limit_is_no_data_and_says_why() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let huge = vec![0x89; MAX_BYTES + 1];
    let owner = ScriptedOwner::new(
        "image/png",
        Script::Late {
            delay: Duration::from_millis(10),
            payload: huge,
        },
    );
    let progress = owner.progress();

    take_clipboard(&test, &owner);
    wait_until("the owner to answer", || progress.has_answered.get());
    wait_for(Duration::from_millis(200));

    assert!(!is_enabled(&test, "paste"), "nothing is pasteable");
    test.window.paste_from_keyboard();
    wait_until("the refusal", || {
        test.window.shown_message() == ox_core::clipboard::TOO_LARGE_TO_PASTE
    });
    assert!(names_ending(&fixture, "Documents", "png").is_empty());
}

/// A picture the owner refuses to convert is no data, never a crash
/// (GitHub issue #20): nothing is pasted and nothing is written.
///
/// parity: CLIP-015, CLIP-016
#[gtk::test]
fn a_picture_conversion_the_owner_refuses_is_no_data() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let refusing_owner = ScriptedOwner::new("image/png", Script::Fail);
    let progress = refusing_owner.progress();

    take_clipboard(&test, &refusing_owner);
    wait_until("the owner to refuse", || progress.has_answered.get());
    wait_for(Duration::from_millis(200));

    assert!(!is_enabled(&test, "paste"));
    test.window.paste_from_keyboard();
    wait_for(Duration::from_millis(200));
    assert_eq!(test.window.shown_message(), "", "nothing is said");
    assert!(names_ending(&fixture, "Documents", "png").is_empty());
}

/// Two pastes of the same picture in the same second make two files: the
/// second takes the next free number and never writes over the first.
///
/// parity: CLIP-015
#[gtk::test]
fn pasting_the_same_picture_twice_never_overwrites_the_first_file() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    take_clipboard(&test, &offering("image/png", &png));
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    pasted_names(&fixture, "Documents", "png", 1);
    test.window.paste_from_keyboard();
    let names = pasted_names(&fixture, "Documents", "png", 2);

    assert_ne!(names[0], names[1], "the second paste has a name of its own");
    for name in &names {
        assert!(is_pasted_name(name, "png"), "{name}");
        assert_eq!(
            fs::read(fixture.path(&format!("Documents/{name}"))).expect("a pasted picture"),
            png,
            "{name} holds the picture the clipboard gave it"
        );
    }
}

/// Paste into folder writes the clipboard's picture into the selected
/// folder, as pasting a file list there copies the items into it.
///
/// parity: CLIP-015, CMD-019
#[gtk::test]
fn paste_into_a_folder_writes_the_picture_there() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Documents"]);
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    take_clipboard(&test, &offering("image/png", &png));
    wait_until("Paste into folder to be enabled", || {
        is_enabled(&test, "paste-into")
    });
    test.activate("paste-into", Some(&fixture.uri_of("Documents")));
    let names = pasted_names(&fixture, "Documents", "png", 1);

    assert_eq!(
        fs::read(fixture.path(&format!("Documents/{}", names[0]))).expect("the pasted picture"),
        png
    );
    // The file went into a folder that is not the one on screen, so this tab
    // keeps its own selection: reloading it would select an item it does not
    // hold.
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(
        test.window.folder_pane().model().selected_uris(),
        [fixture.uri_of("Documents")],
        "the selection this tab had"
    );
}

/// parity: CLIP-016
#[gtk::test]
fn an_empty_file_list_payload_is_no_file_list() {
    let fixture = Fixture::standard();
    let test = window_with_a_file_clipboard(&fixture);
    let empty_lists = gdk::ContentProvider::new_union(&[offering(GNOME, b""), offering(URI_LIST, b"")]);

    take_clipboard(&test, &empty_lists);

    wait_until("Paste to be disabled", || !is_enabled(&test, "paste"));
}

/// GitHub issue #20: a conversion the owner refuses reads as no data,
/// never as a crash.
///
/// parity: CLIP-016
#[gtk::test]
fn a_conversion_the_owner_refuses_is_no_file_list() {
    let fixture = Fixture::standard();
    let test = window_with_a_file_clipboard(&fixture);
    let refusing_owner = ScriptedOwner::new(GNOME, Script::Fail);
    let progress = refusing_owner.progress();

    take_clipboard(&test, &refusing_owner);

    wait_until("the owner to refuse", || progress.has_answered.get());
    wait_until("Paste to be disabled", || !is_enabled(&test, "paste"));
}

/// Ported from
/// `test_owner_change_between_uri_list_and_marker_rejects_mixed_clipboard`:
/// a slow owner's cut, answered after another owner took the clipboard,
/// is neither pasted nor allowed to hide the new owner's files.
///
/// parity: CLIP-006, CLIP-016
#[gtk::test]
fn a_read_overtaken_by_an_owner_change_is_dropped() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let slow_cut = format!("cut\n{}", fixture.uri_of("Notes 10.txt"));
    let slow_owner = ScriptedOwner::new(
        GNOME,
        Script::Late {
            delay: Duration::from_millis(300),
            payload: slow_cut.into_bytes(),
        },
    );
    let progress = slow_owner.progress();
    take_clipboard(&test, &slow_owner);
    wait_until("the window to read the slow owner", || progress.was_asked.get());

    let new_list = format!("{}\r\n", fixture.uri_of("Notes 2.txt"));
    take_clipboard(&test, &offering(URI_LIST, new_list.as_bytes()));
    wait_until("the slow owner to answer", || progress.has_answered.get());
    // The overtaken read ends after the owner's answer reaches it.
    wait_for(Duration::from_millis(200));

    assert!(is_enabled(&test, "paste"), "the new owner's files stay pasteable");
    test.activate("paste", None);
    wait_until("the new owner's item to be copied", || {
        fixture.path("Documents/Notes 2.txt").is_file()
    });
    assert!(
        fixture.path("Notes 10.txt").is_file(),
        "the slow owner's cut is not moved"
    );
    assert!(!fixture.path("Documents/Notes 10.txt").exists());
}

/// A reader that takes text only, such as a terminal or an editor, pastes
/// the items this app copied as their paths, although the same clipboard
/// also holds the file list: text is published beside the files, never
/// instead of them.
///
/// parity: CLIP-004
#[gtk::test]
fn a_text_reader_gets_the_copied_items_paths() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    let clipboard = test.window.clipboard();

    let read = glib::MainContext::default().block_on(async { clipboard.read_text_future().await });
    let text = read
        .expect("the clipboard answers a text reader")
        .expect("the clipboard holds text");
    let path = fixture.path("Notes 2.txt").to_string_lossy().into_owned();

    assert_eq!(text.as_str(), path);
}
