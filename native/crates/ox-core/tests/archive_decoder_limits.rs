// SPDX-License-Identifier: AGPL-3.0-only
//! Archive decoder memory is bounded before browsing or extracting untrusted data.

mod archive_support;

use std::fs;
use std::process::Command;

use ox_core::archive::{ArchiveBrowser, ArchiveError};
use ox_core::transfer::Cancellation;

use archive_support::{memory_opener, ExtractionFixture};

/// A valid empty TAR in a tiny XZ stream can request a large dictionary.
/// Changing its dictionary property does not change its decoded contents.
#[test]
fn an_xz_dictionary_is_bounded_before_browsing_or_extracting() {
    let fixture = ExtractionFixture::new();
    let script = "import lzma, sys, zlib\n\
                  data = bytearray(lzma.compress(bytes(1024)))\n\
                  assert data[12:16] == bytes([2, 0, 33, 1])\n\
                  data[16] = 30 # 128 MiB dictionary, plus decoder overhead\n\
                  data[20:24] = zlib.crc32(data[12:20]).to_bytes(4, 'little')\n\
                  open(sys.argv[1], 'wb').write(data)\n";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(&fixture.archive)
        .output()
        .expect("Python writes a synthetic XZ stream");
    assert!(output.status.success(), "{:?}", output.stderr);
    let bytes = fs::read(&fixture.archive).unwrap();
    assert!(bytes.len() < 100);
    let browser = ArchiveBrowser::new(memory_opener(bytes), fixture.destination.join("previews"));

    let listed = browser.list(&fixture.archive_uri(), "", &Cancellation::new());
    assert!(
        matches!(listed, Err(ArchiveError::DecoderMemoryLimit)),
        "{listed:?}"
    );
    assert!(matches!(
        fixture.extract("Unpacked"),
        Err(ArchiveError::DecoderMemoryLimit)
    ));
    fixture.assert_no_output();
}

/// A valid Zstandard frame can advertise a huge window even for a tiny
/// payload. The check must run on the first frame and each later frame.
#[test]
fn every_zstandard_frame_has_a_bounded_window() {
    let fixture = ExtractionFixture::new();
    // No declared content size; a 256 MiB window; one final raw block
    // containing the two zero blocks of an empty TAR.
    let mut oversized = vec![0x28, 0xb5, 0x2f, 0xfd, 0, 144, 1, 32, 0];
    oversized.extend_from_slice(&[0; 1024]);
    let empty = ruzstd::encoding::compress_to_vec(&[][..], ruzstd::encoding::CompressionLevel::Fastest);
    for prefix in [Vec::new(), empty] {
        let bytes = [prefix, oversized.clone()].concat();
        let browser = ArchiveBrowser::new(memory_opener(bytes.clone()), fixture.destination.join("previews"));

        let listed = browser.list(&fixture.archive_uri(), "", &Cancellation::new());
        assert!(
            matches!(listed, Err(ArchiveError::DecoderMemoryLimit)),
            "{listed:?}"
        );
        fs::write(&fixture.archive, bytes).unwrap();
        assert!(matches!(
            fixture.extract("Unpacked"),
            Err(ArchiveError::DecoderMemoryLimit)
        ));
        fixture.assert_no_output();
    }
}
