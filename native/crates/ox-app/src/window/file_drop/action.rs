// SPDX-License-Identifier: AGPL-3.0-only
//! What a drop does in a folder: copy, move, link, or ask with the drop
//! menu (DND-017, DND-018).
//!
//! Beyond the Python app, which always copied. The action comes from what
//! the drag offers once the desktop has applied the user's modifier: on
//! Wayland GNOME Shell picks one action (Shift moves, Ctrl copies, Alt
//! asks), and on X11 GTK narrows the offer the same way (Ctrl+Shift
//! links; the middle button asks). When several actions remain, a drop
//! copies. A plain drag of this app's own items then does what Windows
//! Explorer does: it moves them onto a folder of their drive and copies
//! them to another drive ([`drive`](super::drive)); Ctrl held at the drop
//! copies and Shift moves. Ask opens the menu of Windows Explorer's
//! right-button drag: Copy here, Move here, Create links here and Cancel.
//!
//! Safety rule "a plain drop never removes its source" (DND-009): a drag
//! from another app that did not offer a copy when it reached the window
//! is refused, as `NativeFileDrop` refused a source offering only Move.
//! Once the desktop applies a modifier, the offer narrows to the chosen
//! action, so the first offer is what tells the two apart. A drag from
//! another app is never turned into a move; a move of this app's own items
//! is run by its own transfer engine, and the drag is still finished as a
//! copy, so its source never deletes anything.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use ox_core::ops::is_recycle_bin_item;

use super::drive::on_same_drive;
use super::{own_dragged_items, DropDestination};
use crate::icons::Icon;
use crate::window::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use crate::window::window_action::WindowAction;
use crate::window::zip_copies::is_zip_copy;
use crate::window::BrowserWindow;

/// Where a drag comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragOrigin {
    /// A window of this app, which never deletes what it offered.
    ThisApp,
    /// Another app, or another instance of this one.
    OtherApp,
}

impl DragOrigin {
    /// Where `drop`'s drag comes from: GDK has the drag itself only for a
    /// drag of this process.
    pub(crate) fn of(drop: &gdk::Drop) -> Self {
        if drop.drag().is_some() {
            DragOrigin::ThisApp
        } else {
            DragOrigin::OtherApp
        }
    }
}

/// The actions a drag offered when it first reached the window, kept for
/// the rest of its hover (see the module's safety rule).
#[derive(Debug)]
pub(crate) struct FirstOffer {
    /// The drop the offer belongs to.
    drop: glib::WeakRef<gdk::Drop>,
    /// What it offered first.
    offered: gdk::DragAction,
    /// The last folder the drag was over and whether its items are on that
    /// folder's drive, so a hover does not read them again at each motion.
    drive: Option<(String, bool)>,
}

/// The keys that change what a drop does.
const ACTION_KEYS: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK
    .union(gdk::ModifierType::SHIFT_MASK)
    .union(gdk::ModifierType::ALT_MASK);

/// What a drop does with its items in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropAction {
    /// Copies them in: the default.
    Copy,
    /// Moves them in (Shift).
    Move,
    /// Makes links to them (Ctrl+Shift).
    Link,
    /// Asks with the drop menu (Alt, or the middle button).
    Ask,
}

impl DropAction {
    /// Every action, in the order a drop prefers them when the drag
    /// offers several.
    const PREFERENCE: [Self; 4] = [Self::Copy, Self::Move, Self::Link, Self::Ask];

    /// The action a drop runs when the drag offers `offered`: the one the
    /// user's modifier left, or copy when several remain; `None` when the
    /// drag offers none of them.
    pub(crate) fn from_offered(offered: gdk::DragAction) -> Option<Self> {
        Self::PREFERENCE
            .into_iter()
            .find(|action| offered.contains(action.as_drag_action()))
    }

    /// The action a drop from `origin` runs when its drag offered
    /// `first_offered` on arrival and offers `offered` now; `None` when it
    /// is refused.
    pub(crate) fn for_drop(
        origin: DragOrigin,
        first_offered: gdk::DragAction,
        offered: gdk::DragAction,
    ) -> Option<Self> {
        let offers_a_copy = first_offered.contains(gdk::DragAction::COPY);
        if origin == DragOrigin::OtherApp && !offers_a_copy {
            return None;
        }
        Self::from_offered(offered)
    }

    /// The action a drop that settled on `self` runs once the keys `held`
    /// at the drop and the drive are known, as in Windows Explorer: a copy
    /// of this app's own items becomes a move when Shift is held, or when
    /// no key is held and `same_drive` says the items are on the drive of
    /// the folder they are dropped on. Ctrl keeps the copy, and a drop
    /// from another app keeps whatever it settled on.
    pub(crate) fn on_drive(
        self,
        origin: DragOrigin,
        held: gdk::ModifierType,
        same_drive: impl FnOnce() -> bool,
    ) -> Self {
        if origin != DragOrigin::ThisApp || self != Self::Copy {
            return self;
        }
        let held = held & ACTION_KEYS;
        if held == gdk::ModifierType::SHIFT_MASK || (held.is_empty() && same_drive()) {
            Self::Move
        } else {
            self
        }
    }

    /// The action as GTK names it.
    pub(crate) fn as_drag_action(self) -> gdk::DragAction {
        match self {
            Self::Copy => gdk::DragAction::COPY,
            Self::Move => gdk::DragAction::MOVE,
            Self::Link => gdk::DragAction::LINK,
            Self::Ask => gdk::DragAction::ASK,
        }
    }

    /// The name the drop menu's items give `win.drop-choice`.
    const fn name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Link => "link",
            Self::Ask => "ask",
        }
    }

    /// The action a drop menu item names; `None` for Cancel.
    fn from_name(name: &str) -> Option<Self> {
        Self::PREFERENCE.into_iter().find(|action| action.name() == name)
    }
}

/// The target of the drop menu's Cancel.
const CANCEL_CHOICE: &str = "cancel";

/// A drop that waits for the drop menu's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingDrop {
    /// The dropped items.
    uris: Vec<String>,
    /// The folder they were dropped on.
    folder: String,
}

/// The drop menu's items.
fn drop_menu_entries() -> Vec<MenuEntry> {
    let choice = |label: &str, glyph: Icon, target: &str| {
        MenuEntry::from(MenuItem::with_text_target(
            label,
            glyph,
            WindowAction::DropChoice,
            target,
        ))
    };
    vec![
        choice(
            ox_core::i18n::gettext_static("Copy here"),
            Icon::Copy,
            DropAction::Copy.name(),
        ),
        choice(
            ox_core::i18n::gettext_static("Move here"),
            Icon::ArrowRight,
            DropAction::Move.name(),
        ),
        choice(
            ox_core::i18n::gettext_static("Create links here"),
            Icon::Link,
            DropAction::Link.name(),
        ),
        MenuEntry::Divider,
        choice(
            ox_core::i18n::gettext_static("Cancel"),
            Icon::Dismiss,
            CANCEL_CHOICE,
        ),
    ]
}

impl BrowserWindow {
    /// The action `drop` would run now onto `destination`, or `None` when
    /// it is refused.
    pub(super) fn drop_action(
        &self,
        drop: &gdk::Drop,
        destination: Option<&DropDestination>,
    ) -> Option<DropAction> {
        let first_offered = self.first_offer(drop);
        let origin = DragOrigin::of(drop);
        let action = DropAction::for_drop(origin, first_offered, drop.actions())?;
        Some(action.on_drive(origin, self.held_drop_keys(), || {
            self.drop_on_same_drive(drop, destination)
        }))
    }

    /// The keys held now that change what a drop does.
    fn held_drop_keys(&self) -> gdk::ModifierType {
        let keyboard = WidgetExt::display(self)
            .default_seat()
            .and_then(|seat| seat.keyboard());
        keyboard.map_or_else(gdk::ModifierType::empty, |keyboard| {
            keyboard.modifier_state() & ACTION_KEYS
        })
    }

    /// True when this app's own `drop` carries items that are all on the
    /// drive of `destination`, a folder; remembered for the drop's last
    /// folder.
    fn drop_on_same_drive(&self, drop: &gdk::Drop, destination: Option<&DropDestination>) -> bool {
        let Some(DropDestination::Folder(folder)) = destination else {
            return false;
        };
        if let Some((known, same)) = self.known_drive(drop) {
            if known == *folder {
                return same;
            }
        }
        let same = own_dragged_items(drop).is_some_and(|uris| {
            !uris
                .iter()
                .any(|uri| is_zip_copy(uri) || is_recycle_bin_item(uri))
                && on_same_drive(&uris, folder)
        });
        if let Some(offer) = self.imp().first_offer.borrow_mut().as_mut() {
            offer.drive = Some((folder.clone(), same));
        }
        same
    }

    /// The folder and drive answer remembered for `drop`.
    fn known_drive(&self, drop: &gdk::Drop) -> Option<(String, bool)> {
        let first = self.imp().first_offer.borrow();
        let offer = first.as_ref()?;
        (offer.drop.upgrade().as_ref() == Some(drop))
            .then(|| offer.drive.clone())
            .flatten()
    }

    /// What `drop` offered when it first reached the window, remembered
    /// now when it is new.
    fn first_offer(&self, drop: &gdk::Drop) -> gdk::DragAction {
        let mut first = self.imp().first_offer.borrow_mut();
        let is_known = first
            .as_ref()
            .is_some_and(|offer| offer.drop.upgrade().as_ref() == Some(drop));
        if !is_known {
            *first = Some(FirstOffer {
                drop: drop.downgrade(),
                offered: drop.actions(),
                drive: None,
            });
        }
        first
            .as_ref()
            .map_or_else(|| drop.actions(), |offer| offer.offered)
    }

    /// Opens the drop menu where the drop happened, keeping the drop of
    /// `uris` onto `folder` until the user answers.
    pub(super) fn ask_drop_action(&self, uris: Vec<String>, folder: String) {
        self.imp()
            .pending_drop
            .replace(Some(PendingDrop { uris, folder }));
        let menu = self.drop_menu_popover();
        menu.set_entries(drop_menu_entries());
        let (x, y) = self.imp().drop_point.get();
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let target = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        menu.set_pointing_to(Some(&target));
        menu.popup();
    }

    /// The drop menu, parented to the folder pane, whose coordinates the
    /// drop point is in.
    fn drop_menu_popover(&self) -> MenuPopover {
        if let Some(menu) = self.imp().drop_menu.get() {
            return menu.clone();
        }
        let menu = MenuPopover::new(Vec::new());
        menu.set_offset(0, 0);
        menu.set_parent(self.folder_pane());
        self.imp()
            .drop_menu
            .set(menu.clone())
            .expect("the drop menu is built once");
        menu
    }

    /// Runs the drop menu's answer `choice` on the waiting drop (`copy`,
    /// `move`, `link` or `cancel`). A menu closed without an answer leaves
    /// the drop waiting, unrun, until the next drop menu replaces it.
    pub(in crate::window) fn answer_drop_menu(&self, choice: &str) {
        let Some(pending) = self.imp().pending_drop.take() else {
            return;
        };
        let Some(action) = DropAction::from_name(choice).filter(|action| *action != DropAction::Ask) else {
            return;
        };
        let destination = super::DropDestination::Folder(pending.folder);
        self.run_drop(&pending.uris, Some(destination), action);
    }

    /// The drop menu, for tests.
    #[cfg(test)]
    pub(in crate::window) fn drop_menu(&self) -> MenuPopover {
        self.drop_menu_popover()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One offer and the action a drop runs for it.
    struct OfferCase {
        offered: gdk::DragAction,
        action: Option<DropAction>,
    }

    /// parity: DND-017, DND-018
    #[test]
    fn a_drop_runs_the_modifiers_action_and_copies_when_several_remain() {
        let cases = [
            OfferCase {
                offered: gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK,
                action: Some(DropAction::Copy),
            },
            OfferCase {
                offered: gdk::DragAction::MOVE,
                action: Some(DropAction::Move),
            },
            OfferCase {
                offered: gdk::DragAction::LINK,
                action: Some(DropAction::Link),
            },
            OfferCase {
                offered: gdk::DragAction::ASK,
                action: Some(DropAction::Ask),
            },
            OfferCase {
                offered: gdk::DragAction::MOVE | gdk::DragAction::ASK,
                action: Some(DropAction::Move),
            },
            OfferCase {
                offered: gdk::DragAction::empty(),
                action: None,
            },
        ];

        for case in cases {
            assert_eq!(
                DropAction::from_offered(case.offered),
                case.action,
                "{:?}",
                case.offered
            );
        }
    }

    /// parity: DND-009, DND-017
    #[test]
    fn another_apps_drag_that_offered_no_copy_is_refused() {
        let move_only = gdk::DragAction::MOVE;
        let copy_or_move = gdk::DragAction::COPY | gdk::DragAction::MOVE;

        let move_only_source = DropAction::for_drop(DragOrigin::OtherApp, move_only, move_only);
        let shift_over_a_copy_source = DropAction::for_drop(DragOrigin::OtherApp, copy_or_move, move_only);
        let own_shift_drag = DropAction::for_drop(DragOrigin::ThisApp, move_only, move_only);

        assert_eq!(move_only_source, None, "a plain drop never removes its source");
        assert_eq!(shift_over_a_copy_source, Some(DropAction::Move));
        assert_eq!(
            own_shift_drag,
            Some(DropAction::Move),
            "this app never deletes what it offered"
        );
    }

    /// parity: DND-017
    #[test]
    fn a_plain_drag_of_own_items_moves_within_a_drive_and_copies_across() {
        let none = gdk::ModifierType::empty();
        let control = gdk::ModifierType::CONTROL_MASK;
        let shift = gdk::ModifierType::SHIFT_MASK;
        let alt = gdk::ModifierType::ALT_MASK;
        let own = DragOrigin::ThisApp;
        let copy = DropAction::Copy;

        assert_eq!(copy.on_drive(own, none, || true), DropAction::Move);
        assert_eq!(copy.on_drive(own, none, || false), DropAction::Copy);
        assert_eq!(
            copy.on_drive(own, control, || true),
            DropAction::Copy,
            "Ctrl copies"
        );
        assert_eq!(
            copy.on_drive(own, shift, || false),
            DropAction::Move,
            "Shift moves"
        );
        assert_eq!(copy.on_drive(own, alt, || true), DropAction::Copy);
        assert_eq!(
            copy.on_drive(own, gdk::ModifierType::BUTTON1_MASK, || true),
            DropAction::Move,
            "a held button is no key"
        );
        assert_eq!(
            copy.on_drive(DragOrigin::OtherApp, none, || true),
            DropAction::Copy,
            "another app's items are never moved unasked"
        );
        assert_eq!(
            copy.on_drive(DragOrigin::OtherApp, shift, || true),
            DropAction::Copy
        );
        for settled in [DropAction::Move, DropAction::Link, DropAction::Ask] {
            assert_eq!(settled.on_drive(own, control, || true), settled);
        }
    }

    /// parity: DND-017
    #[test]
    fn the_drive_is_only_read_for_a_plain_drop_of_own_items() {
        let read = std::cell::Cell::new(false);
        let mark = || {
            read.set(true);
            true
        };

        DropAction::Copy.on_drive(DragOrigin::ThisApp, gdk::ModifierType::CONTROL_MASK, mark);
        DropAction::Copy.on_drive(DragOrigin::OtherApp, gdk::ModifierType::empty(), mark);
        DropAction::Link.on_drive(DragOrigin::ThisApp, gdk::ModifierType::empty(), mark);

        assert!(!read.get());
    }

    /// parity: DND-018
    #[test]
    fn the_drop_menu_offers_copy_move_link_and_cancel() {
        let labels: Vec<String> = drop_menu_entries()
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label,
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect();

        assert_eq!(
            labels,
            ["Copy here", "Move here", "Create links here", "-", "Cancel"]
        );
        assert_eq!(DropAction::from_name(CANCEL_CHOICE), None);
    }
}
