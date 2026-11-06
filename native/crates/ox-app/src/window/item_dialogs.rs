// SPDX-License-Identifier: AGPL-3.0-only
//! Properties, previous versions and the window's in-window dialogs.
//!
//! Ports `propertiesDialog` in `v2.0.0:desktop/ui/app.js` and `showModal`
//! for the window's other dialogs. Browse and the snapshot banner are in
//! [`super::snapshot_tabs`], Restore a copy in [`super::version_restore`].
//!
//! PROP-008, a deliberate change from the Python app: Properties are
//! top-level windows of their own, as in Windows 11, not dialogs of the
//! tab that opened them. Several can be open at once, none blocks the
//! browser window, and asking again for an item whose Properties are open
//! brings that window to the front. They close with the browser window
//! that opened them, which they run actions through. Other dialogs
//! (Restore a copy, the archive dialogs, result messages) are shown over
//! the browser window's content.

use std::cell::{OnceCell, RefCell};

use crate::properties::PropertiesWindow;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{same_location, ItemKind};
use ox_core::settings::{template_for, FolderTemplate, SettingsError};
use ox_core::versions::SnapshotLocation;

use crate::dialog::{quiet_text, DialogFrame, DialogLayer, DialogWidth};
use crate::folder_view::item::FileItem;
use crate::locations::Page;
use crate::properties::{
    open_item_properties, open_selection_properties, PropertiesContext, PropertiesTab, PropertiesTarget,
    RestoreRequest, SnapshotBanner, SnapshotTarget,
};

use super::actions::{plain_action, text_action};
use super::session::TabId;
use super::window_action::WindowAction;
use super::{BrowserWindow, ButtonStyle};

/// The window's dialogs and the tabs browsing snapshots.
#[derive(Debug, Default)]
pub(super) struct ItemDialogs {
    /// The layer over the window's content; set by `install_item_dialogs`.
    layer: OnceCell<DialogLayer>,
    /// The banner under the command bar; set by `install_item_dialogs`.
    banner: OnceCell<SnapshotBanner>,
    /// The Properties windows this window opened and are still open.
    properties: RefCell<Vec<PropertiesWindow>>,
    /// A caller's activation token for the next Properties window
    /// (`ShowItemProperties`, INT-023).
    startup_id: RefCell<Option<String>>,
    /// The tokens Properties windows took, for tests.
    #[cfg(test)]
    used_startup_ids: RefCell<Vec<String>>,
    /// The tabs Browse opened, with the snapshot each shows
    /// (`snapshot_tabs`).
    pub(super) snapshot_tabs: RefCell<Vec<(TabId, SnapshotLocation)>>,
}

impl BrowserWindow {
    fn item_dialogs(&self) -> &ItemDialogs {
        &self.imp().item_dialogs
    }

    /// The layer the window's dialogs are shown on.
    pub(crate) fn dialog_layer(&self) -> &DialogLayer {
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
                // Alt+Enter is an application accelerator, which reaches
                // the window from a dialog in front too; only the items
                // behind it have Properties.
                if !window.dialog_in_front() {
                    window.open_properties(PropertiesTab::General);
                }
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

    /// Enables Properties for a selection (PROP-002) or a real folder
    /// with nothing selected, and Previous versions for one item.
    pub(super) fn update_properties_actions(&self) {
        let selected = self.folder_pane().model().summary().count;
        let on_page = self.current_uri().as_deref().and_then(Page::from_uri).is_some();
        // Items inside a ZIP opened like a folder have no file of their
        // own to describe or to find versions of (ARC-026).
        let in_zip = self
            .current_uri()
            .is_some_and(|uri| ox_core::location::is_archive_location(&uri));
        let one_item = !in_zip && (selected == 1 || (selected == 0 && !on_page));
        self.set_action_enabled(WindowAction::Properties, one_item || (!in_zip && selected > 1));
        self.set_action_enabled(WindowAction::PreviousVersions, one_item);
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

    /// Whether `uri` is a standard folder (Documents, Downloads and the
    /// others), which keeps its name.
    pub(crate) fn is_standard_folder(&self, uri: &str) -> bool {
        self.context()
            .known_folders()
            .into_iter()
            .any(|place| place.known_folder.is_some() && same_location(&place.uri, uri))
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
        let selected = self.folder_pane().model().selected_items();
        if selected.len() > 1 && tab != PropertiesTab::PreviousVersions {
            let entries = selected.iter().map(|item| item.entry().clone()).collect();
            self.show_selection_properties(entries, tab);
            return;
        }
        if let Some(target) = self.properties_target() {
            self.show_properties(target, tab);
        }
    }

    /// Opens Properties of the location `uri` on `tab`; the location need
    /// not be listed: it is queried first, off the main thread, to learn
    /// whether it is a folder.
    pub(super) fn open_properties_of(&self, uri: &str, tab: PropertiesTab) {
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
                    // The root is Local Disk, as the sidebar names it.
                    Ok(info) if info.display_name() == "/" => {
                        (window.imp().locations.borrow().base_name(&uri), item_kind(&info))
                    }
                    Ok(info) => (info.display_name().to_string(), item_kind(&info)),
                    // The General tab says why the item cannot be read.
                    Err(_) => (window.imp().locations.borrow().base_name(&uri), ItemKind::File),
                };
                let target = window.named_target(uri, name, kind);
                window.show_properties(target, tab);
            }
        ));
    }

    /// What a Properties window needs from this window.
    fn properties_context(&self) -> PropertiesContext {
        PropertiesContext {
            versions: self.context().previous_versions().clone(),
            locations: self.imp().locations.borrow().clone(),
            usershares: crate::properties::system_usershares(self.context().desktop_integration().sandbox()),
            relocation: self.context().folder_relocation(),
            brave: self.context().desktop_integration().brave(),
        }
    }

    /// Shows Properties of `target` on `tab`, once its share is mounted
    /// (NET-004).
    fn show_properties(&self, target: PropertiesTarget, tab: PropertiesTab) {
        let uri = target.uri.clone();
        self.after_mounting(&uri, move |window| window.show_mounted_properties(target, tab));
    }

    /// Opens the Properties window of `target` on `tab`, or brings the
    /// one open for it to the front on that tab, as Windows does.
    fn show_mounted_properties(&self, target: PropertiesTarget, tab: PropertiesTab) {
        let open = self
            .properties_windows()
            .into_iter()
            .find(|window| window.widget_name() == target.uri.as_str());
        if let Some(window) = open {
            window.select_tab(tab.page_name());
            self.hand_over_startup_id(&window);
            window.present();
            return;
        }
        let window = open_item_properties(self, target, self.properties_context(), tab);
        self.present_properties(&window);
    }

    /// Opens Properties of several selected items (PROP-002).
    fn show_selection_properties(&self, entries: Vec<ox_core::entry::Entry>, tab: PropertiesTab) {
        let window = open_selection_properties(self, entries, &self.properties_context(), tab);
        self.present_properties(&window);
    }

    /// Shows the Properties window `window` and keeps it until it closes.
    fn present_properties(&self, window: &PropertiesWindow) {
        self.keep_properties(window);
        self.hand_over_startup_id(window);
        window.present();
    }

    /// Gives `window` the caller's activation token kept for it, if any,
    /// so the desktop lets it come to the front.
    fn hand_over_startup_id(&self, window: &PropertiesWindow) {
        if let Some(startup_id) = self.item_dialogs().startup_id.take() {
            window.set_startup_id(&startup_id);
            #[cfg(test)]
            self.item_dialogs().used_startup_ids.borrow_mut().push(startup_id);
        }
    }

    /// Keeps a caller's activation token for the next Properties window.
    pub(crate) fn hand_startup_id_to_properties(&self, startup_id: &str) {
        self.item_dialogs()
            .startup_id
            .replace(Some(startup_id.to_owned()));
    }

    /// The activation tokens Properties windows took, for tests.
    #[cfg(test)]
    pub(crate) fn properties_startup_ids(&self) -> Vec<String> {
        self.item_dialogs().used_startup_ids.borrow().clone()
    }

    /// Keeps `window` among this window's Properties until it closes.
    fn keep_properties(&self, window: &PropertiesWindow) {
        window.connect_closed_once(glib::clone!(
            #[weak(rename_to = browser)]
            self,
            #[weak]
            window,
            move || browser.properties_closed(&window)
        ));
        self.item_dialogs().properties.borrow_mut().push(window.clone());
    }

    /// Takes over the Properties windows `previous` opened, as its last tab
    /// moves here and it closes: they stay open, with the changes they
    /// hold, and act through this window from now on.
    pub(super) fn adopt_properties_windows(&self, previous: &BrowserWindow) {
        let windows = previous.item_dialogs().properties.take();
        for window in windows {
            window.set_opener(self);
            self.keep_properties(&window);
        }
    }

    /// Forgets a Properties window that closed.
    fn properties_closed(&self, window: &PropertiesWindow) {
        self.item_dialogs()
            .properties
            .borrow_mut()
            .retain(|open| open != window);
        // A lookup may have found snapshot collections, which are
        // read-only from now on and mark the tabs inside them.
        self.refresh_snapshot_roots();
        self.render_tabs();
    }

    /// The Properties windows this window opened that are open.
    pub(crate) fn properties_windows(&self) -> Vec<PropertiesWindow> {
        self.item_dialogs().properties.borrow().clone()
    }

    /// Shows a dialog that belongs to the window.
    pub(super) fn present_window_dialog(&self, frame: &DialogFrame) {
        frame.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.dialog_closed()
        ));
        self.dialog_layer().present(frame);
    }

    /// After a window dialog closed, the folder takes focus again.
    fn dialog_closed(&self) {
        if self.dialog_layer().shown().is_none() {
            self.folder_pane().focus_view();
        }
    }

    /// Forgets which snapshot tab `id` browsed, as the tab closes.
    pub(super) fn forget_snapshot_tab(&self, id: TabId) {
        self.item_dialogs()
            .snapshot_tabs
            .borrow_mut()
            .retain(|(tab, _)| *tab != id);
    }

    /// True while a dialog is in front of the items: one on the dialog
    /// layer, or a dialog window of its own over this window.
    pub(super) fn dialog_in_front(&self) -> bool {
        self.dialog_layer().shown().is_some() || self.shows_dialog()
    }

    /// Closes every Properties window this window opened, as it closes,
    /// which stops their work: folders being counted and previous versions
    /// being looked up.
    pub(super) fn close_properties_windows(&self) {
        let windows = self.item_dialogs().properties.take();
        for window in windows {
            window.close();
        }
    }

    /// A dialog titled `title` showing `text`, with OK (`showMessage`).
    pub(super) fn show_result_dialog(&self, title: &str, text: &str) {
        let frame = DialogFrame::new(title, DialogWidth::Standard);
        frame.body().append(&quiet_text(text));
        frame.add_closing_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent, || {});
        self.present_window_dialog(&frame);
    }

    /// The template the folder at `uri` is optimized for, if it was
    /// customized (PROP-036).
    pub(crate) fn folder_template(&self, uri: &str) -> Option<FolderTemplate> {
        template_for(&self.context().settings_data().preferences.folder_views, uri)
    }

    /// Optimizes the folder at `uri` for `template`, and its sub-folders
    /// with `subfolders` (PROP-036); `reply` hears the outcome.
    pub(crate) fn customize_folder(
        &self,
        uri: &str,
        template: FolderTemplate,
        subfolders: bool,
        reply: impl FnOnce(Result<(), SettingsError>) + 'static,
    ) {
        self.context()
            .customize_folder(uri.to_owned(), template, subfolders, reply);
    }

    /// Shows the new icon or folder picture of the item at `uri` in every
    /// browser window of the application at once, as OK or Apply on the
    /// Customize tab does in Windows (PROP-016, PROP-037).
    pub(crate) fn announce_item_icon(&self, uri: &str) {
        let windows = self.application().map(|app| app.windows()).unwrap_or_default();
        let mut reached_self = false;
        for window in windows
            .iter()
            .filter_map(|window| window.downcast_ref::<BrowserWindow>())
        {
            reached_self |= window == self;
            window.refresh_item_icon(uri);
        }
        if !reached_self {
            self.refresh_item_icon(uri);
        }
    }

    /// Shows the new icon of the item at `uri` in the folder view
    /// (PROP-016).
    pub(crate) fn refresh_item_icon(&self, uri: &str) {
        for pane in self.folder_panes() {
            pane.owners().refresh_custom_icon(uri);
        }
    }

    /// Lists again every tab and split pane showing `folder`, keeping their selection,
    /// after something was written there; the search cache reads it
    /// again too (SRCH-033).
    pub(super) fn reload_tabs_showing(&self, folder: &str) {
        let changed = crate::search::changed_folders([folder], []);
        self.context().search_cache().folders_written(changed);
        let tabs: Vec<TabId> = {
            let session = self.imp().session.borrow();
            let showing = session.panes().filter(|tab| same_location(tab.uri(), folder));
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
