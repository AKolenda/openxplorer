// SPDX-License-Identifier: AGPL-3.0-only
//! The thin line that runs over the top of the folder pane while a folder
//! is listed.
//!
//! Ports `#loading-line` in `v2.0.0:desktop/ui/index.html` and `.loading-line` in
//! `v2.0.0:desktop/ui/style.css`: a 2-pixel line laid over the pane, so showing it
//! never moves the items, with a bar sliding across it
//! (`resources/skin/folder-views.css`). It appears only when a listing
//! takes longer than [`APPEARANCE_DELAY`], so folders that list at once
//! never flash it (ui-spec.md M06). The status bar says "Loading…" only
//! while the line shows ([`LoadingLine::is_shown`]).

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// How long a listing runs before the line shows. Shorter listings, which
/// are most of them, show nothing at all: Dolphin waits as long before it
/// says a folder is loading.
pub(crate) const APPEARANCE_DELAY: Duration = Duration::from_millis(300);

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::LoadingLine`].
    #[derive(Debug, Default)]
    pub(crate) struct LoadingLine {
        /// The timer that will show the line, while one runs. It clears
        /// itself when it fires, so it is never removed twice.
        pub(super) pending: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoadingLine {
        const NAME: &'static str = "OxLoadingLine";
        type Type = super::LoadingLine;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for LoadingLine {
        fn constructed(&self) {
            self.parent_constructed();
            let line = self.obj();
            line.set_valign(gtk::Align::Start);
            line.set_hexpand(true);
            line.set_can_target(false);
            line.set_visible(false);
            line.add_css_class("loading-line");
            line.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
                "Loading",
            ))]);
        }

        fn dispose(&self) {
            self.obj().cancel_pending();
        }
    }

    impl WidgetImpl for LoadingLine {}
    impl BoxImpl for LoadingLine {}
}

glib::wrapper! {
    /// The loading line, to lay over the top of the folder pane.
    pub(crate) struct LoadingLine(ObjectSubclass<imp::LoadingLine>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl LoadingLine {
    /// A hidden line.
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    /// Shows the line [`APPEARANCE_DELAY`] after a listing starts, or hides
    /// it at once when the listing is over.
    pub(crate) fn set_loading(&self, loading: bool) {
        if !loading {
            self.cancel_pending();
            self.set_visible(false);
            return;
        }
        let already_on_its_way = self.is_visible() || self.imp().pending.borrow().is_some();
        if already_on_its_way {
            return;
        }
        let timer = glib::timeout_add_local_once(
            APPEARANCE_DELAY,
            glib::clone!(
                #[weak(rename_to = line)]
                self,
                move || {
                    line.imp().pending.take();
                    line.set_visible(true);
                }
            ),
        );
        self.imp().pending.replace(Some(timer));
    }

    /// Whether the line shows now: the listing has run for the delay.
    pub(crate) fn is_shown(&self) -> bool {
        self.is_visible()
    }

    fn cancel_pending(&self) {
        if let Some(timer) = self.imp().pending.take() {
            timer.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::test_support::harness::{wait_for, wait_until};

    /// parity: VIEW-047
    #[gtk::test]
    fn the_line_shows_once_a_listing_has_run_for_the_delay() {
        let line = LoadingLine::new();
        let started = Instant::now();
        line.set_loading(true);
        assert!(!line.is_visible(), "a listing that just started shows no line");
        wait_until("the loading line", || line.is_visible());
        assert!(started.elapsed() >= APPEARANCE_DELAY);
        line.set_loading(false);
        assert!(!line.is_visible(), "the line goes when the listing ends");
    }

    #[gtk::test]
    fn a_listing_that_ends_at_once_never_shows_the_line() {
        let line = LoadingLine::new();
        line.set_loading(true);
        line.set_loading(false);
        wait_for(APPEARANCE_DELAY * 2);
        assert!(!line.is_visible());
    }
}
