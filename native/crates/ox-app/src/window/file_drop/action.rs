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
//! action, so the first offer is what tells the two apart. A plain drag
//! from another app is never turned into a move (Shift held over it still
//! moves, as the user asked); a move of this app's own items
//! is run by its own transfer engine, and the drag is still finished as a
//! copy, so its source never deletes anything.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use ox_core::ops::is_recycle_bin_item;

use super::drive::{answer_within, on_same_drive};
use super::DropDestination;
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

/// What deciding a drop reads from it: GTK's [`gdk::Drop`], or a stand-in
/// in tests, as GDK makes drops only for a real drag. It holds nothing
/// that reads the disk, as the decision runs at every motion of a drag on
/// the GTK thread.
pub(crate) trait OfferedDrop {
    /// The object that stands for the drop while it lasts.
    fn identity(&self) -> glib::Object;
    /// What the drag offers now, once the desktop applied any modifier.
    fn offered(&self) -> gdk::DragAction;
    /// Where the drag comes from.
    fn origin(&self) -> DragOrigin;
}

impl OfferedDrop for gdk::Drop {
    fn identity(&self) -> glib::Object {
        self.clone().upcast()
    }

    fn offered(&self) -> gdk::DragAction {
        self.actions()
    }

    fn origin(&self) -> DragOrigin {
        DragOrigin::of(self)
    }
}

/// The actions a drag offered when it first reached the window, kept for
/// the rest of its hover (see the module's safety rule).
#[derive(Debug)]
pub(crate) struct FirstOffer {
    /// The drop the offer belongs to.
    drop: glib::WeakRef<glib::Object>,
    /// What it offered first.
    offered: gdk::DragAction,
}

/// What a drop runs, once the keys held at the drop are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropRun {
    /// This action.
    Run(DropAction),
    /// A plain drop of this app's own items: a move onto a folder of their
    /// drive and a copy anywhere else, decided once the items are read,
    /// off the GTK thread ([`drop_run_action`]).
    MoveWithinDrive,
}

impl DropRun {
    /// The action shown while the drag hovers, before the drive is known:
    /// a copy for a plain drop of this app's own items.
    pub(crate) fn shown(self) -> DropAction {
        match self {
            Self::Run(action) => action,
            Self::MoveWithinDrive => DropAction::Copy,
        }
    }
}

/// How long a drop waits to learn whether its items are on the drive of
/// the folder they are dropped on; past it, the drop copies.
const DRIVE_TIMEOUT: Duration = Duration::from_secs(2);

/// The action `run` takes on `uris` dropped onto `destination`. A plain
/// drop of this app's own items moves them onto a folder of their drive
/// and copies them anywhere else, and items out of a ZIP or the Recycle
/// Bin keep their own rules. The drive is read on a worker thread, for at
/// most [`DRIVE_TIMEOUT`], so a share that stopped answering never
/// freezes the window: a drive that does not answer in time counts as
/// another, and the drop copies.
pub(crate) async fn drop_run_action(
    run: DropRun,
    uris: &[String],
    destination: &DropDestination,
) -> DropAction {
    let DropRun::MoveWithinDrive = run else {
        return run.shown();
    };
    let DropDestination::Folder(folder) = destination else {
        return DropAction::Copy;
    };
    if uris
        .iter()
        .any(|uri| is_zip_copy(uri) || is_recycle_bin_item(uri))
    {
        return DropAction::Copy;
    }
    let (uris, folder) = (uris.to_vec(), folder.clone());
    let same = answer_within(DRIVE_TIMEOUT, move || on_same_drive(&uris, &folder)).await;
    if same {
        DropAction::Move
    } else {
        DropAction::Copy
    }
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

    /// What a drop that settled on `self` runs once the keys `held` at the
    /// drop are known, as in Windows Explorer: a copy of this app's own
    /// items becomes a move when Shift is held, and with no key held it
    /// moves within their drive ([`DropRun::MoveWithinDrive`]). Ctrl keeps
    /// the copy, and a drop from another app keeps whatever it settled on.
    pub(crate) fn with_keys(self, origin: DragOrigin, held: gdk::ModifierType) -> DropRun {
        if origin != DragOrigin::ThisApp || self != Self::Copy {
            return DropRun::Run(self);
        }
        let held = held & ACTION_KEYS;
        if held == gdk::ModifierType::SHIFT_MASK {
            DropRun::Run(Self::Move)
        } else if held.is_empty() {
            DropRun::MoveWithinDrive
        } else {
            DropRun::Run(self)
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
    /// What `drop` would run now, or `None` when it is refused. Nothing
    /// is read from disk: this runs at every motion of a drag.
    pub(super) fn drop_run(&self, drop: &impl OfferedDrop) -> Option<DropRun> {
        let first_offered = self.first_offer(drop);
        let origin = drop.origin();
        let action = DropAction::for_drop(origin, first_offered, drop.offered())?;
        Some(action.with_keys(origin, self.held_drop_keys()))
    }

    /// The action `drop` shows now, or `None` when it is refused.
    pub(super) fn drop_action(&self, drop: &impl OfferedDrop) -> Option<DropAction> {
        self.drop_run(drop).map(DropRun::shown)
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

    /// What `drop` offered when it first reached the window, remembered
    /// now when it is new.
    fn first_offer(&self, drop: &impl OfferedDrop) -> gdk::DragAction {
        let identity = drop.identity();
        let mut first = self.imp().first_offer.borrow_mut();
        let is_known = first
            .as_ref()
            .is_some_and(|offer| offer.drop.upgrade().as_ref() == Some(&identity));
        if !is_known {
            *first = Some(FirstOffer {
                drop: identity.downgrade(),
                offered: drop.offered(),
            });
        }
        first
            .as_ref()
            .map_or_else(|| drop.offered(), |offer| offer.offered)
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
    fn a_plain_drag_of_own_items_moves_within_a_drive_and_keys_choose_otherwise() {
        let none = gdk::ModifierType::empty();
        let control = gdk::ModifierType::CONTROL_MASK;
        let shift = gdk::ModifierType::SHIFT_MASK;
        let alt = gdk::ModifierType::ALT_MASK;
        let own = DragOrigin::ThisApp;
        let copy = DropAction::Copy;

        assert_eq!(copy.with_keys(own, none), DropRun::MoveWithinDrive);
        assert_eq!(
            copy.with_keys(own, control),
            DropRun::Run(DropAction::Copy),
            "Ctrl copies"
        );
        assert_eq!(
            copy.with_keys(own, shift),
            DropRun::Run(DropAction::Move),
            "Shift moves"
        );
        assert_eq!(copy.with_keys(own, alt), DropRun::Run(DropAction::Copy));
        assert_eq!(
            copy.with_keys(own, gdk::ModifierType::BUTTON1_MASK),
            DropRun::MoveWithinDrive,
            "a held button is no key"
        );
        assert_eq!(
            copy.with_keys(DragOrigin::OtherApp, none),
            DropRun::Run(DropAction::Copy),
            "another app's items are never moved unasked"
        );
        for settled in [DropAction::Move, DropAction::Link, DropAction::Ask] {
            assert_eq!(settled.with_keys(own, none), DropRun::Run(settled));
        }
        assert_eq!(
            DropRun::MoveWithinDrive.shown(),
            DropAction::Copy,
            "a hover shows a copy"
        );
    }

    /// The drop moves its items within their drive and copies them across
    /// drives, out of a ZIP and onto anything but a folder; a drop that
    /// settled on an action keeps it.
    ///
    /// parity: DND-017
    #[gtk::test]
    fn a_plain_drop_of_own_items_reads_their_drive_once_dropped() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let item = folder.path().join("Notes.txt");
        std::fs::write(&item, "notes").expect("the item is written");
        let destination = folder.path().join("Documents");
        std::fs::create_dir(&destination).expect("the folder is made");
        let uri = |path: &std::path::Path| gtk::gio::File::for_path(path).uri().to_string();
        let items = [uri(&item)];
        let into_folder = DropDestination::Folder(uri(&destination));
        let into_share = DropDestination::Folder("smb://nas/share/".to_owned());
        let run = |run, items: &[String], destination: &DropDestination| {
            glib::MainContext::default().block_on(drop_run_action(run, items, destination))
        };

        assert_eq!(
            run(DropRun::MoveWithinDrive, &items, &into_folder),
            DropAction::Move
        );
        assert_eq!(
            run(DropRun::MoveWithinDrive, &items, &into_share),
            DropAction::Copy
        );
        assert_eq!(
            run(DropRun::MoveWithinDrive, &items, &DropDestination::RecycleBin),
            DropAction::Copy
        );
        assert_eq!(
            run(DropRun::Run(DropAction::Link), &items, &into_folder),
            DropAction::Link
        );
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
