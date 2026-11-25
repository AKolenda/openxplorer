// SPDX-License-Identifier: AGPL-3.0-only
//! Window actions shared by buttons, menus, rows and keyboard shortcuts,
//! and the application's keyboard accelerators.
//!
//! Ports the command handlers and the keyboard table of `v2.0.0:desktop/ui/app.js`
//! (`onKey`, the `keydown` handler of `setup`). Every action is a
//! `gio::ActionEntry` on the window, so a widget only names the action
//! ([`WindowAction`]) and its target.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::{PreferencesUpdate, Theme, SIDEBAR_ICON_SIZES};

use crate::application::AppAction;
use crate::text_size::Step;

use super::details_pane::PaneMode;
use super::folder_pane::FolderView;
use super::preferences::Preference;
use super::selection_keys::SelectionChange;
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

impl BrowserWindow {
    /// Runs `run`, which moves the keyboard into the address bar or the
    /// search box, unless a dialog is shown over the window: as a Windows
    /// dialog keeps Alt+D, Ctrl+L, F4 and Ctrl+F from the window behind it,
    /// those keys no longer turn the dimmed address bar into a selected
    /// field behind the dialog.
    fn unless_dialog_shown(&self, run: impl Fn(&Self)) {
        if self.dialog_layer().shown().is_none() {
            run(self);
        }
    }
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
pub(super) fn choice_action(
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
        // F4 on a drop-down list, as in the Open and Save dialog's file
        // types, shows its items rather than opening the address bar.
        self.add_controller(crate::modal::f4_shows_a_list());
        self.install_drag_escape();
        self.install_view_keys();
        self.install_tab_move_actions();
        self.install_navigation_actions();
        self.install_address_actions();
        self.install_compact_density_action();
        self.install_file_name_extensions_action();
        self.install_item_check_boxes_action();
        self.install_ctrl_for_item_checks();
        self.install_crumb_actions();
        self.install_selection_actions();
        self.install_view_actions();
        self.install_full_screen();
        self.install_sort_actions();
        self.install_appearance_actions();
        self.install_settings_actions();
        self.install_network_actions();
        self.install_disk_tool_actions();
        self.install_search_actions();
        self.install_search_layout_actions();
        self.install_link_target_action();
        self.install_administrator_action();
        self.install_service_actions();
        self.install_saved_search_actions();
        self.install_details_pane_actions();
        self.install_view_option_actions();
        self.install_stop_action();
        self.install_integration_actions();
        self.install_context_menu_actions();
        self.install_home_page_actions();
        self.install_gallery_actions();
        let [journal, clipboard] = self.install_file_actions();
        let mut handlers = self.imp().handlers.borrow_mut();
        handlers.journal = Some(journal);
        handlers.clipboard = Some(clipboard);
    }

    fn install_tab_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::NewTab, |window| {
                // Not while a tab is dragged: the strip keeps the tabs it
                // had when the drag began, so a new one would not show in
                // front until it is over (TAB-032).
                if window.refuse_while_tabs_move() {
                    return;
                }
                let home = window.home_location();
                if let Err(error) = window.add_tab(&home) {
                    window.show_message(&error.to_string());
                }
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
            plain_action(WindowAction::Location, |window| {
                window.unless_dialog_shown(BrowserWindow::edit_address);
            }),
            plain_action(WindowAction::AddressHistory, |window| {
                window.unless_dialog_shown(BrowserWindow::edit_address_from_history);
            }),
            plain_action(WindowAction::CopyAddress, BrowserWindow::copy_address),
            plain_action(WindowAction::PasteAddress, BrowserWindow::paste_address),
            text_action(WindowAction::CopyAddressOf, BrowserWindow::copy_address_of),
            text_action(
                WindowAction::CopyAddressAsText,
                BrowserWindow::copy_address_as_text,
            ),
            plain_action(
                WindowAction::DeleteAddressHistory,
                BrowserWindow::delete_address_history,
            ),
            plain_action(WindowAction::Search, |window| {
                window.unless_dialog_shown(BrowserWindow::focus_search);
            }),
            text_action(WindowAction::GoTo, BrowserWindow::navigate_or_report),
            text_action(WindowAction::MountVolume, BrowserWindow::mount_volume),
        ]);
    }

    fn install_selection_actions(&self) {
        self.install_group_heading_actions();
        self.add_action_entries([
            plain_action(WindowAction::Open, BrowserWindow::open_selection),
            plain_action(WindowAction::SelectAll, |window| {
                window.change_selection(SelectionChange::All);
            }),
            plain_action(WindowAction::SelectNone, |window| {
                window.change_selection(SelectionChange::Clear);
            }),
            plain_action(WindowAction::InvertSelection, |window| {
                window.change_selection(SelectionChange::Invert);
            }),
            plain_action(
                WindowAction::SelectMatching,
                BrowserWindow::ask_to_select_matching,
            ),
            plain_action(WindowAction::PinSelected, BrowserWindow::pin_selected),
            plain_action(WindowAction::PinFolder, BrowserWindow::pin_folder),
            text_action(WindowAction::PinLocation, BrowserWindow::pin_location),
            plain_action(
                WindowAction::PlaceMoreOptions,
                BrowserWindow::show_place_more_options,
            ),
            plain_action(WindowAction::CopyPath, BrowserWindow::copy_path),
            plain_action(WindowAction::Share, BrowserWindow::share),
            plain_action(WindowAction::About, BrowserWindow::show_about),
            plain_action(WindowAction::Help, BrowserWindow::show_help),
            plain_action(
                WindowAction::NetworkDiscoveryHelp,
                BrowserWindow::show_network_discovery_help,
            ),
            plain_action(
                WindowAction::DismissNetworkInfoBar,
                BrowserWindow::dismiss_network_info_bar,
            ),
            plain_action(
                WindowAction::KeyboardShortcuts,
                BrowserWindow::show_keyboard_shortcuts,
            ),
            plain_action(WindowAction::License, BrowserWindow::show_license),
            plain_action(
                WindowAction::ContextMenu,
                BrowserWindow::open_context_menu_from_keyboard,
            ),
        ]);
        self.set_action_enabled(WindowAction::Open, false);
        self.set_action_enabled(WindowAction::Share, false);
    }

    fn install_view_actions(&self) {
        let preferences = self.context().settings_data().preferences;
        let view = FolderView::from_setting(preferences.view);
        if preferences.preview_pane {
            self.details_pane().set_mode(PaneMode::Preview);
        }
        self.add_action_entries([
            choice_action(WindowAction::View, view.as_str(), |window, key| {
                let Some(view) = FolderView::from_key(key) else {
                    return false;
                };
                // Gallery offers its four icon sizes only (HOME-031).
                if window.folder_pane().shows_gallery() {
                    let FolderView::Icons(size) = view else {
                        return false;
                    };
                    let named = crate::folder_view::gallery_view::nearest_named(size);
                    if named != size {
                        return false;
                    }
                }
                window.change_view(view);
                true
            }),
            toggle_action(
                WindowAction::Hidden,
                preferences.show_hidden,
                BrowserWindow::set_hidden_files_shown,
            ),
            // The Details pane and the Preview pane share the pane: one
            // is open at most, as in Windows (VIEW-070).
            toggle_action(
                WindowAction::DetailsPane,
                preferences.show_details_pane && !preferences.preview_pane,
                |window, shown| {
                    window.open_pane(PaneMode::Details, shown);
                    window.save_preference(Preference::DetailsPane(shown));
                },
            ),
            toggle_action(
                WindowAction::PreviewPane,
                preferences.preview_pane,
                |window, shown| {
                    window.open_pane(PaneMode::Preview, shown);
                    window.save_preference(Preference::PreviewPane(shown));
                },
            ),
            toggle_action(
                WindowAction::Sidebar,
                !preferences.hide_sidebar,
                |window, shown| {
                    window.show_sidebar(shown);
                    window.save_preference(Preference::Sidebar(shown));
                },
            ),
            choice_action(
                WindowAction::SidebarIconSize,
                &preferences.sidebar_icon_size.to_string(),
                |window, key| {
                    let size = key.parse::<u32>().ok();
                    let Some(size) = size.filter(|size| SIDEBAR_ICON_SIZES.contains(size)) else {
                        return false;
                    };
                    window.sidebar().set_icon_size(size);
                    window.save_preference(Preference::SidebarIconSize(size));
                    true
                },
            ),
        ]);
    }

    /// Show hidden files (Ctrl+H, View > Show > Hidden items and Folder
    /// Options): one setting for
    /// every folder, as in Windows. Shows the choice here at once and
    /// saves it, after which every other window follows.
    fn set_hidden_files_shown(&self, shown: bool) {
        self.follow_hidden_files(shown);
        let update = PreferencesUpdate {
            show_hidden: Some(shown),
            ..PreferencesUpdate::default()
        };
        self.context()
            .update_preferences(update, self.preference_failure_reply());
    }

    /// Lists or hides hidden files in every pane, without saving the
    /// choice.
    pub(super) fn follow_hidden_files(&self, shown: bool) {
        let mut changed = false;
        for pane in self.folder_panes() {
            changed |= pane.model().set_show_hidden(shown);
        }
        self.set_action_state(WindowAction::Hidden, &shown.to_variant());
        if changed {
            self.update_content();
            // The pane beside the active one shows or loses its items too.
            self.update_beside_pane();
            // The folder's item count changes with it.
            self.update_details_pane();
            // Gallery walks hidden folders only while they are shown.
            self.reload_gallery_tabs();
        }
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

    /// The user chose `view`: shows it and saves it as the view of every
    /// tab and new window (`changeView` in app.js), or of the folder when
    /// each folder keeps its own (VIEW-020). As in Windows 11, the new view
    /// shows from the top, and only a selected item that would be out of
    /// sight there scrolls it, just far enough to show the first one. When
    /// the list had the keyboard, the first selected item takes it, else
    /// the first item, without scrolling: GTK scrolled to the item that
    /// had it last, selected or not, so a switch with nothing selected
    /// could open the new view part way down.
    fn change_view(&self, view: FolderView) {
        let pane = self.folder_pane();
        let had_keyboard = pane.view_has_focus();
        let selected = pane.model().selected_positions();
        self.show_view(view);
        // The new view takes the keyboard on its first selected item, else
        // its first item, without scrolling: GTK would otherwise scroll to
        // the item that last had it there, selected or not.
        if had_keyboard {
            let first_shown = || pane.first_shown_position();
            if let Some(target) = selected.first().copied().or_else(first_shown) {
                pane.focus_item(target);
            }
        }
        pane.restore_scroll_position(0.0);
        if selected.is_empty() {
            self.remember_style();
            return;
        }
        // Once the new view has its items laid out.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                let pane = window.folder_pane();
                // Something else came first: another selection, or a text
                // field such as a rename in place took the keyboard.
                let typing = GtkWindowExt::focus(&window).is_some_and(|focus| focus.is::<gtk::Text>());
                let count = pane.model().n_items();
                if pane.model().selected_positions() != selected || typing {
                    return;
                }
                if let Some(first) = selected.first().copied().filter(|first| *first < count) {
                    pane.scroll_into_view(first);
                }
            }
        ));
        self.remember_style();
    }

    /// Shows `view` in the folder pane and the status bar, without saving
    /// it as the preferred view.
    pub(crate) fn show_view(&self, view: FolderView) {
        self.reset_typeahead();
        self.folder_pane().show_view(view);
        self.status_bar().show_view(view);
        self.update_expandability();
    }
}

/// The window's keyboard shortcuts of `onKey` that work from text fields
/// too: each action and its accelerators, as GTK parses them. The keys a
/// text field keeps are in [`super::window_keys`], [`super::file_ops`] and,
/// for the history keys, [`super::navigation_buttons`].
const WINDOW_ACCELERATORS: [(WindowAction, &[&str]); 14] = [
    // Ctrl+F5 is Windows' other Refresh key (CMD-039).
    (WindowAction::Refresh, &["F5", "<Primary>r", "<Primary>F5"]),
    (WindowAction::Location, &["<Primary>l", "<Alt>d"]),
    (WindowAction::AddressHistory, &["F4"]),
    // Ctrl+E and F3 are Explorer's other search keys (CMD-036).
    (WindowAction::Search, &["<Primary>f", "<Primary>e", "F3"]),
    // Explorer's Alt+P shows its Preview pane, which is the details pane
    // here, as it previews the selection too (VIEW-063).
    // Windows' keys: Alt+Shift+P for the Details pane, Alt+P for the
    // Preview pane (VIEW-063, VIEW-070).
    (WindowAction::DetailsPane, &["<Alt><Shift>p"]),
    (WindowAction::PreviewPane, &["<Alt>p"]),
    // Explorer's Full screen (CMD-036).
    (WindowAction::FullScreen, &["F11"]),
    (WindowAction::Settings, &["<Primary>comma"]),
    // Dolphin's Open Terminal and Open Terminal Here (OPEN-021); Ctrl+Shift+F4
    // is its Terminal panel key, which opens the terminal here (OPEN-022).
    (WindowAction::OpenTerminal, &["<Shift>F4", "<Primary><Shift>F4"]),
    (WindowAction::OpenTerminalHere, &["<Shift><Alt>F4"]),
    // Dolphin's Open Preferred Search Tool (OPEN-024).
    (WindowAction::SearchTool, &["<Primary><Shift>f"]),
    // Dolphin's Handbook and GNOME's Keyboard Shortcuts (CMD-033, CMD-032).
    (WindowAction::Help, &["F1"]),
    (WindowAction::KeyboardShortcuts, &["<Primary>question"]),
    // Windows File Explorer's Ctrl+plus on the number pad: "Resize all
    // columns to fit text" (VIEW-072). Ctrl+plus on the main keyboard
    // keeps changing the text size.
    (WindowAction::FitAllColumns, &["<Primary>KP_Add"]),
];

/// Ctrl+Q: quit the application, from any window and any focus (TAB-058).
pub(super) const QUIT_ACCELERATORS: &[&str] = &["<Primary>q"];

/// Alt+Enter: Properties of the selection or the folder (`onKey`).
const PROPERTIES_ACCELERATORS: &[&str] = &["<Alt>Return", "<Alt>KP_Enter"];

/// Lets the text-size keys work inside `dialog`, a modal window of its
/// own that the application's accelerators do not reach: they run the
/// text-size actions of the browser window it belongs to, as the web
/// app's key handler did in its sign-in and other dialogs (VIEW-043).
/// The dialog may sit on another dialog, so the first window up the chain
/// of transient parents that has the actions runs them; a Properties
/// window, which has no parent, passes them to the browser window that
/// opened it.
pub(crate) fn follow_text_size_keys(dialog: &impl IsA<gtk::Window>) {
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
    for step in Step::ALL {
        for accelerator in step.accelerators() {
            let trigger = gtk::ShortcutTrigger::parse_string(&accelerator);
            let name = WindowAction::TextSize(step).detailed_name();
            let run = gtk::CallbackAction::new(move |dialog, _| {
                // A key the setting turned off does nothing here either.
                if !MAIN_PLUS_MINUS_ON.get() && Step::is_main_plus_minus(&accelerator) {
                    return glib::Propagation::Proceed;
                }
                let mut owner = dialog.downcast_ref::<gtk::Window>().and_then(owner_of);
                while let Some(window) = owner {
                    if window.activate_action(&name, None).is_ok() {
                        break;
                    }
                    owner = owner_of(&window);
                }
                glib::Propagation::Stop
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
        }
    }
    dialog.as_ref().add_controller(shortcuts);
}

/// The window `window` belongs to: its transient parent, or for a
/// Properties window, a top-level window of its own, its opener.
fn owner_of(window: &gtk::Window) -> Option<gtk::Window> {
    window.transient_for().or_else(|| {
        window
            .downcast_ref::<crate::properties::PropertiesWindow>()
            .and_then(crate::properties::PropertiesWindow::opener)
            .map(Cast::upcast)
    })
}

thread_local! {
    /// Whether the main keyboard's Ctrl+plus and Ctrl+minus change the
    /// text size, as [`set_text_size_keys`] last set them; dialogs follow
    /// it too ([`follow_text_size_keys`]).
    static MAIN_PLUS_MINUS_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Gives the text-size actions their keys: with Ctrl+plus and Ctrl+minus
/// on the main keyboard only when `main_plus_minus`, as Settings >
/// Appearance > "Ctrl+plus and Ctrl+minus change text size" says.
pub(crate) fn set_text_size_keys(app: &gtk::Application, main_plus_minus: bool) {
    MAIN_PLUS_MINUS_ON.set(main_plus_minus);
    for step in Step::ALL {
        let keys = step.accelerators_with(main_plus_minus);
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        app.set_accels_for_action(&WindowAction::TextSize(step).detailed_name(), &keys);
    }
}

/// Installs the keyboard shortcuts of the window actions that work from
/// any focus.
pub(crate) fn install_accelerators(app: &gtk::Application) {
    for (action, keys) in WINDOW_ACCELERATORS {
        app.set_accels_for_action(&action.detailed_name(), keys);
    }
    app.set_accels_for_action(&AppAction::Quit.detailed_name(), QUIT_ACCELERATORS);
    app.set_accels_for_action(&WindowAction::Properties.detailed_name(), PROPERTIES_ACCELERATORS);
    set_text_size_keys(app, true);
    let action = WindowAction::View.detailed_name();
    for view in FolderView::NAMED {
        if let Some((accelerator, _)) = view.shortcut() {
            app.set_accels_for_action(&format!("{action}::{}", view.as_str()), &[accelerator]);
        }
    }
}
