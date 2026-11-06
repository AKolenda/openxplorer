// SPDX-License-Identifier: AGPL-3.0-only
//! How a menu opens: as Windows 11's menus do, it fades in while it slides
//! a few pixels in from the edge nearest what it opens from (below its
//! button or the pointer it slides down, above them up, beside its row
//! sideways), in [`OPENING_MOTION`] on Fluent's fast-in curve (ui-spec.md
//! M02, M07). The skin draws the motion (menus.css) as a CSS animation of
//! the menu's contents, which GTK skips while the desktop turns animations
//! off (`gtk-enable-animations`, ACC-010): the menu then shows at once.
//!
//! GTK carries an animation on while its name stays, even across a hide
//! and show it never restyled the menu in between, so each opening takes
//! the other of two names and starts afresh. The menu slides the way it
//! was asked to open, then, once GTK has placed its popup, the way GTK
//! placed it, which differs when GTK flipped it at the monitor's edge.
//!
//! GTK tells the compositor that the contents of a popup whose background
//! is opaque are opaque, whatever their CSS opacity (`get_opaque_rect` in
//! GTK 4.14's gtknative.c), and a compositor then draws the fading menu
//! over black, as Weston did. GTK does read the widget's own opacity
//! there, so the contents stay just short of opaque while the menu opens.
//!
//! A menu closes at once, without Windows' fade: fading would keep its
//! popup, which holds the pointer grab, on screen after the command chosen
//! in it has opened the next menu, as "Show more options" does, and
//! Wayland refuses a second grabbing popup that is not the first's child.

use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::MenuPopover;

/// How long a menu takes to fade and slide in: Fluent's fast entrance,
/// `ControlFastAnimationDuration` (ui-spec.md M02). menus.css says the same.
pub(crate) const OPENING_MOTION: Duration = Duration::from_millis(167);

/// The contents' own opacity while the menu opens: short enough of 1 that
/// GTK does not call them opaque, too close to it to see. GTK keeps a
/// widget's opacity in eight bits, so this is 254/255; 0.999 would round
/// back to 1.
const OPENING_OPACITY: f64 = 0.995;

/// The two classes an opening takes in turn, which name its animation.
const OPENING_CLASSES: [&str; 2] = ["opening-a", "opening-b"];

/// Which way a menu slides as it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slide {
    /// Down, out of the button or pointer above it.
    Down,
    /// Up, out of the button or pointer below it.
    Up,
    /// To the right, out of the row or button at its left.
    Right,
    /// To the left, out of the row or button at its right.
    Left,
}

impl Slide {
    /// Every way, for clearing the classes.
    const ALL: [Slide; 4] = [Slide::Down, Slide::Up, Slide::Right, Slide::Left];

    /// The class the skin slides the menu this way by.
    const fn css_class(self) -> &'static str {
        match self {
            Slide::Down => "slides-down",
            Slide::Up => "slides-up",
            Slide::Right => "slides-right",
            Slide::Left => "slides-left",
        }
    }

    /// The way a menu asked to open at `position` of its anchor slides.
    fn asked_for(position: gtk::PositionType) -> Self {
        match position {
            gtk::PositionType::Top => Slide::Up,
            gtk::PositionType::Right => Slide::Right,
            gtk::PositionType::Left => Slide::Left,
            _ => Slide::Down,
        }
    }

    /// The way a popup slides that GTK placed with its `surface_anchor`
    /// on the `rect_anchor` of its anchor, as GTK finds where it placed a
    /// popover (`update_popover_layout` in gtkpopover.c): beside the
    /// anchor when the two meet on opposite sides across, else below or
    /// above it. `None` when they meet on no opposite sides.
    fn placed(rect_anchor: gdk::Gravity, surface_anchor: gdk::Gravity) -> Option<Self> {
        match (across(rect_anchor), across(surface_anchor)) {
            (Side::End, Side::Start) => return Some(Slide::Right),
            (Side::Start, Side::End) => return Some(Slide::Left),
            _ => {}
        }
        match (down(rect_anchor), down(surface_anchor)) {
            (Side::End, Side::Start) => Some(Slide::Down),
            (Side::Start, Side::End) => Some(Slide::Up),
            _ => None,
        }
    }
}

/// Where along an axis a gravity points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The left or the top.
    Start,
    /// The middle.
    Middle,
    /// The right or the bottom.
    End,
}

/// Where across `gravity` points.
fn across(gravity: gdk::Gravity) -> Side {
    match gravity {
        gdk::Gravity::NorthWest | gdk::Gravity::West | gdk::Gravity::SouthWest => Side::Start,
        gdk::Gravity::NorthEast | gdk::Gravity::East | gdk::Gravity::SouthEast => Side::End,
        _ => Side::Middle,
    }
}

/// Where down `gravity` points.
fn down(gravity: gdk::Gravity) -> Side {
    match gravity {
        gdk::Gravity::NorthWest | gdk::Gravity::North | gdk::Gravity::NorthEast => Side::Start,
        gdk::Gravity::SouthWest | gdk::Gravity::South | gdk::Gravity::SouthEast => Side::End,
        _ => Side::Middle,
    }
}

impl MenuPopover {
    /// Starts the menu's opening motion afresh, sliding the way it asks
    /// to open, as it is about to show.
    pub(super) fn start_opening_motion(&self) {
        let imp = self.imp();
        let second = !imp.opening_turn.get();
        imp.opening_turn.set(second);
        let [first_class, second_class] = OPENING_CLASSES;
        let (class, other) = if second {
            (second_class, first_class)
        } else {
            (first_class, second_class)
        };
        self.remove_css_class(other);
        self.add_css_class(class);
        imp.opening_since.set(Some(Instant::now()));
        imp.placed.set(false);
        self.set_slide(Slide::asked_for(self.position()));
        self.keep_translucent_while_opening();
    }

    /// Keeps the menu's contents from counting as opaque until it has
    /// faded in, so the compositor blends it with what is under it.
    fn keep_translucent_while_opening(&self) {
        let imp = self.imp();
        if let Some(timer) = imp.opaque_again.take() {
            timer.remove();
        }
        let Some(contents) = self.first_child() else {
            return;
        };
        if !self.settings().is_gtk_enable_animations() {
            contents.set_opacity(1.0);
            return;
        }
        contents.set_opacity(OPENING_OPACITY);
        let timer = glib::timeout_add_local_once(
            OPENING_MOTION,
            glib::clone!(
                #[weak(rename_to = menu)]
                self,
                move || {
                    menu.imp().opaque_again.replace(None);
                    if let Some(contents) = menu.first_child() {
                        contents.set_opacity(1.0);
                    }
                }
            ),
        );
        imp.opaque_again.replace(Some(timer));
    }

    /// Watches where GTK places the menu's popup, once the menu has one.
    pub(super) fn follow_placement(&self) {
        let Some(surface) = self.surface() else { return };
        surface.connect_layout(glib::clone!(
            #[weak(rename_to = menu)]
            self,
            move |_, _, _| menu.slide_as_placed()
        ));
    }

    /// Slides the way GTK placed the menu's popup when GTK first lays it
    /// out after it shows, or again while it is still opening: GTK flips
    /// a menu that has no room where it asked to be.
    pub(super) fn slide_as_placed(&self) {
        let imp = self.imp();
        let opening = imp
            .opening_since
            .get()
            .is_some_and(|since| since.elapsed() <= OPENING_MOTION);
        if imp.placed.get() && !opening {
            return;
        }
        let Some(popup) = self
            .surface()
            .and_then(|surface| surface.dynamic_cast::<gdk::Popup>().ok())
        else {
            return;
        };
        imp.placed.set(true);
        if let Some(slide) = Slide::placed(popup.rect_anchor(), popup.surface_anchor()) {
            self.set_slide(slide);
        }
    }

    /// Slides the menu `slide`'s way.
    fn set_slide(&self, slide: Slide) {
        if self.imp().slide.replace(Some(slide)) == Some(slide) && self.has_css_class(slide.css_class()) {
            return;
        }
        for other in Slide::ALL.into_iter().filter(|other| *other != slide) {
            self.remove_css_class(other.css_class());
        }
        self.add_css_class(slide.css_class());
    }

    /// Which way the menu slid as it opened last, for tests.
    #[cfg(test)]
    pub(crate) fn slide(&self) -> Option<Slide> {
        self.imp().slide.get()
    }

    /// The class naming the animation of the menu's last opening, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn opening_class(&self) -> Option<&'static str> {
        OPENING_CLASSES
            .into_iter()
            .find(|class| self.has_css_class(class))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::Icon;
    use crate::test_support::harness::{skin, wait_until};
    use crate::window::menu_popover::{MenuEntry, MenuItem};
    use crate::window::window_action::WindowAction;

    #[test]
    fn a_menu_slides_out_of_the_side_gtk_placed_it_on() {
        use gdk::Gravity::{North, NorthEast, NorthWest, South, SouthEast, SouthWest, West};
        // Below a button, its left edges lined up (position Bottom,
        // halign Start), and flipped above it.
        assert_eq!(Slide::placed(SouthWest, NorthWest), Some(Slide::Down));
        assert_eq!(Slide::placed(NorthWest, SouthWest), Some(Slide::Up));
        assert_eq!(Slide::placed(South, North), Some(Slide::Down));
        // Beside a row, its top level with the row's (position Right,
        // valign Start), and flipped to the row's left.
        assert_eq!(Slide::placed(NorthEast, NorthWest), Some(Slide::Right));
        assert_eq!(Slide::placed(NorthWest, NorthEast), Some(Slide::Left));
        assert_eq!(Slide::placed(SouthEast, West), Some(Slide::Right));
        // Over its anchor: no side to slide out of.
        assert_eq!(Slide::placed(NorthWest, NorthWest), None);
    }

    /// A menu of a few rows opening at `anchor` in a button filling a
    /// window 900 pixels tall at the top of check.py's 1440 x 1000 screen.
    fn open_menu_at(anchor: gdk::Rectangle) -> (gtk::Window, MenuPopover) {
        let _ = skin();
        let button = gtk::Button::builder().label("Files").vexpand(true).build();
        let window = gtk::Window::builder()
            .default_width(400)
            .default_height(900)
            .child(&button)
            .build();
        window.present();
        wait_until("the window to be shown", || window.height() == 900);
        let entries: Vec<MenuEntry> = (0..5)
            .map(|row| MenuItem::new(&format!("Item {row}"), Icon::Copy, WindowAction::CopyAddress).into())
            .collect();
        let menu = MenuPopover::new(entries);
        menu.set_parent(&button);
        menu.set_pointing_to(Some(&anchor));
        menu.popup();
        wait_until("the menu to open", || menu.is_mapped());
        (window, menu)
    }

    /// A menu opening below the pointer slides down out of it, and one GTK
    /// flips above the pointer, near the bottom of the monitor, slides up.
    ///
    /// parity: LOOK-019
    #[gtk::test]
    fn a_menu_flipped_above_the_pointer_slides_up() {
        let (window, menu) = open_menu_at(gdk::Rectangle::new(20, 300, 1, 1));
        assert_eq!(
            menu.slide(),
            Some(Slide::Down),
            "below the pointer, with room there"
        );
        menu.popdown();
        menu.set_pointing_to(Some(&gdk::Rectangle::new(20, 860, 1, 1)));
        menu.popup();
        wait_until("the menu to open again", || menu.is_mapped());
        assert_eq!(menu.position(), gtk::PositionType::Bottom, "asked to open below");
        wait_until("the menu to slide up", || menu.slide() == Some(Slide::Up));
        menu.popdown();
        menu.unparent();
        window.destroy();
        crate::test_support::harness::settle();
    }
}
