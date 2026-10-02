// SPDX-License-Identifier: AGPL-3.0-only
//! The search information strip above the columns while searching.
//!
//! Ports `renderSearchInfo` in `v2.0.0:desktop/ui/app.js` and `.search-info` in
//! `v2.0.0:desktop/ui/style.css` (SRCH-012): a search glyph; the caption (the
//! error, "Searching…" or where the search looked); the "Search scope"
//! list; the search options Dolphin and Windows offer: file names or
//! names and contents (SRCH-036), kind and date modified (SRCH-037);
//! "Cache this folder" while the folder is not, or only partly,
//! indexed; for cached results a note on how fresh they are; Dolphin's
//! "Save search" (SRCH-038); Dolphin's "Keep Filter When Changing
//! Folders" as a pin toggle (SRCH-005); and a clear button. [`SearchInfoStrip`] is a `GtkBox` subclass laid out by the
//! template `resources/ui/search-info-strip.ui`; the scope list is the
//! app's compact drop-down ([`ChoiceButton`]), whose chevron and check
//! mark are bundled icons.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::{DateFacet, KindFacet, SearchFacets, SearchIn};

use super::report::{SearchReport, FRESHNESS_TOOLTIP};
use super::source::SearchScope;
use crate::icons::{self, Icon};
use crate::settings_page::ChoiceButton;
use crate::window::WindowAction;

/// Emitted when the user chose another scope.
const SCOPE_CHANGED: &str = "scope-changed";
/// Emitted when the user chose another search option.
const OPTIONS_CHANGED: &str = "options-changed";
/// Emitted when the user clicked the clear button.
const CLEAR_REQUESTED: &str = "clear-requested";

/// What a search matches, in the order the list shows them.
const SEARCH_IN: [(SearchIn, &str); 2] = [
    (SearchIn::Names, crate::i18n::message_id("File names")),
    (
        SearchIn::NamesAndContents,
        crate::i18n::message_id("Names and contents"),
    ),
];

/// Why the contents cannot be searched in every cached folder.
const NAMES_ONLY_TOOLTIP: &str = crate::i18n::message_id("The search cache holds names only");

/// The strip's search glyph (`icon('search',15)`).
const SEARCH_GLYPH: i32 = 15;
/// The glyphs of the strip's buttons.
const BUTTON_GLYPH: i32 = 16;

mod imp {
    use std::cell::{Cell, OnceCell};
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;

    use super::{CLEAR_REQUESTED, OPTIONS_CHANGED, SCOPE_CHANGED};
    use crate::settings_page::ChoiceButton;

    /// Private state of [`super::SearchInfoStrip`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/search-info-strip.ui")]
    pub(crate) struct SearchInfoStrip {
        /// The search glyph at the start.
        #[template_child]
        pub(super) glyph: TemplateChild<gtk::Image>,
        /// What was searched, or why it failed.
        #[template_child]
        pub(super) caption: TemplateChild<gtk::Label>,
        /// Holds the scope list.
        #[template_child]
        pub(super) scope_slot: TemplateChild<gtk::Box>,
        /// Holds the search options.
        #[template_child]
        pub(super) options_slot: TemplateChild<gtk::Box>,
        /// "Cache this folder".
        #[template_child]
        pub(super) cache_button: TemplateChild<gtk::Button>,
        /// The plus before "Cache this folder".
        #[template_child]
        pub(super) cache_glyph: TemplateChild<gtk::Image>,
        /// How fresh cached results are.
        #[template_child]
        pub(super) freshness: TemplateChild<gtk::Label>,
        /// "Save search".
        #[template_child]
        pub(super) save_button: TemplateChild<gtk::Button>,
        /// "Keep search when changing folders".
        #[template_child]
        pub(super) keep_button: TemplateChild<gtk::ToggleButton>,
        /// The pin of the keep button.
        #[template_child]
        pub(super) keep_glyph: TemplateChild<gtk::Image>,
        /// "Clear search".
        #[template_child]
        pub(super) clear_button: TemplateChild<gtk::Button>,
        /// The cross of the clear button.
        #[template_child]
        pub(super) clear_glyph: TemplateChild<gtk::Image>,
        /// The scope list, built by `constructed`.
        pub(super) scope: OnceCell<ChoiceButton>,
        /// File names, or names and contents; built by `constructed`.
        pub(super) search_in: OnceCell<ChoiceButton>,
        /// The kind of item shown; built by `constructed`.
        pub(super) kind: OnceCell<ChoiceButton>,
        /// When the items shown were modified; built by `constructed`.
        pub(super) date: OnceCell<ChoiceButton>,
        /// Set while the strip shows the window's choices in its lists,
        /// so the lists' changes are not reported as the user's.
        pub(super) is_showing: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchInfoStrip {
        const NAME: &'static str = "OxSearchInfoStrip";
        type Type = super::SearchInfoStrip;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(strip: &glib::subclass::InitializingObject<Self>) {
            strip.init_template();
        }
    }

    impl ObjectImpl for SearchInfoStrip {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder(SCOPE_CHANGED).build(),
                    Signal::builder(OPTIONS_CHANGED).build(),
                    Signal::builder(CLEAR_REQUESTED).build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "search-info-strip.ui");
            self.obj().finish_template();
        }
    }

    impl WidgetImpl for SearchInfoStrip {}
    impl BoxImpl for SearchInfoStrip {}
}

/// The choices the strip shows while searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ShownOptions {
    /// Where the search looks.
    pub scope: SearchScope,
    /// Names, or names and contents.
    pub search_in: SearchIn,
    /// The kind and date options.
    pub facets: SearchFacets,
}

glib::wrapper! {
    /// The strip that says what a search looked at, above the columns.
    pub(crate) struct SearchInfoStrip(ObjectSubclass<imp::SearchInfoStrip>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl SearchInfoStrip {
    /// Gives the template its glyphs, the scope list and the buttons'
    /// work.
    fn finish_template(&self) {
        let imp = self.imp();
        icons::set_icon(&imp.glyph, Icon::Search, SEARCH_GLYPH);
        icons::set_icon(&imp.cache_glyph, Icon::Add, BUTTON_GLYPH);
        icons::set_icon(&imp.keep_glyph, Icon::Pin, BUTTON_GLYPH);
        icons::set_icon(&imp.clear_glyph, Icon::Dismiss16, BUTTON_GLYPH);
        imp.freshness
            .set_tooltip_text(Some(ox_core::i18n::gettext_static(FRESHNESS_TOOLTIP)));
        WindowAction::CacheFolder.assign_to(&*imp.cache_button);
        WindowAction::SaveSearch.assign_to(&*imp.save_button);
        imp.clear_button.connect_clicked(glib::clone!(
            #[weak(rename_to = strip)]
            self,
            move |_| strip.emit_by_name::<()>(CLEAR_REQUESTED, &[])
        ));
        self.add_scope_list();
        self.add_options();
    }

    /// The search options, which report the user's choices.
    fn add_options(&self) {
        let imp = self.imp();
        let search_in = SEARCH_IN.map(|(_, label)| ox_core::i18n::gettext_static(label));
        let kinds = KindFacet::ALL.map(KindFacet::label);
        let dates = DateFacet::ALL.map(DateFacet::label);
        let lists = [
            (
                &imp.search_in,
                &search_in[..],
                ox_core::i18n::gettext_static("Search in"),
            ),
            (&imp.kind, &kinds[..], ox_core::i18n::gettext_static("Kind")),
            (
                &imp.date,
                &dates[..],
                ox_core::i18n::gettext_static("Date modified"),
            ),
        ];
        for (slot, labels, name) in lists {
            let labels: Vec<String> = labels.iter().map(|label| (*label).to_owned()).collect();
            let list = ChoiceButton::new(&labels);
            list.button
                .update_property(&[gtk::accessible::Property::Label(name)]);
            list.button.set_tooltip_text(Some(name));
            list.choices.connect_selected_notify(glib::clone!(
                #[weak(rename_to = strip)]
                self,
                move |_| strip.report_choice(OPTIONS_CHANGED)
            ));
            imp.options_slot.append(&list.button);
            slot.set(list).expect("constructed adds the search options once");
        }
    }

    /// Emits `signal` for a list the user changed; a list the strip set
    /// while showing the window's choices reports nothing, because the
    /// window already holds them and may be busy showing them.
    fn report_choice(&self, signal: &str) {
        if !self.imp().is_showing.get() {
            self.emit_by_name::<()>(signal, &[]);
        }
    }

    /// Runs `show` without reporting the lists it changes.
    fn showing(&self, show: impl FnOnce()) {
        let is_showing = &self.imp().is_showing;
        let was_showing = is_showing.replace(true);
        show();
        is_showing.set(was_showing);
    }

    /// The option list in `slot`.
    fn option_list(slot: &std::cell::OnceCell<ChoiceButton>) -> &ChoiceButton {
        slot.get().expect("constructed adds the search options")
    }

    /// Whether names or names and contents are searched, as chosen.
    pub(crate) fn search_in(&self) -> SearchIn {
        let position = Self::option_list(&self.imp().search_in).choices.selected() as usize;
        SEARCH_IN
            .get(position)
            .map(|(search_in, _)| *search_in)
            .unwrap_or_default()
    }

    /// The kind and date options, as chosen.
    pub(crate) fn facets(&self) -> SearchFacets {
        let imp = self.imp();
        let kind = Self::option_list(&imp.kind).choices.selected() as usize;
        let date = Self::option_list(&imp.date).choices.selected() as usize;
        SearchFacets {
            kind: KindFacet::ALL.get(kind).copied().unwrap_or_default(),
            date: DateFacet::ALL.get(date).copied().unwrap_or_default(),
        }
    }

    /// Shows `search_in` and `facets` in the lists without reporting them
    /// as the user's choice. Contents cannot be searched in `scope` every
    /// cached folder, whose cache holds names only, so the list shows
    /// "File names" there.
    fn show_options(&self, search_in: SearchIn, facets: SearchFacets, scope: SearchScope) {
        let imp = self.imp();
        let names_only = scope == SearchScope::AllCachedFolders;
        let search_in = if names_only { SearchIn::Names } else { search_in };
        let search_in_position = SEARCH_IN.iter().position(|(shown, _)| *shown == search_in);
        let kind = KindFacet::ALL.iter().position(|shown| *shown == facets.kind);
        let date = DateFacet::ALL.iter().position(|shown| *shown == facets.date);
        let shown = [
            (&imp.search_in, search_in_position),
            (&imp.kind, kind),
            (&imp.date, date),
        ];
        self.showing(|| {
            for (slot, position) in shown {
                let choices = &Self::option_list(slot).choices;
                let position = u32::try_from(position.unwrap_or_default()).unwrap_or_default();
                if choices.selected() != position {
                    choices.set_selected(position);
                }
            }
        });
        let search_in_button = &Self::option_list(&imp.search_in).button;
        search_in_button.set_sensitive(!names_only);
        search_in_button.set_tooltip_text(Some(if names_only {
            ox_core::i18n::gettext_static(NAMES_ONLY_TOOLTIP)
        } else {
            ox_core::i18n::gettext_static("Search in")
        }));
    }

    /// The "Search scope" list, which reports the user's choice.
    fn add_scope_list(&self) {
        let labels: Vec<String> = SearchScope::ALL
            .iter()
            .map(|scope| scope.label().to_owned())
            .collect();
        let scope = ChoiceButton::new(&labels);
        scope
            .button
            .update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
                "Search scope",
            ))]);
        scope.choices.connect_selected_notify(glib::clone!(
            #[weak(rename_to = strip)]
            self,
            move |_| strip.report_choice(SCOPE_CHANGED)
        ));
        let imp = self.imp();
        imp.scope_slot.append(&scope.button);
        imp.scope
            .set(scope)
            .expect("constructed adds the scope list once");
    }

    fn scope_list(&self) -> &ChoiceButton {
        self.imp().scope.get().expect("constructed adds the scope list")
    }

    /// The scope the list shows.
    pub(crate) fn scope(&self) -> SearchScope {
        let position = self.scope_list().choices.selected() as usize;
        SearchScope::ALL.get(position).copied().unwrap_or_default()
    }

    /// Shows `report`, with `scope` chosen, or hides the strip while
    /// nothing is searched.
    pub(crate) fn show_report(&self, report: Option<&SearchReport>, options: ShownOptions) {
        let Some(report) = report else {
            // The lists start from their defaults at the next search.
            self.show_scope(SearchScope::default());
            self.show_options(options.search_in, SearchFacets::default(), SearchScope::default());
            self.set_visible(false);
            return;
        };
        let imp = self.imp();
        imp.caption.set_text(report.caption());
        let note = report.freshness_note();
        imp.freshness.set_text(note.unwrap_or_default());
        imp.freshness.set_visible(note.is_some());
        imp.cache_button.set_visible(report.offers_to_cache_folder());
        self.show_scope(options.scope);
        self.show_options(options.search_in, options.facets, options.scope);
        self.set_visible(true);
    }

    /// Shows `scope` in the list without reporting it as the user's choice.
    fn show_scope(&self, scope: SearchScope) {
        let choices = &self.scope_list().choices;
        let position = SearchScope::ALL.iter().position(|shown| *shown == scope);
        let position = u32::try_from(position.unwrap_or_default()).unwrap_or_default();
        if choices.selected() != position {
            self.showing(|| choices.set_selected(position));
        }
    }

    /// Whether the search stays when the tab opens another folder
    /// (SRCH-005); off until the user presses the pin.
    pub(crate) fn keeps_search(&self) -> bool {
        self.imp().keep_button.is_active()
    }

    /// Presses or releases the pin, as the user would, for tests.
    #[cfg(test)]
    pub(crate) fn set_keeps_search(&self, keeps: bool) {
        self.imp().keep_button.set_active(keeps);
    }

    /// Calls `callback` when the user chose another scope.
    pub(crate) fn connect_scope_changed(
        &self,
        callback: impl Fn(SearchScope) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_local(SCOPE_CHANGED, false, move |values| {
            let strip = values
                .first()
                .and_then(|value| value.get::<SearchInfoStrip>().ok());
            if let Some(strip) = strip {
                callback(strip.scope());
            }
            None
        })
    }

    /// Calls `callback` when the user chose another search option, with
    /// what is searched and the kind and date options now chosen.
    pub(crate) fn connect_options_changed(
        &self,
        callback: impl Fn(SearchIn, SearchFacets) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_local(OPTIONS_CHANGED, false, move |values| {
            let strip = values
                .first()
                .and_then(|value| value.get::<SearchInfoStrip>().ok());
            if let Some(strip) = strip {
                callback(strip.search_in(), strip.facets());
            }
            None
        })
    }

    /// Calls `callback` when the user clicked "Clear search".
    pub(crate) fn connect_clear_requested(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(CLEAR_REQUESTED, false, move |_| {
            callback();
            None
        })
    }

    /// The caption shown, for tests.
    #[cfg(test)]
    pub(crate) fn caption(&self) -> String {
        self.imp().caption.text().into()
    }

    /// The note shown on cached results, for tests.
    #[cfg(test)]
    pub(crate) fn shown_note(&self) -> Option<String> {
        let freshness = &self.imp().freshness;
        freshness.is_visible().then(|| freshness.text().into())
    }

    /// Whether "Cache this folder" is offered, for tests.
    #[cfg(test)]
    pub(crate) fn offers_to_cache_folder(&self) -> bool {
        self.imp().cache_button.is_visible()
    }

    /// Chooses `scope` as the user would, for tests.
    #[cfg(test)]
    pub(crate) fn choose_scope(&self, scope: SearchScope) {
        self.scope_list().choices.choose_labelled(scope.label());
    }

    /// Chooses what is searched as the user would, for tests.
    #[cfg(test)]
    pub(crate) fn choose_search_in(&self, search_in: SearchIn) {
        let label = SEARCH_IN
            .iter()
            .find(|(shown, _)| *shown == search_in)
            .map(|(_, label)| *label);
        Self::option_list(&self.imp().search_in)
            .choices
            .choose_labelled(label.unwrap_or_default());
    }

    /// Chooses a kind as the user would, for tests.
    #[cfg(test)]
    pub(crate) fn choose_kind(&self, kind: KindFacet) {
        Self::option_list(&self.imp().kind)
            .choices
            .choose_labelled(kind.label());
    }

    /// Clicks "Save search", for tests.
    #[cfg(test)]
    pub(crate) fn click_save(&self) {
        self.imp().save_button.emit_clicked();
    }

    /// Clicks "Clear search", for tests.
    #[cfg(test)]
    pub(crate) fn click_clear(&self) {
        self.imp().clear_button.emit_clicked();
    }
}
