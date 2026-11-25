// SPDX-License-Identifier: AGPL-3.0-only
//! What the app lets go of once nothing needs it: memory comes back down
//! after a folder of photos is left.

use std::time::Duration;

use gtk::prelude::*;

use crate::folder_view::cells::FileCell;
use crate::picture_guard::{idle_decoder_ids, UnusedForTests};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};
use crate::test_support::pictures::jpeg_sized;
use crate::thumbnails::kept_preview_count;

/// Browsing photos keeps their decoded previews, for rows scrolled back
/// into view, and picture decoder processes, about 20 MB each, for the
/// next pictures. Once the photos are left and nothing has asked for a
/// picture for a while, both are let go instead of being held for as
/// long as the app runs.
///
/// parity: VIEW-057
#[gtk::test]
fn previews_and_picture_decoders_are_let_go_once_unused() {
    // Long enough that the decoders are still waiting when the test looks
    // for them on a slow runner, well inside the harness's wait.
    let _shortened = UnusedForTests::shorten_to(Duration::from_secs(2));
    let fixture = Fixture::standard();
    for number in 0..24_u32 {
        let color = 0x2040_60ff_u32.wrapping_add(number.wrapping_mul(0x0a0b_0c00));
        std::fs::write(
            fixture.path(&format!("Harbor {number:02}.jpg")),
            jpeg_sized(320, 240, color),
        )
        .expect("a photo");
    }
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("large"));
    test.wait_for_listing("the photos");
    wait_until("the photos' previews", || {
        descendants::<FileCell>(&test.window)
            .iter()
            .filter(|cell| cell.shows_custom_icon())
            .count()
            >= 8
    });
    assert!(
        kept_preview_count() > 0,
        "the previews are kept while they are used"
    );
    // The first previews show while the rest of the folder is still being
    // decoded, each picture by a decoder taken from the pool, so none may
    // wait yet; they wait once the photos are done.
    wait_until("decoder processes to wait for the next pictures", || {
        !idle_decoder_ids().is_empty()
    });
    let decoders = idle_decoder_ids();

    let empty = tempfile::tempdir().expect("a temporary folder");
    let empty_uri = gtk::gio::File::for_path(empty.path()).uri();
    test.window.navigate(&empty_uri).expect("a folder");
    test.wait_for_listing("the empty folder");
    wait_until("the previews and decoders to be let go", || {
        kept_preview_count() == 0 && idle_decoder_ids().is_empty()
    });
    for decoder in decoders {
        assert!(
            !std::path::Path::new(&format!("/proc/{decoder}")).exists(),
            "decoder {decoder} still runs"
        );
    }
}
