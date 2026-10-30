// SPDX-License-Identifier: AGPL-3.0-only
//! The application's identity on the session bus, and the name of the
//! build.
//!
//! Ports `APP_ID` in `v2.0.0:desktop/runtime_guard.py`.

/// The application ID, which `build.rs` chooses. It is the preview's
/// `io.winspace.Development.Native` unless the build sets `OX_APP_ID` to
/// the Python app's `io.winspace.Development`: the preview differs from it
/// so both can run side by side, and the native app takes that ID over (a
/// compatibility contract) when it replaces the Python app
/// (native/packaging/README.md, "Application ID").
pub(crate) const APP_ID: &str = env!("OX_APP_ID");

/// Whether this is the native preview, built with its own application ID
/// so it runs beside the stable app (`openxplorer-native`).
pub(crate) const IS_PREVIEW: bool = ends_with(APP_ID, ".Native");

/// The gettext domain of this build's catalogues (`ox_core::i18n`): each
/// channel is its own domain, named after its package, so a machine with
/// both packages installed has one catalogue each instead of one file
/// both packages own, which no package format allows. The packages name
/// the catalogue from the same channel (`native/tools/package_data.py`).
pub(crate) const I18N_DOMAIN: &str = if IS_PREVIEW { PREVIEW_DOMAIN } else { STABLE_DOMAIN };

/// The preview's catalogue, beside the stable app's.
const PREVIEW_DOMAIN: &str = "openxplorer-native";

/// The stable app's catalogue, whose template is `native/po/openxplorer.pot`.
const STABLE_DOMAIN: &str = "openxplorer";

/// What this build is called in the status bar, About this build and the
/// About settings (`#status-mode` in `v2.0.0:desktop/ui/app.js`): the product and
/// its version, and "native preview" in the preview.
pub(crate) const BUILD_NAME: &str = if IS_PREVIEW {
    concat!("OpenXplorer ", env!("CARGO_PKG_VERSION"), " native preview")
} else {
    concat!("OpenXplorer ", env!("CARGO_PKG_VERSION"))
};

/// Whether `text` ends with `suffix`, at compile time.
const fn ends_with(text: &str, suffix: &str) -> bool {
    let (text, suffix) = (text.as_bytes(), suffix.as_bytes());
    if suffix.len() > text.len() {
        return false;
    }
    let start = text.len() - suffix.len();
    let mut index = 0;
    while index < suffix.len() {
        if text[start + index] != suffix[index] {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the preview's application ID makes a preview, whose name says
    /// so; the stable build is named by its product and version.
    #[test]
    fn the_build_name_follows_the_channel() {
        assert!(ends_with("io.winspace.Development.Native", ".Native"));
        assert!(!ends_with("io.winspace.Development", ".Native"));
        assert_eq!(IS_PREVIEW, APP_ID == "io.winspace.Development.Native");
        assert_eq!(BUILD_NAME.ends_with(" native preview"), IS_PREVIEW);
    }
}
