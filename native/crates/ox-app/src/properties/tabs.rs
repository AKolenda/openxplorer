// SPDX-License-Identifier: AGPL-3.0-only
//! The tab row and panels every Properties dialog shares: the tab buttons
//! from the left over a bottom rule (`.properties-tabs`), and one panel
//! per tab with the same shortest height (`.properties-panel`), so the
//! dialog keeps its height while a panel fills in or another tab shows.

use gtk::prelude::*;

use super::PropertiesTab;

/// The shortest height of a tab's panel (`.properties-panel`).
const PANEL_MIN_HEIGHT: i32 = 290;

/// The tab buttons and the panels they switch between.
#[derive(Debug, Clone)]
pub(super) struct PropertiesTabs {
    /// The row under the tab buttons, with its bottom rule.
    tab_row: gtk::Box,
    /// One page per tab.
    pages: gtk::Stack,
}

impl Default for PropertiesTabs {
    fn default() -> Self {
        Self::new()
    }
}

impl PropertiesTabs {
    /// A tab row with no tabs yet.
    pub(super) fn new() -> Self {
        let pages = gtk::Stack::builder().vhomogeneous(false).build();
        // The tabs keep their own width, from the left.
        let switcher = gtk::StackSwitcher::builder()
            .stack(&pages)
            .halign(gtk::Align::Start)
            .build();
        let tab_row = gtk::Box::builder().css_classes(["properties-tabs"]).build();
        tab_row.append(&switcher);
        Self { tab_row, pages }
    }

    /// The row of tab buttons, to place above [`Self::pages`].
    pub(super) fn tab_row(&self) -> &gtk::Box {
        &self.tab_row
    }

    /// The panels, to place below [`Self::tab_row`].
    pub(super) fn pages(&self) -> &gtk::Stack {
        &self.pages
    }

    /// Adds `panel` as the page of `tab`.
    pub(super) fn add_page(&self, tab: PropertiesTab, panel: &impl IsA<gtk::Widget>) {
        panel.add_css_class("properties-panel");
        panel.set_size_request(-1, PANEL_MIN_HEIGHT);
        self.pages.add_titled(panel, Some(tab.page_name()), tab.label());
    }

    /// Shows `tab`, or General when there is no such tab.
    pub(super) fn select_tab(&self, tab: PropertiesTab) {
        let name = if self.pages.child_by_name(tab.page_name()).is_some() {
            tab.page_name()
        } else {
            PropertiesTab::General.page_name()
        };
        self.pages.set_visible_child_name(name);
    }

    /// The tab shown.
    pub(super) fn selected_tab(&self) -> PropertiesTab {
        let name = self.pages.visible_child_name();
        name.as_deref()
            .and_then(PropertiesTab::from_page_name)
            .unwrap_or(PropertiesTab::General)
    }
}
