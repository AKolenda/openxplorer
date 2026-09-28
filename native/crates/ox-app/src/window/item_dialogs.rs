// SPDX-License-Identifier: AGPL-3.0-only
//! Properties, previous versions and the window's in-window dialogs.
//!
//! Ports the tab ownership of `propertiesDialog` in `desktop/ui/app.js`
//! (`attachTabDialog`, `suspendTabDialog`, `restoreTabDialog`,
//! `discardTabDialog`) and `showModal` for the window's other dialogs.
//! Browse and the snapshot banner are in [`super::snapshot_tabs`],
//! Restore a copy in [`super::version_restore`].
//!
//! PROP-008: a Properties dialog belongs to the tab that opened it.
//! Showing another tab withdraws it; showing its tab again brings it back
//! as it was; closing its tab discards it. Its tab's tooltip says
//! `· Properties open`. Other dialogs (Restore a copy, the archive
//! dialogs, result messages) belong to the window and stay in front
//! across tab switches.

use std::cell::{OnceCell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{same_location, ItemKind};
use ox_core::versions::SnapshotLocation;

use crate::dialog_layer::{quiet_text, DialogFrame, DialogLayer, DialogWidth};
use crate::folder_view::item::FileItem;
use crate::locations::Page;
use crate::properties::{
    PropertiesContext, PropertiesTab, PropertiesTarget, PropertiesView, RestoreRequest, SnapshotBanner,
    SnapshotTarget,
};

use super::actions::{plain_action, text_action};
use super::session::TabId;
use super::window_action::WindowAction;
use super::{BrowserWindow, ButtonStyle};

/// A Properties dialog and the tab it belongs to.
#[derive(Debug)]
struct TabProperties {
    tab: TabId,
    frame: DialogFrame,
    view: PropertiesView,
}

/// The window's dialogs and the tabs browsing snapshots.
#[derive(Debug, Default)]
pub(super) struct ItemDialogs {
    /// The layer over the window's content; set by `install_item_dialogs`.
    layer: OnceCell<DialogLayer>,
    /// The banner under the command bar; set by `install_item_dialogs`.
    banner: OnceCell<SnapshotBanner>,
    /// The Properties dialogs, one per tab at most.
    properties: RefCell<Vec<TabProperties>>,
    /// The tabs Browse opened, with the snapshot each shows
    /// (`snapshot_tabs`).
    pub(super) snapshot_tabs: RefCell<Vec<(TabId, SnapshotLocation)>>,
}

impl BrowserWindow {
    fn item_dialogs(&self) -> &ItemDialogs {
        &self.imp().item_dialogs
    }

    /// The layer the window's dialogs are shown on.
    pub(super) fn dialog_layer(&self) -> &DialogLayer {
        self.item_dialogs()
            .layer
            .get()
            .expect("BrowserWindow::new installs the dialog layer")
    }

    /// The banner under the command bar.
    pub(super) fn snapshot_banner(&self) -> &SnapshotBanner {
        self.item_dialogs()
            .banner
            .get()
            .expect("BrowserWindow::new installs the snapshot banner")
    }

    /// Puts the dialog layer over the content and the snapshot banner under
    /// the command bar, and adds the Properties actions.
    pub(super) fn install_item_dialogs(&self) {
        let banner = SnapshotBanner::default();
        if let Some(column) = self.command_bar().parent().and_downcast::<gtk::Box>() {
            column.insert_child_after(&banner, Some(self.command_bar()));
        }
        let dialogs = self.item_dialogs();
        dialogs.banner.set(banner).expect("installed once");
        let layer = DialogLayer::install_over(self);
        dialogs.layer.set(layer).expect("installed once");
        self.add_action_entries([
            plain_action(WindowAction::Properties, |window| {
                window.open_properties(PropertiesTab::General);
            }),
            plain_action(WindowAction::PreviousVersions, |window| {
                window.open_properties(PropertiesTab::PreviousVersions);
            }),
            text_action(WindowAction::PropertiesOf, |window, uri| {
                window.open_properties_of(uri, PropertiesTab::General);
            }),
            text_action(WindowAction::PreviousVersionsOf, |window, uri| {
                window.open_properties_of(uri, PropertiesTab::PreviousVersions);
            }),
            tuple_action(WindowAction::BrowseSnapshot, |window, target| {
                if let Some(target) = SnapshotTarget::from_variant(target) {
                    window.browse_snapshot(target);
                }
            }),
            tuple_action(WindowAction::RestoreVersion, |window, target| {
                if let Some(request) = RestoreRequest::from_variant(target) {
                    window.ask_restore_destination(&request);
                }
            }),
        ]);
    }

    /// Enables Properties and Previous versions for one selected item, or
    /// for a real folder with nothing selected.
    pub(super) fn update_properties_actions(&self) {
        let selected = self.folder_pane().model().summary().count;
        let on_page = self.current_uri().as_deref().and_then(Page::from_uri).is_some();
        let enabled = selected == 1 || (selected == 0 && !on_page);
        self.set_action_enabled(WindowAction::Properties, enabled);
        self.set_action_enabled(WindowAction::PreviousVersions, enabled);
    }

    /// What Properties describes: the first selected item, or the folder
    /// (`selected()[0] || menuEntry()`).
    fn properties_target(&self) -> Option<PropertiesTarget> {
        let first = self.folder_pane().model().first_selected();
        if let Some(item) = first.and_then(|position| self.folder_pane().model().item(position)) {
            return Some(self.target_of_item(&item));
        }
        let uri = self.current_uri().filter(|uri| Page::from_uri(uri).is_none())?;
        let title = self.imp().locations.borrow().title_for(&uri);
        Some(self.named_target(uri, title, ItemKind::Folder))
    }

    fn target_of_item(&self, item: &FileItem) -> PropertiesTarget {
        let entry = item.entry();
        let kind = if entry.is_dir {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        self.named_target(entry.navigation_uri().to_owned(), entry.name.clone(), kind)
    }

    /// A target titled with the standard folder's label when `uri` is one
    /// (`findKnownFolder`).
    fn named_target(&self, uri: String, name: String, kind: ItemKind) -> PropertiesTarget {
        let known = self
            .context()
            .known_folders()
            .into_iter()
            .find(|place| place.known_folder.is_some() && same_location(&place.uri, &uri));
        let known_folder = known.as_ref().and_then(|place| place.known_folder);
        let title = known.map_or(name, |place| place.label);
        PropertiesTarget {
            uri,
            title,
            kind,
            known_folder,
        }
    }

    /// Opens Properties of the selection or the folder on `tab`.
    pub(super) fn open_properties(&self, tab: PropertiesTab) {
        if let Some(target) = self.properties_target() {
            self.show_properties(target, tab);
        }
    }

    /// Opens Properties of the location `uri` on `tab`; the location need
    /// not be listed: it is queried first, off the main thread, to learn
    /// whether it is a folder.
    fn open_properties_of(&self, uri: &str, tab: PropertiesTab) {
        let file = gio::File::for_uri(uri);
        let uri = uri.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let queried = file
                    .query_info_future(
                        "standard::type,standard::display-name",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                    )
                    .await;
                let (name, kind) = match queried {
                    Ok(info) => (info.display_name().to_string(), item_kind(&info)),
                    // The General tab says why the item cannot be read.
                    Err(_) => (window.imp().locations.borrow().base_name(&uri), ItemKind::File),
                };
                let target = window.named_target(uri, name, kind);
                window.show_properties(target, tab);
            }
        ));
    }

    /// Shows Properties of `target` on `tab`, owned by the active tab; a
    /// dialog the tab had is replaced.
    fn show_properties(&self, target: PropertiesTarget, tab: PropertiesTab) {
        let Some(owner) = self.imp().session.borrow().active_id() else {
            return;
        };
        self.discard_dialog_of_tab(owner);
        let context = PropertiesContext {
            versions: self.context().previous_versions().clone(),
            locations: self.imp().locations.borrow().clone(),
            folder_size: self.measured_folder_size(&target.uri),
        };
        let title = target.dialog_title();
        let view = PropertiesView::new(target, context, tab);
        let frame = DialogFrame::new(&title, view.dialog_width());
        frame.add_css_class("properties-dialog");
        frame.body().append(&view);
        frame.add_closing_button("Close", ButtonStyle::Accent, || {});
        frame.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |frame| window.properties_closed(frame)
        ));
        let entry = TabProperties {
            tab: owner,
            frame: frame.clone(),
            view,
        };
        self.item_dialogs().properties.borrow_mut().push(entry);
        self.dialog_layer().present(&frame);
        self.render_tabs();
    }

    /// Forgets a Properties dialog that was closed, stops its work and
    /// gives keyboard focus back to the folder.
    fn properties_closed(&self, frame: &DialogFrame) {
        let closed = {
            let mut properties = self.item_dialogs().properties.borrow_mut();
            let position = properties.iter().position(|entry| &entry.frame == frame);
            position.map(|position| properties.remove(position))
        };
        if let Some(closed) = closed {
            closed.view.cancel_work();
        }
        // A lookup may have found snapshot collections, which are
        // read-only from now on and mark the tabs inside them.
        self.refresh_snapshot_roots();
        self.render_tabs();
        self.folder_pane().focus_view();
    }

    /// Shows a dialog that belongs to the window rather than to a tab.
    pub(super) fn present_window_dialog(&self, frame: &DialogFrame) {
        self.withdraw_tab_dialog();
        frame.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.dialog_closed()
        ));
        self.dialog_layer().present(frame);
    }

    /// After a window dialog closed: the active tab's Properties come
    /// back, else the folder takes focus again.
    fn dialog_closed(&self) {
        let active = self.imp().session.borrow().active_id();
        if let Some(active) = active {
            self.show_dialog_of_tab(active);
        }
        if self.dialog_layer().shown().is_none() {
            self.folder_pane().focus_view();
        }
    }

    /// Withdraws the tab dialog shown, keeping it for its tab.
    fn withdraw_tab_dialog(&self) {
        let layer = self.dialog_layer();
        let Some(shown) = layer.shown() else {
            return;
        };
        let is_tab_dialog = self
            .item_dialogs()
            .properties
            .borrow()
            .iter()
            .any(|entry| entry.frame == shown);
        if is_tab_dialog {
            layer.withdraw();
        }
    }

    /// Shows tab `id`'s Properties when it comes to the front, and
    /// withdraws another tab's (`restoreTabDialog`, `suspendTabDialog`).
    /// A window dialog stays in front.
    pub(super) fn show_dialog_of_tab(&self, id: TabId) {
        let layer = self.dialog_layer();
        let tab_frame = self
            .item_dialogs()
            .properties
            .borrow()
            .iter()
            .find(|entry| entry.tab == id)
            .map(|entry| entry.frame.clone());
        let shown = layer.shown();
        let is_window_dialog = shown.as_ref().is_some_and(|frame| {
            !self
                .item_dialogs()
                .properties
                .borrow()
                .iter()
                .any(|entry| &entry.frame == frame)
        });
        if is_window_dialog || shown == tab_frame {
            return;
        }
        layer.withdraw();
        if let Some(frame) = tab_frame {
            layer.present(&frame);
        }
    }

    /// Discards tab `id`'s Properties when the tab closes
    /// (`discardTabDialog`), and forgets which snapshot it browsed.
    pub(super) fn discard_dialog_of_tab(&self, id: TabId) {
        let frame = self
            .item_dialogs()
            .properties
            .borrow()
            .iter()
            .find(|entry| entry.tab == id)
            .map(|entry| entry.frame.clone());
        if let Some(frame) = frame {
            frame.close();
        }
        self.item_dialogs()
            .snapshot_tabs
            .borrow_mut()
            .retain(|(tab, _)| *tab != id);
    }

    /// True when tab `id` has a Properties dialog, open or withdrawn.
    pub(super) fn has_properties(&self, id: TabId) -> bool {
        let properties = self.item_dialogs().properties.borrow();
        properties.iter().any(|entry| entry.tab == id)
    }

    /// Every open Properties view, for updates such as a measured size.
    pub(super) fn properties_views(&self) -> Vec<PropertiesView> {
        let properties = self.item_dialogs().properties.borrow();
        properties.iter().map(|entry| entry.view.clone()).collect()
    }

    /// A dialog titled `title` showing `text`, with OK (`showMessage`).
    pub(super) fn show_result_dialog(&self, title: &str, text: &str) {
        let frame = DialogFrame::new(title, DialogWidth::Standard);
        frame.body().append(&quiet_text(text));
        frame.add_closing_button("OK", ButtonStyle::Accent, || {});
        self.present_window_dialog(&frame);
    }

    /// Lists again every tab showing `folder`, keeping their selection.
    pub(super) fn reload_tabs_showing(&self, folder: &str) {
        let tabs: Vec<TabId> = {
            let session = self.imp().session.borrow();
            let showing = session
                .tabs()
                .iter()
                .filter(|tab| same_location(tab.uri(), folder));
            showing.map(|tab| tab.id).collect()
        };
        for id in tabs {
            self.load_tab(id, super::loading::LoadMode::Reload);
        }
    }
}

/// A folder or a file, as `info` says without following a link.
fn item_kind(info: &gio::FileInfo) -> ItemKind {
    if info.file_type() == gio::FileType::Directory {
        ItemKind::Folder
    } else {
        ItemKind::File
    }
}

/// An action whose target is a `(sss)` tuple.
fn tuple_action(
    window_action: WindowAction,
    run: impl Fn(&BrowserWindow, &glib::Variant) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
        .parameter_type(Some(glib::VariantTy::new("(sss)").expect("a valid variant type")))
        .activate(move |window: &BrowserWindow, _, target| {
            if let Some(target) = target {
                run(window, target);
            }
        })
        .build()
}
