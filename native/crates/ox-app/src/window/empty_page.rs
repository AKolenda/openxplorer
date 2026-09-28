// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane's empty page: loading, an empty or filtered-out folder,
//! and a location that could not be listed, with Try again.
//!
//! Ports the empty-state branch of `renderRows` in `desktop/ui/app.js`.

use gtk::prelude::*;

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
    /// The folder is still being listed.
    Loading,
    /// The folder could not be listed: the error text and a Try again button.
    Unavailable(String),
    /// The filter hides every item.
    NoMatches,
    /// The folder has no items.
    EmptyFolder,
}

/// The empty page's widgets.
#[derive(Debug)]
pub(super) struct EmptyPage {
    /// The page, centred in the folder pane.
    pub root: gtk::Box,
    spinner: gtk::Spinner,
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
        let spinner = gtk::Spinner::new();
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
        root.append(&spinner);
        root.append(&icon);
        root.append(&title);
        root.append(&message);
        root.append(&retry);
        Self {
            root,
            spinner,
            icon,
            title,
            message,
            retry,
        }
    }

    /// Shows `state`, with the app.js wording (`renderRows`).
    pub(super) fn show(&self, state: &EmptyState) {
        let loading = *state == EmptyState::Loading;
        self.spinner.set_visible(loading);
        self.spinner.set_spinning(loading);
        self.icon.set_visible(!loading);
        let glyph = match state {
            EmptyState::Unavailable(_) => Icon::Organization,
            _ => Icon::Folder,
        };
        icons::set_icon(&self.icon, glyph, STATE_GLYPH);
        let (title, message) = match state {
            EmptyState::Loading => ("Loading…", ""),
            EmptyState::Unavailable(error) => ("This location is unavailable", error.as_str()),
            EmptyState::NoMatches => ("No matching items", "Try a different filter."),
            EmptyState::EmptyFolder => ("This folder is empty", ""),
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

    /// True when a Try again button that refreshes is shown, for tests.
    #[cfg(test)]
    pub(super) fn offers_try_again(&self) -> bool {
        let retry = &self.retry;
        let refresh = WindowAction::Refresh.detailed_name();
        let runs_refresh = retry.action_name().as_deref() == Some(refresh.as_str());
        retry.is_visible() && runs_refresh
    }
}
