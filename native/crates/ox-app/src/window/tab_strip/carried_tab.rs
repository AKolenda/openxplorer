// SPDX-License-Identifier: AGPL-3.0-only
//! What follows the pointer while GTK's drag and drop carries a tab out of
//! its strip (TAB-034): the tab itself, as it looks in front and lifted,
//! as wide as it is in the strip, with its icon, title and close glyph in
//! the font the strip draws its titles in. Chromium carries a tab this way
//! where the desktop cannot move a real window with it; GTK cannot move
//! one, as it has no `xdg-toplevel-drag-v1`.
//!
//! It is built anew from what the strip shows, never drawn from the
//! window, so it looks right from the drag's first frame. A margin around
//! it leaves room for its shadow; the drag holds it at that margin plus the
//! point where the tab was grabbed.

use gtk::pango;
use gtk::prelude::*;

use super::{mark_previous_version, tab_icon, title, TabView, CLOSE_GLYPH, ICON_TO_TITLE};
use crate::icons::{self, Icon};

/// The room around the carried tab for its shadow.
pub(in crate::window) const MARGIN: i32 = 12;

/// The tab `view`, as it follows the pointer: `width` wide when given,
/// else as wide as a strip lays it out, its title in `font`, with `margin`
/// around it.
pub(super) fn carried_tab(
    view: &TabView,
    width: Option<i32>,
    font: Option<&pango::FontDescription>,
    margin: i32,
) -> gtk::Widget {
    let tab = gtk::Box::builder()
        .spacing(ICON_TO_TITLE)
        .width_request(width.unwrap_or(-1))
        .margin_start(margin)
        .margin_end(margin)
        .margin_top(margin)
        .margin_bottom(margin)
        .css_classes(["tab", "active", "tab-dragging"])
        .build();
    tab.append(&tab_icon(view.icon));
    let label = title(&view.title);
    if let Some(font) = font {
        let attributes = pango::AttrList::new();
        attributes.insert(pango::AttrFontDesc::new(font));
        label.set_attributes(Some(&attributes));
    }
    tab.append(&label);
    if let Some(snapshot) = &view.previous_version {
        mark_previous_version(&tab, &view.title, snapshot);
    }
    // The close button's look; nothing in a drag takes clicks.
    let close = gtk::Button::builder()
        .child(&icons::image(Icon::Dismiss16, CLOSE_GLYPH))
        .focusable(false)
        .can_target(false)
        .valign(gtk::Align::Center)
        .css_classes(["tab-close"])
        .build();
    tab.append(&close);
    tab.upcast()
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use crate::test_support::harness::{descendants, wait_for_frames, Fixture, TestWindow};

    /// The tab that follows the pointer out of the strip looks like the
    /// tab in front: its title, the tab's classes and width, and the font
    /// the strip draws the title in, whatever the window's font is.
    ///
    /// parity: TAB-034
    #[gtk::test]
    fn the_carried_tab_looks_like_the_tab_in_front() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri_of("Documents"));
        wait_for_frames(&test.window, 3);
        let strip = test.window.tab_strip();
        let id = test
            .window
            .imp()
            .session
            .borrow()
            .active_id()
            .expect("a tab in front");
        let shown = strip.tab_widget_of(id).expect("the tab is shown");
        let shown_title = descendants::<gtk::Label>(&shown)
            .into_iter()
            .find(|label| label.has_css_class("tab-title"))
            .expect("a title");

        let carried = strip.carried_tab(id).expect("a carried tab");
        let title = descendants::<gtk::Label>(&carried)
            .into_iter()
            .find(|label| label.has_css_class("tab-title"))
            .expect("a title");
        let font = title
            .attributes()
            .and_then(|attributes| attributes.attributes().into_iter().next())
            .and_then(|attribute| attribute.downcast::<gtk::pango::AttrFontDesc>().ok())
            .map(|attribute| attribute.desc());

        assert_eq!(title.text(), shown_title.text());
        assert!(carried.has_css_class("tab") && carried.has_css_class("active"));
        #[expect(clippy::cast_possible_truncation, reason = "a tab's width")]
        let width = strip.tab_width(id).map(|width| width.round() as i32);
        assert_eq!(Some(carried.width_request()), width, "as wide as the tab");
        assert_eq!(
            font.map(|font| font.to_string()),
            shown_title
                .pango_context()
                .font_description()
                .map(|font| font.to_string()),
            "in the strip's font"
        );
    }
}
