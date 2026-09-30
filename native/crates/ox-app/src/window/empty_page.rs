// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane's empty page: an empty or filtered-out folder, and a
//! location that could not be listed, with Try again.
//!
//! Ports the empty-state branch of `renderRows` in `desktop/ui/app.js`.
//! A folder that is still being listed shows no page of its own: the
//! pane keeps its blank list, as Windows Explorer and Dolphin do (see
//! [`super::loading`]).

use gtk::prelude::*;
use ox_core::location::{is_smb_server, same_location, split_location, TRASH_URI};

use crate::icons::{self, Icon};

use super::button_style::ButtonStyle;
use super::window_action::WindowAction;

/// The folder or network glyph above the title.
const STATE_GLYPH: i32 = 44;
/// Pixels between the glyph, the title, the message and Try again
/// (`.empty-state{gap:12px}`).
const PART_GAP: i32 = 12;
/// The widest the message gets before it wraps, in characters: about the
/// 460 pixels of `.empty-state p{max-width:460px}`.
const MESSAGE_WIDTH_CHARS: i32 = 65;

/// What the empty page says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EmptyState {
    /// The folder could not be listed: the error text and a Try again button.
    Unavailable(String),
    /// A search found nothing; the message says why (SRCH-013).
    NoMatches(String),
    /// The folder has no items.
    EmptyFolder,
    /// The Recycle Bin has no items (OPS-040).
    EmptyRecycleBin,
    /// A server lists no shared folders.
    NoShares,
    /// The phone and camera list (`mtp://`) has no devices.
    NoDevices,
}

impl EmptyState {
    /// What an empty listing of `uri` says: Dolphin's placeholders for
    /// the Recycle Bin, a server without shares and the device list, and
    /// "This folder is empty" elsewhere.
    pub(super) fn empty_listing(uri: &str) -> EmptyState {
        if same_location(uri, TRASH_URI) {
            EmptyState::EmptyRecycleBin
        } else if is_smb_server(uri) {
            EmptyState::NoShares
        } else if split_location(uri)
            .is_ok_and(|parts| parts.scheme.eq_ignore_ascii_case("mtp") && parts.authority.is_empty())
        {
            EmptyState::NoDevices
        } else {
            EmptyState::EmptyFolder
        }
    }
}

/// The empty page's widgets.
#[derive(Debug)]
pub(super) struct EmptyPage {
    /// The page, centred in the folder pane.
    pub root: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    message: gtk::Label,
    retry: gtk::Button,
}

/// A centred, wrapping label. Wrapping keeps a narrow window at its size
/// when a folder is empty (`.empty-state{text-align:center}` wraps in
/// app.js too).
fn centred_text() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build()
}

impl EmptyPage {
    /// A page that says nothing yet.
    pub(super) fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(PART_GAP)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["empty-state"])
            .build();
        let icon = icons::image(Icon::Folder, STATE_GLYPH);
        let title = centred_text();
        title.add_css_class("empty-title");
        let message = centred_text();
        message.set_max_width_chars(MESSAGE_WIDTH_CHARS);
        message.set_selectable(true);
        let retry = gtk::Button::builder()
            .label("Try again")
            .action_name(WindowAction::Refresh.detailed_name())
            .halign(gtk::Align::Center)
            .css_classes([ButtonStyle::Bordered.css_class()])
            .visible(false)
            .build();
        root.append(&icon);
        root.append(&title);
        root.append(&message);
        root.append(&retry);
        Self {
            root,
            icon,
            title,
            message,
            retry,
        }
    }

    /// Shows `state`, with the app.js wording (`renderRows`).
    pub(super) fn show(&self, state: &EmptyState) {
        let glyph = match state {
            EmptyState::Unavailable(_) => Icon::Organization,
            EmptyState::EmptyRecycleBin => Icon::Delete,
            _ => Icon::Folder,
        };
        icons::set_icon(&self.icon, glyph, STATE_GLYPH);
        let (title, message) = match state {
            EmptyState::Unavailable(error) => ("This location is unavailable", error.as_str()),
            EmptyState::NoMatches(reason) => ("No matching items", reason.as_str()),
            EmptyState::EmptyFolder => ("This folder is empty", "Create a folder or paste files here."),
            EmptyState::EmptyRecycleBin => ("Recycle Bin is empty", ""),
            EmptyState::NoShares => ("No shared folders found", ""),
            EmptyState::NoDevices => ("No MTP-compatible devices found", ""),
        };
        self.title.set_text(title);
        self.message.set_text(message);
        self.message.set_visible(!message.is_empty());
        self.retry
            .set_visible(matches!(state, EmptyState::Unavailable(_)));
    }

    /// The title shown, for tests.
    #[cfg(test)]
    pub(super) fn title(&self) -> String {
        self.title.text().to_string()
    }

    /// The message shown under the title, for tests.
    #[cfg(test)]
    pub(super) fn message(&self) -> String {
        self.message.text().to_string()
    }

    /// True when a Try again button that refreshes is shown, for tests.
    #[cfg(test)]
    pub(super) fn offers_try_again(&self) -> bool {
        let retry = &self.retry;
        let refresh = WindowAction::Refresh.detailed_name();
        let runs_refresh = retry.action_name().as_deref() == Some(refresh.as_str());
        retry.is_visible() && runs_refresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-048
    #[test]
    fn empty_listings_say_what_is_missing_where() {
        assert_eq!(
            EmptyState::empty_listing("trash:///"),
            EmptyState::EmptyRecycleBin
        );
        assert_eq!(EmptyState::empty_listing("smb://nas/"), EmptyState::NoShares);
        assert_eq!(
            EmptyState::empty_listing("smb://nas/Team"),
            EmptyState::EmptyFolder
        );
        assert_eq!(EmptyState::empty_listing("mtp:///"), EmptyState::NoDevices);
        assert_eq!(
            EmptyState::empty_listing("file:///tmp/empty"),
            EmptyState::EmptyFolder
        );
    }
}
