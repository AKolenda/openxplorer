// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging files and folders out of the folder views and the sidebar
//! (DND-001 to DND-008).
//!
//! Ports `makeFileDraggable`, `beginNativeFileDrag`, `markNativeFileDrag`
//! and `finishNativeFileDrag` of `v2.0.0:desktop/ui/app.js` and `NativeFileDrag`
//! of `v2.0.0:desktop/native_file_drag.py` on GTK's own drag source. Dragging a
//! selected item carries the whole selection; dragging another item
//! selects only it first; a sidebar row carries its folder. What the drag
//! offers is built in [`payload`]. While the drag lasts its items are
//! dimmed, and clicks on items are ignored from its start until shortly
//! after its end, so the gesture never opens or reselects an item. Screen
//! readers hear the drag start, with what it carries, and end (ACC-002).
//!
//! Safety rule "a drag out never deletes" (DND-008): a drag started
//! without a modifier offers Copy (and Ask, the drop menu) only, so no app
//! moves the items unasked; holding Shift or Ctrl+Shift as it starts adds
//! Move or Link, as those keys do in Windows Explorer. Copy is always
//! offered, so a receiver can always finish the drop as a copy. The window
//! never deletes what it offered, whatever the receiver answers, and
//! nothing is mounted or downloaded during the gesture. No drag starts
//! from blank space, with the middle button, while a file operation runs
//! or is being planned, or while a dialog or a menu is open.
//!
//! A drag with the secondary button is Windows Explorer's right-button
//! drag (DND-027): it offers Copy and Ask whatever keys are held, and its
//! drop in any window of this app asks with the right-drag menu. Pressing
//! the secondary button selects the item, and the context menu opens only
//! when it is released without dragging.

mod label;
mod payload;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::Entry;
use ox_core::network::local_path;

use crate::announcement::announce;
use crate::folder_view::item::FileItem;
use crate::icons::{Art, ArtImage};

use super::zip_copies::ZipDragContent;
use super::BrowserWindow;

pub(crate) use label::{show_drag_label, DragLabel, DragVerb};
#[cfg(test)]
pub(crate) use label::{shown_drag_glyph, shown_drag_label};
use payload::dragged_uris;
pub(crate) use payload::{is_draggable_location, DragPayload, DragRefusal, DraggedItems};

/// How long clicks on items are ignored after a drag starts
/// (`suppressClickUntil = now + 700` in app.js).
const CLICKS_PAUSE_AFTER_START: Duration = Duration::from_millis(700);

/// How long clicks on items are ignored after a drag ends (`+ 400`).
const CLICKS_PAUSE_AFTER_END: Duration = Duration::from_millis(400);

/// The edge of the icon that follows the pointer.
const DRAG_ICON_SIZE: i32 = 48;

/// Shown when some dragged items have no local path.
const REMOTE_ONLY_MESSAGE: &str = crate::i18n::message_id(
    "This network item needs an app that supports SMB addresses. For a \
                                   local-only editor, open it through an existing local mount.",
);

/// Said to screen readers when a drag ends, dropped or cancelled.
const DRAG_ENDED: &str = crate::i18n::message_id("Drag ended");

/// What screen readers hear as a drag of `uris` starts: the item's name,
/// or how many items move.
fn drag_announcement(uris: &[String]) -> String {
    match uris {
        [uri] => {
            let name = gio::File::for_uri(uri)
                .basename()
                .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
            ox_core::i18n::format_message("Dragging {name}", &[("name", &name)])
        }
        _ => ox_core::i18n::format_message(
            "Dragging {len} items",
            &[("len", &ox_core::format::group_digits(uris.len() as u64))],
        ),
    }
}

/// The drag this window started: what it offers and how its icon looks.
#[derive(Debug, Clone)]
pub(crate) struct OutgoingDrag {
    /// What the drag offers.
    payload: DragPayload,
    /// The art of the first item, for the drag icon.
    art: Art,
}

/// The mouse button a drag of files is made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragButton {
    /// The primary button: the drop does what the keys ask (DND-017).
    Primary,
    /// The secondary button: the drop asks with Windows Explorer's
    /// right-drag menu (DND-027).
    Secondary,
}

impl DragButton {
    /// The drag button GDK numbers `button`; `None` for the middle and
    /// other buttons, which drag nothing.
    pub(crate) fn of(button: u32) -> Option<Self> {
        match button {
            gdk::BUTTON_PRIMARY => Some(Self::Primary),
            gdk::BUTTON_SECONDARY => Some(Self::Secondary),
            _ => None,
        }
    }
}

thread_local! {
    /// The button of the drag being prepared, kept for its start: GTK no
    /// longer reports the pressed button once the drag begins.
    static PREPARED_BUTTON: std::cell::Cell<Option<DragButton>> = const { std::cell::Cell::new(None) };

    /// The drag of this process made with the secondary button, while it
    /// lasts. A drop in any window of the process reads it, so the drop
    /// asks with the menu wherever it lands.
    static SECONDARY_DRAG: std::cell::RefCell<glib::WeakRef<gdk::Drag>> =
        std::cell::RefCell::new(glib::WeakRef::new());
}

thread_local! {
    /// The items this process drags now, from any of its windows. They
    /// take no drop while they are dragged, as in Windows Explorer: a drop
    /// on one of them goes to the folder shown.
    static DRAGGED: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
}

/// True while the item at `uri` is being dragged by this process.
pub(crate) fn is_being_dragged(uri: &str) -> bool {
    DRAGGED.with(|dragged| dragged.borrow().contains(uri))
}

/// The items this process drags now, from any of its windows.
pub(crate) fn dragged_now() -> Vec<String> {
    DRAGGED.with(|dragged| dragged.borrow().iter().cloned().collect())
}

/// The button `drag`, a drag of this process, was made with.
pub(crate) fn drag_button(drag: &gdk::Drag) -> DragButton {
    let is_secondary = SECONDARY_DRAG.with(|secondary| secondary.borrow().upgrade().as_ref() == Some(drag));
    if is_secondary {
        DragButton::Secondary
    } else {
        DragButton::Primary
    }
}

/// The actions a file drag with `button` offers, from the modifiers held
/// as it starts (see the module's safety rule). Copy and Ask are always
/// offered, so a receiver can finish as a copy and the desktop's "ask"
/// modifier opens the drop menu. A right-button drag offers only those
/// two whatever the keys, as its drop asks.
fn offered_actions(button: DragButton, modifiers: gdk::ModifierType) -> gdk::DragAction {
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let chosen = match (button, shift, control) {
        (DragButton::Secondary, _, _) | (DragButton::Primary, false, _) => gdk::DragAction::empty(),
        (DragButton::Primary, true, true) => gdk::DragAction::LINK,
        (DragButton::Primary, true, false) => gdk::DragAction::MOVE,
    };
    gdk::DragAction::COPY | gdk::DragAction::ASK | chosen
}

/// The button `source`'s gesture is pressed with now, while its drag is
/// prepared, remembered for the drag's start; `None` for one that drags
/// nothing.
fn pressed_button(source: &gtk::DragSource) -> Option<DragButton> {
    let button = DragButton::of(source.current_button());
    PREPARED_BUTTON.with(|prepared| prepared.set(button));
    button
}

/// Marks `drag`, as it starts, as made with the secondary button when its
/// preparation said so (see [`PREPARED_BUTTON`]).
fn mark_drag_button(drag: &gdk::Drag) {
    let button = PREPARED_BUTTON.with(std::cell::Cell::take);
    let secondary = (button == Some(DragButton::Secondary)).then_some(drag);
    SECONDARY_DRAG.with(|marked| marked.borrow().set(secondary));
}

/// The actions a drag of `content` with `button` offers with `modifiers`
/// held as it starts: [`offered_actions`], except that items inside a ZIP
/// are only copied or moved. Their copies are removed after a day, so
/// nothing may link to them, and asking would offer links (ARC-026).
pub(super) fn drag_actions(
    content: &gdk::ContentProvider,
    button: DragButton,
    modifiers: gdk::ModifierType,
) -> gdk::DragAction {
    let offered = offered_actions(button, modifiers);
    if content.is::<ZipDragContent>() {
        offered & (gdk::DragAction::COPY | gdk::DragAction::MOVE)
    } else {
        offered
    }
}

/// The icon that follows the pointer: the first item's art, with the
/// number of items beside it when there are several, and under them the
/// label that says what a release would do ([`label`]).
fn drag_icon(art: Art, count: usize) -> gtk::Widget {
    let pictures = gtk::Box::builder().spacing(4).build();
    pictures.append(&ArtImage::new(art, DRAG_ICON_SIZE));
    if count > 1 {
        let badge = gtk::Label::builder()
            .label(ox_core::format::group_digits(count as u64))
            .valign(gtk::Align::Start)
            .css_classes(["drag-count"])
            .build();
        pictures.append(&badge);
    }
    let icon = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(["drag-icon"])
        .build();
    icon.append(&pictures);
    icon.append(&label::drag_label());
    icon.upcast()
}

/// A drag source for files, pressed with either button: the prepare
/// handlers refuse the others ([`pressed_button`]), as the middle button
/// opens items in a new tab.
fn file_drag_source() -> gtk::DragSource {
    let source = gtk::DragSource::new();
    source.set_button(0);
    source
}

/// Lets This PC's `tile` of the drive at `uri` be dragged, as Windows
/// Explorer lets a drive be dragged into a folder to make a shortcut to it
/// there ("Create link in Documents"). The tile finds its window when the
/// drag starts, as it is drawn before it is in one.
pub(super) fn attach_drive_tile_drag(tile: &gtk::FlowBoxChild, uri: &str) {
    let source = file_drag_source();
    let uri = uri.to_owned();
    let window_of = |source: &gtk::DragSource| {
        source
            .widget()
            .and_then(|widget| widget.root())
            .and_downcast::<BrowserWindow>()
    };
    source.connect_prepare(move |source, _, _| {
        let button = pressed_button(source)?;
        let content = window_of(source)?.drive_tile_drag_content(uri.clone())?;
        source.set_actions(offered_actions(button, source.current_event_state()));
        Some(content)
    });
    source.connect_drag_begin(move |source, drag| {
        mark_drag_button(drag);
        if let Some(window) = window_of(source) {
            window.begin_file_drag(drag);
        }
    });
    source.connect_drag_end(move |source, drag, _| {
        super::drag_surface::keep_finished_drag(drag);
        SECONDARY_DRAG.with(|secondary| secondary.borrow().set(None));
        label::forget_drag_label();
        if let Some(window) = window_of(source) {
            window.end_file_drag();
        }
    });
    tile.add_controller(source);
}

/// True when a menu or other popover inside `widget` is open.
pub(super) fn has_open_popover(widget: &gtk::Widget) -> bool {
    let mut child = widget.first_child();
    while let Some(current) = child {
        let is_open_popover = current.is::<gtk::Popover>() && current.is_mapped();
        if is_open_popover || (current.is_visible() && has_open_popover(&current)) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

impl BrowserWindow {
    /// Lets items of `view` be dragged out to other windows and apps.
    pub(super) fn attach_file_drag(&self, view: &gtk::Widget) {
        let source = file_drag_source();
        // Before the gestures of the rows inside the view, which could
        // take the press first.
        source.set_propagation_phase(gtk::PropagationPhase::Capture);
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            #[upgrade_or]
            None,
            move |source, x, y| {
                let button = pressed_button(source)?;
                // The empty part of a details row is no item to drag: a
                // primary press there draws a rubber band (SEL-012).
                if button == DragButton::Primary && window.is_blank_space(&view, x, y) {
                    return None;
                }
                let position = window.folder_pane().owners().position_at(&view, x, y)?;
                let content = window.drag_content_for(position)?;
                source.set_actions(drag_actions(&content, button, source.current_event_state()));
                Some(content)
            }
        ));
        self.follow_file_drag(&source);
        view.add_controller(source);
    }

    /// Lets the sidebar's folders be dragged out: to pin them, or to copy
    /// them into another folder or app.
    pub(super) fn attach_sidebar_file_drag(&self) {
        let source = file_drag_source();
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            None,
            move |source, _, y| {
                let button = pressed_button(source)?;
                let uri = window.sidebar().location_at(y)?;
                let content = window.sidebar_drag_content(uri)?;
                source.set_actions(offered_actions(button, source.current_event_state()));
                Some(content)
            }
        ));
        self.follow_file_drag(&source);
        self.sidebar().list().add_controller(source);
    }

    /// What dragging This PC's tile of the drive at `uri` offers: the
    /// drive, which a drop into a folder makes a shortcut to
    /// ([`super::drive_shortcuts`]); `None` while an operation runs.
    fn drive_tile_drag_content(&self, uri: String) -> Option<gdk::ContentProvider> {
        if !(is_draggable_location(&uri) && self.may_start_drag()) {
            return None;
        }
        self.offer_drag(
            DragPayload::new(vec![uri], local_path),
            Art::Glyph(crate::icons::Icon::HardDrive),
        )
    }

    /// Shows the feedback of `source`'s drags as they start and end.
    fn follow_file_drag(&self, source: &gtk::DragSource) {
        source.connect_drag_begin(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, drag| {
                mark_drag_button(drag);
                window.begin_file_drag(drag);
            }
        ));
        source.connect_drag_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, drag, _| {
                super::drag_surface::keep_finished_drag(drag);
                SECONDARY_DRAG.with(|secondary| secondary.borrow().set(None));
                label::forget_drag_label();
                window.end_file_drag();
            }
        ));
    }

    /// What dragging the item at `position` offers, selecting only it
    /// first when it is not selected; `None` while an operation runs or
    /// when the selection may not leave the app, which the toast explains.
    pub(super) fn drag_content_for(&self, position: u32) -> Option<gdk::ContentProvider> {
        if !self.may_start_drag() {
            return None;
        }
        let model = self.folder_pane().model();
        if !model.selection().is_selected(position) {
            model.select_only(position);
        }
        let items = model.selected_items();
        let entries: Vec<&Entry> = items.iter().map(FileItem::entry).collect();
        let art = items.first().map_or(Art::Folder, FileItem::art);
        let uris: Vec<String> = entries.iter().map(|entry| entry.uri.clone()).collect();
        // Inside a ZIP opened like a folder, the drop gets copies, made
        // when it asks for them (ARC-026).
        if let Some(inside) = super::zip_copies::zip_items(&uris) {
            let content = self.zip_drag_content(inside);
            // The drag's icon and feedback count the items; the drop reads
            // only the copies the content makes.
            let payload = DragPayload {
                uris,
                exported: Vec::new(),
                text: String::new(),
                remote_only: 0,
            };
            self.imp()
                .outgoing_drag
                .replace(Some(OutgoingDrag { payload, art }));
            return Some(content);
        }
        let prepared = dragged_uris(&entries).and_then(|uris| DragPayload::new(uris, local_path));
        self.offer_drag(prepared, art)
    }

    /// What dragging the sidebar's folder at `uri` offers; `None` for a
    /// page, an unmounted drive or while an operation runs.
    fn sidebar_drag_content(&self, uri: String) -> Option<gdk::ContentProvider> {
        let may_leave = is_draggable_location(&uri) && self.may_start_drag();
        if !may_leave {
            return None;
        }
        // A drive is dragged as a drive, as This PC's tiles drag it.
        let is_drive = !self.split_drives(std::slice::from_ref(&uri)).drives.is_empty();
        let art = if is_drive {
            Art::Glyph(crate::icons::Icon::HardDrive)
        } else {
            Art::Folder
        };
        self.offer_drag(DragPayload::new(vec![uri], local_path), art)
    }

    /// True while nothing holds drags back: no file operation runs or is
    /// planned, and no dialog, sign-in prompt or menu is open (DND-006).
    fn may_start_drag(&self) -> bool {
        self.imp().file_operations.borrow().is_idle()
            && self.dialog_layer().shown().is_none()
            && !self.has_open_dialog()
            && !has_open_popover(self.upcast_ref())
    }

    /// Remembers the drag `prepared` describes and returns its content, or
    /// shows why it cannot start.
    fn offer_drag(
        &self,
        prepared: Result<DragPayload, DragRefusal>,
        art: Art,
    ) -> Option<gdk::ContentProvider> {
        match prepared {
            Ok(payload) => {
                let content = payload.content();
                self.imp()
                    .outgoing_drag
                    .replace(Some(OutgoingDrag { payload, art }));
                Some(content)
            }
            Err(refusal) => {
                self.show_message(&refusal.to_string());
                None
            }
        }
    }

    /// The drag started: its icon follows the pointer, and the window
    /// shows the drag's feedback.
    fn begin_file_drag(&self, drag: &gdk::Drag) {
        self.cancel_slow_click_rename();
        let Some(outgoing) = self.show_file_drag_feedback() else {
            return;
        };
        let icon = drag_icon(outgoing.art, outgoing.payload.uris.len());
        gtk::DragIcon::for_drag(drag).set_child(Some(&icon));
    }

    /// Dims the dragged items, resets type-to-select, pauses clicks and
    /// warns about items without a local path (`markNativeFileDrag`); the
    /// drag, or `None` when none is prepared.
    pub(super) fn show_file_drag_feedback(&self) -> Option<OutgoingDrag> {
        let outgoing = self.imp().outgoing_drag.borrow().clone()?;
        self.reset_typeahead();
        self.pause_item_clicks(CLICKS_PAUSE_AFTER_START);
        let dragged: HashSet<String> = outgoing.payload.uris.iter().cloned().collect();
        DRAGGED.with(|now| now.replace(dragged.clone()));
        self.quiet_tooltips_for_drag(true);
        self.folder_pane().owners().show_dragged_items(dragged);
        if outgoing.payload.remote_only > 0 {
            self.show_message(ox_core::i18n::gettext_static(REMOTE_ONLY_MESSAGE));
        }
        let announcement = drag_announcement(&outgoing.payload.uris);
        announce(self, &announcement, gtk::AccessibleAnnouncementPriority::Medium);
        Some(outgoing)
    }

    /// The drag ended, dropped or cancelled: the items show plainly again
    /// and clicks resume shortly (`finishNativeFileDrag`). GTK tells a
    /// source to delete its data after a move; this window never does
    /// (DND-008), whatever the receiver answered.
    pub(super) fn end_file_drag(&self) {
        if self.imp().outgoing_drag.replace(None).is_some() {
            announce(
                self,
                ox_core::i18n::gettext_static(DRAG_ENDED),
                gtk::AccessibleAnnouncementPriority::Medium,
            );
        }
        DRAGGED.with(|now| now.borrow_mut().clear());
        self.quiet_tooltips_for_drag(false);
        for pane in self.folder_panes() {
            pane.owners().show_dragged_items(HashSet::new());
        }
        self.pause_item_clicks(CLICKS_PAUSE_AFTER_END);
    }

    /// Ignores clicks that open items for `pause`.
    fn pause_item_clicks(&self, pause: Duration) {
        self.imp().item_clicks_resume_at.set(Some(Instant::now() + pause));
    }

    /// True while clicks that open items are ignored around a drag.
    pub(super) fn are_item_clicks_paused(&self) -> bool {
        self.imp()
            .item_clicks_resume_at
            .get()
            .is_some_and(|resume_at| Instant::now() < resume_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locations::Page;
    use crate::test_support::harness::{Fixture, TestWindow};

    /// One set of modifiers and the actions a drag started with them
    /// offers.
    struct ModifierCase {
        held: gdk::ModifierType,
        offered: gdk::DragAction,
    }

    /// parity: DND-008, DND-017
    #[test]
    fn a_plain_drag_offers_copy_and_modifiers_as_it_starts_add_move_or_link() {
        let copy_or_ask = gdk::DragAction::COPY | gdk::DragAction::ASK;
        let cases = [
            ModifierCase {
                held: gdk::ModifierType::empty(),
                offered: copy_or_ask,
            },
            ModifierCase {
                held: gdk::ModifierType::CONTROL_MASK,
                offered: copy_or_ask,
            },
            ModifierCase {
                held: gdk::ModifierType::SHIFT_MASK,
                offered: copy_or_ask | gdk::DragAction::MOVE,
            },
            ModifierCase {
                held: gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK,
                offered: copy_or_ask | gdk::DragAction::LINK,
            },
        ];

        for case in cases {
            assert_eq!(
                offered_actions(DragButton::Primary, case.held),
                case.offered,
                "{:?}",
                case.held
            );
        }
    }

    /// A right-button drag offers Copy and Ask whatever keys are held, as
    /// its drop asks (DND-027); the middle button drags nothing.
    ///
    /// parity: DND-008, DND-027
    #[test]
    fn a_right_button_drag_offers_copy_and_ask_whatever_the_keys() {
        let copy_or_ask = gdk::DragAction::COPY | gdk::DragAction::ASK;
        for held in [
            gdk::ModifierType::empty(),
            gdk::ModifierType::SHIFT_MASK,
            gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK,
        ] {
            assert_eq!(
                offered_actions(DragButton::Secondary, held),
                copy_or_ask,
                "{held:?}"
            );
        }
        assert_eq!(DragButton::of(gdk::BUTTON_PRIMARY), Some(DragButton::Primary));
        assert_eq!(DragButton::of(gdk::BUTTON_SECONDARY), Some(DragButton::Secondary));
        assert_eq!(DragButton::of(gdk::BUTTON_MIDDLE), None);
    }

    /// The folder views and the sidebar start drags with either button.
    ///
    /// parity: DND-027
    #[gtk::test]
    fn items_and_sidebar_folders_can_be_dragged_with_either_button() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let buttons = |widget: &gtk::Widget| -> Vec<u32> {
            widget
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|controller| controller.downcast::<gtk::DragSource>().ok())
                .map(|source| source.button())
                .collect()
        };
        let view = test.window.folder_pane().input_widget();

        assert_eq!(
            buttons(view.upcast_ref()),
            [0],
            "any button, the middle one refused as it starts"
        );
        assert_eq!(buttons(test.window.sidebar().list().upcast_ref()), [0]);
    }

    /// parity: DND-001, DND-003, SIDE-016
    #[gtk::test]
    fn a_sidebar_folder_can_be_dragged_and_a_page_cannot() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let home_row = test.window.sidebar().middle_of("Home");
        let home = test
            .window
            .sidebar()
            .location_at(home_row)
            .expect("Home opens a folder");

        let content = test.window.sidebar_drag_content(home.clone());
        let page = test.window.sidebar_drag_content(Page::ThisPc.uri().to_owned());

        let content = content.expect("a sidebar folder can be dragged");
        let own = content
            .value(DraggedItems::static_type())
            .expect("the drag offers its own items to this process");
        assert_eq!(own.get::<DraggedItems>().expect("the dragged items").0, [home]);
        assert!(page.is_none(), "a page is not a folder to drag");
        let local_disk = test.window.sidebar_drag_content("file:///".to_owned());
        assert!(local_disk.is_some(), "a drive is dragged as its folder");
    }

    /// parity: DND-004
    #[gtk::test]
    fn dragging_an_unmounted_smb_item_says_which_apps_can_open_it() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let no_mount = |_: &str| None;
        let payload =
            DragPayload::new(vec!["smb://nas/share/plan.odt".to_owned()], no_mount).expect("one item fits");
        test.window.imp().outgoing_drag.replace(Some(OutgoingDrag {
            payload,
            art: Art::Folder,
        }));

        test.window.show_file_drag_feedback();
        let message = test.window.shown_message();
        test.window.end_file_drag();

        assert_eq!(message, REMOTE_ONLY_MESSAGE);
    }

    /// parity: ACC-002
    #[test]
    fn a_drag_is_announced_by_its_item_or_its_count() {
        let one = ["file:///home/demo/Report%202026.odt".to_owned()];
        let two = ["file:///home/demo/a".to_owned(), "file:///home/demo/b".to_owned()];
        assert_eq!(drag_announcement(&one), "Dragging Report 2026.odt");
        assert_eq!(drag_announcement(&two), "Dragging 2 items");
    }
}
