// SPDX-License-Identifier: AGPL-3.0-only
//! The Settings page: categories on the left, the chosen one on the right.
//!
//! Ports `renderSettingsPage`, `settingsSearch` and `appendV07Settings` in
//! `v2.0.0:desktop/ui/app.js`, laid out as the owner asked (SET-019 and the
//! settings mockup its note names): a list of categories with a
//! settings search at its top, and only the chosen category on the right,
//! as flat groups of one-line rows, the details of each in an ⓘ bubble
//! ([`info_bubble`]) and the rarely changed ones folded away. A search
//! shows the matching settings of every category on one page of results.
//! [`SettingsPage`] is a `GtkBox` subclass whose frame is the template
//! `resources/ui/settings-page.ui`; each category builds its page in a
//! module of its own ([`general`], [`appearance`], [`files_folders`],
//! [`archives`], [`confirmations`], [`indexing`], [`default_apps`] with
//! [`brave`], [`about`]) as a [`section::SettingsSection`], from the
//! pieces in [`row`], [`group`], [`parts`] and [`choice_list`]. The list
//! and the search are in [`navigation`].
//!
//! A window builds the page the first time Settings is shown
//! ([`building`]). Every row reads and writes the shared settings file
//! through ox-core, with the Python app's keys and checks, so both apps
//! stay in step, and changes show at once in every window.

mod about;
mod appearance;
mod archives;
mod bindings;
mod brave;
mod building;
mod category_row;
mod choice_list;
mod confirmations;
mod default_apps;
mod files_folders;
mod general;
mod group;
mod indexed_folders;
mod indexing;
mod info_bubble;
mod navigation;
mod pages;
mod parts;
mod row;
mod search;
mod section;
mod startup;
mod status_card;
mod troubleshooting;

#[cfg(test)]
mod tests;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::IndexRoot;
use ox_core::settings::Preferences;

use crate::app_context::AppContext;
use crate::icons::{self, Icon};
use crate::locations::Page;
use crate::window::{show_bundled_clear_icon, show_bundled_magnifier};

pub(crate) use choice_list::ChoiceButton;
pub(crate) use indexed_folders::{index_candidates, CandidateSources, IndexCandidate};
#[cfg(test)]
pub(crate) use pages::Subpage;
pub(crate) use pages::{Category, SettingsView};
pub(crate) use row::PageWidth;

/// Emitted when "Back to files" is clicked.
const BACK_TO_FILES: &str = "back-to-files";
/// Emitted with a message for the window's message line.
const MESSAGE: &str = "message";

/// The glyph of "Back to files".
const BACK_GLYPH: i32 = 16;

/// The class of the page in a narrow window, which narrows the category
/// list (`resources/skin/settings.css`).
const NARROW_CLASS: &str = "narrow";

/// A handler the page connected to an object that outlives it: the
/// shared skin or context.
#[derive(Debug)]
struct SharedHandler {
    /// What the handler is connected to.
    object: glib::Object,
    /// The handler, disconnected when the page goes away.
    id: glib::SignalHandlerId,
}

/// Updates a control from the preferences as last read or saved
/// ([`SettingsPage::follow_preferences`]).
type PreferenceFollower = Box<dyn Fn(&Preferences)>;

/// Work to do each time Settings opens, such as reading which app opens
/// folders now.
type OpenedHook = Box<dyn Fn()>;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::indexed_folders::IndexedFolders;
    use super::pages::{Category, SettingsView, Subpage};
    use super::row::PageWidth;
    use super::search::SearchQuery;
    use super::section::SettingsSection;
    use super::{OpenedHook, PreferenceFollower, SharedHandler, BACK_TO_FILES, MESSAGE};
    use crate::app_context::AppContext;

    /// Private state of [`super::SettingsPage`].
    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "../resources/ui/settings-page.ui")]
    pub(crate) struct SettingsPage {
        /// "Your explorer, your way."
        #[template_child]
        pub(super) subtitle: TemplateChild<gtk::Label>,
        /// "Search settings".
        #[template_child]
        pub(super) search_entry: TemplateChild<gtk::SearchEntry>,
        /// The categories.
        #[template_child]
        pub(super) category_list: TemplateChild<gtk::ListBox>,
        /// "Back to files".
        #[template_child]
        pub(super) back_button: TemplateChild<gtk::Button>,
        /// The arrow of "Back to files".
        #[template_child]
        pub(super) back_glyph: TemplateChild<gtk::Image>,
        /// One page per category and sub-page, and the no-match page.
        #[template_child]
        pub(super) pages: TemplateChild<gtk::Stack>,
        /// The windows' shared state, set by `SettingsPage::bind`.
        pub(super) context: OnceCell<AppContext>,
        /// Set once the categories and sub-pages are built, the first time
        /// Settings is shown.
        pub(super) is_built: Cell<bool>,
        /// The width the page was last fitted to, which the sections take
        /// when they are built.
        pub(super) page_width: Cell<PageWidth>,
        /// The categories' pages.
        pub(super) category_sections: RefCell<HashMap<Category, SettingsSection>>,
        /// The box on each category's own page that holds its section
        /// while no search is typed.
        pub(super) category_hosts: RefCell<HashMap<Category, gtk::Box>>,
        /// The page of search results, where every category's section is
        /// while a search is typed.
        pub(super) results: OnceCell<gtk::Box>,
        /// "3 matching settings", at the top of the search results.
        pub(super) match_count: OnceCell<gtk::Label>,
        /// The sub-pages.
        pub(super) subpages: RefCell<HashMap<Subpage, SettingsSection>>,
        /// What the right side shows.
        pub(super) view: Cell<SettingsView>,
        /// What the settings search looks for.
        pub(super) query: RefCell<SearchQuery>,
        /// The folders the search index keeps and can take.
        pub(super) indexed_folders: OnceCell<IndexedFolders>,
        /// The folder shown before Settings opened, where a typed folder
        /// to index starts.
        pub(super) index_origin: RefCell<Option<String>>,
        /// Controls that show a preference.
        pub(super) followers: RefCell<Vec<PreferenceFollower>>,
        /// Set while the controls show the current preferences, so their
        /// change handlers save nothing.
        pub(super) showing_preferences: Cell<bool>,
        /// Work to do each time Settings opens.
        pub(super) opened_hooks: RefCell<Vec<OpenedHook>>,
        /// On the shared skin and context, which outlive the page.
        pub(super) handlers: RefCell<Vec<SharedHandler>>,
    }

    impl std::fmt::Debug for SettingsPage {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("SettingsPage")
                .field("view", &self.view.get())
                .field("query", &self.query.borrow())
                .finish_non_exhaustive()
        }
    }

    impl SettingsPage {
        /// "3 matching settings", at the top of the search results.
        ///
        /// # Panics
        ///
        /// Before the navigation is built with the page.
        pub(super) fn match_count(&self) -> &gtk::Label {
            self.match_count
                .get()
                .expect("the navigation is built with the page")
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsPage {
        const NAME: &'static str = "OxSettingsPage";
        type Type = super::SettingsPage;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(page: &glib::subclass::InitializingObject<Self>) {
            page.init_template();
        }
    }

    impl ObjectImpl for SettingsPage {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder(BACK_TO_FILES).build(),
                    Signal::builder(MESSAGE)
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "settings-page.ui");
            self.obj().finish_template();
        }

        fn dispose(&self) {
            for handler in self.handlers.take() {
                handler.object.disconnect(handler.id);
            }
            self.followers.take();
            self.opened_hooks.take();
        }
    }

    impl WidgetImpl for SettingsPage {
        /// Settings is shown: the first time, the page builds its
        /// sections; every time, it reads what may have changed while it
        /// was hidden.
        fn map(&self) {
            self.obj().build_pages_once();
            self.parent_map();
            self.obj().refresh();
        }
    }

    impl BoxImpl for SettingsPage {}
}

glib::wrapper! {
    /// The Settings page of one window.
    pub(crate) struct SettingsPage(ObjectSubclass<imp::SettingsPage>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl SettingsPage {
    /// Gives the template what it cannot express: the subtitle and the
    /// back arrow.
    fn finish_template(&self) {
        let imp = self.imp();
        imp.subtitle.set_text(Page::Settings.subtitle());
        icons::set_icon(&imp.back_glyph, Icon::ArrowLeft, BACK_GLYPH);
        show_bundled_magnifier(&imp.search_entry);
        show_bundled_clear_icon(&imp.search_entry);
        // Typing anywhere on the page outside a text field starts a
        // settings search, as in GNOME Settings.
        imp.search_entry.set_key_capture_widget(Some(self));
        imp.back_button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.emit_by_name::<()>(BACK_TO_FILES, &[])
        ));
    }

    /// Shows the settings `context` shares, and keeps the rows showing its
    /// values from now on. The rows are built the first time Settings is
    /// shown or opened.
    ///
    /// # Panics
    ///
    /// When called twice: a page belongs to one window's context.
    pub(crate) fn bind(&self, context: &AppContext) {
        self.imp()
            .context
            .set(context.clone())
            .expect("a settings page is bound once");
        self.follow_shared_state();
    }

    /// The windows' shared state.
    pub(super) fn context(&self) -> &AppContext {
        self.imp()
            .context
            .get()
            .expect("the window binds its settings page when it is created")
    }

    /// Calls `on_back` when "Back to files" is clicked.
    pub(crate) fn connect_back_to_files(&self, on_back: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(BACK_TO_FILES, false, move |_| {
            on_back();
            None
        })
    }

    /// Calls `show` with every message for the window's message line.
    pub(crate) fn connect_message(&self, show: impl Fn(&str) + 'static) -> glib::SignalHandlerId {
        self.connect_local(MESSAGE, false, move |values| {
            let message = values.get(1).and_then(|value| value.get::<String>().ok());
            show(message.as_deref().unwrap_or_default());
            None
        })
    }

    /// Settings opened, at `view` when one is asked for, else where it was
    /// left, with keyboard focus on the chosen category, so arrow keys,
    /// Escape and typing reach the page at once. Showing the page reads
    /// what may have changed while it was hidden.
    pub(crate) fn open(&self, view: Option<SettingsView>) {
        // Built here too: a window opened at Settings is not shown yet.
        self.build_pages_once();
        if let Some(view) = view {
            // A view asked for by name shows all of it, not the rows an
            // earlier search left, which may be none.
            self.search("");
            self.show_view(view);
        }
        self.focus_chosen_category();
    }

    /// What the settings search box holds, for tests.
    #[cfg(test)]
    pub(crate) fn search_text(&self) -> String {
        self.imp().search_entry.text().to_string()
    }

    /// "Back to files", for tests.
    #[cfg(test)]
    pub(crate) fn back_to_files_button(&self) -> gtk::Button {
        self.imp().back_button.get()
    }

    /// Lists the indexed `roots` and the other `candidates` on the Indexed
    /// folders page; a folder typed there starts at `origin`, the folder
    /// shown before Settings.
    pub(crate) fn show_indexed_folders(
        &self,
        candidates: &[IndexCandidate],
        roots: &[IndexRoot],
        origin: Option<&str>,
    ) {
        let imp = self.imp();
        imp.index_origin.replace(origin.map(str::to_owned));
        if let Some(folders) = imp.indexed_folders.get() {
            folders.show(candidates, roots, self);
        }
    }

    /// The folder shown before Settings opened, if any.
    pub(super) fn index_origin(&self) -> Option<String> {
        self.imp().index_origin.borrow().clone()
    }

    /// Lays every page out for a window `width` wide, and the pages built
    /// later too.
    pub(crate) fn fit_to_width(&self, width: PageWidth) {
        self.imp().page_width.set(width);
        self.fit_sections_to_width();
        match width {
            PageWidth::Roomy => self.remove_css_class(NARROW_CLASS),
            PageWidth::Narrow => self.add_css_class(NARROW_CLASS),
        }
    }
}
