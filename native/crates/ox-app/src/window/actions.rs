// SPDX-License-Identifier: AGPL-3.0-only
//! Window actions shared by buttons, menus, rows and keyboard shortcuts,
//! and the application's keyboard accelerators.
//!
//! Ports the command handlers and the keyboard table of `desktop/ui/app.js`
//! (`keydown` in `setupKeys`). Every action is a `gio::ActionEntry` on the
//! window, so a widget only names the action and its target.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::details;
use crate::folder_view::grid::IconSize;
use crate::folder_view::sorting::{SortColumn, SortDirection};
use crate::text_size::Step;
use crate::theme::ThemePreference;

use super::content::FolderView;
use super::preferences::Preference;
use super::session::{TabId, TabPlacement};
use super::BrowserWindow;

/// An action without a target.
fn plain(name: &str, run: impl Fn(&BrowserWindow) + 'static) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(name)
        .activate(move |window: &BrowserWindow, _, _| run(window))
        .build()
}

/// An action whose target is a string (a location or a volume id).
fn with_text(name: &str, run: impl Fn(&BrowserWindow, &str) + 'static) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(name)
        .parameter_type(Some(glib::VariantTy::STRING))
        .activate(move |window: &BrowserWindow, _, target| {
            if let Some(text) = target.and_then(glib::Variant::str) {
                run(window, text);
            }
        })
        .build()
}

/// An action whose target is a tab.
fn with_tab(name: &str, run: impl Fn(&BrowserWindow, TabId) + 'static) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(name)
        .parameter_type(Some(glib::VariantTy::UINT64))
        .activate(move |window: &BrowserWindow, _, target| {
            if let Some(id) = target.and_then(TabId::from_variant) {
                run(window, id);
            }
        })
        .build()
}

/// A radio action: `apply` returns false for a value it does not accept,
/// and the state changes only when it accepts it.
fn choice(
    name: &str,
    initial: &str,
    apply: impl Fn(&BrowserWindow, &str) -> bool + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(name)
        .parameter_type(Some(glib::VariantTy::STRING))
        .state(initial.to_variant())
        .activate(move |window: &BrowserWindow, action, target| {
            let Some(value) = target.and_then(glib::Variant::str) else {
                return;
            };
            if apply(window, value) {
                action.set_state(&value.to_variant());
            }
        })
        .build()
}

/// A check action that calls `apply` with its new state.
fn toggle(
    name: &str,
    initial: bool,
    apply: impl Fn(&BrowserWindow, bool) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(name)
        .state(initial.to_variant())
        .activate(move |window: &BrowserWindow, action, _| {
            let current = action.state().and_then(|state| state.get::<bool>());
            let next = !current.unwrap_or(false);
            action.set_state(&next.to_variant());
            apply(window, next);
        })
        .build()
}

impl BrowserWindow {
    /// Enables or disables the window action `name`.
    pub(super) fn set_action_enabled(&self, name: &str, enabled: bool) {
        let action = self.lookup_action(name).and_downcast::<gio::SimpleAction>();
        if let Some(action) = action {
            action.set_enabled(enabled);
        }
    }

    /// Sets the state of the stateful window action `name`.
    pub(super) fn set_action_state(&self, name: &str, state: &glib::Variant) {
        let action = self.lookup_action(name).and_downcast::<gio::SimpleAction>();
        if let Some(action) = action {
            action.set_state(state);
        }
    }

    /// Adds every window action (`win.*`).
    pub(super) fn install_actions(&self) {
        self.install_tab_actions();
        self.install_navigation_actions();
        self.install_selection_actions();
        self.install_view_actions();
        self.install_appearance_actions();
        self.install_unported_actions();
    }

    fn install_tab_actions(&self) {
        self.add_action_entries([
            plain("new-tab", |window| {
                let home = window.imp().locations.borrow().home_uri();
                window.open_tab_or_report(&home, TabPlacement::Foreground);
            }),
            plain("close-tab", |window| {
                let active = window.imp().session.borrow().active;
                if let Some(id) = active {
                    window.close_tab(id);
                }
            }),
            plain("next-tab", |window| window.cycle_tabs(1)),
            plain("previous-tab", |window| window.cycle_tabs(-1)),
            with_tab("select-tab", BrowserWindow::switch_tab),
            with_tab("close-tab-by-id", BrowserWindow::close_tab),
            with_text("open-tab", |window, uri| {
                window.open_tab_or_report(uri, TabPlacement::Foreground);
            }),
            with_text("open-tab-background", |window, uri| {
                window.open_tab_or_report(uri, TabPlacement::Background);
            }),
        ]);
    }

    fn install_navigation_actions(&self) {
        self.add_action_entries([
            plain("back", |window| window.go_history(-1)),
            plain("forward", |window| window.go_history(1)),
            plain("up", BrowserWindow::go_up),
            plain("refresh", BrowserWindow::refresh),
            plain("location", BrowserWindow::edit_address),
            plain("search", |window| {
                window.chrome().search.entry.grab_focus();
            }),
            with_text("go-to", BrowserWindow::navigate_or_report),
            with_text("mount-volume", BrowserWindow::mount_volume),
            with_text("open-server-address", BrowserWindow::open_server_address),
        ]);
    }

    fn install_selection_actions(&self) {
        self.add_action_entries([
            plain("open", |window| {
                // Enter and Open act on exactly one item, as app.js does.
                let positions = window.content().model.selected_positions();
                if let [position] = positions.as_slice() {
                    window.activate_item(*position);
                }
            }),
            plain("select-all", |window| window.content().model.select_all()),
            plain("select-none", |window| window.content().model.select_none()),
            plain("invert-selection", |window| {
                window.content().model.invert_selection();
            }),
            plain("pin-selected", BrowserWindow::pin_selected),
            plain("pin-folder", BrowserWindow::pin_folder),
            plain("copy-path", BrowserWindow::copy_path),
            plain("about", BrowserWindow::show_about),
            plain("context-menu", BrowserWindow::open_context_menu_from_keyboard),
        ]);
        self.set_action_enabled("open", false);
    }

    fn install_view_actions(&self) {
        let preferences = self.context().settings_data().preferences;
        let view = FolderView::from_setting(&preferences.view);
        self.add_action_entries([
            choice("view", view.key(), |window, key| {
                let Some(view) = FolderView::from_key(key) else {
                    return false;
                };
                window.show_view(view);
                window.save_preference(Preference::View(view));
                true
            }),
            choice("sort", SortColumn::Name.key(), |window, key| {
                let Some(column) = SortColumn::from_key(key) else {
                    return false;
                };
                let (_, direction) = details::current_sort(&window.content().details);
                details::sort_by(&window.content().details, column, direction);
                true
            }),
            choice("direction", SortDirection::Ascending.key(), |window, key| {
                let Some(direction) = SortDirection::from_key(key) else {
                    return false;
                };
                let (column, _) = details::current_sort(&window.content().details);
                details::sort_by(&window.content().details, column, direction);
                true
            }),
            toggle(
                "hidden",
                preferences.show_hidden,
                BrowserWindow::set_hidden_files_shown,
            ),
            toggle("details-pane", preferences.details, |window, shown| {
                window.fit_details_pane();
                window.save_preference(Preference::DetailsPane(shown));
            }),
        ]);
        self.follow_header_sorting();
    }

    /// Show hidden files: lists or hides them, and saves the choice.
    fn set_hidden_files_shown(&self, shown: bool) {
        self.content().model.set_show_hidden(shown);
        self.update_content();
        // The folder's item count changes with it.
        self.update_details_pane();
        self.save_preference(Preference::ShowHidden(shown));
    }

    /// Keeps the Sort menu in step with sorting by a column header.
    fn follow_header_sorting(&self) {
        let Some(sorter) = self.content().details.sorter() else {
            return;
        };
        sorter.connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let (column, direction) = details::current_sort(&window.content().details);
                window.set_action_state("sort", &column.key().to_variant());
                window.set_action_state("direction", &direction.key().to_variant());
            }
        ));
    }

    fn install_appearance_actions(&self) {
        let theme = self.skin().preference().key();
        self.add_action_entries([choice("theme", theme, |window, key| {
            let Some(preference) = ThemePreference::from_key(key) else {
                return false;
            };
            window.skin().set_preference(preference);
            window.save_preference(Preference::Theme(preference));
            true
        })]);
        let steps = Step::ALL.map(|step| {
            plain(step.action_name(), move |window| {
                let size = step.apply(window.skin().text_size());
                window.skin().set_text_size(size);
                window.save_preference(Preference::TextSize(size));
            })
        });
        self.add_action_entries(steps);
    }

    fn open_tab_or_report(&self, address: &str, placement: TabPlacement) {
        if let Err(error) = self.open_tab(address, placement) {
            self.chrome().show_message(error.message());
        }
    }

    /// Shows `view` in the folder pane and the status bar, without saving
    /// it as the preferred view.
    pub(crate) fn show_view(&self, view: FolderView) {
        self.reset_typeahead();
        self.content().show_view(view);
        self.chrome().status.show_view(view);
    }
}

/// Installs the keyboard shortcuts of every window action.
pub(crate) fn install_accelerators(app: &gtk::Application) {
    let fixed: [(&str, &[&str]); 12] = [
        ("win.new-tab", &["<Primary>t"]),
        ("win.close-tab", &["<Primary>w"]),
        ("win.next-tab", &["<Primary>Tab", "<Primary>Page_Down"]),
        ("win.previous-tab", &["<Primary><Shift>Tab", "<Primary>Page_Up"]),
        ("win.back", &["<Alt>Left"]),
        ("win.forward", &["<Alt>Right"]),
        ("win.up", &["<Alt>Up"]),
        ("win.refresh", &["F5", "<Primary>r"]),
        ("win.location", &["<Primary>l", "<Alt>d"]),
        ("win.search", &["<Primary>f"]),
        ("win.hidden", &["<Primary>h"]),
        ("app.new-window", &["<Primary>n"]),
    ];
    for (action, keys) in fixed {
        app.set_accels_for_action(action, keys);
    }
    for step in Step::ALL {
        let keys = step.accelerators();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        app.set_accels_for_action(&format!("win.{}", step.action_name()), &keys);
    }
    for size in IconSize::ALL {
        let detailed = format!("win.view::{}", size.key());
        app.set_accels_for_action(&detailed, &[size.accelerator()]);
    }
}
