// SPDX-License-Identifier: AGPL-3.0-only
//! When the GitHub star prompt is due (UPD-023): each start of the app
//! is counted until the prompt is dismissed, and from the sixth on, the
//! prompt pops up once per session, in the first window left idle.

use gtk::subclass::prelude::*;
use ox_core::settings::PreferencesUpdate;

use super::AppContext;

/// How many starts go by before the prompt first pops up.
const LAUNCHES_BEFORE_ASKING: u32 = 5;

impl AppContext {
    /// Counts this start of the app; once there have been more than
    /// [`LAUNCHES_BEFORE_ASKING`], the star prompt is due this session.
    /// Nothing is counted once the prompt was dismissed.
    pub(crate) fn count_launch_for_star_prompt(&self) {
        let preferences = self.settings_data().preferences;
        if preferences.star_prompt_dismissed {
            return;
        }
        let launches = preferences.star_prompt_launches.saturating_add(1);
        self.remember_preferences(PreferencesUpdate {
            star_prompt_launches: Some(launches),
            ..PreferencesUpdate::default()
        });
        self.imp().star_prompt_due.set(launches > LAUNCHES_BEFORE_ASKING);
    }

    /// Whether the star prompt is due and no window has shown it yet.
    pub(crate) fn star_prompt_due(&self) -> bool {
        let preferences = self.settings_data().preferences;
        self.imp().star_prompt_due.get() && preferences.ask_for_star && !preferences.star_prompt_dismissed
    }

    /// Takes the star prompt for one window: true at most once a session,
    /// while it is due.
    pub(crate) fn take_star_prompt(&self) -> bool {
        let due = self.star_prompt_due();
        self.imp().star_prompt_due.set(false);
        due
    }

    /// Never shows the star prompt again: Dismiss, the close button or
    /// Star on GitHub was pressed.
    pub(crate) fn dismiss_star_prompt(&self) {
        self.imp().star_prompt_due.set(false);
        self.remember_preferences(PreferencesUpdate {
            star_prompt_dismissed: Some(true),
            ..PreferencesUpdate::default()
        });
    }
}
