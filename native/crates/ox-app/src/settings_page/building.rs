// SPDX-License-Identifier: AGPL-3.0-only
//! Building the Settings page's categories, sub-pages and list, the first
//! time Settings is shown.
//!
//! The Python app rendered its settings page each time Settings opened
//! (`renderSettingsPage` in `v2.0.0:desktop/ui/app.js`). Every native window
//! holds a Settings page, and most never show it, so a window builds its
//! eight categories, their sub-pages and some sixty rows only when
//! Settings is first shown or opened, rather than when the window is
//! created.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::pages::{Category, SettingsView, Subpage};
use super::section::SettingsSection;
use super::SettingsPage;
use super::{
    about, appearance, archives, confirmations, default_apps, files_folders, general, indexed_folders,
    indexing, troubleshooting,
};

impl SettingsPage {
    /// Builds every category, sub-page and the category list, the first
    /// time it is called, laid out for the width the page last took.
    pub(super) fn build_pages_once(&self) {
        let imp = self.imp();
        if imp.is_built.replace(true) {
            return;
        }
        self.add_category_sections();
        self.add_subpages();
        self.build_navigation();
        self.fit_sections_to_width();
        self.show_view(SettingsView::default());
    }

    fn add_category_sections(&self) {
        // The Default apps page builds the ZIP route's group, which the
        // ZIP & archives page shows.
        let (default_apps, zip_files) = default_apps::build(self);
        for category in Category::ALL {
            let section = match category {
                Category::General => general::build(self),
                Category::Appearance => appearance::build(self),
                Category::FilesAndFolders => files_folders::build(self),
                Category::Archives => archives::build(self, &zip_files),
                Category::Confirmations => confirmations::build(self),
                Category::Search => indexing::build(self),
                Category::DefaultApps => default_apps.clone(),
                Category::About => about::build(self),
            };
            self.add_page(category, &section);
            self.imp()
                .category_sections
                .borrow_mut()
                .insert(category, section);
        }
    }

    fn add_subpages(&self) {
        let (indexed_page, folders) = indexed_folders::build(self);
        self.imp()
            .indexed_folders
            .set(folders)
            .expect("the sub-pages are built once");
        let subpages = [
            (Subpage::IndexedFolders, indexed_page),
            (Subpage::FolderSizes, files_folders::build_folder_sizes()),
            (Subpage::Troubleshooting, troubleshooting::build()),
        ];
        for (subpage, section) in subpages {
            if let Some(back) = section.back_button() {
                back.connect_clicked(glib::clone!(
                    #[weak(rename_to = settings)]
                    self,
                    move |_| settings.show_view(SettingsView::Category(subpage.category()))
                ));
            }
            self.add_named_page(subpage.as_str(), &section);
            self.imp().subpages.borrow_mut().insert(subpage, section);
        }
    }

    /// Adds `section`, the page of `category`, to the stack, and keeps
    /// the box that holds it: a search shows every category's section on
    /// one page of results, and puts each back here when it ends.
    fn add_page(&self, category: Category, section: &SettingsSection) {
        let content = self.add_named_page(category.as_str(), section);
        self.imp().category_hosts.borrow_mut().insert(category, content);
    }

    /// Adds `section` to the stack as `name`, scrolling on its own, and
    /// returns the box that holds it.
    fn add_named_page(&self, name: &str, section: &SettingsSection) -> gtk::Box {
        let content = gtk::Box::builder().css_classes(["settings-content"]).build();
        content.append(section);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        self.imp().pages.add_named(&scrolled, Some(name));
        content
    }

    /// Lays every section out for the width the page last took.
    pub(super) fn fit_sections_to_width(&self) {
        let width = self.imp().page_width.get();
        for section in self.all_sections() {
            section.fit_to_width(width);
        }
    }

    /// Every category's page and every sub-page built so far.
    fn all_sections(&self) -> Vec<SettingsSection> {
        let imp = self.imp();
        let categories = imp.category_sections.borrow();
        let subpages = imp.subpages.borrow();
        categories.values().chain(subpages.values()).cloned().collect()
    }
}
