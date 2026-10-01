// SPDX-License-Identifier: AGPL-3.0-only
//! Building the Settings page's categories, sub-pages and list, the first
//! time Settings is shown.
//!
//! The Python app rendered its settings page each time Settings opened
//! (`renderSettingsPage` in `v2.0.0:desktop/ui/app.js`). Every native window
//! holds a Settings page, and most never show it, so a window builds its
//! six categories, their sub-pages and some forty rows only when Settings
//! is first shown or opened, rather than when the window is created.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::pages::{Category, SettingsView, Subpage};
use super::section::SettingsSection;
use super::SettingsPage;
use super::{
    about, appearance, brave, default_apps, indexed_folders, indexing, troubleshooting, windows_tabs,
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
        for category in Category::ALL {
            let section = self.build_category(category);
            self.add_page(category.as_str(), &section);
            self.imp()
                .category_sections
                .borrow_mut()
                .insert(category, section);
        }
    }

    /// The page of `category`, from the category's own module.
    fn build_category(&self, category: Category) -> SettingsSection {
        match category {
            Category::Appearance => appearance::build(self),
            Category::SearchAndIndexing => indexing::build(self),
            Category::DefaultApps => default_apps::build(self),
            Category::WindowsAndTabs => windows_tabs::build(self),
            Category::BraveAndDownloads => brave::build(self),
            Category::About => about::build(self),
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
            (Subpage::FolderSizes, indexing::build_folder_sizes()),
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
            self.add_page(subpage.as_str(), &section);
            self.imp().subpages.borrow_mut().insert(subpage, section);
        }
    }

    /// Adds `section` to the stack as `name`, scrolling on its own.
    fn add_page(&self, name: &str, section: &SettingsSection) {
        let content = gtk::Box::builder().css_classes(["settings-content"]).build();
        content.append(section);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        self.imp().pages.add_named(&scrolled, Some(name));
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
