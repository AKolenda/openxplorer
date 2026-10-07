// SPDX-License-Identifier: AGPL-3.0-only
//! The left side of Settings, and which page the right side shows: the
//! category list, the settings search and the keyboard.
//!
//! Ports `settingsSearch` and the section links of `renderSettingsPage` in
//! `v2.0.0:desktop/ui/app.js`. The search filters the rows of every category at
//! once and shows the matches of all of them on one page of results, each
//! category's under its name, as the settings mockup does. The category
//! list hides while a search is typed, so nobody has to look for matches
//! category by category; "N matching settings" shows under the search box
//! instead. Enter jumps to the first match, Escape leaves the
//! search, and arrow keys move through the categories. Escape on a
//! sub-page goes back to its category.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::category_row::CategoryRow;
use super::pages::{Category, SettingsView};
use super::search::{match_count_text, SearchQuery};
use super::section::SettingsSection;
use super::SettingsPage;
use crate::window::children;

/// The name of the page shown when no setting matches the search.
const NO_MATCHES_PAGE: &str = "no-matches";

/// The name of the page of search results.
const RESULTS_PAGE: &str = "search-results";

/// Room kept above a row the search jumps to, in pixels.
const JUMP_MARGIN: f64 = 24.0;

impl SettingsPage {
    /// Fills the category list, and connects the list, the search box and
    /// the Escape key.
    pub(super) fn build_navigation(&self) {
        let imp = self.imp();
        for category in Category::ALL {
            imp.category_list.append(&CategoryRow::new(category));
        }
        let no_matches = gtk::Label::builder()
            .label(ox_core::i18n::gettext("No settings match your search."))
            .valign(gtk::Align::Start)
            .css_classes(["settings-no-matches"])
            .build();
        imp.pages.add_named(&no_matches, Some(NO_MATCHES_PAGE));
        let results = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["settings-content", "settings-results"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&results)
            .build();
        imp.pages.add_named(&scrolled, Some(RESULTS_PAGE));
        imp.results.set(results).expect("the navigation is built once");
        self.connect_category_list();
        self.connect_search_entry();
        self.go_back_on_escape();
    }

    fn connect_category_list(&self) {
        let list = &self.imp().category_list;
        list.connect_row_selected(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_, row| {
                let Some(category) = row.and_then(category_of) else {
                    return;
                };
                // Selecting the row of the category shown already, as a
                // sub-page does, keeps the page.
                if page.view().category() != category {
                    page.show_view(SettingsView::Category(category));
                }
            }
        ));
        // Enter on a category moves into its page.
        list.connect_row_activated(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_, _| page.focus_page()
        ));
        list.set_filter_func(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            true,
            move |row| page.lists_category(row)
        ));
    }

    fn connect_search_entry(&self) {
        let entry = &self.imp().search_entry;
        entry.connect_changed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |entry| page.apply_search(&entry.text())
        ));
        entry.connect_activate(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| {
                page.jump_to_first_match();
            }
        ));
        // Escape in the search box.
        entry.connect_stop_search(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.leave_search()
        ));
    }

    /// Escape on a sub-page returns to its category; elsewhere on the page
    /// it ends a search.
    fn go_back_on_escape(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if key != gdk::Key::Escape || !modifiers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                page.step_back()
            }
        ));
        self.add_controller(keys);
    }

    /// Leaves a sub-page or the search, whichever is open.
    pub(super) fn step_back(&self) -> glib::Propagation {
        if let SettingsView::Subpage(subpage) = self.view() {
            self.show_view(SettingsView::Category(subpage.category()));
            self.focus_chosen_category();
            return glib::Propagation::Stop;
        }
        if self.imp().query.borrow().is_empty() {
            return glib::Propagation::Proceed;
        }
        self.leave_search();
        glib::Propagation::Stop
    }

    /// What the right side shows.
    pub(crate) fn view(&self) -> SettingsView {
        self.imp().view.get()
    }

    /// Shows `view` on the right and highlights its category on the left.
    /// While a search is typed, a category is shown as its matches among
    /// the results.
    pub(super) fn show_view(&self, view: SettingsView) {
        let imp = self.imp();
        imp.view.set(view);
        match view {
            SettingsView::Category(category) if self.is_searching() => {
                imp.pages.set_visible_child_name(RESULTS_PAGE);
                self.scroll_to(self.category_section(category).upcast_ref());
            }
            _ => imp.pages.set_visible_child_name(view.as_str()),
        }
        let row = self.category_list_row(view.category());
        if let Some(row) = row.filter(|row| !row.is_selected()) {
            imp.category_list.select_row(Some(&row));
        }
    }

    /// Whether a search is typed.
    fn is_searching(&self) -> bool {
        !self.imp().query.borrow().is_empty()
    }

    /// Puts every category's section on the page of results while
    /// `searching`, each under its title, and back on its own page when
    /// the search ends.
    fn gather_results(&self, searching: bool) {
        let imp = self.imp();
        let Some(results) = imp.results.get() else {
            return;
        };
        let gathered = results.first_child().is_some();
        if searching == gathered {
            return;
        }
        let hosts = imp.category_hosts.borrow();
        for category in Category::ALL {
            let section = self.category_section(category);
            let Some(host) = hosts.get(&category) else {
                continue;
            };
            let (from, to) = if searching {
                (host, results)
            } else {
                (results, host)
            };
            from.remove(&section);
            to.append(&section);
            section.set_searching(searching);
            section.set_visible(true);
        }
    }

    /// The page of `category`.
    ///
    /// # Panics
    ///
    /// Before the pages are built, the first time Settings is shown or
    /// opened ([`SettingsPage::build_pages_once`]).
    pub(super) fn category_section(&self, category: Category) -> SettingsSection {
        self.imp()
            .category_sections
            .borrow()
            .get(&category)
            .cloned()
            .expect("the pages are built before a category is asked for")
    }

    /// Types `text` into the settings search, as the user would.
    pub(crate) fn search(&self, text: &str) {
        self.build_pages_once();
        self.imp().search_entry.set_text(text);
    }

    /// Moves keyboard focus into the settings search (Ctrl+F on the page).
    pub(crate) fn focus_search(&self) {
        self.imp().search_entry.grab_focus();
    }

    /// Filters every category's rows by `typed`, and shows the categories
    /// with matches, their counts and the first page that has any.
    fn apply_search(&self, typed: &str) {
        let imp = self.imp();
        let query = SearchQuery::parse(typed);
        self.gather_results(!query.is_empty());
        let mut total = 0;
        for category_row in self.category_rows() {
            let section = self.category_section(category_row.category());
            let matches = section.apply_query(&query);
            category_row.show_matches(matches, &query);
            // Among the results, a category without matches leaves out
            // its title too.
            section.set_visible(query.is_empty() || matches > 0);
            total += matches;
        }
        imp.match_count.set_text(&match_count_text(total));
        imp.match_count.set_visible(!query.is_empty());
        // Every match shows on one page of results, so the categories
        // are not offered while searching.
        imp.category_list.set_visible(query.is_empty());
        imp.query.replace(query);
        imp.category_list.invalidate_filter();
        self.show_search_results();
    }

    /// Shows the results from the top, or says that nothing matches. The
    /// page the user was on stays chosen, so ending the search returns to
    /// it.
    fn show_search_results(&self) {
        let imp = self.imp();
        if !self.is_searching() {
            imp.pages.set_visible_child_name(self.view().as_str());
            return;
        }
        let any_match = Category::ALL
            .into_iter()
            .any(|category| self.matches_in(category) > 0);
        if !any_match {
            imp.pages.set_visible_child_name(NO_MATCHES_PAGE);
            return;
        }
        imp.pages.set_visible_child_name(RESULTS_PAGE);
        if let Some(scrolled) = self.shown_scroller() {
            scrolled.vadjustment().set_value(0.0);
        }
    }

    /// The scrolled page the right side shows now.
    fn shown_scroller(&self) -> Option<gtk::ScrolledWindow> {
        self.imp()
            .pages
            .visible_child()
            .and_downcast::<gtk::ScrolledWindow>()
    }

    /// How many rows of `category` match the search typed now.
    fn matches_in(&self, category: Category) -> usize {
        let row = self.category_list_row(category);
        row.map_or(0, |row| row.matches())
    }

    /// The rows of the category list, top to bottom.
    pub(super) fn category_rows(&self) -> Vec<CategoryRow> {
        let rows = children(&*self.imp().category_list);
        rows.filter_map(|row| row.downcast::<CategoryRow>().ok())
            .collect()
    }

    /// The list row of `category`.
    pub(super) fn category_list_row(&self, category: Category) -> Option<CategoryRow> {
        let rows = self.category_rows();
        rows.into_iter().find(|row| row.category() == category)
    }

    /// Whether the list shows `row`: always, or while searching only when
    /// its category has matches.
    fn lists_category(&self, row: &gtk::ListBoxRow) -> bool {
        if self.imp().query.borrow().is_empty() {
            return true;
        }
        category_of(row).is_some_and(|category| self.matches_in(category) > 0)
    }

    /// Shows the first setting, a status card or a row, that matches the
    /// search and gives its control keyboard focus, as Enter in the Python
    /// app's search clicked the first result. False when nothing matches.
    pub(super) fn jump_to_first_match(&self) -> bool {
        if self.imp().query.borrow().is_empty() {
            return false;
        }
        let first_match = Category::ALL.into_iter().find_map(|category| {
            let target = self.category_section(category).first_match()?;
            Some((category, target))
        });
        let Some((category, target)) = first_match else {
            return false;
        };
        self.show_view(SettingsView::Category(category));
        target.jump_here();
        self.scroll_to(target.widget());
        true
    }

    /// Scrolls the shown page so `setting` is near its top, once the page
    /// has been laid out.
    fn scroll_to(&self, setting: &gtk::Widget) {
        let Some(scrolled) = self.shown_scroller() else {
            return;
        };
        let setting = setting.downgrade();
        glib::idle_add_local_once(move || {
            let content = scrolled.child().and_then(|viewport| viewport.first_child());
            let (Some(setting), Some(content)) = (setting.upgrade(), content) else {
                return;
            };
            let top = setting.compute_point(&content, &graphene::Point::zero());
            if let Some(top) = top {
                scrolled.vadjustment().set_value(f64::from(top.y()) - JUMP_MARGIN);
            }
        });
    }

    /// Empties the search, which shows every row again, and puts keyboard
    /// focus back on the category list.
    fn leave_search(&self) {
        self.imp().search_entry.set_text("");
        self.focus_chosen_category();
    }

    /// Gives the chosen category's row in the list keyboard focus.
    pub(super) fn focus_chosen_category(&self) {
        if let Some(row) = self.imp().category_list.selected_row() {
            row.grab_focus();
        }
    }

    /// Moves keyboard focus to the first control of the page shown.
    fn focus_page(&self) {
        if let Some(page) = self.imp().pages.visible_child() {
            page.child_focus(gtk::DirectionType::TabForward);
        }
    }
}

/// The category a row of the category list stands for.
fn category_of(row: &gtk::ListBoxRow) -> Option<Category> {
    row.downcast_ref::<CategoryRow>().map(CategoryRow::category)
}
