// SPDX-License-Identifier: AGPL-3.0-only
//! Clipboard text and images written as a new file (CLIP-015).
//!
//! Dolphin's "Paste Clipboard Contents…" and Nautilus's paste of a
//! screenshot turn data on the clipboard into one file. The data is never
//! read as a file list ([`super::decode`] cannot even name plain text): it
//! becomes a file of its own, under a name the app makes.
//!
//! The name is the local time it was pasted, `2026-02-05 14-30-00.png`.
//! Hyphens rather than the colons of a clock because a name is written to
//! wherever the paste goes: FAT, exFAT and NTFS forbid `:`, and an SMB
//! share is usually Windows' own file system ([`crate::transfer::limits`]
//! has the same rule for names the destination cannot store).

use glib::DateTime;

/// The text format a paste reads and writes as UTF-8: what GTK converts
/// every text target an owner declares into (Nautilus reads text the same
/// way, with `gdk_clipboard_read_text_async`).
pub const TEXT_MIME_TYPE: &str = "text/plain;charset=utf-8";
/// The extension of pasted text.
pub const TEXT_EXTENSION: &str = "txt";

/// The image formats a paste writes from the owner's own bytes, in the
/// order it prefers them, with the extension each is written under.
///
/// PNG comes first: an application that copies one picture often offers
/// several encodings of it (a browser offers `image/png` beside
/// `image/jpeg`), and PNG is the one that lost nothing on the way. The
/// formats are written as they are, never re-encoded; a picture offered
/// only as a texture goes through [`glib`]'s PNG encoder instead.
pub const IMAGE_MIME_TYPES: [(&str, &str); 5] = [
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
    ("image/bmp", "bmp"),
];

/// The extension the texture of an image is written under, when the owner
/// offers none of [`IMAGE_MIME_TYPES`]: GTK encodes it as PNG.
pub const TEXTURE_EXTENSION: &str = "png";

/// The name format of a pasted file: the local time, to the second.
const NAME_FORMAT: &str = "%Y-%m-%d %H-%M-%S";

/// Why Paste refuses text or an image over
/// [`MAX_BYTES`](super::MAX_BYTES): it is no data to the app, and saying so
/// beats a paste that silently writes nothing.
pub const TOO_LARGE_TO_PASTE: &str =
    crate::i18n::message_id("The clipboard's text or image is too large to paste as a file.");

/// The name used when the system clock cannot say what time it is, so a
/// paste still writes one file under a name the app can number.
const UNTIMED_NAME: &str = "Clipboard";

/// The extension an image offered as `mime_type` is written under, when
/// the app keeps the owner's bytes.
pub fn image_extension(mime_type: &str) -> Option<&'static str> {
    IMAGE_MIME_TYPES
        .iter()
        .find(|(offered, _)| *offered == mime_type)
        .map(|(_, extension)| *extension)
}

/// What a paste turns clipboard data into: one file's content and the
/// extension of its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PastedContent {
    /// Text copied in another application, written as UTF-8 under `.txt`.
    ///
    /// The bytes are the owner's text as GTK converted it into UTF-8, and
    /// nothing else is changed: a line ending it kept is kept, so the file
    /// holds what was copied and no editor finds text the user never saw.
    Text(Vec<u8>),
    /// An image, with the extension of the format it was copied in.
    Image {
        /// `png`, `jpg`, `webp`, `gif` or `bmp`.
        extension: &'static str,
        /// The image's own bytes, or GTK's PNG encoding of its texture.
        bytes: Vec<u8>,
    },
}

impl PastedContent {
    /// The extension of the file this content is written as.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Text(_) => TEXT_EXTENSION,
            Self::Image { extension, .. } => extension,
        }
    }

    /// The bytes the new file holds.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(bytes) | Self::Image { bytes, .. } => bytes,
        }
    }

    /// The name a paste of this content makes at `time`, such as
    /// `2026-02-05 14-30-00.txt` on the 5th of February 2026 at 14:30.
    pub fn file_name_at(&self, time: &DateTime) -> String {
        let stamp = time.format(NAME_FORMAT).unwrap_or_else(|_| UNTIMED_NAME.into());
        format!("{stamp}.{}", self.extension())
    }

    /// [`Self::file_name_at`] at the local time now, which is the name a
    /// paste gives its file.
    pub fn file_name_now(&self) -> String {
        // Without a local time zone the UTC clock still names the file,
        // and a clock that cannot answer at all has a fixed name.
        match DateTime::now_local().or_else(|_| DateTime::now_utc()) {
            Ok(now) => self.file_name_at(&now),
            Err(_) => format!("{UNTIMED_NAME}.{}", self.extension()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: CLIP-015
    #[test]
    fn image_formats_are_written_in_their_own_encoding_and_extension() {
        assert_eq!(image_extension("image/png"), Some("png"));
        assert_eq!(image_extension("image/jpeg"), Some("jpg"));
        assert_eq!(image_extension("image/webp"), Some("webp"));
        assert_eq!(image_extension("image/gif"), Some("gif"));
        assert_eq!(image_extension("image/bmp"), Some("bmp"));
        assert_eq!(image_extension("image/tiff"), None, "texture fallback instead");
        assert_eq!(image_extension("text/plain"), None);
        assert_eq!(IMAGE_MIME_TYPES[0], ("image/png", "png"), "PNG is preferred");
    }

    /// parity: CLIP-015
    #[test]
    fn a_pasted_file_is_named_after_the_local_time() {
        let time = DateTime::from_local(2026, 2, 5, 14, 30, 0.0).expect("a valid local time");
        let text = PastedContent::Text(b"plan\n".to_vec());
        let image = PastedContent::Image {
            extension: "jpg",
            bytes: vec![0xff, 0xd8],
        };

        assert_eq!(text.file_name_at(&time), "2026-02-05 14-30-00.txt");
        assert_eq!(image.file_name_at(&time), "2026-02-05 14-30-00.jpg");
        assert!(
            !text.file_name_at(&time).contains(':'),
            "a name FAT, NTFS and SMB accept"
        );
    }

    /// parity: CLIP-015
    #[test]
    fn the_name_of_a_paste_now_is_the_local_time_and_the_contents_extension() {
        let text = PastedContent::Text(b"".to_vec());

        let name = text.file_name_now();
        let (stamp, extension) = name.rsplit_once('.').expect("a name with an extension");

        assert_eq!(extension, "txt", "{name}");
        assert_eq!(stamp.len(), "2026-02-05 14-30-00".len(), "{name}");
        assert_eq!(text.bytes(), b"");
    }
}
