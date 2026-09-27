// SPDX-License-Identifier: AGPL-3.0-only
//! A native browsing window with independent tab histories and listings.

mod actions;
mod chrome;
mod content;
mod input;
mod navigation;
mod places;
mod session;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::SettingsData;

use crate::folder_view::model::FolderModel;
use crate::theme::{system::SystemScheme, Skin};
use crate::{icons, locations, typeahead};

use chrome::Chrome;
use content::Content;
use session::Session;

pub(crate) use actions::install_accelerators;

/// Controller and widgets of one desktop window.
///
/// The application retains the controller until the window is destroyed.
/// Signal handlers hold weak references, so closing a window cancels its
/// listings and monitors without retaining the widget tree.
pub struct BrowserWindow {
    window: gtk::ApplicationWindow,
    chrome: Chrome,
    content: Content,
    sidebar: gtk::ListBox,
    sidebar_uris: RefCell<Vec<Option<String>>>,
    session: RefCell<Session>,
    settings: SettingsData,
    home_uri: String,
    skin: Rc<Skin>,
    system_scheme: Rc<SystemScheme>,
    appearance_signal: Cell<Option<usize>>,
    changing_model: Cell<bool>,
    typeahead: RefCell<typeahead::Controller>,
    typeahead_timer: RefCell<Option<glib::SourceId>>,
    volume_monitor: gio::VolumeMonitor,
    volume_signals: RefCell<Vec<glib::SignalHandlerId>>,
}

impl BrowserWindow {
    /// Creates an empty window. Call [`Self::add_tab`] before presenting it.
    /// Settings are read as a snapshot; browsing never writes integration defaults.
    pub fn new(
        app: &gtk::Application,
        settings: SettingsData,
        skin: Rc<Skin>,
        system_scheme: Rc<SystemScheme>,
    ) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("OpenXplorer")
            .default_width(1320)
            .default_height(810)
            .css_classes(["ox"])
            .build();
        let chrome = Chrome::new(&window);
        let content = Content::new(skin.appearance());
        content.model.set_show_hidden(settings.preferences.show_hidden);
        content.inspector.set_visible(settings.preferences.details);
        if settings.preferences.view == "grid" {
            content.views.set_visible_child_name("grid");
        }
        let sidebar = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .build();
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(150)
            .child(&sidebar)
            .build();
        sidebar_scroll.add_css_class("sidebar");
        chrome.workspace.set_start_child(Some(&sidebar_scroll));
        chrome.workspace.set_end_child(Some(&content.root));
        chrome
            .workspace
            .set_position(settings.preferences.sidebar_width.unwrap_or(220) as i32);
        let browser = Rc::new(Self {
            window,
            chrome,
            content,
            sidebar,
            sidebar_uris: RefCell::new(Vec::new()),
            session: RefCell::new(Session::default()),
            settings,
            home_uri: gio::File::for_path(glib::home_dir()).uri().to_string(),
            skin,
            system_scheme,
            appearance_signal: Cell::new(None),
            changing_model: Cell::new(false),
            typeahead: RefCell::new(typeahead::Controller::default()),
            typeahead_timer: RefCell::new(None),
            volume_monitor: gio::VolumeMonitor::get(),
            volume_signals: RefCell::new(Vec::new()),
        });
        browser.connect_widgets();
        browser.install_actions();
        browser.install_input();
        browser.watch_volumes();
        browser.render_sidebar();
        browser
    }

    /// The native top-level window, for presenting and desktop integration.
    pub fn widget(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    /// The active folder's sorted, filtered native selection model.
    pub fn folder_model(&self) -> &FolderModel {
        &self.content.model
    }

    /// Number of tabs owned by this window.
    pub fn tab_count(&self) -> usize {
        self.session.borrow().tabs.len()
    }

    /// The active location, or `None` before the first tab is added.
    pub fn current_uri(&self) -> Option<String> {
        self.session
            .borrow()
            .active()
            .map(|tab| tab.history.current().to_string())
    }

    /// Whether the active tab is still receiving directory entries.
    pub fn is_loading(&self) -> bool {
        self.session.borrow().active().is_some_and(|tab| tab.loading)
    }

    /// The active listing's failure, if one occurred.
    pub fn load_error(&self) -> Option<String> {
        self.session.borrow().active().and_then(|tab| tab.error.clone())
    }

    /// Shows a recoverable startup or integration message in the window.
    pub fn notify(&self, message: &str) {
        self.show_message(message);
    }

    fn connect_widgets(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.content
            .model
            .selection()
            .connect_selection_changed(move |_, _, _| {
                if let Some(browser) = weak.upgrade() {
                    if !browser.changing_model.get() {
                        browser.save_selection();
                    }
                    browser.update_status();
                    browser.update_inspector();
                    browser.set_enabled("open", browser.content.model.summary().count == 1);
                }
            });
        let weak = Rc::downgrade(self);
        self.content
            .model
            .sorted()
            .connect_items_changed(move |_, _, _, _| {
                if let Some(browser) = weak.upgrade() {
                    browser.update_status();
                }
            });
        let weak = Rc::downgrade(self);
        self.chrome.search.connect_search_changed(move |search| {
            if let Some(browser) = weak.upgrade() {
                browser.content.model.set_query(search.text().as_str());
                browser.update_content();
            }
        });
        let weak = Rc::downgrade(self);
        self.chrome.entry.connect_activate(move |entry| {
            if let Some(browser) = weak.upgrade() {
                match browser.navigate(entry.text().as_str()) {
                    Ok(()) => browser.finish_address(),
                    Err(error) => browser.show_message(error.message()),
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.sidebar.connect_row_activated(move |_, row| {
            if let Some(browser) = weak.upgrade() {
                let uri = browser
                    .sidebar_uris
                    .borrow()
                    .get(row.index() as usize)
                    .cloned()
                    .flatten();
                if let Some(uri) = uri {
                    browser.navigate_or_report(&uri);
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.content.details.connect_activate(move |_, position| {
            if let Some(browser) = weak.upgrade() {
                browser.activate_item(position);
            }
        });
        let weak = Rc::downgrade(self);
        self.content.grid.connect_activate(move |_, position| {
            if let Some(browser) = weak.upgrade() {
                browser.activate_item(position);
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_scale_factor_notify(move |_| {
            if let Some(browser) = weak.upgrade() {
                browser.content.icons.redraw();
                browser.update_inspector();
            }
        });
        let weak = Rc::downgrade(self);
        self.system_scheme.connect_changed(move |dark| {
            if let Some(browser) = weak.upgrade() {
                let appearance = browser.skin.preference().resolve(dark);
                browser.skin.set_appearance(appearance);
            }
        });
        let weak = Rc::downgrade(self);
        let signal = self.skin.connect_changed(move |appearance| {
            if let Some(browser) = weak.upgrade() {
                browser.content.icons.set_appearance(appearance);
                browser.update_inspector();
                if let Some(action) = browser
                    .window
                    .lookup_action("theme")
                    .and_downcast::<gio::SimpleAction>()
                {
                    action.set_state(&browser.skin.preference().key().to_variant());
                }
            }
        });
        self.appearance_signal.set(Some(signal));
    }

    fn show_message(&self, message: &str) {
        self.chrome.message.set_text(message);
        self.chrome.message.set_visible(!message.is_empty());
    }

    fn update_status(&self) {
        let count = self.content.model.n_items();
        let selected = self.content.model.summary();
        let mut text = if count == 1 {
            "1 item".to_string()
        } else {
            format!("{count} items")
        };
        if selected.count > 0 {
            text.push_str(&format!("  ·  {} selected", selected.count));
            if selected.has_files {
                text.push_str(&format!("  ·  {}", ox_core::format::pretty_bytes(selected.bytes)));
            }
        }
        if self.is_loading() {
            text.push_str("  ·  Loading…");
        }
        self.chrome.status.set_text(&text);
    }

    fn update_inspector(&self) {
        let items = self.content.model.selected_items();
        let (name, info, art) = if let [item] = items.as_slice() {
            let entry = item.entry();
            let size = entry.size.map(ox_core::format::pretty_bytes).unwrap_or_default();
            let info = format!(
                "{}\n{}\n\n{}",
                entry.type_label,
                size,
                locations::address_text(&entry.uri)
            );
            (entry.name.clone(), info, item.art().clone())
        } else if items.is_empty() {
            (
                self.current_uri()
                    .map(|uri| locations::title_for(&uri, &self.home_uri))
                    .unwrap_or_default(),
                "Select a file or folder to see its details.".to_string(),
                icons::ArtKind::Folder,
            )
        } else {
            (
                format!("{} items selected", items.len()),
                "Select one item to see its details.".to_string(),
                icons::ArtKind::Folder,
            )
        };
        self.content.inspector_name.set_text(&name);
        self.content.inspector_info.set_text(&info);
        icons::set_art(
            &self.content.inspector_icon,
            &art,
            112,
            self.skin.appearance(),
            self.window.scale_factor(),
        );
    }
}

impl Drop for BrowserWindow {
    fn drop(&mut self) {
        if let Some(signal) = self.appearance_signal.take() {
            self.skin.disconnect_changed(signal);
        }
        if let Some(timer) = self.typeahead_timer.get_mut().take() {
            timer.remove();
        }
        for signal in self.volume_signals.get_mut().drain(..) {
            self.volume_monitor.disconnect(signal);
        }
    }
}
