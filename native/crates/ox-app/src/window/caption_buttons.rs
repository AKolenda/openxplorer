// SPDX-License-Identifier: AGPL-3.0-only
//! The window's own minimise, maximise and close buttons.
//!
//! Ports `.window-buttons` in `v2.0.0:desktop/ui/index.html` and `style.css`:
//! 46-pixel-wide, full-height buttons with thin 12-pixel glyphs and a red
//! close hover. `GtkWindowControls` can only draw the icon theme's bold
//! symbolic icons, so these are plain buttons running GTK's built-in
//! `window.minimize` and `window.toggle-maximized` actions, and the
//! window's `win.close-window`, which asks first whether the window may
//! close ([`super::closing`]).
//! They still follow GNOME: the desktop's `gtk-decoration-layout` decides
//! which buttons exist and on which side, and changes to it apply at once.
//!
//! [`CaptionButtons`] follows the window it is placed in, as GTK's own
//! window controls do: it starts when the widget gets a root window and
//! stops when it loses it, so the window template only names its side.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};

use super::widget_tree::remove_children;
use super::window_action::WindowAction;

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

/// Which end of the title bar a set of caption buttons sits at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "OxCaptionSide")]
pub(crate) enum CaptionSide {
    /// The start, before the tabs: the part of `gtk-decoration-layout`
    /// before the colon.
    #[default]
    Start,
    /// The end, after the open-windows button: the part after the colon.
    End,
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

    /// The action the button runs. Close asks first whether the window
    /// may close (`askClose`, TAB-049), so it runs the window's own action
    /// rather than GTK's `window.close`.
    fn action(self) -> String {
        match self {
            Caption::Minimize => "window.minimize".to_owned(),
            Caption::Maximize => "window.toggle-maximized".to_owned(),
            Caption::Close => WindowAction::CloseWindow.detailed_name(),
        }
    }

    /// The glyph the button shows while the window is in `state`.
    const fn glyph(self, state: WindowState) -> Icon {
        match (self, state) {
            (Caption::Minimize, _) => Icon::Subtract,
            (Caption::Maximize, WindowState::Normal) => Icon::Maximize,
            (Caption::Maximize, WindowState::Maximized) => Icon::SquareMultiple,
            (Caption::Close, _) => Icon::Dismiss,
        }
    }

    /// The tooltip and accessible name (index.html's `title`) while the
    /// window is in `state`.
    fn tooltip(self, state: WindowState) -> &'static str {
        match (self, state) {
            (Caption::Minimize, _) => ox_core::i18n::gettext_static("Minimize"),
            (Caption::Maximize, WindowState::Normal) => ox_core::i18n::gettext_static("Maximize"),
            (Caption::Maximize, WindowState::Maximized) => ox_core::i18n::gettext_static("Restore"),
            (Caption::Close, _) => ox_core::i18n::gettext_static("Close window"),
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
pub(super) fn captions_for(layout: &str, side: CaptionSide) -> Vec<Caption> {
    let (start, end) = layout.split_once(':').unwrap_or((layout, ""));
    let names = match side {
        CaptionSide::Start => start,
        CaptionSide::End => end,
    };
    names.split(',').filter_map(Caption::from_layout_name).collect()
}

/// The desktop settings the buttons follow and the handler that follows
/// them.
#[derive(Debug)]
struct LayoutWatch {
    settings: gtk::Settings,
    handler: glib::SignalHandlerId,
}

/// The window the buttons follow and the handler that follows whether it
/// is maximised.
#[derive(Debug)]
struct WindowWatch {
    window: glib::WeakRef<gtk::Window>,
    handler: glib::SignalHandlerId,
}

#[expect(
    unreachable_pub,
    reason = "the glib::Properties derive always makes the side property's accessors pub"
)]
mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{CaptionSide, LayoutWatch, WindowState, WindowWatch};

    /// Private state of [`super::CaptionButtons`].
    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::CaptionButtons)]
    pub(crate) struct CaptionButtons {
        /// The end of the title bar the buttons sit at; the window
        /// template sets it.
        #[property(get, construct_only, builder(CaptionSide::Start))]
        pub(super) side: Cell<CaptionSide>,
        /// Whether the window is maximised, which the middle caption shows.
        pub(super) window_state: Cell<WindowState>,
        /// The desktop setting that decides which captions exist.
        pub(super) layout_watch: RefCell<Option<LayoutWatch>>,
        /// The window whose state the buttons show.
        pub(super) window_watch: RefCell<Option<WindowWatch>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CaptionButtons {
        const NAME: &'static str = "OxCaptionButtons";
        type Type = super::CaptionButtons;
        type ParentType = gtk::Box;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CaptionButtons {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().add_css_class("caption-buttons");
        }

        fn dispose(&self) {
            self.obj().stop_following_window();
        }
    }

    impl WidgetImpl for CaptionButtons {
        fn root(&self) {
            self.parent_root();
            self.obj().follow_window();
        }

        fn unroot(&self) {
            self.obj().stop_following_window();
            self.parent_unroot();
        }
    }

    impl BoxImpl for CaptionButtons {}
}

glib::wrapper! {
    /// The caption buttons on one side of the title bar.
    pub(crate) struct CaptionButtons(ObjectSubclass<imp::CaptionButtons>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl CaptionButtons {
    /// Shows the captions the desktop puts on this side of the window the
    /// buttons now belong to, and keeps them in step with the desktop
    /// setting and the window state.
    fn follow_window(&self) {
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };
        self.follow_decoration_layout(&window.settings());
        let handler = window.connect_maximized_notify(glib::clone!(
            #[weak(rename_to = buttons)]
            self,
            move |window| buttons.set_window_state(WindowState::of(window))
        ));
        let watch = WindowWatch {
            window: window.downgrade(),
            handler,
        };
        self.imp().window_watch.replace(Some(watch));
        self.set_window_state(WindowState::of(&window));
    }

    /// Disconnects from the window and the desktop settings, so a closed
    /// window's buttons leave no handler behind.
    fn stop_following_window(&self) {
        let imp = self.imp();
        if let Some(watch) = imp.window_watch.take() {
            if let Some(window) = watch.window.upgrade() {
                window.disconnect(watch.handler);
            }
        }
        if let Some(watch) = imp.layout_watch.take() {
            watch.settings.disconnect(watch.handler);
        }
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
        captions_for(&layout, self.side())
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
        .child(&icons::image(caption.glyph(state), GLYPH_SIZE))
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
                captions_for(case.layout, CaptionSide::Start),
                case.start,
                "{}",
                case.layout
            );
            assert_eq!(
                captions_for(case.layout, CaptionSide::End),
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
        assert_eq!(Caption::Maximize.glyph(normal), Icon::Maximize);
        assert_eq!(Caption::Maximize.tooltip(normal), "Maximize");
        assert_eq!(Caption::Maximize.glyph(maximized), Icon::SquareMultiple);
        assert_eq!(Caption::Maximize.tooltip(maximized), "Restore");
    }
}
