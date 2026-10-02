// SPDX-License-Identifier: AGPL-3.0-only
//! The Zorin + Brave setup and troubleshooting guide, a page that the
//! Troubleshooting row of Default apps opens.
//!
//! Ports the `zorin-guide` details of `appendV07Settings` in
//! `v2.0.0:desktop/ui/app.js`: the same steps, numbered again and pointing at
//! where the controls are in the native Settings, then how Restore
//! previous undoes the changes.

use super::parts;
use super::section::{PageKind, SettingsSection};
use crate::icons::Icon;

/// The title of the Zorin + Brave guide, as the Python app named it.
pub(super) const GUIDE_TITLE: &str = crate::i18n::message_id("Zorin + Brave setup and troubleshooting");

/// The steps of the Python app's "Zorin + Brave setup and
/// troubleshooting", numbered again (the Python list had two fourth
/// steps) and pointing at where the controls are now.
const GUIDE: [&str; 7] = [
    crate::i18n::message_id(
        "1. Click Make OpenXplorer default, then Enable in Show in folder from browsers. Both \
     folder/SMB handlers and the optional FileManager1 service are configured for your account.",
    ),
    crate::i18n::message_id(
        "2. Close other file-manager windows. If Show in folder says waiting, log out of Zorin and log \
     back in. OpenXplorer does not terminate Files or Dolphin.",
    ),
    crate::i18n::message_id(
        "3. Clicking a ZIP filename in Brave opens its ZIP handler, not your folder handler. Turn on \
     ZIP files in Default apps to change that association. Restart Brave after changing it.",
    ),
    crate::i18n::message_id(
        "4. Downloads → Show in folder is a different action. Test, in Show in folder from browsers, \
     checks FileManager1, not Brave or its portal. The status must show OpenXplorer as the owner, \
     not just enabled.",
    ),
    crate::i18n::message_id(
        "5. Flatpak/Snap Brave or a remembered portal choice may still use another handler. In a \
     chooser, select OpenXplorer. Do not disable your desktop portal: file-picker dialogs remain \
     system dialogs.",
    ),
    crate::i18n::message_id(
        "6. Pin the installed OpenXplorer folder icon to your Zorin panel. Right-click it → Open \
     windows… lists existing windows; New window creates another. Super+E is a separate system \
     keyboard shortcut.",
    ),
    crate::i18n::message_id(
        "Restore previous removes OpenXplorer's unmodified per-user reveal/autostart files and restores \
     recorded file handlers. No system packages are removed.",
    ),
];

/// The Zorin + Brave setup and troubleshooting page.
pub(super) fn build() -> SettingsSection {
    let guide = SettingsSection::new(
        ox_core::i18n::gettext_static(GUIDE_TITLE),
        ox_core::i18n::gettext_static(
            "Make OpenXplorer Zorin's file explorer and Brave's Show in folder, and undo it.",
        ),
        PageKind::Subpage,
    );
    let (restore, steps) = GUIDE.split_last().expect("the guide has steps");
    for step in steps {
        guide.append_text(&parts::paragraph(ox_core::i18n::gettext_static(step)));
    }
    guide.append_text(&parts::note(Icon::Info, ox_core::i18n::gettext_static(restore)));
    guide
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_page::search::shown_text;

    /// The page shows the six numbered steps once each, in order, then
    /// what Restore previous removes.
    ///
    /// parity: INT-018
    #[gtk::test]
    fn the_guide_shows_each_step_once_in_order() {
        let text = shown_text(&build());
        let positions: Vec<usize> = (1..=6)
            .map(|step| text.find(&format!("{step}. ")).expect("every step is shown"))
            .collect();
        assert!(positions.is_sorted(), "{text}");
        assert_eq!(text.matches("4. ").count(), 1, "one fourth step");
        let topics = [
            "ZIP handler",
            "FileManager1",
            "Flatpak/Snap",
            "Open windows…",
            "Restore previous",
        ];
        for topic in topics {
            assert!(text.contains(topic), "{topic} in {text}");
        }
    }
}
