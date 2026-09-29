// SPDX-License-Identifier: AGPL-3.0-only
//! The search information strip above the columns while searching.
//!
//! Ports `renderSearchInfo` in `desktop/ui/app.js` and `.search-info` in
//! `desktop/ui/style.css` (SRCH-012): a search glyph; the caption (the
//! error, "Searching…" or where the search looked); the "Search scope"
//! list; "Cache this folder" while the folder is not, or only partly,
//! indexed; for cached results a note on how fresh they are; Dolphin's
//! "Keep Filter When Changing Folders" as a pin toggle (SRCH-005); and a
//! clear button. [`SearchInfoStrip`] is a `GtkBox` subclass laid out by the
//! template `resources/ui/search-info-strip.ui`; the scope list is the
//! app's compact drop-down ([`ChoiceButton`]), whose chevron and check
//! mark are bundled icons.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::report::{SearchReport, FRESHNESS_TOOLTIP};
use super::source::SearchScope;
use crate::icons::{self, Icon};
use crate::settings_page::ChoiceButton;
use crate::window::WindowAction;

/// Emitted when the user chose another scope.
const SCOPE_CHANGED: &str = "scope-changed";
/// Emitted when the user clicked the clear button.
const CLEAR_REQUESTED: &str = "clear-requested";

/// The strip's search glyph (`icon('search',15)`).
const SEARCH_GLYPH: i32 = 15;
/// The glyphs of the strip's buttons.
const BUTTON_GLYPH: i32 = 16;

mod imp {
    use std::cell::OnceCell;
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;

    use super::{CLEAR_REQUESTED, SCOPE_CHANGED};
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
        /// "Cache this folder".
        #[template_child]
        pub(super) cache_button: TemplateChild<gtk::Button>,
        /// The plus before "Cache this folder".
        #[template_child]
        pub(super) cache_glyph: TemplateChild<gtk::Image>,
        /// How fresh cached results are.
        #[template_child]
        pub(super) freshness: TemplateChild<gtk::Label>,
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
                    Signal::builder(CLEAR_REQUESTED).build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            self.obj().finish_template();
        }
    }

    impl WidgetImpl for SearchInfoStrip {}
    impl BoxImpl for SearchInfoStrip {}
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
        imp.freshness.set_tooltip_text(Some(FRESHNESS_TOOLTIP));
        WindowAction::CacheFolder.assign_to(&*imp.cache_button);
        imp.clear_button.connect_clicked(glib::clone!(
            #[weak(rename_to = strip)]
            self,
            move |_| strip.emit_by_name::<()>(CLEAR_REQUESTED, &[])
        ));
        self.add_scope_list();
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
            .update_property(&[gtk::accessible::Property::Label("Search scope")]);
        scope.choices.connect_selected_notify(glib::clone!(
            #[weak(rename_to = strip)]
            self,
            move |_| strip.emit_by_name::<()>(SCOPE_CHANGED, &[])
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
    pub(crate) fn show_report(&self, report: Option<&SearchReport>, scope: SearchScope) {
        let Some(report) = report else {
            self.set_visible(false);
            return;
        };
        let imp = self.imp();
        imp.caption.set_text(report.caption());
        let note = report.freshness_note();
        imp.freshness.set_text(note.unwrap_or_default());
        imp.freshness.set_visible(note.is_some());
        imp.cache_button.set_visible(report.offers_to_cache_folder());
        self.show_scope(scope);
        self.set_visible(true);
    }

    /// Shows `scope` in the list without reporting it as the user's choice.
    fn show_scope(&self, scope: SearchScope) {
        let choices = &self.scope_list().choices;
        let position = SearchScope::ALL.iter().position(|shown| *shown == scope);
        let position = u32::try_from(position.unwrap_or_default()).unwrap_or_default();
        if choices.selected() != position {
            // Set while the window already holds `scope`, so the change
            // it announces is a no-op for the window.
            choices.set_selected(position);
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

    /// Clicks "Clear search", for tests.
    #[cfg(test)]
    pub(crate) fn click_clear(&self) {
        self.imp().clear_button.emit_clicked();
    }
}
