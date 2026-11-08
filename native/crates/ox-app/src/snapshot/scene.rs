// SPDX-License-Identifier: AGPL-3.0-only
//! The scene steps of the snapshot hook (`OPENXPLORER_SCENE`): what to do
//! in the window, after its first listing, before it is saved.
//!
//! The website's tour (`tools/capture-native-tour.py`) pictures states
//! that the other variables cannot reach: a selection, an open context
//! menu, Properties, several tabs. A scene is a list of steps separated by
//! `;`, run in order, each once the window has settled after the one
//! before:
//!
//! - `select=<name>`: selects the item called `<name>` in the active tab;
//! - `action=<name>`: runs the window action `win.<name>`, such as
//!   `context-menu`, `properties` or `details-pane`;
//! - `action=<name>:<target>`: runs it with a string target, such as
//!   `open-tab-background:/home/demo/Documents`;
//! - `submenu=<label>`: opens the submenu of the open context menu's
//!   item `<label>`, such as `Send to`, as Right on its row does;
//! - `tab=<name>`: shows the tab named `<name>` (`general`, `security`,
//!   `details`, `versions`, ...) of the Properties window opened last;
//! - `row=<property>`: selects the Details row of that property (such
//!   as `Camera model`) in the Properties window opened last, with the
//!   keyboard on it, so the list scrolls to it;
//! - `focus=<name>`: puts the keyboard on the shown button that runs
//!   `win.<name>` (such as `share` or `properties`), with its focus ring,
//!   as a Tab to it would; with several, the last in the window, which is
//!   the details pane's;
//! - `wait=<milliseconds>`: lets work the app does in the background, such
//!   as building the search cache, finish first (at most ten seconds);
//! - `key=<accelerator>`: presses the key, as GTK writes it (such as
//!   `<Primary><Shift>e` or `KP_Multiply`), where keyboard focus is in the
//!   browser window, never in a dialog the scene opened: the
//!   application's accelerators first, then the key handlers and shortcut
//!   controllers from the window down to the focused widget and back up,
//!   as GTK hands a key event round (GTK cannot make a key event itself);
//! - `zoom=<steps>`: the layout that many steps bigger, or smaller when
//!   negative, as Ctrl+wheel notches over the file list give it;
//! - `type=<text>`: puts the text in the text field with keyboard focus,
//!   in the dialog the scene opened last if there is one, such as the
//!   keyboard shortcuts window's search.
//! - `search=<text>`: types `<text>` into the search box, as a key
//!   press there does; the next step, or the picture, does not wait for
//!   the search to finish, so a last `search=` pictures a search while it
//!   runs;
//! - `option=<name>`: opens the search strip's option list `<name>`
//!   (`Kind`, `Date modified`, `Size`, `Search in`), or with `Search
//!   options` the command bar's Search options, as a click does; a
//!   following `submenu=` opens one of its submenus;
//! - `choose=<name>:<label>`: chooses `<label>` in that list, whether the
//!   search filter bar shows or not.
//!
//! The steps drive the window through the same actions its buttons, menus
//! and keys use, so the picture shows the app as a user would see it. They
//! exist for pictures only, in an isolated session with fictional files:
//! an action that changes files changes them for real.

use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::window::BrowserWindow;

/// The longest `wait=` step.
const LONGEST_WAIT: Duration = Duration::from_secs(10);

/// One step of a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SceneStep {
    /// Select only the item with this name.
    Select(String),
    /// Run the window action `win.<name>`, with a string target or none.
    Action {
        /// The action's name, without `win.`.
        name: String,
        /// Its string target, if it takes one.
        target: Option<String>,
    },
    /// Open the submenu of the open context menu's item with this label.
    Submenu(String),
    /// Show this tab of the Properties window opened last.
    Tab(String),
    /// Select and focus this Details row of the Properties window opened
    /// last.
    Row(String),
    /// Put the keyboard on the shown button that runs `win.<name>`.
    Focus(String),
    /// Wait this long before the next step.
    Wait(Duration),
    /// Press this key, as GTK parses it, where keyboard focus is.
    Key(String),
    /// Zoom the layout this many Ctrl+wheel steps.
    Zoom(i32),
    /// Put this text in the focused text field.
    Type(String),
    /// Type this into the search box.
    Search(String),
    /// Open the search strip's option list with this name.
    Option(String),
    /// Choose an option, by its label, in the search strip's list.
    Choose {
        /// The list's name.
        list: String,
        /// The option's label.
        label: String,
    },
}

impl SceneStep {
    /// Parses a scene: steps separated by `;`. Empty steps are skipped,
    /// so a trailing `;` does no harm.
    pub(crate) fn parse_scene(text: &str) -> Option<Vec<Self>> {
        text.split(';')
            .map(str::trim)
            .filter(|step| !step.is_empty())
            .map(Self::parse)
            .collect()
    }

    /// Parses one step, such as `select=Notes.md`,
    /// `action=open-tab:/home/demo` or `wait=500`.
    fn parse(step: &str) -> Option<Self> {
        let (verb, argument) = step.split_once('=')?;
        let argument = argument.trim();
        if argument.is_empty() {
            return None;
        }
        match verb.trim() {
            "select" => Some(Self::Select(argument.to_owned())),
            "submenu" => Some(Self::Submenu(argument.to_owned())),
            "tab" => Some(Self::Tab(argument.to_owned())),
            "row" => Some(Self::Row(argument.to_owned())),
            "focus" => Some(Self::Focus(argument.to_owned())),
            "search" => Some(Self::Search(argument.to_owned())),
            "option" => Some(Self::Option(argument.to_owned())),
            "choose" => {
                let (list, label) = argument.split_once(':')?;
                Some(Self::Choose {
                    list: list.trim().to_owned(),
                    label: label.trim().to_owned(),
                })
            }
            "action" => {
                let (name, target) = match argument.split_once(':') {
                    Some((name, target)) => (name, Some(target.to_owned())),
                    None => (argument, None),
                };
                Some(Self::Action {
                    name: name.to_owned(),
                    target,
                })
            }
            // GTK reads the key when the step runs: it is not started yet.
            "key" => Some(Self::Key(argument.to_owned())),
            "zoom" => argument.parse().ok().map(Self::Zoom),
            "type" => Some(Self::Type(argument.to_owned())),
            "wait" => {
                let pause = Duration::from_millis(argument.parse().ok()?);
                (pause <= LONGEST_WAIT).then_some(Self::Wait(pause))
            }
            _ => None,
        }
    }

    /// Runs the step in `window` and returns how long to wait before the
    /// next one, for an action or submenu at least as long as a menu it
    /// opens takes to fade in; the error says why it could not run.
    pub(crate) fn run(&self, window: &BrowserWindow) -> Result<Duration, String> {
        match self {
            Self::Select(name) => window
                .select_named(name)
                .then_some(Duration::ZERO)
                .ok_or_else(|| format!("no item called “{name}” is listed")),
            Self::Action { name, target } => {
                let target = target.as_deref().map(ToVariant::to_variant);
                WidgetExt::activate_action(window, &format!("win.{name}"), target.as_ref())
                    .map(|()| crate::window::MENU_OPENING_MOTION)
                    .map_err(|_| format!("the window has no action “{name}”"))
            }
            Self::Submenu(label) => (window.open_context_submenu(label)
                || window.open_search_options_submenu(label))
            .then_some(crate::window::MENU_OPENING_MOTION)
            .ok_or_else(|| format!("the open menu has no submenu “{label}”")),
            Self::Tab(name) => {
                let properties = window
                    .properties_windows()
                    .pop()
                    .ok_or_else(|| "no Properties window is open".to_owned())?;
                properties.select_tab(name);
                (properties.selected_tab().as_deref() == Some(name.as_str()))
                    .then_some(Duration::ZERO)
                    .ok_or_else(|| format!("the Properties window has no tab “{name}”"))
            }
            Self::Row(name) => {
                let properties = window
                    .properties_windows()
                    .pop()
                    .ok_or_else(|| "no Properties window is open".to_owned())?;
                crate::properties::focus_detail_row(&properties, name)
                    .then_some(Duration::ZERO)
                    .ok_or_else(|| format!("the Details tab has no row “{name}”"))
            }
            Self::Focus(name) => {
                let action = format!("win.{name}");
                let button = crate::window::widget_tree::descendants::<gtk::Button>(window)
                    .into_iter()
                    .filter(|button| button.is_drawable() && button.action_name().as_deref() == Some(&action))
                    .last()
                    .ok_or_else(|| format!("no shown button runs “{name}”"))?;
                button.grab_focus();
                window.set_focus_visible(true);
                Ok(Duration::ZERO)
            }
            Self::Wait(pause) => Ok(*pause),
            Self::Key(accelerator) => {
                // A key shows where keyboard focus is, as on a desktop.
                window.set_focus_visible(true);
                press_key(window, accelerator)
                    .then_some(crate::window::MENU_OPENING_MOTION)
                    .ok_or_else(|| format!("{accelerator} is not a key, or nothing in the window took it"))
            }
            Self::Zoom(steps) => {
                window.zoom_view(*steps);
                Ok(Duration::ZERO)
            }
            Self::Type(text) => {
                let parent: &gtk::Window = window.upcast_ref();
                let dialog = gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|toplevel| toplevel.downcast::<gtk::Window>().ok())
                    .filter(|other| other.is_visible() && other.transient_for().as_ref() == Some(parent))
                    .last();
                let field = GtkWindowExt::focus(dialog.as_ref().unwrap_or(parent))
                    .and_then(|focus| {
                        focus
                            .ancestor(gtk::Entry::static_type())
                            .and_downcast::<gtk::Editable>()
                            .or_else(|| focus.downcast::<gtk::Editable>().ok())
                    })
                    .ok_or_else(|| "no text field has keyboard focus".to_owned())?;
                field.set_text(text);
                field.set_position(-1);
                Ok(Duration::ZERO)
            }
            Self::Search(text) => {
                window.search_folder(text);
                Ok(Duration::ZERO)
            }
            Self::Option(name) => window
                .open_search_option(name)
                .then_some(crate::window::MENU_OPENING_MOTION)
                .ok_or_else(|| format!("the search strip has no list “{name}”")),
            Self::Choose { list, label } => window
                .choose_search_option(list, label)
                .then_some(Duration::ZERO)
                .ok_or_else(|| format!("the search list “{list}” has no option “{label}”")),
        }
    }
}

/// Presses `accelerator` where keyboard focus is in `window`, as GTK
/// hands a key event round: the application's accelerators, then each
/// widget's key handlers and shortcuts from the window down to the
/// focused widget in the capture phase, and back up in the bubble phase.
/// Whether something took the key.
fn press_key(window: &BrowserWindow, accelerator: &str) -> bool {
    let Some((key, modifiers)) = gtk::accelerator_parse(accelerator) else {
        return false;
    };
    if let Some(app) = window.application() {
        for detailed in app.actions_for_accel(accelerator) {
            let Ok((name, target)) = gio::Action::parse_detailed_name(&detailed) else {
                continue;
            };
            if WidgetExt::activate_action(window, &name, target.as_ref()).is_ok() {
                return true;
            }
        }
    }
    let mut chain: Vec<gtk::Widget> = Vec::new();
    let mut widget = GtkWindowExt::focus(window).or_else(|| Some(window.clone().upcast()));
    while let Some(current) = widget {
        widget = current.parent();
        chain.push(current);
    }
    let capture = chain
        .iter()
        .rev()
        .any(|widget| takes_key(widget, gtk::PropagationPhase::Capture, key, modifiers));
    capture
        || chain.iter().enumerate().any(|(index, widget)| {
            (index == 0 && takes_key(widget, gtk::PropagationPhase::Target, key, modifiers))
                || takes_key(widget, gtk::PropagationPhase::Bubble, key, modifiers)
        })
}

/// Whether one of `widget`'s controllers in `phase` takes the key.
fn takes_key(
    widget: &gtk::Widget,
    phase: gtk::PropagationPhase,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> bool {
    let controllers: Vec<gtk::EventController> = widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventController>().ok())
        .filter(|controller| controller.propagation_phase() == phase)
        .collect();
    controllers.iter().any(|controller| {
        if let Some(shortcuts) = controller.downcast_ref::<gtk::ShortcutController>() {
            return shortcuts
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .filter_map(|shortcut| shortcut.downcast::<gtk::Shortcut>().ok())
                .filter(|shortcut| {
                    shortcut
                        .trigger()
                        .is_some_and(|trigger| triggered(&trigger, key, modifiers))
                })
                .any(|shortcut| {
                    shortcut.action().is_some_and(|action| {
                        action.activate(
                            gtk::ShortcutActionFlags::empty(),
                            widget,
                            shortcut.arguments().as_ref(),
                        )
                    })
                });
        }
        controller
            .downcast_ref::<gtk::EventControllerKey>()
            .is_some_and(|keys| {
                keys.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &0_u32, &modifiers])
            })
    })
}

/// Whether `trigger`, or one of its alternatives, is `key` with exactly
/// `modifiers`.
fn triggered(trigger: &gtk::ShortcutTrigger, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    if let Some(alternatives) = trigger.downcast_ref::<gtk::AlternativeTrigger>() {
        if let Some(list) = alternatives.dynamic_cast_ref::<gio::ListModel>() {
            return list
                .iter::<gtk::ShortcutTrigger>()
                .filter_map(Result::ok)
                .any(|alternative| triggered(&alternative, key, modifiers));
        }
        return triggered(&alternatives.first(), key, modifiers)
            || triggered(&alternatives.second(), key, modifiers);
    }
    trigger
        .downcast_ref::<gtk::KeyvalTrigger>()
        .is_some_and(|keys| keys.keyval() == key && keys.modifiers() == modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_lists_selections_and_actions_in_order() {
        let scene = SceneStep::parse_scene(
            "select=Notes.md; action=context-menu;submenu=Send to;action=open-tab:/home/demo;tab=details;row=Camera model;focus=share;wait=250",
        )
        .expect("a valid scene");
        assert_eq!(
            scene,
            [
                SceneStep::Select("Notes.md".to_owned()),
                SceneStep::Action {
                    name: "context-menu".to_owned(),
                    target: None
                },
                SceneStep::Submenu("Send to".to_owned()),
                SceneStep::Action {
                    name: "open-tab".to_owned(),
                    target: Some("/home/demo".to_owned())
                },
                SceneStep::Tab("details".to_owned()),
                SceneStep::Row("Camera model".to_owned()),
                SceneStep::Focus("share".to_owned()),
                SceneStep::Wait(Duration::from_millis(250)),
            ]
        );
    }

    #[test]
    fn a_scene_can_type_a_search_and_open_or_choose_its_options() {
        let scene =
            SceneStep::parse_scene("search=notes;option=Size;choose=Kind:Documents").expect("a valid scene");
        assert_eq!(
            scene,
            [
                SceneStep::Search("notes".to_owned()),
                SceneStep::Option("Size".to_owned()),
                SceneStep::Choose {
                    list: "Kind".to_owned(),
                    label: "Documents".to_owned()
                },
            ]
        );
        assert_eq!(SceneStep::parse_scene("choose=Kind"), None);
    }

    #[test]
    fn an_unknown_or_empty_step_is_refused() {
        assert_eq!(SceneStep::parse_scene("rename=Notes.md"), None);
        assert_eq!(SceneStep::parse_scene("select="), None);
        assert_eq!(SceneStep::parse_scene("tab="), None);
        assert_eq!(SceneStep::parse_scene("row="), None);
        assert_eq!(SceneStep::parse_scene("focus="), None);
        assert_eq!(SceneStep::parse_scene("details-pane"), None);
        assert_eq!(SceneStep::parse_scene("wait=soon"), None);
        assert_eq!(
            SceneStep::parse_scene("wait=60000"),
            None,
            "longer than ten seconds"
        );
        assert_eq!(SceneStep::parse_scene("zoom=big"), None);
    }

    #[test]
    fn a_scene_takes_keys_and_zoom_steps() {
        assert_eq!(
            SceneStep::parse_scene("key=<Primary><Shift>e; key=KP_Multiply; zoom=-2; type=tab"),
            Some(vec![
                SceneStep::Key("<Primary><Shift>e".to_owned()),
                SceneStep::Key("KP_Multiply".to_owned()),
                SceneStep::Zoom(-2),
                SceneStep::Type("tab".to_owned()),
            ])
        );
    }
}
