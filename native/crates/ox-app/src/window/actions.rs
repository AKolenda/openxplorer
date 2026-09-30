// SPDX-License-Identifier: AGPL-3.0-only
//! Window actions shared by buttons, menus, rows and keyboard shortcuts,
//! and the application's keyboard accelerators.
//!
//! Ports the command handlers and the keyboard table of `desktop/ui/app.js`
//! (`onKey`, the `keydown` handler of `setup`). Every action is a
//! `gio::ActionEntry` on the window, so a widget only names the action
//! ([`WindowAction`]) and its target.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::Theme;

use crate::application::AppAction;
use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};
use crate::text_size::Step;

use super::folder_pane::FolderView;
use super::preferences::Preference;
use super::session::{Direction, TabId, TabPlacement};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// An action without a target.
pub(super) fn plain_action(
    window_action: WindowAction,
    run: impl Fn(&BrowserWindow) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
        .activate(move |window: &BrowserWindow, _, _| run(window))
        .build()
}

/// An action whose target is a string (a location or a volume id).
pub(super) fn text_action(
    window_action: WindowAction,
    run: impl Fn(&BrowserWindow, &str) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
        .parameter_type(Some(glib::VariantTy::STRING))
        .activate(move |window: &BrowserWindow, _, target| {
            if let Some(text) = target.and_then(glib::Variant::str) {
                run(window, text);
            }
        })
        .build()
}

/// An action whose target is a tab.
pub(super) fn tab_action(
    window_action: WindowAction,
    run: impl Fn(&BrowserWindow, TabId) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
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
fn choice_action(
    window_action: WindowAction,
    initial: &str,
    apply: impl Fn(&BrowserWindow, &str) -> bool + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
        .parameter_type(Some(glib::VariantTy::STRING))
        .state(initial.to_variant())
        .activate(move |window: &BrowserWindow, state_action, target| {
            let Some(value) = target.and_then(glib::Variant::str) else {
                return;
            };
            if apply(window, value) {
                state_action.set_state(&value.to_variant());
            }
        })
        .build()
}

/// A check action that calls `apply` with its new state.
pub(super) fn toggle_action(
    window_action: WindowAction,
    initial: bool,
    apply: impl Fn(&BrowserWindow, bool) + 'static,
) -> gio::ActionEntry<BrowserWindow> {
    gio::ActionEntry::builder(window_action.name())
        .state(initial.to_variant())
        .activate(move |window: &BrowserWindow, state_action, _| {
            let current = state_action.state().and_then(|state| state.get::<bool>());
            let next = !current.unwrap_or(false);
            state_action.set_state(&next.to_variant());
            apply(window, next);
        })
        .build()
}

impl BrowserWindow {
    /// The registered action behind `action`.
    fn simple_action(&self, action: WindowAction) -> gio::SimpleAction {
        self.lookup_action(action.name())
            .and_downcast::<gio::SimpleAction>()
            .expect("install_actions registers every window action as a simple action")
    }

    /// Enables or disables the window action `action`.
    pub(super) fn set_action_enabled(&self, action: WindowAction, enabled: bool) {
        self.simple_action(action).set_enabled(enabled);
    }

    /// Sets the state of the stateful window action `action`.
    pub(super) fn set_action_state(&self, action: WindowAction, state: &glib::Variant) {
        self.simple_action(action).set_state(state);
    }

    /// The state of the stateful window action `action`.
    pub(super) fn window_action_state(&self, action: WindowAction) -> Option<glib::Variant> {
        self.action_state(action.name())
    }

    /// Adds every window action (`win.*`).
    pub(super) fn install_actions(&self) {
        self.install_tab_actions();
        self.install_tab_commands();
        self.install_closing_actions();
        self.install_window_keys();
        self.install_tab_move_actions();
        self.install_navigation_actions();
        self.install_address_actions();
        self.install_crumb_actions();
        self.install_selection_actions();
        self.install_view_actions();
        self.install_sort_actions();
        self.install_appearance_actions();
        self.install_settings_actions();
        self.install_network_actions();
        self.install_disk_tool_actions();
        self.install_search_actions();
        self.install_saved_search_actions();
        self.install_details_pane_actions();
        self.install_integration_actions();
        self.install_unported_actions();
        self.install_context_menu_actions();
        let [journal, clipboard] = self.install_file_actions();
        let mut handlers = self.imp().handlers.borrow_mut();
        handlers.journal = Some(journal);
        handlers.clipboard = Some(clipboard);
    }

    fn install_tab_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::NewTab, |window| {
                let home = window.imp().locations.borrow().home_uri();
                window.open_tab_or_report(&home, TabPlacement::Foreground);
            }),
            plain_action(WindowAction::CloseTab, |window| {
                let active = window.imp().session.borrow().active_id();
                if let Some(id) = active {
                    window.close_tab(id);
                }
            }),
            plain_action(WindowAction::NextTab, |window| {
                window.cycle_tabs(Direction::Forward);
            }),
            plain_action(WindowAction::PreviousTab, |window| {
                window.cycle_tabs(Direction::Backward);
            }),
            tab_action(WindowAction::SelectTab, BrowserWindow::switch_tab),
            tab_action(WindowAction::CloseTabById, BrowserWindow::close_tab),
            text_action(WindowAction::OpenTab, |window, uri| {
                window.open_tab_or_report(uri, TabPlacement::Foreground);
            }),
            text_action(WindowAction::OpenTabBackground, |window, uri| {
                window.open_tab_or_report(uri, TabPlacement::Background);
            }),
            text_action(WindowAction::DropChoice, BrowserWindow::answer_drop_menu),
        ]);
    }

    fn install_navigation_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::Back, |window| {
                window.go_history(Direction::Backward);
            }),
            plain_action(WindowAction::Forward, |window| {
                window.go_history(Direction::Forward);
            }),
            plain_action(WindowAction::Up, BrowserWindow::go_up),
            plain_action(WindowAction::Home, BrowserWindow::go_home),
            gio::ActionEntry::builder(WindowAction::GoHistory.name())
                .parameter_type(Some(glib::VariantTy::INT32))
                .activate(|window: &BrowserWindow, _, target| {
                    if let Some(steps) = target.and_then(glib::Variant::get::<i32>) {
                        window.go_history_by(steps);
                    }
                })
                .build(),
            plain_action(WindowAction::Refresh, BrowserWindow::refresh),
            plain_action(WindowAction::Location, BrowserWindow::edit_address),
            plain_action(
                WindowAction::AddressHistory,
                BrowserWindow::edit_address_from_history,
            ),
            plain_action(WindowAction::CopyAddress, BrowserWindow::copy_address),
            plain_action(WindowAction::PasteAddress, BrowserWindow::paste_address),
            plain_action(WindowAction::Search, BrowserWindow::focus_search),
            text_action(WindowAction::GoTo, BrowserWindow::navigate_or_report),
            text_action(WindowAction::MountVolume, BrowserWindow::mount_volume),
            text_action(
                WindowAction::OpenServerAddress,
                BrowserWindow::open_server_address,
            ),
        ]);
    }

    fn install_selection_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::Open, |window| {
                // Enter and Open act on exactly one item, as app.js does.
                let positions = window.folder_pane().model().selected_positions();
                if let [position] = positions.as_slice() {
                    window.activate_item(*position);
                }
            }),
            plain_action(WindowAction::SelectAll, |window| {
                window.folder_pane().model().select_all();
            }),
            plain_action(WindowAction::SelectNone, |window| {
                window.folder_pane().model().select_none();
            }),
            plain_action(WindowAction::InvertSelection, |window| {
                window.folder_pane().model().invert_selection();
            }),
            plain_action(
                WindowAction::SelectMatching,
                BrowserWindow::ask_to_select_matching,
            ),
            plain_action(WindowAction::PinSelected, BrowserWindow::pin_selected),
            plain_action(WindowAction::PinFolder, BrowserWindow::pin_folder),
            plain_action(WindowAction::CopyPath, BrowserWindow::copy_path),
            plain_action(WindowAction::About, BrowserWindow::show_about),
            plain_action(
                WindowAction::ContextMenu,
                BrowserWindow::open_context_menu_from_keyboard,
            ),
        ]);
        self.set_action_enabled(WindowAction::Open, false);
    }

    fn install_view_actions(&self) {
        let preferences = self.context().settings_data().preferences;
        let view = FolderView::from_setting(preferences.view);
        self.add_action_entries([
            choice_action(WindowAction::View, view.as_str(), |window, key| {
                let Some(view) = FolderView::from_key(key) else {
                    return false;
                };
                window.change_view(view);
                true
            }),
            toggle_action(
                WindowAction::Hidden,
                preferences.show_hidden,
                BrowserWindow::set_hidden_files_shown,
            ),
            toggle_action(
                WindowAction::DetailsPane,
                preferences.show_details_pane,
                |window, shown| {
                    window.fit_details_pane();
                    window.save_preference(Preference::DetailsPane(shown));
                },
            ),
        ]);
    }

    /// The Sort menu's column and direction choices, which follow sorting
    /// by a column header too.
    fn install_sort_actions(&self) {
        self.add_action_entries([
            choice_action(WindowAction::Sort, SortColumn::Name.as_str(), |window, key| {
                let Some(column) = SortColumn::from_key(key) else {
                    return false;
                };
                window.sort_by_column(column);
                true
            }),
            choice_action(
                WindowAction::Direction,
                SortDirection::Ascending.as_str(),
                |window, key| {
                    let Some(direction) = SortDirection::from_key(key) else {
                        return false;
                    };
                    window.sort_in_direction(direction);
                    true
                },
            ),
        ]);
        self.follow_header_sorting();
    }

    /// Sorts the details view by `column`, keeping the direction.
    fn sort_by_column(&self, column: SortColumn) {
        let details = self.folder_pane().details();
        details.sort_by(SortOrder {
            column,
            ..details.sort_order()
        });
    }

    /// Sorts the details view in `direction`, keeping the column.
    fn sort_in_direction(&self, direction: SortDirection) {
        let details = self.folder_pane().details();
        details.sort_by(SortOrder {
            direction,
            ..details.sort_order()
        });
    }

    /// Show hidden files: lists or hides them, and saves the choice.
    fn set_hidden_files_shown(&self, shown: bool) {
        self.folder_pane().model().set_show_hidden(shown);
        self.update_content();
        // The folder's item count changes with it.
        self.update_details_pane();
        self.save_preference(Preference::ShowHidden(shown));
    }

    /// Keeps the Sort menu in step with sorting by a column header.
    fn follow_header_sorting(&self) {
        let Some(sorter) = self.folder_pane().details().column_view().sorter() else {
            return;
        };
        sorter.connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let order = window.folder_pane().details().sort_order();
                window.set_action_state(WindowAction::Sort, &order.column.as_str().to_variant());
                window.set_action_state(WindowAction::Direction, &order.direction.as_str().to_variant());
            }
        ));
    }

    fn install_appearance_actions(&self) {
        let theme = self.skin().theme().as_str();
        self.add_action_entries([choice_action(WindowAction::Theme, theme, |window, key| {
            let Some(theme) = Theme::from_key(key) else {
                return false;
            };
            window.skin().set_theme(theme);
            window.save_preference(Preference::Theme(theme));
            true
        })]);
        let steps = Step::ALL.map(|step| {
            plain_action(WindowAction::TextSize(step), move |window| {
                let size = step.apply(window.skin().text_size());
                window.change_text_size(size);
            })
        });
        self.add_action_entries(steps);
    }

    /// Ctrl+F: the settings search on the Settings tab, else the search
    /// box.
    fn focus_search(&self) {
        if self.shows_settings() {
            self.settings_page().focus_search();
        } else {
            self.search_box().focus();
        }
    }

    /// Opens a tab for `address`, showing a refused address in the
    /// message line.
    pub(super) fn open_tab_or_report(&self, address: &str, placement: TabPlacement) {
        if let Err(error) = self.open_tab(address, placement) {
            self.show_message(&error.to_string());
        }
    }

    /// The user chose `view`: shows it from the top and saves it as the
    /// view of every tab and new window (`changeView` in app.js).
    fn change_view(&self, view: FolderView) {
        self.show_view(view);
        self.folder_pane().restore_scroll_position(0.0);
        self.save_preference(Preference::View(view));
    }

    /// Shows `view` in the folder pane and the status bar, without saving
    /// it as the preferred view.
    pub(crate) fn show_view(&self, view: FolderView) {
        self.reset_typeahead();
        self.folder_pane().show_view(view);
        self.status_bar().show_view(view);
    }
}

/// The window's keyboard shortcuts of `onKey` that work from text fields
/// too: each action and its accelerators, as GTK parses them. The keys a
/// text field keeps are in [`super::window_keys`], [`super::file_ops`] and,
/// for the history keys, [`super::navigation_buttons`].
const WINDOW_ACCELERATORS: [(WindowAction, &[&str]); 6] = [
    (WindowAction::Refresh, &["F5", "<Primary>r"]),
    (WindowAction::Location, &["<Primary>l", "<Alt>d"]),
    (WindowAction::AddressHistory, &["F4"]),
    (WindowAction::Search, &["<Primary>f"]),
    (WindowAction::DetailsPane, &["<Alt><Shift>p"]),
    (WindowAction::Settings, &["<Primary>comma"]),
];

/// Ctrl+Q: quit the application, from any window and any focus (TAB-058).
const QUIT_ACCELERATORS: &[&str] = &["<Primary>q"];

/// Alt+Enter: Properties of the selection or the folder (`onKey`).
const PROPERTIES_ACCELERATORS: &[&str] = &["<Alt>Return", "<Alt>KP_Enter"];

/// Lets the text-size keys work inside `dialog`, a modal window of its
/// own that the application's accelerators do not reach: they run the
/// text-size actions of the browser window it belongs to, as the web
/// app's key handler did in its sign-in and other dialogs (VIEW-043).
/// The dialog may sit on another dialog, so the first window up the chain
/// of transient parents that has the actions runs them.
pub(crate) fn follow_text_size_keys(dialog: &impl IsA<gtk::Window>) {
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
    for step in Step::ALL {
        for accelerator in step.accelerators() {
            let trigger = gtk::ShortcutTrigger::parse_string(&accelerator);
            let name = WindowAction::TextSize(step).detailed_name();
            let run = gtk::CallbackAction::new(move |dialog, _| {
                let mut owner = dialog
                    .downcast_ref::<gtk::Window>()
                    .and_then(GtkWindowExt::transient_for);
                while let Some(window) = owner {
                    if window.activate_action(&name, None).is_ok() {
                        break;
                    }
                    owner = window.transient_for();
                }
                glib::Propagation::Stop
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
        }
    }
    dialog.as_ref().add_controller(shortcuts);
}

/// Installs the keyboard shortcuts of the window actions that work from
/// any focus.
pub(crate) fn install_accelerators(app: &gtk::Application) {
    for (action, keys) in WINDOW_ACCELERATORS {
        app.set_accels_for_action(&action.detailed_name(), keys);
    }
    app.set_accels_for_action(&AppAction::Quit.detailed_name(), QUIT_ACCELERATORS);
    app.set_accels_for_action(&WindowAction::Properties.detailed_name(), PROPERTIES_ACCELERATORS);
    for step in Step::ALL {
        let keys = step.accelerators();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        app.set_accels_for_action(&WindowAction::TextSize(step).detailed_name(), &keys);
    }
    let action = WindowAction::View.detailed_name();
    for view in FolderView::ZOOM_ORDER {
        let (accelerator, _) = view.shortcut();
        app.set_accels_for_action(&format!("{action}::{}", view.as_str()), &[accelerator]);
    }
}
