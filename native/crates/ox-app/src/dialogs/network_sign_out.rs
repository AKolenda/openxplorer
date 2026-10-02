// SPDX-License-Identifier: AGPL-3.0-only
//! "Sign out of `<host>`?": what Sign out of server forgets.
//!
//! Ports the dialog of `signOut` in `v2.0.0:desktop/ui/app.js` (NET-020). It only
//! asks; the window signs out.

use gtk::prelude::*;
use ox_core::network::ForgetScope;

use crate::window::{ButtonStyle, Dialog};

/// What the dialog says under its heading.
const MESSAGE: &str = crate::i18n::message_id(
    "This disconnects all SMB mounts for this server in your desktop session, including other \
applications. Close files on this server first.",
);

/// The note under the check boxes.
const NOTE: &str = crate::i18n::message_id(
    "Pinned shortcuts remain. With saved credentials removed, opening a share will ask you to sign in \
again. This does not delete files on the server. Hostname aliases may have separate saved credentials.",
);

/// Whether Sign out also clears the server's cached file names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchCacheChoice {
    /// The cached names stay, the default.
    Keep,
    /// "Also clear cached filenames for this server".
    Clear,
}

/// What the user chose in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SignOutChoice {
    /// "Forget saved credentials for this server": checked is
    /// [`ForgetScope::AllScopes`], the default.
    pub forget: ForgetScope,
    /// Whether the server's cached file names are cleared too.
    pub search_cache: SearchCacheChoice,
}

/// The Sign out dialog for `host`, over `parent`. Its Sign out button
/// calls `on_sign_out` with the dialog, to run the sign-out in, and the
/// choice.
pub(crate) fn sign_out_dialog(
    parent: &impl IsA<gtk::Window>,
    host: &str,
    on_sign_out: impl Fn(&Dialog, SignOutChoice) + 'static,
) -> Dialog {
    let title = ox_core::i18n::format_message("Sign out of {host}?", &[("host", &(host).to_string())]);
    let dialog = Dialog::new(parent, &title, ox_core::i18n::gettext_static(MESSAGE));
    let forget = dialog.add_check_button(
        &ox_core::i18n::gettext("Forget saved credentials for this server"),
        true,
    );
    let clear_cache = dialog.add_check_button(
        &ox_core::i18n::gettext("Also clear cached filenames for this server"),
        false,
    );
    dialog.add_note(ox_core::i18n::gettext_static(NOTE));
    dialog.add_cancel_button();
    dialog.add_button(&ox_core::i18n::gettext("Sign out"), ButtonStyle::Accent);
    dialog.connect_confirmed(move |dialog| {
        let forget = if forget.is_active() {
            ForgetScope::AllScopes
        } else {
            ForgetScope::SessionOnly
        };
        let search_cache = if clear_cache.is_active() {
            SearchCacheChoice::Clear
        } else {
            SearchCacheChoice::Keep
        };
        on_sign_out(dialog, SignOutChoice { forget, search_cache });
    });
    dialog
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::test_support::harness::{descendants, settle};

    /// parity: NET-020
    #[gtk::test]
    fn the_dialog_forgets_credentials_and_keeps_the_cache_by_default() {
        let parent = gtk::Window::new();
        let choices = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&choices);
        let dialog = sign_out_dialog(&parent, "nas", move |_, choice| heard.borrow_mut().push(choice));

        let texts = dialog.texts();
        for expected in [
            "Sign out of nas?",
            MESSAGE,
            "Forget saved credentials for this server",
            "Also clear cached filenames for this server",
            NOTE,
            "Cancel",
            "Sign out",
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected} in {texts:?}"
            );
        }
        dialog.press("Sign out");
        settle();
        let boxes = descendants::<gtk::CheckButton>(&dialog);
        boxes[0].set_active(false);
        boxes[1].set_active(true);
        dialog.press("Sign out");
        settle();

        let expected = [
            SignOutChoice {
                forget: ForgetScope::AllScopes,
                search_cache: SearchCacheChoice::Keep,
            },
            SignOutChoice {
                forget: ForgetScope::SessionOnly,
                search_cache: SearchCacheChoice::Clear,
            },
        ];
        assert_eq!(*choices.borrow(), expected);
        dialog.close();
        parent.close();
    }
}
