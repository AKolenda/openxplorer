// SPDX-License-Identifier: AGPL-3.0-only
//! The body of the Properties dialog: its tabs and their panels.
//!
//! Ports `propertiesDialog` in `desktop/ui/app.js` (PROP-001, PROP-003,
//! PROP-006): the tabs General, Location (standard folders only),
//! Permissions, Checksums (files only, PROP-014) and Previous versions, the item's properties read once
//! when the dialog opens, and the versions looked up the first time their
//! tab is shown. [`PropertiesView`] is a widget subclass the dialog frame
//! holds; the window keeps the frame, and so the view with everything it
//! read, while the dialog's tab is in the background.

use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::EntryError;
use ox_core::folder_locations::FolderRelocation;
use ox_core::location::{is_smb_location, is_smb_server, is_smb_share_root, ItemKind, LocationContext};
use ox_core::versions::is_conventional_snapshot;
use ox_core::versions::PreviousVersions;

use super::checksums_panel::ChecksumsPanel;
use super::folder_sizes::FolderSizeState;
use super::general_panel::{self, GeneralFacts};
use super::location_panel::LocationPanel;
use super::metadata::{read_properties, ItemProperties};
use super::versions_panel::VersionsPanel;
use super::{PropertiesTab, PropertiesTarget};
use crate::dialog_layer::{quiet_text, DialogFrame, DialogWidth};
use ox_core::integration::BraveIntegration;

/// Shown on the General tab while the properties are read.
const READING: &str = "Reading file properties…";

/// The shortest height of a tab's panel (`.properties-panel`).
const PANEL_MIN_HEIGHT: i32 = 290;

/// What a Properties dialog needs from the window that opens it.
#[derive(Debug, Clone)]
pub(crate) struct PropertiesContext {
    /// The previous-versions service every window shares.
    pub versions: Arc<PreviousVersions>,
    /// Display names for the home folder and devices.
    pub locations: LocationContext,
    /// The folder's measured size, if it was measured this session.
    pub folder_size: Option<FolderSizeState>,
    /// Moves a standard folder, for the Location tab.
    pub relocation: Arc<FolderRelocation>,
    /// Brave's download-folder integration, for the Location tab's
    /// follow-up.
    pub brave: BraveIntegration,
}

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::super::checksums_panel::ChecksumsPanel;
    use super::super::general_panel::FolderRows;
    use super::super::versions_panel::VersionsPanel;
    use super::super::PropertiesTarget;

    /// Private state of [`super::PropertiesView`].
    #[derive(Debug, Default)]
    pub(crate) struct PropertiesView {
        /// The item described; set by `new`.
        pub(super) target: OnceCell<PropertiesTarget>,
        /// The row under the tab buttons, with its bottom rule.
        pub(super) tab_row: gtk::Box,
        /// The tab buttons above the panels.
        pub(super) switcher: gtk::StackSwitcher,
        /// One page per tab.
        pub(super) pages: gtk::Stack,
        /// The General tab.
        pub(super) general: gtk::Box,
        /// The Permissions tab.
        pub(super) permissions: gtk::Box,
        /// The Previous versions tab; set by `new`.
        pub(super) versions: OnceCell<VersionsPanel>,
        /// The Checksums tab of a file.
        pub(super) checksums: OnceCell<ChecksumsPanel>,
        /// The Size and Contains values of a folder, once the properties
        /// are read, so a scan's progress can update them.
        pub(super) folder_rows: RefCell<Option<FolderRows>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PropertiesView {
        const NAME: &'static str = "OxPropertiesView";
        type Type = super::PropertiesView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BoxLayout>();
        }
    }

    impl ObjectImpl for PropertiesView {
        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            view.layout_manager()
                .and_downcast::<gtk::BoxLayout>()
                .expect("class_init sets a box layout")
                .set_orientation(gtk::Orientation::Vertical);
            view.add_css_class("properties-view");
            self.switcher.set_stack(Some(&self.pages));
            // The tabs keep their own width, from the left, as `.properties-tabs`.
            self.switcher.set_halign(gtk::Align::Start);
            self.tab_row.add_css_class("properties-tabs");
            self.tab_row.append(&self.switcher);
            self.tab_row.set_parent(&*view);
            self.pages.set_parent(&*view);
        }

        fn dispose(&self) {
            self.tab_row.unparent();
            self.pages.unparent();
        }
    }

    impl WidgetImpl for PropertiesView {}
}

glib::wrapper! {
    /// The tabs and panels of one Properties dialog.
    pub(crate) struct PropertiesView(ObjectSubclass<imp::PropertiesView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PropertiesView {
    /// The Properties of `target`, opened on `initial` (General when the
    /// item has no such tab). It starts reading the item's properties at
    /// once; the versions are looked up when their tab is first shown.
    pub(crate) fn new(target: PropertiesTarget, context: PropertiesContext, initial: PropertiesTab) -> Self {
        let view: Self = glib::Object::new();
        let imp = view.imp();
        let versions = VersionsPanel::new(&target, Arc::clone(&context.versions), context.locations.clone());
        imp.versions
            .set(versions)
            .expect("a new view has no versions panel yet");
        imp.target.set(target).expect("a new view has no target yet");
        view.add_pages(&context);
        view.select_tab(initial);
        view.follow_selected_tab();
        view.read_properties(context);
        view
    }

    /// The item described.
    pub(crate) fn target(&self) -> &PropertiesTarget {
        self.imp().target.get().expect("new sets the target")
    }

    fn versions_panel(&self) -> &VersionsPanel {
        self.imp().versions.get().expect("new sets the versions panel")
    }

    /// Adds one page per tab the item has.
    fn add_pages(&self, context: &PropertiesContext) {
        let imp = self.imp();
        let pages = &imp.pages;
        pages.set_vhomogeneous(false);
        imp.general.set_orientation(gtk::Orientation::Vertical);
        imp.general.append(&quiet_text(READING));
        imp.permissions.set_orientation(gtk::Orientation::Vertical);
        self.add_page(PropertiesTab::General, imp.general.upcast_ref());
        if let Some(folder) = self.target().known_folder {
            let relocation = Arc::clone(&context.relocation);
            let location = LocationPanel::new(folder, relocation, context.brave.clone());
            self.add_page(PropertiesTab::Location, location.upcast_ref());
        }
        self.add_page(PropertiesTab::Permissions, imp.permissions.upcast_ref());
        if self.target().kind == ItemKind::File {
            let checksums = ChecksumsPanel::new(&self.target().uri);
            self.add_page(PropertiesTab::Checksums, checksums.widget().upcast_ref());
            imp.checksums.set(checksums).expect("added once");
        }
        self.add_page(
            PropertiesTab::PreviousVersions,
            self.versions_panel().upcast_ref(),
        );
    }

    fn add_page(&self, tab: PropertiesTab, panel: &gtk::Widget) {
        panel.add_css_class("properties-panel");
        panel.set_size_request(-1, PANEL_MIN_HEIGHT);
        self.imp()
            .pages
            .add_titled(panel, Some(tab.page_name()), tab.label());
    }

    /// Shows `tab`, or General when the item has no such tab.
    pub(crate) fn select_tab(&self, tab: PropertiesTab) {
        let pages = &self.imp().pages;
        let name = if pages.child_by_name(tab.page_name()).is_some() {
            tab.page_name()
        } else {
            PropertiesTab::General.page_name()
        };
        pages.set_visible_child_name(name);
    }

    /// The tab shown.
    pub(crate) fn selected_tab(&self) -> PropertiesTab {
        let name = self.imp().pages.visible_child_name();
        name.as_deref()
            .and_then(PropertiesTab::from_page_name)
            .unwrap_or(PropertiesTab::General)
    }

    /// Looks the versions up the first time their tab is shown, and tells
    /// the dialog to widen for the versions list.
    fn follow_selected_tab(&self) {
        self.imp().pages.connect_visible_child_name_notify(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.tab_shown()
        ));
        self.tab_shown();
    }

    fn tab_shown(&self) {
        if self.selected_tab() == PropertiesTab::PreviousVersions {
            self.versions_panel().load_once();
        }
        self.notify_tab_changed();
    }

    /// How wide the dialog is for the tab shown: wider for the versions
    /// list (`versions-modal`).
    pub(crate) fn dialog_width(&self) -> DialogWidth {
        if self.selected_tab() == PropertiesTab::PreviousVersions {
            DialogWidth::Versions
        } else {
            DialogWidth::Properties
        }
    }

    /// Asks the frame around the view to fit the tab shown.
    fn notify_tab_changed(&self) {
        let frame = self
            .ancestor(DialogFrame::static_type())
            .and_downcast::<DialogFrame>();
        if let Some(frame) = frame {
            frame.set_width(self.dialog_width());
        }
    }

    /// Reads the item's properties off the main thread and fills the
    /// General and Permissions tabs.
    fn read_properties(&self, context: PropertiesContext) {
        let uri = self.target().uri.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let read = read_properties(uri).await;
                view.show_properties(read, &context);
            }
        ));
    }

    fn show_properties(&self, read: Result<ItemProperties, EntryError>, context: &PropertiesContext) {
        let imp = self.imp();
        let properties = match read {
            Ok(properties) => properties,
            Err(error) => {
                general_panel::show_read_failure(&imp.general, &imp.permissions, &error.to_string());
                return;
            }
        };
        let facts = GeneralFacts {
            properties: &properties,
            locations: &context.locations,
            folder_size: context.folder_size.as_ref(),
            snapshot_roots: &context.locations.snapshot_roots,
            can_rename: self.can_rename(&properties, context),
        };
        let folder_rows = general_panel::fill_general(&imp.general, &facts);
        imp.folder_rows.replace(folder_rows);
        general_panel::fill_permissions(&imp.permissions, &properties);
    }

    /// Whether the name can be edited: an item in a local or shared folder,
    /// not a standard folder, a share or server, or inside a previous version.
    fn can_rename(&self, properties: &ItemProperties, context: &PropertiesContext) -> bool {
        let uri = &properties.entry.uri;
        let is_share = is_smb_server(uri) || is_smb_share_root(uri);
        let is_read_only = context.locations.is_snapshot_location(uri) || is_conventional_snapshot(uri);
        let is_renamable_place = uri.starts_with("file:") || is_smb_location(uri);
        self.target().known_folder.is_none()
            && properties.parent_uri.is_some()
            && !is_share
            && !is_read_only
            && is_renamable_place
    }

    /// Shows the folder's new measured size, if this dialog describes the
    /// folder at `uri`.
    pub(crate) fn show_folder_size(&self, uri: &str, state: &FolderSizeState) {
        if super::size_key(uri) != super::size_key(&self.target().uri) {
            return;
        }
        if let Some(rows) = self.imp().folder_rows.borrow().as_ref() {
            rows.show(state);
        }
    }

    /// Stops the work the dialog started: a versions lookup in progress
    /// (`finish` in `propertiesDialog`).
    pub(crate) fn cancel_work(&self) {
        self.versions_panel().cancel();
        if let Some(checksums) = self.imp().checksums.get() {
            checksums.cancel();
        }
    }

    /// The Checksums tab of a file, for tests.
    #[cfg(test)]
    pub(crate) fn checksums(&self) -> Option<&ChecksumsPanel> {
        self.imp().checksums.get()
    }

    /// The Size value shown, for tests.
    #[cfg(test)]
    pub(crate) fn size_text(&self) -> Option<String> {
        let rows = self.imp().folder_rows.borrow().clone()?;
        Some(rows.size.text().to_string())
    }

    /// The General tab, for tests.
    #[cfg(test)]
    pub(crate) fn general_panel(&self) -> gtk::Box {
        self.imp().general.clone()
    }

    /// The Permissions tab, for tests.
    #[cfg(test)]
    pub(crate) fn permissions_panel(&self) -> gtk::Box {
        self.imp().permissions.clone()
    }

    /// The Previous versions tab, for tests.
    #[cfg(test)]
    pub(crate) fn versions(&self) -> VersionsPanel {
        self.versions_panel().clone()
    }

    /// The tab labels, for tests.
    #[cfg(test)]
    pub(crate) fn tab_labels(&self) -> Vec<String> {
        let pages = self.imp().pages.pages();
        (0..pages.n_items())
            .filter_map(|position| pages.item(position).and_downcast::<gtk::StackPage>())
            .filter_map(|page| page.title())
            .map(String::from)
            .collect()
    }
}
