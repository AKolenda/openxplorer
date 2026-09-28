// SPDX-License-Identifier: AGPL-3.0-only
//! The window's own minimise, maximise and close buttons.
//!
//! Ports `.window-buttons` in `desktop/ui/index.html` and `style.css`:
//! 46-pixel-wide, full-height buttons with thin 12-pixel glyphs and a red
//! close hover. `GtkWindowControls` can only draw the icon theme's bold
//! symbolic icons, so these are plain buttons running GTK's built-in
//! `window.minimize`, `window.toggle-maximized` and `window.close` actions.
//! They still follow GNOME: the desktop's `gtk-decoration-layout` decides
//! which buttons exist and on which side, and changes to it apply at once.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Glyph};

use super::widget_tree::remove_children;

/// One caption button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Caption {
    /// Minimise the window.
    Minimize,
    /// Maximise the window, or restore it when maximised.
    Maximize,
    /// Close the window.
    Close,
}

impl Caption {
    /// The caption named in `gtk-decoration-layout`, if it is one of ours.
    fn from_layout_name(name: &str) -> Option<Self> {
        match name.trim() {
            "minimize" => Some(Caption::Minimize),
            "maximize" => Some(Caption::Maximize),
            "close" => Some(Caption::Close),
            _ => None,
        }
    }

    const fn action(self) -> &'static str {
        match self {
            Caption::Minimize => "window.minimize",
            Caption::Maximize => "window.toggle-maximized",
            Caption::Close => "window.close",
        }
    }

    /// The glyph and tooltip (index.html's `title`) in `maximized` state.
    const fn look(self, maximized: bool) -> (Glyph, &'static str) {
        match (self, maximized) {
            (Caption::Minimize, _) => (Glyph::Minus, "Minimize"),
            (Caption::Maximize, false) => (Glyph::Maximize, "Maximize"),
            (Caption::Maximize, true) => (Glyph::Restore, "Restore"),
            (Caption::Close, _) => (Glyph::Close, "Close window"),
        }
    }

    const fn css_class(self) -> &'static str {
        match self {
            Caption::Minimize => "minimize",
            Caption::Maximize => "maximize",
            Caption::Close => "close",
        }
    }
}

/// The captions `layout` puts on `side`. The part before the colon is the
/// start of the title bar and the part after it the end; a layout without
/// a colon puts everything at the start, as GTK does.
pub(super) fn captions_for(layout: &str, side: gtk::PackType) -> Vec<Caption> {
    let (start, end) = layout.split_once(':').unwrap_or((layout, ""));
    let names = match side {
        gtk::PackType::End => end,
        _ => start,
    };
    names.split(',').filter_map(Caption::from_layout_name).collect()
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::CaptionButtons`].
    #[derive(Debug, Default)]
    pub struct CaptionButtons {
        pub(super) side: Cell<Option<gtk::PackType>>,
        pub(super) maximized: Cell<bool>,
        pub(super) settings: RefCell<Option<(gtk::Settings, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CaptionButtons {
        const NAME: &'static str = "OxCaptionButtons";
        type Type = super::CaptionButtons;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for CaptionButtons {
        fn dispose(&self) {
            if let Some((settings, handler)) = self.settings.take() {
                settings.disconnect(handler);
            }
        }
    }

    impl WidgetImpl for CaptionButtons {}
    impl BoxImpl for CaptionButtons {}
}

glib::wrapper! {
    /// The caption buttons on one side of the title bar.
    pub struct CaptionButtons(ObjectSubclass<imp::CaptionButtons>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl CaptionButtons {
    /// The caption buttons the desktop puts on `side` of `window`'s title
    /// bar, kept in step with the desktop setting and the window state.
    pub fn new(window: &gtk::Window, side: gtk::PackType) -> Self {
        let buttons: Self = glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .build();
        buttons.add_css_class("caption-buttons");
        buttons.imp().side.set(Some(side));
        buttons.follow_decoration_layout(&window.settings());
        window.connect_maximized_notify(glib::clone!(
            #[weak]
            buttons,
            move |window| buttons.set_maximized(window.is_maximized())
        ));
        buttons.set_maximized(window.is_maximized());
        buttons
    }

    fn follow_decoration_layout(&self, settings: &gtk::Settings) {
        let handler = settings.connect_gtk_decoration_layout_notify(glib::clone!(
            #[weak(rename_to = buttons)]
            self,
            move |_| buttons.rebuild()
        ));
        self.imp().settings.replace(Some((settings.clone(), handler)));
    }

    fn set_maximized(&self, maximized: bool) {
        self.imp().maximized.set(maximized);
        self.rebuild();
    }

    /// The captions shown, in order.
    fn captions(&self) -> Vec<Caption> {
        let settings = self.imp().settings.borrow();
        let layout = settings
            .as_ref()
            .and_then(|(settings, _)| settings.gtk_decoration_layout())
            .unwrap_or_default();
        let side = self.imp().side.get().unwrap_or(gtk::PackType::End);
        captions_for(&layout, side)
    }

    fn rebuild(&self) {
        remove_children(self);
        let maximized = self.imp().maximized.get();
        for caption in self.captions() {
            self.append(&caption_button(caption, maximized));
        }
    }
}

fn caption_button(caption: Caption, maximized: bool) -> gtk::Button {
    let (glyph, tooltip) = caption.look(maximized);
    let button = gtk::Button::builder()
        .child(&icons::glyph(glyph, 12))
        .tooltip_text(tooltip)
        .action_name(caption.action())
        .focus_on_click(false)
        .css_classes(["caption", caption.css_class()])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decoration layout and the captions each side shows.
    struct LayoutCase {
        layout: &'static str,
        start: &'static [Caption],
        end: &'static [Caption],
    }

    #[test]
    fn the_decoration_layout_decides_which_captions_show_where() {
        use Caption::{Close, Maximize, Minimize};
        let cases = [
            LayoutCase {
                layout: "menu:minimize,maximize,close",
                start: &[],
                end: &[Minimize, Maximize, Close],
            },
            LayoutCase {
                layout: "close,minimize,maximize:",
                start: &[Close, Minimize, Maximize],
                end: &[],
            },
            LayoutCase {
                layout: "appmenu:close",
                start: &[],
                end: &[Close],
            },
            LayoutCase {
                layout: "minimize,close",
                start: &[Minimize, Close],
                end: &[],
            },
        ];
        for case in cases {
            assert_eq!(
                captions_for(case.layout, gtk::PackType::Start),
                case.start,
                "{}",
                case.layout
            );
            assert_eq!(
                captions_for(case.layout, gtk::PackType::End),
                case.end,
                "{}",
                case.layout
            );
        }
    }

    #[test]
    fn the_maximize_button_offers_restore_when_maximized() {
        assert_eq!(Caption::Maximize.look(false), (Glyph::Maximize, "Maximize"));
        assert_eq!(Caption::Maximize.look(true), (Glyph::Restore, "Restore"));
    }
}
