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
//! - `wait=<milliseconds>`: lets work the app does in the background, such
//!   as building the search cache, finish first (at most ten seconds).
//!
//! The steps drive the window through the same actions its buttons, menus
//! and keys use, so the picture shows the app as a user would see it. They
//! exist for pictures only, in an isolated session with fictional files:
//! an action that changes files changes them for real.

use std::time::Duration;

use gtk::prelude::*;

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
    /// Wait this long before the next step.
    Wait(Duration),
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
            "wait" => {
                let pause = Duration::from_millis(argument.parse().ok()?);
                (pause <= LONGEST_WAIT).then_some(Self::Wait(pause))
            }
            _ => None,
        }
    }

    /// Runs the step in `window` and returns how long to wait before the
    /// next one; the error says why it could not run.
    pub(crate) fn run(&self, window: &BrowserWindow) -> Result<Duration, String> {
        match self {
            Self::Select(name) => window
                .select_named(name)
                .then_some(Duration::ZERO)
                .ok_or_else(|| format!("no item called “{name}” is listed")),
            Self::Action { name, target } => {
                let target = target.as_deref().map(ToVariant::to_variant);
                WidgetExt::activate_action(window, &format!("win.{name}"), target.as_ref())
                    .map(|()| Duration::ZERO)
                    .map_err(|_| format!("the window has no action “{name}”"))
            }
            Self::Wait(pause) => Ok(*pause),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_lists_selections_and_actions_in_order() {
        let scene = SceneStep::parse_scene(
            "select=Notes.md; action=context-menu;action=open-tab:/home/demo;wait=250",
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
                SceneStep::Action {
                    name: "open-tab".to_owned(),
                    target: Some("/home/demo".to_owned())
                },
                SceneStep::Wait(Duration::from_millis(250)),
            ]
        );
    }

    #[test]
    fn an_unknown_or_empty_step_is_refused() {
        assert_eq!(SceneStep::parse_scene("rename=Notes.md"), None);
        assert_eq!(SceneStep::parse_scene("select="), None);
        assert_eq!(SceneStep::parse_scene("details-pane"), None);
        assert_eq!(SceneStep::parse_scene("wait=soon"), None);
        assert_eq!(
            SceneStep::parse_scene("wait=60000"),
            None,
            "longer than ten seconds"
        );
    }
}
