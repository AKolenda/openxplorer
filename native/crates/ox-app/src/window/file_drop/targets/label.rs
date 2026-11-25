// SPDX-License-Identifier: AGPL-3.0-only
//! What the label on the drag icon says over each drop target (DND-028):
//! "→ Move to Documents", "+ Copy to Documents" or "Create link in
//! Documents", as Windows 11 File Explorer's drag says, for a drag of this
//! app's own items made with the primary button, over a folder of the
//! folder view, the folder shown, a folder or drive of the navigation pane,
//! a crumb or a tab.
//!
//! The label says what a release there would do, by the same rules as the
//! drop ([`DropAction::for_button_drop`] and [`drop_run_action`]), and it
//! follows the keys held as they change. A plain drag moves within the
//! items' drive and copies to another; which one is read on a thread of its
//! own, as for the drop, once per folder hovered, and the label shows once
//! it is known, normally within a frame. Where a release would do nothing
//! (moved back into their own folder) or nothing takes the drop, there is
//! no label, as in Windows. A right-button drag shows none: Windows shows
//! its menu on release instead (DND-027). Drags from other apps show none
//! either, as their icon is drawn by the other app. "Open with" over a
//! program and "Pin to Quick access" stay the folder pane's notes.
//!
//! What the label needs from the dragged items, which may be 100,000, is
//! read once per drag ([`DraggedFacts`]), never at each motion.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{parent_location, same_location, TRASH_URI};
use ox_core::ops::is_recycle_bin_item;

use super::spot::DropSpot;
use crate::window::file_drag::{
    drag_generation, dragged_now, show_drag_label, DragButton, DragLabel, DragVerb,
};
use crate::window::file_drop::action::{drop_run_action, DragOrigin, OfferedDrop};
use crate::window::file_drop::{DropAction, DropDestination, DropRun};
use crate::window::sidebar::SidebarDropSpot;
use crate::window::zip_copies::is_zip_copy;
use crate::window::BrowserWindow;

/// Where a drag of this app's items hovers, for its label.
#[derive(Debug, Clone)]
struct LabelHover {
    /// The drop the hover belongs to.
    drop: glib::WeakRef<glib::Object>,
    /// Where a release would send the items.
    destination: DropDestination,
    /// What a release would run, from the keys held now.
    run: DropRun,
    /// The name the label gives that place.
    place: String,
}

/// What a plain drop of a drag's items into one folder does: a move within
/// their drive, else a copy; `None` while that is being read.
#[derive(Debug)]
struct DriveAnswer {
    /// The drop the answer belongs to.
    drop: glib::WeakRef<glib::Object>,
    /// The folder.
    folder: String,
    /// What a plain drop there does.
    action: Option<DropAction>,
}

/// The labels of the drag over the window: where it hovers, and the last
/// drive read for it.
#[derive(Debug, Default)]
pub(crate) struct DropLabels {
    /// Where the drag hovers now, while it is over a place that takes it.
    hover: Option<LabelHover>,
    /// What a plain drop into the last folder hovered does.
    drive: Option<DriveAnswer>,
}

/// What the label reads from the items of a drag, once: they stay the
/// same while it lasts, and scanning 100,000 of them at every motion would
/// hold up the window.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct DraggedFacts {
    /// Whether any item is in the Recycle Bin.
    from_recycle_bin: bool,
    /// Whether any item is a copy taken out of a ZIP.
    has_zip_copy: bool,
    /// The folder every item is directly in, when there are items and
    /// they all are in one.
    shared_folder: Option<String>,
}

impl DraggedFacts {
    /// The facts of `dragged`.
    pub(super) fn of(dragged: &[String]) -> Self {
        let mut parents = dragged.iter().map(|uri| parent_location(uri));
        let first = parents.next().flatten();
        let shared_folder = first
            .filter(|first| parents.all(|parent| parent.is_some_and(|parent| same_location(&parent, first))));
        Self {
            from_recycle_bin: dragged.iter().any(|uri| is_recycle_bin_item(uri)),
            has_zip_copy: dragged.iter().any(|uri| is_zip_copy(uri)),
            shared_folder,
        }
    }

    /// Whether every item is directly in `folder`.
    fn all_in(&self, folder: &str) -> bool {
        self.shared_folder
            .as_deref()
            .is_some_and(|shared| same_location(shared, folder))
    }
}

thread_local! {
    /// The facts of the items dragged now, with the drag generation they
    /// were read at.
    static FACTS: RefCell<Option<(u64, Rc<DraggedFacts>)>> = const { RefCell::new(None) };
}

/// The facts of the items this process drags now, read at the first
/// motion of a drag and kept until its items change.
pub(super) fn dragged_facts() -> Rc<DraggedFacts> {
    let generation = drag_generation();
    FACTS.with(|facts| {
        let mut facts = facts.borrow_mut();
        if let Some((read_at, known)) = facts.as_ref() {
            if *read_at == generation {
                return Rc::clone(known);
            }
        }
        let read = Rc::new(DraggedFacts::of(&dragged_now()));
        *facts = Some((generation, Rc::clone(&read)));
        read
    })
}

/// True when `weak` is `identity`.
fn is_drop(weak: &glib::WeakRef<glib::Object>, identity: &glib::Object) -> bool {
    weak.upgrade().as_ref() == Some(identity)
}

/// What a release that runs `action` with `dragged` onto `destination`
/// does, as the label says it; `None` where it does nothing the label
/// names: a move back into the items' own folder, a link to copies taken
/// out of a ZIP (refused), a drop that asks, or a place that is no folder.
/// Recycle Bin items are moved out whatever the keys (OPS-046), and items
/// dropped on the Recycle Bin are moved there.
pub(super) fn label_verb(
    action: DropAction,
    destination: &DropDestination,
    dragged: &DraggedFacts,
) -> Option<DragVerb> {
    if matches!(action, DropAction::Ask(_)) {
        return None;
    }
    let from_recycle_bin = dragged.from_recycle_bin;
    match destination {
        DropDestination::RecycleBin => (!from_recycle_bin).then_some(DragVerb::Move),
        DropDestination::Folder(_) | DropDestination::Volume(_) if from_recycle_bin => Some(DragVerb::Move),
        DropDestination::Folder(folder) => {
            let in_own_folder = dragged.all_in(folder);
            match action {
                DropAction::Move if in_own_folder => None,
                DropAction::Link if dragged.has_zip_copy => None,
                other => verb_of(other),
            }
        }
        DropDestination::Volume(_) => verb_of(action),
        DropDestination::QuickAccess { .. } | DropDestination::Program(_) | DropDestination::NewTabs => None,
    }
}

/// The label's verb for `action`.
fn verb_of(action: DropAction) -> Option<DragVerb> {
    match action {
        DropAction::Move => Some(DragVerb::Move),
        DropAction::Copy => Some(DragVerb::Copy),
        DropAction::Link => Some(DragVerb::Link),
        DropAction::Ask(_) => None,
    }
}

impl BrowserWindow {
    /// Says on the drag icon what releasing `drop` at `spot` would do, as
    /// `run` decides; hides the label where nothing takes the drop.
    pub(super) fn show_drop_label(
        &self,
        drop: &impl OfferedDrop,
        spot: Option<&DropSpot>,
        run: Option<DropRun>,
    ) {
        let labelled = drop.origin() == DragOrigin::ThisApp && drop.button() == DragButton::Primary;
        let hover = spot.zip(run).filter(|_| labelled).and_then(|(spot, run)| {
            Some(LabelHover {
                drop: drop.identity().downgrade(),
                place: self.drop_place_name(spot)?,
                destination: spot.destination(),
                run,
            })
        });
        let label = hover.as_ref().and_then(|hover| self.hover_label(hover));
        self.imp().drop_labels.borrow_mut().hover = hover;
        show_drag_label(label.as_ref());
    }

    /// Hides the drag's label: it left the place it hovered.
    pub(super) fn hide_drop_label(&self) {
        self.imp().drop_labels.borrow_mut().hover = None;
        show_drag_label(None);
    }

    /// The name the label gives the place a drop at `spot` goes into: the
    /// navigation pane's row as it is labelled, else the folder's title.
    fn drop_place_name(&self, spot: &DropSpot) -> Option<String> {
        if let DropSpot::Sidebar(spot) = spot {
            return match spot {
                SidebarDropSpot::Folder { index, .. } | SidebarDropSpot::Volume { index, .. } => {
                    self.sidebar().row_label(*index)
                }
                SidebarDropSpot::Pin { .. } => None,
            };
        }
        let locations = self.imp().locations.borrow();
        match spot.destination() {
            DropDestination::Folder(folder) => Some(locations.title_for(&folder)),
            DropDestination::RecycleBin => Some(locations.title_for(TRASH_URI)),
            _ => None,
        }
    }

    /// What the label says for `hover`; `None` while a plain drop's drive
    /// is being read, or where the label names nothing.
    fn hover_label(&self, hover: &LabelHover) -> Option<DragLabel> {
        let dragged = dragged_facts();
        let action = match (&hover.run, &hover.destination) {
            (DropRun::Run(action), _) => *action,
            (DropRun::MoveWithinDrive, DropDestination::Folder(folder)) => {
                // Recycle Bin items always move out, so need no drive. Items
                // back in their own folder do: on a share a plain drop
                // copies them there.
                if dragged.from_recycle_bin {
                    DropAction::Move
                } else {
                    let identity = hover.drop.upgrade()?;
                    self.plain_drop_action(&identity, folder)?
                }
            }
            (DropRun::MoveWithinDrive, _) => DropAction::Copy,
        };
        let verb = label_verb(action, &hover.destination, &dragged)?;
        Some(DragLabel {
            verb,
            place: hover.place.clone(),
        })
    }

    /// What a plain drop of the items of the drag `identity` into `folder`
    /// does; `None` until that is read, off the GTK thread as for the drop
    /// itself, once per folder. Once read, the label of a drag still
    /// hovering over that folder shows it.
    fn plain_drop_action(&self, identity: &glib::Object, folder: &str) -> Option<DropAction> {
        let mut labels = self.imp().drop_labels.borrow_mut();
        let known = labels
            .drive
            .as_ref()
            .filter(|answer| answer.folder == folder && is_drop(&answer.drop, identity));
        if let Some(answer) = known {
            return answer.action;
        }
        labels.drive = Some(DriveAnswer {
            drop: identity.downgrade(),
            folder: folder.to_owned(),
            action: None,
        });
        drop(labels);
        let dragged = dragged_now();
        let destination = DropDestination::Folder(folder.to_owned());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            identity,
            async move {
                let action = drop_run_action(DropRun::MoveWithinDrive, &dragged, &destination).await;
                window.take_drive_answer(&identity, &destination, action);
            }
        ));
        None
    }

    /// Keeps what a plain drop of the drag `identity` onto `destination`
    /// does, and shows it while the drag still hovers there.
    fn take_drive_answer(&self, identity: &glib::Object, destination: &DropDestination, action: DropAction) {
        let hover = {
            let mut labels = self.imp().drop_labels.borrow_mut();
            let DropDestination::Folder(folder) = destination else {
                return;
            };
            let Some(answer) = labels
                .drive
                .as_mut()
                .filter(|answer| &answer.folder == folder && is_drop(&answer.drop, identity))
            else {
                return;
            };
            answer.action = Some(action);
            labels.hover.clone().filter(|hover| {
                is_drop(&hover.drop, identity)
                    && &hover.destination == destination
                    && hover.run == DropRun::MoveWithinDrive
            })
        };
        if let Some(hover) = hover {
            show_drag_label(self.hover_label(&hover).as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder the tests drop into, and items of another folder.
    const FOLDER: &str = "file:///home/demo/Documents";
    const ITEMS: [&str; 2] = ["file:///home/demo/a.txt", "file:///home/demo/b.txt"];

    fn items() -> DraggedFacts {
        DraggedFacts::of(&ITEMS.iter().map(|uri| (*uri).to_owned()).collect::<Vec<_>>())
    }

    /// The dragged items are read once per drag: every motion after the
    /// first gets the same facts, and a new drag reads its own.
    ///
    /// parity: DND-028
    #[test]
    fn the_dragged_items_are_read_once_per_drag() {
        crate::window::file_drag::set_dragged_for_tests(&ITEMS);
        let first = dragged_facts();
        assert!(
            Rc::ptr_eq(&first, &dragged_facts()),
            "the next motion reads nothing again"
        );
        assert!(first.all_in("file:///home/demo"));

        crate::window::file_drag::set_dragged_for_tests(&["file:///home/demo/Documents/c.txt"]);
        let next = dragged_facts();
        assert!(!Rc::ptr_eq(&first, &next));
        assert!(next.all_in(FOLDER));
        crate::window::file_drag::set_dragged_for_tests(&[]);
        assert_eq!(*dragged_facts(), DraggedFacts::default());
    }

    /// The facts of a drag of 100,000 items say whether they all are in
    /// one folder, as checking each item at each motion did.
    ///
    /// parity: DND-028
    #[test]
    fn the_facts_of_a_huge_drag_tell_its_own_folder() {
        let many: Vec<String> = (0..100_000).map(|n| format!("{FOLDER}/file%20{n}.txt")).collect();
        let facts = DraggedFacts::of(&many);
        assert!(facts.all_in(FOLDER));
        assert!(
            facts.all_in(&format!("{FOLDER}/")),
            "the same folder written with a slash"
        );
        assert!(!facts.all_in("file:///home/demo"));
        assert!(!facts.from_recycle_bin && !facts.has_zip_copy);
        let mut mixed = many;
        mixed.push("file:///home/demo/elsewhere.txt".to_owned());
        assert!(!DraggedFacts::of(&mixed).all_in(FOLDER));
        assert!(!DraggedFacts::of(&[]).all_in(FOLDER), "no items are in no folder");
    }

    /// The label names what the drop runs, and nothing where a release
    /// does nothing or asks.
    ///
    /// parity: DND-028
    #[test]
    fn the_label_names_what_the_drop_runs_and_nothing_where_it_does_nothing() {
        let folder = DropDestination::Folder(FOLDER.to_owned());
        let own_folder = DropDestination::Folder("file:///home/demo".to_owned());
        let asks = DropAction::Ask(DragOrigin::ThisApp);

        assert_eq!(
            label_verb(DropAction::Move, &folder, &items()),
            Some(DragVerb::Move)
        );
        assert_eq!(
            label_verb(DropAction::Copy, &folder, &items()),
            Some(DragVerb::Copy)
        );
        assert_eq!(
            label_verb(DropAction::Link, &folder, &items()),
            Some(DragVerb::Link)
        );
        assert_eq!(
            label_verb(asks, &folder, &items()),
            None,
            "a drop that asks says nothing"
        );
        assert_eq!(
            label_verb(DropAction::Move, &own_folder, &items()),
            None,
            "a move back into their own folder does nothing"
        );
        assert_eq!(
            label_verb(DropAction::Copy, &own_folder, &items()),
            Some(DragVerb::Copy),
            "Ctrl in their own folder makes copies"
        );
        assert_eq!(
            label_verb(DropAction::Copy, &DropDestination::RecycleBin, &items()),
            Some(DragVerb::Move),
            "the Recycle Bin always moves"
        );
        assert_eq!(
            label_verb(
                DropAction::Copy,
                &DropDestination::Volume("drive".to_owned()),
                &items()
            ),
            Some(DragVerb::Copy)
        );
        assert_eq!(
            label_verb(DropAction::Copy, &DropDestination::NewTabs, &items()),
            None
        );
        assert_eq!(
            label_verb(
                DropAction::Copy,
                &DropDestination::QuickAccess { before: None },
                &items()
            ),
            None,
            "pinning keeps its own note"
        );
    }
}
