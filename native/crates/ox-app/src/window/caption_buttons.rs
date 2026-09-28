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

/// The edge of a caption glyph: thin 12-pixel lines, as index.html draws them.
const GLYPH_SIZE: i32 = 12;

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

/// Whether the window is maximised, which decides what the middle caption
/// offers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum WindowState {
    /// The window has a size of its own.
    #[default]
    Normal,
    /// The window fills the screen.
    Maximized,
}

impl WindowState {
    /// The state `window` is in now.
    fn of(window: &gtk::Window) -> Self {
        if window.is_maximized() {
            WindowState::Maximized
        } else {
            WindowState::Normal
        }
    }
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

    /// The glyph the button shows while the window is in `state`.
    const fn glyph(self, state: WindowState) -> Glyph {
        match (self, state) {
            (Caption::Minimize, _) => Glyph::Minus,
            (Caption::Maximize, WindowState::Normal) => Glyph::Maximize,
            (Caption::Maximize, WindowState::Maximized) => Glyph::Restore,
            (Caption::Close, _) => Glyph::Close,
        }
    }

    /// The tooltip and accessible name (index.html's `title`) while the
    /// window is in `state`.
    const fn tooltip(self, state: WindowState) -> &'static str {
        match (self, state) {
            (Caption::Minimize, _) => "Minimize",
            (Caption::Maximize, WindowState::Normal) => "Maximize",
            (Caption::Maximize, WindowState::Maximized) => "Restore",
            (Caption::Close, _) => "Close window",
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

/// The desktop settings the buttons follow and the handler that follows
/// them, disconnected when the buttons go away.
#[derive(Debug)]
struct LayoutWatch {
    settings: gtk::Settings,
    handler: glib::SignalHandlerId,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{LayoutWatch, WindowState};

    /// Private state of [`super::CaptionButtons`].
    #[derive(Debug, Default)]
    pub struct CaptionButtons {
        pub(super) side: OnceCell<gtk::PackType>,
        pub(super) window_state: Cell<WindowState>,
        pub(super) layout_watch: RefCell<Option<LayoutWatch>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CaptionButtons {
        const NAME: &'static str = "OxCaptionButtons";
        type Type = super::CaptionButtons;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for CaptionButtons {
        fn dispose(&self) {
            if let Some(watch) = self.layout_watch.take() {
                watch.settings.disconnect(watch.handler);
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
    ///
    /// # Panics
    ///
    /// Never: a new object has no side yet.
    pub fn new(window: &gtk::Window, side: gtk::PackType) -> Self {
        let buttons: Self = glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .build();
        buttons.add_css_class("caption-buttons");
        let side_is_new = buttons.imp().side.set(side).is_ok();
        assert!(side_is_new, "a new object has no side yet");
        buttons.follow_decoration_layout(&window.settings());
        window.connect_maximized_notify(glib::clone!(
            #[weak]
            buttons,
            move |window| buttons.set_window_state(WindowState::of(window))
        ));
        buttons.set_window_state(WindowState::of(window));
        buttons
    }

    fn follow_decoration_layout(&self, settings: &gtk::Settings) {
        let handler = settings.connect_gtk_decoration_layout_notify(glib::clone!(
            #[weak(rename_to = buttons)]
            self,
            move |_| buttons.rebuild()
        ));
        let watch = LayoutWatch {
            settings: settings.clone(),
            handler,
        };
        self.imp().layout_watch.replace(Some(watch));
    }

    fn set_window_state(&self, state: WindowState) {
        self.imp().window_state.set(state);
        self.rebuild();
    }

    /// The captions shown, in order.
    fn captions(&self) -> Vec<Caption> {
        let watch = self.imp().layout_watch.borrow();
        let layout = watch
            .as_ref()
            .and_then(|watch| watch.settings.gtk_decoration_layout())
            .unwrap_or_default();
        let side = self.imp().side.get().expect("CaptionButtons::new sets the side");
        captions_for(&layout, *side)
    }

    fn rebuild(&self) {
        remove_children(self);
        let state = self.imp().window_state.get();
        for caption in self.captions() {
            self.append(&caption_button(caption, state));
        }
    }
}

fn caption_button(caption: Caption, state: WindowState) -> gtk::Button {
    let tooltip = caption.tooltip(state);
    let button = gtk::Button::builder()
        .child(&icons::glyph(caption.glyph(state), GLYPH_SIZE))
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
        let normal = WindowState::Normal;
        let maximized = WindowState::Maximized;
        assert_eq!(Caption::Maximize.glyph(normal), Glyph::Maximize);
        assert_eq!(Caption::Maximize.tooltip(normal), "Maximize");
        assert_eq!(Caption::Maximize.glyph(maximized), Glyph::Restore);
        assert_eq!(Caption::Maximize.tooltip(maximized), "Restore");
    }
}
