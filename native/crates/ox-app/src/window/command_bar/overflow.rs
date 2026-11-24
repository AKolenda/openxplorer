// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar's commands that do not fit move into "…", as Windows
//! 11 File Explorer's command bar overflows.
//!
//! [`OverflowLayout`] lays out the bar's commands from New to "…" (`See
//! more`). While all of them fit it places them one after another, as a
//! box does; when the window, or a large text size, leaves too little
//! room, it leaves out commands from the right, the one before "…" first,
//! until the rest fit. New and "…" always stay, so every command is
//! still reachable: the ones left out are listed at the top of "…"
//! ([`super::CommandBar`] builds them). A separator stays only between
//! two shown commands.
//!
//! The commands left out keep their own visibility (Share where the
//! desktop cannot share, Extract all outside an archive): the layout
//! only stops drawing and placing them, with `set_child_visible`, which
//! never starts another layout. It tells the bar which commands it left
//! out after the allocation, so the menu is rebuilt outside it.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// The gap between two commands, in pixels, as `.command-group`'s
/// `border-spacing` was: 4, and 1 in a window 1190 pixels or narrower.
pub(super) const GAP: i32 = 4;
/// The gap in a window 1190 pixels wide or narrower.
pub(super) const NARROW_GAP: i32 = 1;

/// Whether `widget` always stays in the bar: New and "…".
fn always_shown(widget: &gtk::Widget) -> bool {
    widget.has_css_class("new-command") || widget.has_css_class("more-command")
}

/// Whether `widget` is a separator between groups of commands.
fn is_separator(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Separator>()
}

/// The children of `group` a layout places, with their natural widths.
fn laid_out_children(group: &gtk::Widget) -> Vec<(gtk::Widget, i32)> {
    let mut children = Vec::new();
    let mut child = group.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.should_layout() {
            let natural = widget.measure(gtk::Orientation::Horizontal, -1).1;
            children.push((widget, natural));
        }
    }
    children
}

/// Which of `children` show: every command `kept` says, and each
/// separator with a shown command on both sides of it before the next
/// separator; "…" does not count, as no separator comes before it.
fn shown_with_separators(children: &[(gtk::Widget, i32)], kept: &[bool]) -> Vec<bool> {
    let mut shown = kept.to_vec();
    for index in 0..children.len() {
        if !is_separator(&children[index].0) {
            continue;
        }
        let command_shown = |range: &mut dyn Iterator<Item = usize>| {
            range
                .take_while(|other| !is_separator(&children[*other].0))
                .any(|other| kept[other] && !children[other].0.has_css_class("more-command"))
        };
        let before = command_shown(&mut (0..index).rev());
        let after = command_shown(&mut (index + 1..children.len()));
        shown[index] = before && after;
    }
    shown
}

/// The width `children` take where `shown` says, with `gap` between them.
fn width_of(children: &[(gtk::Widget, i32)], shown: &[bool], gap: i32) -> i32 {
    let widths: Vec<i32> = children
        .iter()
        .zip(shown)
        .filter(|(_, shown)| **shown)
        .map(|((_, natural), _)| *natural)
        .collect();
    let gaps = gap * i32::try_from(widths.len().saturating_sub(1)).unwrap_or(0);
    widths.iter().sum::<i32>() + gaps
}

/// Which of `children` show in `width` pixels: all when they fit, else
/// all but the commands from the right, before "…", until the rest fit.
fn fit(children: &[(gtk::Widget, i32)], width: i32, gap: i32) -> Vec<bool> {
    let mut kept: Vec<bool> = children.iter().map(|(widget, _)| !is_separator(widget)).collect();
    let mut shown = shown_with_separators(children, &kept);
    let mut candidates = (0..children.len())
        .rev()
        .filter(|index| !is_separator(&children[*index].0) && !always_shown(&children[*index].0));
    while width_of(children, &shown, gap) > width {
        let Some(index) = candidates.next() else {
            break;
        };
        kept[index] = false;
        shown = shown_with_separators(children, &kept);
    }
    shown
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{always_shown, fit, is_separator, laid_out_children, width_of, GAP};

    /// Private state of [`super::OverflowLayout`].
    #[derive(Default)]
    pub(crate) struct OverflowLayout {
        /// The gap between two commands.
        pub(super) gap: Cell<i32>,
        /// The commands left out at the last allocation, in the bar's
        /// order.
        pub(super) left_out: RefCell<Vec<gtk::Widget>>,
        /// Called after an allocation that left out other commands.
        pub(super) on_change: RefCell<Option<Box<dyn Fn()>>>,
    }

    impl std::fmt::Debug for OverflowLayout {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("OverflowLayout")
                .field("gap", &self.gap)
                .field("left_out", &self.left_out)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OverflowLayout {
        const NAME: &'static str = "OxCommandOverflowLayout";
        type Type = super::OverflowLayout;
        type ParentType = gtk::LayoutManager;

        fn new() -> Self {
            Self {
                gap: Cell::new(GAP),
                ..Self::default()
            }
        }
    }

    impl ObjectImpl for OverflowLayout {}

    impl LayoutManagerImpl for OverflowLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            group: &gtk::Widget,
            orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let children = laid_out_children(group);
            if orientation == gtk::Orientation::Vertical {
                let (minimum, natural) = children.iter().fold((0, 0), |(minimum, natural), (child, _)| {
                    let (child_minimum, child_natural, _, _) = child.measure(orientation, -1);
                    (minimum.max(child_minimum), natural.max(child_natural))
                });
                return (minimum, natural, -1, -1);
            }
            let gap = self.gap.get();
            let all: Vec<bool> = vec![true; children.len()];
            // The least: New and "…" alone.
            let least: Vec<bool> = children.iter().map(|(child, _)| always_shown(child)).collect();
            (
                width_of(&children, &least, gap),
                width_of(&children, &all, gap),
                -1,
                -1,
            )
        }

        fn allocate(&self, group: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let children = laid_out_children(group);
            let gap = self.gap.get();
            let shown = fit(&children, width, gap);
            let rtl = group.direction() == gtk::TextDirection::Rtl;
            let mut left = 0;
            let mut left_out = Vec::new();
            for ((child, natural), shown) in children.iter().zip(&shown) {
                child.set_child_visible(*shown);
                if !*shown {
                    if !is_separator(child) {
                        left_out.push(child.clone());
                    }
                    continue;
                }
                let x = if rtl { width - left - natural } else { left };
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a command bar is a few thousand pixels wide"
                )]
                let place = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, 0.0));
                child.allocate(*natural, height, -1, Some(place));
                left += natural + gap;
            }
            let changed = *self.left_out.borrow() != left_out;
            if changed {
                self.left_out.replace(left_out);
                if let Some(on_change) = self.on_change.borrow().as_ref() {
                    on_change();
                }
            }
        }
    }
}

glib::wrapper! {
    /// Lays the command bar's commands out, leaving out from the right
    /// the ones that do not fit.
    pub(crate) struct OverflowLayout(ObjectSubclass<imp::OverflowLayout>)
        @extends gtk::LayoutManager;
}

impl OverflowLayout {
    /// Lays `group`'s children out, calling `on_change` after an
    /// allocation that left out other commands; `on_change` runs once
    /// the allocation is over.
    pub(super) fn install(group: &gtk::Box, on_change: impl Fn() + 'static) -> Self {
        let layout: Self = glib::Object::new();
        // Told during an allocation, the bar rebuilds its menu after it.
        let waiting = Rc::new(RefCell::new(None::<glib::SourceId>));
        let on_change = Rc::new(on_change);
        layout.imp().on_change.replace(Some(Box::new(move || {
            if waiting.borrow().is_some() {
                return;
            }
            let waiting_now = Rc::clone(&waiting);
            let on_change = Rc::clone(&on_change);
            let source = glib::idle_add_local_once(move || {
                waiting_now.replace(None);
                on_change();
            });
            waiting.replace(Some(source));
        })));
        group.set_layout_manager(Some(layout.clone()));
        layout
    }

    /// Sets the gap between two commands.
    pub(super) fn set_gap(&self, gap: i32) {
        if self.imp().gap.replace(gap) != gap {
            self.layout_changed();
        }
    }

    /// The commands left out at the last allocation, in the bar's order.
    pub(super) fn left_out(&self) -> Vec<gtk::Widget> {
        self.imp().left_out.borrow().clone()
    }
}
