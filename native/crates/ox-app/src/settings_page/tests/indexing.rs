// SPDX-License-Identifier: AGPL-3.0-only
//! GTK tests of Search & indexing with the search cache running: the
//! status card, the Indexed folders page, its table of folders to add, the
//! path field and the options the index service follows.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::{PinIndexing, RootOrigin, RootStatus};
use ox_core::settings::BookmarkRequest;

use super::{switch_of, SettingsTest};
use crate::settings_page::indexed_folders::IndexedFolders;
use crate::settings_page::pages::{Category, SettingsView, Subpage};
use crate::settings_page::section::SettingsSection;
use crate::settings_page::status_card::StatusCard;
use crate::settings_store::Change;
use crate::test_support::harness::{descendants, wait_for, wait_until};

impl SettingsTest {
    /// Settings on the standard fixture, with the search cache running.
    fn with_search_cache() -> Self {
        let settings = Self::open();
        settings.test.start_search_cache();
        settings
    }

    /// The lists of the Indexed folders page.
    fn indexed_folders(&self) -> &IndexedFolders {
        self.page
            .imp()
            .indexed_folders
            .get()
            .expect("the page is built when Settings opens")
    }

    /// The Indexed folders page.
    fn indexed_folders_page(&self) -> SettingsSection {
        self.page.imp().subpages.borrow()[&Subpage::IndexedFolders].clone()
    }

    /// The row of the indexed folder labelled `label`.
    fn indexed_row(&self, label: &str) -> gtk::Box {
        let rows = descendants::<gtk::Box>(&self.indexed_folders_page());
        let mut indexed = rows.into_iter().filter(|row| row.has_css_class("indexed-folder"));
        indexed
            .find(|row| labels_in(row).iter().any(|text| text == label))
            .unwrap_or_else(|| panic!("{label} is listed as indexed"))
    }

    /// Clicks the button of `label`'s row whose tooltip is `action`.
    fn click_folder_action(&self, label: &str, action: &str) {
        let row = self.indexed_row(label);
        let buttons = descendants::<gtk::Button>(&row);
        let button = buttons
            .into_iter()
            .find(|button| button.tooltip_text().as_deref() == Some(action))
            .unwrap_or_else(|| panic!("{label} offers {action}"));
        button.emit_clicked();
    }

    /// Pins `uri` as `label`, as another window would.
    fn pin(&self, uri: &str, label: &str) {
        let request = BookmarkRequest::new(uri, label);
        let change: Change = Box::new(move |settings| settings.pin_many(&[request], None, None).map(|_| ()));
        self.test.context.change_settings(change, |result| {
            result.expect("the settings file takes the pin");
        });
    }
}

/// The texts of every label in `widget`.
fn labels_in(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    let labels = descendants::<gtk::Label>(widget);
    labels.iter().map(|label| label.text().to_string()).collect()
}

/// The text of the label with `class` in `widget`.
fn label_with_class(widget: &impl IsA<gtk::Widget>, class: &str) -> String {
    let labels = descendants::<gtk::Label>(widget);
    let label = labels.into_iter().find(|label| label.has_css_class(class));
    label.map(|label| label.text().to_string()).unwrap_or_default()
}

/// Ported from the enabled rows of `renderSettingsCache` in
/// `v2.0.0:desktop/ui/app.js`: an indexed folder shows its state and how many
/// names it holds, and is no longer offered to add.
///
/// parity: SET-006, SRCH-022
#[gtk::test]
fn an_indexed_folder_shows_its_state_and_leaves_the_suggestions() {
    let settings = SettingsTest::with_search_cache();
    let folder = settings.fixture.uri();

    settings.test.index_folder(&folder);

    wait_until("the page to list the folder", || {
        settings
            .indexed_folders()
            .shown_labels()
            .first()
            .map(String::as_str)
            == Some("Indexed")
    });
    let row = settings.indexed_row("Indexed");
    assert_eq!(label_with_class(&row, "status-chip"), "Ready");
    assert_eq!(
        label_with_class(&row, "setting-value"),
        "4 names · Live local events"
    );
    let suggested = settings.indexed_folders().suggestions().listed_labels();
    assert!(
        !suggested.contains(&"Example projects".to_owned()),
        "{suggested:?}"
    );
    let card = descendants::<StatusCard>(&settings.page.category_section(Category::Search));
    assert_eq!(card[0].title(), "Instant search is on for 1 folder");
    settings
        .page
        .show_view(SettingsView::Subpage(Subpage::IndexedFolders));
    crate::test_support::harness::capture(&settings.test.window, "settings-indexed-folders.png");
}

/// Ported from the Add button of `renderSettingsPage` in
/// `v2.0.0:desktop/ui/app.js`: the typed folder is read relative to the folder
/// shown before Settings, indexed, and the field empties.
///
/// parity: SET-007, SRCH-019
#[gtk::test]
fn a_typed_folder_is_indexed_relative_to_the_folder_shown_before() {
    let settings = SettingsTest::with_search_cache();
    let field = descendants::<gtk::Entry>(&settings.indexed_folders_page())
        .into_iter()
        .next()
        .expect("the page has the Add a folder field");

    field.set_text("Documents");
    field.emit_activate();

    settings
        .test
        .wait_for_root(&settings.fixture.uri_of("Documents"), RootStatus::Ready);
    assert_eq!(field.text().as_str(), "");
}

/// The power-user table indexes every checked folder at once.
///
/// parity: SET-006
#[gtk::test]
fn index_selected_indexes_every_checked_folder() {
    let settings = SettingsTest::with_search_cache();
    let table = settings.indexed_folders().suggestions();
    let origin = settings.fixture.uri();
    wait_until("the folder shown before to be offered", || {
        table.listed_labels().first().map(String::as_str) == Some("Example projects")
    });

    table.check("Example projects");
    table.click_index_selected();

    settings.test.wait_for_root(&origin, RootStatus::Ready);
    wait_until("the table to drop the indexed folder", || {
        !table.listed_labels().contains(&"Example projects".to_owned())
    });
}

/// Clear keeps the folder but deletes its names; "Remove from index"
/// stops indexing it and offers it to add again.
///
/// parity: SRCH-019, SRCH-023
#[gtk::test]
fn clearing_and_removing_act_on_one_folder() {
    let settings = SettingsTest::with_search_cache();
    let folder = settings.fixture.uri();
    settings.test.index_folder(&folder);
    wait_until("the row to show", || {
        !settings.indexed_folders().shown_labels().is_empty()
    });

    settings.click_folder_action("Indexed", "Clear cached names only");

    wait_until("the names to be cleared", || {
        settings
            .test
            .find_root(&folder)
            .is_some_and(|root| root.entry_count == 0)
    });

    settings.click_folder_action("Indexed", "Remove from index");

    wait_until("the folder to stop being indexed", || {
        settings
            .test
            .find_root(&folder)
            .is_some_and(|root| !root.is_enabled())
    });
    let suggested = settings.indexed_folders().suggestions().listed_labels();
    assert!(
        suggested.contains(&"Example projects".to_owned()),
        "{suggested:?}"
    );
}

/// With watching off nothing updates by itself, and "Refresh all" picks
/// the new file up.
///
/// parity: SET-008, SRCH-023, SRCH-026
#[gtk::test]
fn refresh_all_rescans_while_watching_is_off() {
    let settings = SettingsTest::with_search_cache();
    let folder = settings.fixture.uri();
    settings.test.index_folder(&folder);
    switch_of(&settings.row("Watch folders for changes")).set_active(false);
    wait_until("the service to pause", || {
        settings
            .test
            .find_root(&folder)
            .is_some_and(|root| root.update_mode.as_str() == "Paused")
    });

    settings.fixture.write("added later.txt");
    wait_for(Duration::from_millis(1500));
    let before = settings.test.find_root(&folder).map(|root| root.entry_count);
    assert_eq!(before, Some(4), "a paused index does not watch");
    let card = descendants::<StatusCard>(&settings.page.category_section(Category::Search));
    let refresh_all = descendants::<gtk::Button>(&card[0])
        .into_iter()
        .next()
        .expect("Refresh all");
    refresh_all.emit_clicked();

    wait_until("the rescan to find the new file", || {
        settings
            .test
            .find_root(&folder)
            .is_some_and(|root| root.entry_count == 5)
    });
}

/// parity: SRCH-040
#[gtk::test]
fn the_pinned_folders_switch_removes_what_pinning_added() {
    let settings = SettingsTest::with_search_cache();
    let documents = settings.fixture.uri_of("Documents");
    settings.pin(&documents, "Team files");
    wait_until("the pin to be indexed", || {
        settings
            .test
            .find_root(&documents)
            .is_some_and(|root| root.origin == RootOrigin::Pin)
    });
    wait_until("the page to list it", || {
        settings
            .indexed_folders()
            .shown_labels()
            .contains(&"Team files".to_owned())
    });
    let row = settings.indexed_row("Team files");
    let tags = descendants::<gtk::Box>(&row);
    assert!(
        tags.iter().any(|tag| tag.has_css_class("pinned-tag")),
        "the row has a Pinned tag"
    );

    switch_of(&settings.row("Index pinned folders automatically")).set_active(false);

    wait_until("the pinned folder to leave the index", || {
        settings.test.find_root(&documents).is_none()
    });
    let cache = settings.test.context.search_cache().clone();
    let read = std::rc::Rc::new(std::cell::Cell::new(None));
    let sink = std::rc::Rc::clone(&read);
    gtk::glib::spawn_future_local(async move {
        sink.set(cache.pin_indexing().await.ok());
    });
    wait_until("the switch to be read back", || read.get().is_some());
    assert_eq!(read.get(), Some(PinIndexing::Off));
}
