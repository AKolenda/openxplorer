// SPDX-License-Identifier: AGPL-3.0-only
//! The `GObject` side of [`super::BrowserWindow`]: its template children,
//! the state of its tabs and of each feature, and the class and lifetime
//! hooks GTK calls. The frame is the template `resources/ui/window.ui`,
//! the static layout of `desktop/ui/index.html`.

use std::cell::{Cell, OnceCell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::LocationContext;

use super::activation::Activations;
use super::address_bar::AddressBar;
use super::breakpoints::WindowWidth;
use super::caption_buttons::CaptionButtons;
use super::closing::ClosingState;
use super::command_bar::CommandBar;
use super::connections::ExternalHandlers;
use super::details_pane::DetailsPane;
use super::file_drag::OutgoingDrag;
use super::file_drop::{FirstOffer, PendingDrop, ProgramChecks};
use super::file_ops::FileOperations;
use super::folder_pane::FolderPane;
use super::menu_popover::MenuPopover;
use super::search_box::SearchBox;
use super::session::Session;
use super::session::TabId;
use super::settings_tab::SettingsTabState;
use super::sidebar::Sidebar;
use super::status_bar::StatusBar;
use super::tab_commands::ClosedTab;
use super::tab_moves::OutgoingTabDrag;
use super::tab_strip::TabStrip;
use super::toast::Toast;
use super::transfer_panel::TransferPanel;
use super::type_to_select::Typeahead;
use crate::app_context::AppContext;
use crate::network::WindowNetwork;
use crate::search::{FolderSearch, SearchInfoStrip};
use crate::settings_page::SettingsPage;
use crate::volumes::VolumeRow;

/// Private state of [`super::BrowserWindow`]: the parts of the frame
/// it updates, then the state of its tabs.
#[derive(Debug, Default, gtk::CompositeTemplate)]
#[template(file = "../../resources/ui/window.ui")]
pub(crate) struct BrowserWindow {
    /// The tabs in the title bar.
    #[template_child]
    pub(super) tab_strip: TemplateChild<TabStrip>,
    /// "+", right after the last tab.
    #[template_child]
    pub(super) new_tab_button: TemplateChild<gtk::Button>,
    /// Lists the open windows.
    #[template_child]
    pub(super) open_windows_button: TemplateChild<gtk::MenuButton>,
    /// The history buttons, the address and the search box.
    #[template_child]
    pub(super) navigation_row: TemplateChild<gtk::Box>,
    /// Back, Forward, Up and Refresh.
    #[template_child]
    pub(super) navigation_buttons: TemplateChild<gtk::Box>,
    /// Breadcrumbs or the editable address.
    #[template_child]
    pub(super) address_bar: TemplateChild<AddressBar>,
    /// The search box that filters the folder.
    #[template_child]
    pub(super) search_box: TemplateChild<SearchBox>,
    /// New, the edit commands, Sort, View, More, appearance and Details.
    #[template_child]
    pub(super) command_bar: TemplateChild<CommandBar>,
    /// The workspace, or the Settings page on the Settings tab.
    #[template_child]
    pub(super) surfaces: TemplateChild<gtk::Stack>,
    /// The split between the sidebar and the folder and details panes
    /// (`.sidebar-resizer`).
    #[template_child]
    pub(super) workspace: TemplateChild<gtk::Paned>,
    /// The navigation pane.
    #[template_child]
    pub(super) sidebar: TemplateChild<Sidebar>,
    /// What a search looked at, above the columns while searching.
    #[template_child]
    pub(super) search_strip: TemplateChild<SearchInfoStrip>,
    /// The folder pane.
    #[template_child]
    pub(super) folder_pane: TemplateChild<FolderPane>,
    /// The details pane beside the folder pane.
    #[template_child]
    pub(super) details_pane: TemplateChild<DetailsPane>,
    /// The running file operation's progress and Cancel, over the
    /// folder pane.
    #[template_child]
    pub(super) transfer_panel: TemplateChild<TransferPanel>,
    /// The message at the bottom of the workspace.
    #[template_child]
    pub(super) toast: TemplateChild<Toast>,
    /// Counts, the type-to-select hint and the view buttons.
    #[template_child]
    pub(super) status_bar: TemplateChild<StatusBar>,
    /// The Settings page, shown on the Settings tab.
    #[template_child]
    pub(super) settings_page: TemplateChild<SettingsPage>,
    /// The folder shown before Settings and the places Settings offers
    /// the search index.
    pub(super) settings_tab: RefCell<SettingsTabState>,
    /// What every window shares: the skin, settings and places. It
    /// comes from the application, so [`super::BrowserWindow::new`]
    /// sets it.
    pub(super) context: OnceCell<AppContext>,
    /// The desktop's volume monitor, set by `constructed`. Holding it
    /// keeps the monitor, and so its signals, alive.
    pub(super) volume_monitor: OnceCell<gio::VolumeMonitor>,
    /// The window's sign-in prompts and dialogs and its server
    /// discovery; [`super::BrowserWindow::new`] sets it.
    pub(super) network: OnceCell<WindowNetwork>,
    /// The tabs and which one is active.
    pub(super) session: RefCell<Session>,
    /// The lookups still running per tab, so one that answers after its
    /// tab moved on is dropped (SAFE-013).
    pub(super) activations: RefCell<Activations>,
    /// Display names of the home folder and the mounted devices.
    pub(super) locations: RefCell<LocationContext>,
    /// The drives and devices the volume monitor reported last.
    pub(super) volumes: RefCell<Vec<VolumeRow>>,
    /// The type-to-select prefix of the folder views.
    pub(super) typeahead: RefCell<Typeahead>,
    /// The column Up and Down keep to in the icon grid, across rows of
    /// different lengths; see [`super::grid_keys`].
    pub(super) grid_column: Cell<Option<super::grid_keys::GridColumn>>,
    /// The link to GNOME's previewer (PROP-012).
    pub(super) quick_look: super::quick_look::QuickLook,
    /// The search box's search.
    pub(super) search: RefCell<FolderSearch>,
    /// In tests, a volume id and the root it mounts at, standing in for a
    /// drive the isolated session does not have.
    #[cfg(test)]
    pub(super) test_volume: RefCell<Option<(String, String)>>,
    /// Set while the window swaps or reloads the model, so the
    /// selection it restores is not saved over the tab's selection.
    pub(super) changing_model: Cell<bool>,
    /// Set until the file list takes keyboard focus in a new window or
    /// after Settings hides; see
    /// [`super::BrowserWindow::focus_new_file_list`].
    pub(super) file_list_awaits_focus: Cell<bool>,
    /// Set while a pin request is being checked and saved; another waits
    /// its turn by being ignored, as `state.pinBusy` in app.js.
    pub(super) pinning: Cell<bool>,
    /// The width band the layout was last fitted to.
    pub(super) window_width: Cell<WindowWidth>,
    /// What the window must disconnect when it goes away.
    pub(super) handlers: RefCell<ExternalHandlers>,
    /// The running file operation, Trash support and the file
    /// clipboard.
    pub(super) file_operations: RefCell<FileOperations>,
    /// Whether a close waits for a running write (TAB-049).
    pub(super) closing: Cell<ClosingState>,
    /// The user agreed to close every tab of the window (SET-010).
    pub(super) closing_tabs_confirmed: Cell<bool>,
    /// The file drag this window started, while it lasts.
    pub(super) outgoing_drag: RefCell<Option<OutgoingDrag>>,
    /// Until when clicks that open items are ignored, around a drag.
    pub(super) item_clicks_resume_at: Cell<Option<std::time::Instant>>,
    /// Which files under a drag are programs (DND-026).
    pub(super) program_checks: RefCell<ProgramChecks>,
    /// Where the last drop happened, in the folder pane's
    /// coordinates, for the drop menu.
    pub(super) drop_point: Cell<(f64, f64)>,
    /// The drop that waits for the drop menu's answer.
    pub(super) pending_drop: RefCell<Option<PendingDrop>>,
    /// What the drag over the window offered when it arrived.
    pub(super) first_offer: RefCell<Option<FirstOffer>>,
    /// The drop menu, built when first needed.
    pub(super) drop_menu: OnceCell<MenuPopover>,
    /// The tab a file drag hovers over, and the timer that shows it.
    pub(super) tab_hover: RefCell<Option<(TabId, glib::SourceId)>>,
    /// The folder a file drag hovers over, and the timer that opens it.
    pub(super) folder_hover: RefCell<Option<(String, glib::SourceId)>>,
    /// The scroll of a zone a file drag hovers near the edge of.
    pub(super) drag_scroll: RefCell<Option<super::file_drop::DragScroll>>,
    /// The crumb divider a file drag hovers over, and the timer that
    /// opens its subfolder menu (NAV-021).
    pub(super) divider_hover: RefCell<Option<(String, glib::SourceId)>>,
    /// The subfolder menu a file drag opened, which takes the drop.
    pub(super) drag_crumb_menu: super::crumb_drop::DragCrumbMenu,
    /// The folder listed to complete the typed address (NAV-030).
    pub(super) completion_listing: super::address_completion::CompletionListing,
    /// The tab drag this window started, while it lasts.
    pub(super) outgoing_tab: RefCell<Option<OutgoingTabDrag>>,
    /// The timer that saves the window's size after a resize.
    pub(super) size_save: RefCell<Option<glib::SourceId>>,
    /// The tabs closed in this window, most recent first.
    pub(super) closed_tabs: RefCell<Vec<ClosedTab>>,
    /// The in-window dialogs, Properties by tab, and the tabs that
    /// browse snapshots.
    pub(super) item_dialogs: super::item_dialogs::ItemDialogs,
    /// Measured folder sizes and the running folder-size scan.
    pub(super) size_scans: super::folder_size_scan::SizeScans,
}

#[glib::object_subclass]
impl ObjectSubclass for BrowserWindow {
    const NAME: &'static str = "OxBrowserWindow";
    type Type = super::BrowserWindow;
    type ParentType = gtk::ApplicationWindow;

    fn class_init(klass: &mut Self::Class) {
        // GtkBuilder finds the template's own types by name, so they
        // must be registered before the template is parsed.
        CaptionButtons::ensure_type();
        TabStrip::ensure_type();
        AddressBar::ensure_type();
        SearchBox::ensure_type();
        CommandBar::ensure_type();
        Sidebar::ensure_type();
        SearchInfoStrip::ensure_type();
        FolderPane::ensure_type();
        DetailsPane::ensure_type();
        TransferPanel::ensure_type();
        Toast::ensure_type();
        StatusBar::ensure_type();
        SettingsPage::ensure_type();
        klass.bind_template();
    }

    fn instance_init(window: &glib::subclass::InitializingObject<Self>) {
        window.init_template();
    }
}

impl ObjectImpl for BrowserWindow {
    fn constructed(&self) {
        self.parent_constructed();
        let window = self.obj();
        window.finish_title_bar();
        window.add_navigation_buttons();
        window.watch_quick_look();
        self.volume_monitor
            .set(gio::VolumeMonitor::get())
            .expect("constructed runs once per object");
    }

    fn dispose(&self) {
        self.obj().disconnect_external_handlers();
        self.obj().close_network();
        // Dropping the tabs cancels their listings and folder watches.
        self.session.take();
    }
}

impl WidgetImpl for BrowserWindow {
    fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(width, height, baseline);
        self.obj().follow_width(width);
    }
}

impl WindowImpl for BrowserWindow {
    fn close_request(&self) -> glib::Propagation {
        // Safety rule "an update locks the application" (UPD-005): no
        // window closes while an update installs (closing.rs).
        if let Some(refusal) = self.obj().close_refusal() {
            self.obj().show_message(&refusal);
            return glib::Propagation::Stop;
        }
        // Nor while it writes files: it asks whether to cancel first
        // (TAB-049).
        if !self.obj().may_close_now() {
            return glib::Propagation::Stop;
        }
        // Let go of keyboard focus first. On Wayland, GTK's input method
        // otherwise keeps the focused address entry and later asks a
        // destroyed widget for its cursor position (a Gtk-CRITICAL).
        GtkWindowExt::set_focus(&*self.obj(), None::<&gtk::Widget>);
        self.obj().save_pending_size();
        // A closed window's sign-ins, listings and folder watches end with
        // it, even while something still holds the window (SAFE-011,
        // TAB-050).
        self.obj().close_network();
        for tab in self.session.borrow_mut().tabs_mut() {
            tab.stop_reading();
        }
        self.parent_close_request()
    }
}

impl ApplicationWindowImpl for BrowserWindow {}
