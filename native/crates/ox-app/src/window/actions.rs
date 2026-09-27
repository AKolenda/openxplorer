// SPDX-License-Identifier: AGPL-3.0-only
//! Window actions shared by toolbar buttons, menus and keyboard shortcuts.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::{details, grid, sorting::SortColumn};
use crate::theme::ThemePreference;
use crate::{locations, text_size};

use super::BrowserWindow;

impl BrowserWindow {
    fn action(self: &Rc<Self>, name: &str, callback: impl Fn(&Rc<Self>) + 'static) {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            if let Some(browser) = weak.upgrade() {
                callback(&browser);
            }
        });
        self.window.add_action(&action);
    }

    fn choice(
        self: &Rc<Self>,
        name: &str,
        initial: &str,
        callback: impl Fn(&Rc<Self>, &str) -> bool + 'static,
    ) {
        let action =
            gio::SimpleAction::new_stateful(name, Some(glib::VariantTy::STRING), &initial.to_variant());
        let weak = Rc::downgrade(self);
        action.connect_activate(move |action, value| {
            let Some(value) = value.and_then(|value| value.get::<String>()) else {
                return;
            };
            if let Some(browser) = weak.upgrade() {
                if callback(&browser, &value) {
                    action.set_state(&value.to_variant());
                }
            }
        });
        self.window.add_action(&action);
    }

    fn toggle(self: &Rc<Self>, name: &str, initial: bool, callback: impl Fn(&Rc<Self>, bool) + 'static) {
        let action = gio::SimpleAction::new_stateful(name, None, &initial.to_variant());
        let weak = Rc::downgrade(self);
        action.connect_activate(move |action, _| {
            let value = !action
                .state()
                .and_then(|state| state.get::<bool>())
                .unwrap_or(false);
            if let Some(browser) = weak.upgrade() {
                action.set_state(&value.to_variant());
                callback(&browser, value);
            }
        });
        self.window.add_action(&action);
    }

    pub(super) fn set_enabled(&self, name: &str, enabled: bool) {
        if let Some(action) = self
            .window
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(enabled);
        }
    }

    pub(super) fn install_actions(self: &Rc<Self>) {
        self.action("new-tab", |browser| {
            if let Err(error) = browser.add_tab("home:") {
                browser.show_message(error.message());
            }
        });
        self.action("close-tab", |browser| {
            let active = browser.session.borrow().active;
            if let Some(id) = active {
                browser.close_tab(id);
            }
        });
        self.action("next-tab", |browser| browser.cycle_tabs(1));
        self.action("previous-tab", |browser| browser.cycle_tabs(-1));
        self.action("back", |browser| browser.go_history(-1));
        self.action("forward", |browser| browser.go_history(1));
        self.action("up", |browser| {
            if let Some(parent) = browser.current_uri().as_deref().and_then(locations::parent) {
                browser.navigate_or_report(&parent);
            }
        });
        self.action("refresh", |browser| browser.refresh());
        self.action("location", |browser| browser.edit_address());
        self.action("search", |browser| {
            browser.chrome.search.grab_focus();
        });
        self.action("open", |browser| {
            if let Some(position) = browser.content.model.first_selected() {
                browser.activate_item(position);
            }
        });
        self.set_enabled("open", false);
        self.action("select-all", |browser| browser.content.model.select_all());
        self.action("select-none", |browser| browser.content.model.select_none());
        self.action("invert-selection", |browser| {
            browser.content.model.invert_selection()
        });

        let view = if self.settings.preferences.view == "grid" {
            "large"
        } else {
            "details"
        };
        self.choice("view", view, |browser, value| {
            if value == "details" {
                browser.content.views.set_visible_child_name("details");
            } else if let Some(size) = grid::IconSize::from_key(value) {
                grid::set_icon_size(
                    &browser.content.grid,
                    &browser.content.icons,
                    &browser.content.owners,
                    size,
                );
                browser.content.views.set_visible_child_name("grid");
            } else {
                return false;
            }
            true
        });
        self.choice("sort", "name", |browser, value| {
            let Some(column) = SortColumn::from_key(value) else {
                return false;
            };
            let (_, descending) = details::current_sort(&browser.content.details);
            details::sort_by(&browser.content.details, column, descending);
            true
        });
        self.choice("direction", "ascending", |browser, value| {
            if !matches!(value, "ascending" | "descending") {
                return false;
            }
            let (column, _) = details::current_sort(&browser.content.details);
            details::sort_by(&browser.content.details, column, value == "descending");
            true
        });
        if let Some(sorter) = self.content.details.sorter() {
            let weak = Rc::downgrade(self);
            sorter.connect_changed(move |_, _| {
                if let Some(browser) = weak.upgrade() {
                    let (column, descending) = details::current_sort(&browser.content.details);
                    for (name, value) in [
                        ("sort", column.key()),
                        ("direction", if descending { "descending" } else { "ascending" }),
                    ] {
                        if let Some(action) = browser
                            .window
                            .lookup_action(name)
                            .and_downcast::<gio::SimpleAction>()
                        {
                            action.set_state(&value.to_variant());
                        }
                    }
                }
            });
        }
        self.toggle(
            "hidden",
            self.settings.preferences.show_hidden,
            |browser, show| {
                browser.content.model.set_show_hidden(show);
                browser.update_content();
            },
        );
        self.toggle(
            "details-pane",
            self.settings.preferences.details,
            |browser, show| {
                browser.content.inspector.set_visible(show);
            },
        );
        self.choice("theme", self.skin.preference().key(), |browser, value| {
            if !matches!(value, "system" | "light" | "dark") {
                return false;
            }
            let preference = ThemePreference::parse(value);
            browser
                .skin
                .set_preference(preference, browser.system_scheme.is_dark());
            true
        });
        for (name, step) in [
            ("text-larger", text_size::Step::Increase),
            ("text-smaller", text_size::Step::Decrease),
            ("text-reset", text_size::Step::Reset),
        ] {
            self.action(name, move |browser| {
                browser.skin.set_text_size(step.apply(browser.skin.text_size()));
            });
        }
    }

    fn cycle_tabs(self: &Rc<Self>, delta: isize) {
        let next = self.session.borrow().adjacent(delta);
        if let Some(id) = next {
            self.switch_tab(id);
        }
    }
}

pub(crate) fn install_accelerators(app: &gtk::Application) {
    for (action, keys) in [
        ("win.new-tab", &["<Primary>t"][..]),
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
        (
            "win.text-larger",
            &["<Primary>plus", "<Primary>equal", "<Primary>KP_Add"],
        ),
        ("win.text-smaller", &["<Primary>minus", "<Primary>KP_Subtract"]),
        ("win.text-reset", &["<Primary>0", "<Primary>KP_0"]),
    ] {
        app.set_accels_for_action(action, keys);
    }
}
