// SPDX-License-Identifier: AGPL-3.0-only
//! The MIME types whose default handler the app can become: folders,
//! SMB links and the three spellings of ZIP.
//!
//! Ports `TYPES`, `ZIP_TYPES` and `ALL_TYPES` of
//! `v2.0.0:desktop/desktop_integration.py` and `ZIP_TYPES` of
//! `v2.0.0:desktop/activation.py`. The names are a compatibility contract
//! (AGENTS.md): they are the keys of `previous-defaults.json` and the
//! types in the user's `mimeapps.list`.

/// A MIME type the app can be the default handler for.
///
/// The declaration order is the order the Python app listed them in, so
/// sorted collections of these types keep that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MimeType {
    /// `inode/directory`: folders.
    Directory,
    /// `x-scheme-handler/smb`: `smb://` links.
    SmbLink,
    /// `application/zip`.
    Zip,
    /// `application/x-zip`, an older name for ZIP.
    XZip,
    /// `application/x-zip-compressed`, the name some Windows tools use.
    XZipCompressed,
}

impl MimeType {
    /// The types that making the app the default file manager takes over:
    /// folders and SMB links.
    pub const FOLDER_TYPES: [Self; 2] = [Self::Directory, Self::SmbLink];

    /// Every name a ZIP archive goes by; they always change together.
    pub const ZIP_TYPES: [Self; 3] = [Self::Zip, Self::XZip, Self::XZipCompressed];

    /// All the types, folders first.
    pub const ALL: [Self; 5] = [
        Self::Directory,
        Self::SmbLink,
        Self::Zip,
        Self::XZip,
        Self::XZipCompressed,
    ];

    /// The MIME type's name, for example `inode/directory`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "inode/directory",
            Self::SmbLink => "x-scheme-handler/smb",
            Self::Zip => "application/zip",
            Self::XZip => "application/x-zip",
            Self::XZipCompressed => "application/x-zip-compressed",
        }
    }

    /// The type named `name`, or `None` for any other type.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mime_type| mime_type.as_str() == name)
    }

    /// True for the ZIP types.
    pub fn is_zip(self) -> bool {
        Self::ZIP_TYPES.contains(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_found_by_its_name() {
        for mime_type in MimeType::ALL {
            assert_eq!(MimeType::from_name(mime_type.as_str()), Some(mime_type));
        }
        assert_eq!(MimeType::from_name("application/pdf"), None);
    }

    #[test]
    fn only_the_zip_spellings_are_zip() {
        let zip_types: Vec<_> = MimeType::ALL.into_iter().filter(|kind| kind.is_zip()).collect();

        assert_eq!(zip_types, MimeType::ZIP_TYPES);
    }
}
