// SPDX-License-Identifier: AGPL-3.0-only
//! What other applications put on the clipboard: a plain URI list, an
//! image, nothing at all, empty file lists, a conversion that fails and an
//! owner that changes during a read. Dolphin's cut and plain text are in
//! the `clipboard` tests.
//!
//! Ported from `v2.0.0:desktop/tests/test_file_clipboard_interop.py`, against the
//! display's clipboard instead of its fake GTK clipboard. GitHub issue #20 (1.1.4
//! crashed on KDE Plasma when GTK returned selection data of length -1) is
//! the reason every foreign clipboard here must leave the window running,
//! with Paste disabled unless it holds files.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{GNOME, URI_LIST};

use super::file_ops_support::is_enabled;
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

/// parity: CLIP-016
#[gtk::test]
fn an_image_on_the_clipboard_is_no_file_list() {
    let fixture = Fixture::standard();
    let test = window_with_a_file_clipboard(&fixture);
    let one_blue_pixel = glib::Bytes::from_static(&[0, 120, 212, 255]);
    let image = gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &one_blue_pixel, 4);

    test.window.clipboard().set_texture(&image);

    wait_until("Paste to be disabled", || !is_enabled(&test, "paste"));
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
