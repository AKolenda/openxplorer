// SPDX-License-Identifier: AGPL-3.0-only
//! `.desktop` link files: opening one goes where it points (OPEN-009).
//!
//! Dolphin opens a `Type=Link` desktop entry as its `URL`
//! (`openItemAsFolderUrl`): a folder in the view, a web page in the
//! browser. Only links are followed; an application entry is never run
//! (OPEN-007), and opens in its editor like any other file.

use gtk::glib;

/// How large a link file may be; a real one is a few lines.
const LINK_SIZE_LIMIT: u64 = 64 * 1024;

/// Where a link file points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LinkTarget {
    /// A folder or file the app browses: `file://` or `smb://`.
    Location(String),
    /// A web page or mail address, for the desktop's handler.
    Web(String),
}

/// Why a link file was not followed.
pub(super) const UNSUPPORTED_LINK: &str =
    crate::i18n::message_id("This link points to an address OpenXplorer cannot open.");

/// The target of the desktop entry at `path`, or `None` when it is not a
/// `Type=Link` entry with a URL. An address of another kind is
/// `Err(UNSUPPORTED_LINK)`.
pub(super) fn link_target_of_file(path: &std::path::Path) -> Option<Result<LinkTarget, &'static str>> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > LINK_SIZE_LIMIT {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    link_target(&text)
}

/// The target of the desktop entry `text`, as [`link_target_of_file`].
fn link_target(text: &str) -> Option<Result<LinkTarget, &'static str>> {
    let entry = glib::KeyFile::new();
    entry.load_from_data(text, glib::KeyFileFlags::NONE).ok()?;
    let group = "Desktop Entry";
    if entry.string(group, "Type").ok()?.as_str() != "Link" {
        return None;
    }
    let url = entry.string(group, "URL").ok()?.trim().to_owned();
    let scheme = url.split_once(':').map(|(scheme, _)| scheme.to_ascii_lowercase());
    Some(match scheme.as_deref() {
        Some("file" | "smb") => Ok(LinkTarget::Location(url)),
        Some("http" | "https" | "mailto") => Ok(LinkTarget::Web(url)),
        _ => Err(ox_core::i18n::gettext_static(UNSUPPORTED_LINK)),
    })
}

/// Whether `entry` may be a desktop link, by its content type or name.
pub(super) fn may_be_link(content_type: Option<&str>, name: &str) -> bool {
    content_type == Some("application/x-desktop") || name.to_ascii_lowercase().ends_with(".desktop")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPEN-009
    #[test]
    fn a_link_entry_names_a_location_or_a_web_page_and_nothing_else_is_followed() {
        let link = |url: &str| format!("[Desktop Entry]\nType=Link\nName=Example\nURL={url}\n");
        assert_eq!(
            link_target(&link("file:///home/demo/Projects")),
            Some(Ok(LinkTarget::Location("file:///home/demo/Projects".into())))
        );
        assert_eq!(
            link_target(&link("smb://studio-nas/media")),
            Some(Ok(LinkTarget::Location("smb://studio-nas/media".into())))
        );
        assert_eq!(
            link_target(&link("https://example.org/")),
            Some(Ok(LinkTarget::Web("https://example.org/".into())))
        );
        assert_eq!(
            link_target(&link("javascript:alert(1)")),
            Some(Err(UNSUPPORTED_LINK))
        );
        let application = "[Desktop Entry]\nType=Application\nExec=rm -rf ~\nURL=https://example.org/\n";
        assert_eq!(link_target(application), None, "an application is never followed");
        assert_eq!(link_target("not a desktop entry"), None);
    }
}
