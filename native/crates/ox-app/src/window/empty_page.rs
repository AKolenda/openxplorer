// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane's empty page: loading, an empty or filtered-out folder,
//! and a location that could not be listed, with Try again.
//!
//! Ports the empty-state branch of `renderRows` in `desktop/ui/app.js`.

use gtk::prelude::*;

use crate::icons::{self, Glyph};

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

impl EmptyPage {
    /// A page that says nothing yet.
    pub fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["empty-state"])
            .build();
        let spinner = gtk::Spinner::new();
        let icon = icons::glyph(Glyph::FolderLine, 44);
        let title = gtk::Label::builder().css_classes(["empty-title"]).build();
        let message = gtk::Label::builder()
            .wrap(true)
            .max_width_chars(65)
            .justify(gtk::Justification::Center)
            .selectable(true)
            .build();
        let retry = gtk::Button::builder()
            .label("Try again")
            .action_name("win.refresh")
            .halign(gtk::Align::Center)
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
    pub fn show(&self, state: &EmptyState) {
        let loading = *state == EmptyState::Loading;
        self.spinner.set_visible(loading);
        self.spinner.set_spinning(loading);
        self.icon.set_visible(!loading);
        let glyph = match state {
            EmptyState::Unavailable(_) => Glyph::Network,
            _ => Glyph::FolderLine,
        };
        icons::set_glyph(&self.icon, glyph, 44);
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
    pub fn title(&self) -> String {
        self.title.text().to_string()
    }

    /// True when a Try again button that refreshes is shown, for tests.
    #[cfg(test)]
    pub fn offers_try_again(&self) -> bool {
        let retry = &self.retry;
        retry.is_visible() && retry.action_name().as_deref() == Some("win.refresh")
    }
}
