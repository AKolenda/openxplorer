// SPDX-License-Identifier: AGPL-3.0-only
//! The GitHub star prompt (UPD-023): a card at the top right of the window,
//! like the update notice, asking "Enjoying OpenXplorer?" From the sixth
//! start of the app it pops up once per session in the first window left
//! alone for [`IDLE_SECONDS`] while it has the focus, until it is dismissed.
//!
//! "Star on GitHub" opens the project's page in the default browser; it,
//! Dismiss and the close button each close the card and keep it from
//! popping up again. While the update notice shows, the prompt waits for
//! the next session. The app never asks GitHub anything by itself
//! (SAFE-002): the browser goes there only when the button is pressed.
//!
//! [`StarPrompt`] is a widget subclass the window's overlay holds; the
//! window decides when it shows ([`super::BrowserWindow`]'s
//! `wait_to_offer_star_prompt`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::update::REPOSITORY;

use super::notice_card;
use super::BrowserWindow;
use crate::icons::Icon;

/// How long the window waits without a key, click, scroll or touch before
/// the prompt pops up, in seconds.
const IDLE_SECONDS: u32 = 30;

mod imp {
    use std::cell::OnceCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::StarPrompt`].
    #[derive(Debug, Default)]
    pub(crate) struct StarPrompt {
        /// The card.
        pub(super) surface: OnceCell<gtk::Box>,
        /// Star on GitHub.
        pub(super) star_button: OnceCell<gtk::Button>,
        /// Dismiss.
        pub(super) dismiss_button: OnceCell<gtk::Button>,
        /// The close button.
        pub(super) close_button: OnceCell<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StarPrompt {
        const NAME: &'static str = "OxStarPrompt";
        type Type = super::StarPrompt;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }
    }

    impl ObjectImpl for StarPrompt {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(surface) = self.surface.get() {
                surface.unparent();
            }
        }
    }

    impl WidgetImpl for StarPrompt {}
}

glib::wrapper! {
    /// The GitHub star prompt, hidden until the window pops it up.
    pub(crate) struct StarPrompt(ObjectSubclass<imp::StarPrompt>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl StarPrompt {
    /// The card: a star, "Enjoying OpenXplorer?", Star on GitHub and
    /// Dismiss, and the close button.
    fn build(&self) {
        let card = notice_card::build(self, Icon::Star, &ox_core::i18n::gettext("Enjoying OpenXplorer?"));
        card.text.set_text(&ox_core::i18n::gettext(
            "Give us a star on GitHub to let us know!",
        ));
        card.primary.set_label(&ox_core::i18n::gettext("Star on GitHub"));
        card.secondary.set_label(&ox_core::i18n::gettext("Dismiss"));
        let imp = self.imp();
        imp.surface.set(card.surface).expect("constructed runs once");
        imp.star_button.set(card.primary).expect("constructed runs once");
        imp.dismiss_button
            .set(card.secondary)
            .expect("constructed runs once");
        imp.close_button.set(card.close).expect("constructed runs once");
    }

    /// Calls `star` when Star on GitHub is clicked and `dismiss` on
    /// Dismiss and the close button; the card closes first either way.
    fn connect_choices(&self, star: impl Fn() + 'static, dismiss: impl Fn() + 'static) {
        let imp = self.imp();
        let dismiss = Rc::new(dismiss);
        if let Some(button) = imp.star_button.get() {
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = prompt)]
                self,
                move |_| {
                    prompt.set_visible(false);
                    star();
                }
            ));
        }
        for button in [imp.dismiss_button.get(), imp.close_button.get()]
            .into_iter()
            .flatten()
        {
            let dismiss = Rc::clone(&dismiss);
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = prompt)]
                self,
                move |_| {
                    prompt.set_visible(false);
                    dismiss();
                }
            ));
        }
    }

    /// Clicks Star on GitHub, for tests.
    #[cfg(test)]
    pub(crate) fn click_star(&self) {
        if let Some(button) = self.imp().star_button.get() {
            button.emit_clicked();
        }
    }

    /// Clicks Dismiss, for tests.
    #[cfg(test)]
    pub(crate) fn click_dismiss(&self) {
        if let Some(button) = self.imp().dismiss_button.get() {
            button.emit_clicked();
        }
    }
}

/// The idle timer of a window waiting to offer the prompt, and what
/// restarts it. The window owns the input watch and the focus handler,
/// whose closures own this, so it goes with the window.
#[derive(Default)]
struct IdleWait {
    /// Fires once the window was left alone long enough.
    timer: Option<glib::SourceId>,
    /// Restarts the timer on input.
    input: glib::WeakRef<gtk::EventControllerLegacy>,
    /// Restarts the timer when the window gets the focus.
    activation: Option<glib::SignalHandlerId>,
}

impl BrowserWindow {
    /// Wires the star prompt's buttons and, while the prompt is due this
    /// session, waits for the window to be left alone to pop it up.
    pub(super) fn follow_star_prompt(&self) {
        let prompt = self.star_prompt();
        prompt.connect_choices(
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.context().dismiss_star_prompt();
                    window.open_web_page(REPOSITORY);
                }
            ),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.context().dismiss_star_prompt()
            ),
        );
        if self.context().star_prompt_due() {
            self.wait_to_offer_star_prompt();
        }
    }

    /// Pops the prompt up once the window has gone [`IDLE_SECONDS`]
    /// without a key, click, scroll or touch while it has the focus. Input
    /// and the window getting the focus start the wait over; a window
    /// without the focus when the time is up waits for the next.
    fn wait_to_offer_star_prompt(&self) {
        let wait = Rc::new(RefCell::new(IdleWait::default()));
        let restart = Rc::new(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            wait,
            move || {
                if let Some(timer) = wait.borrow_mut().timer.take() {
                    timer.remove();
                }
                let timer = glib::timeout_add_seconds_local_once(
                    IDLE_SECONDS,
                    glib::clone!(
                        #[weak]
                        window,
                        #[weak]
                        wait,
                        move || {
                            wait.borrow_mut().timer = None;
                            if !window.is_active() {
                                return;
                            }
                            window.stop_waiting_for_star_prompt(&wait);
                            window.offer_star_prompt();
                        }
                    ),
                );
                wait.borrow_mut().timer = Some(timer);
            }
        ));
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        input.connect_event(glib::clone!(
            #[strong]
            restart,
            move |_, event| {
                if matches!(
                    event.event_type(),
                    gdk::EventType::ButtonPress
                        | gdk::EventType::KeyPress
                        | gdk::EventType::Scroll
                        | gdk::EventType::TouchBegin
                ) {
                    restart();
                }
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(input.clone());
        let activation = self.connect_is_active_notify(glib::clone!(
            #[strong]
            restart,
            move |window| {
                if window.is_active() {
                    restart();
                }
            }
        ));
        {
            let mut wait = wait.borrow_mut();
            wait.input.set(Some(&input));
            wait.activation = Some(activation);
        }
        restart();
    }

    /// Stops waiting: drops the input watch and the focus handler, which
    /// hold the wait.
    fn stop_waiting_for_star_prompt(&self, wait: &RefCell<IdleWait>) {
        let mut wait = wait.borrow_mut();
        if let Some(input) = wait.input.upgrade() {
            self.remove_controller(&input);
        }
        if let Some(activation) = wait.activation.take() {
            self.disconnect(activation);
        }
    }

    /// Pops the prompt up in this window, unless another window took it
    /// this session or it was dismissed meanwhile. While the update notice
    /// shows in its place, it waits for the next session.
    pub(super) fn offer_star_prompt(&self) {
        if self.imp().update_notice.is_visible() {
            return;
        }
        if self.context().take_star_prompt() {
            self.star_prompt().set_visible(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;
    use ox_core::update::REPOSITORY;

    use crate::test_support::harness::{wait_until, Fixture, TestWindow};

    /// Counts `starts` starts of the app in `test`'s context, each saved
    /// before the next.
    fn count_starts(test: &TestWindow, starts: std::ops::RangeInclusive<u32>) {
        for start in starts {
            test.context.count_launch_for_star_prompt();
            wait_until("the start to be counted", || {
                test.context.settings_data().preferences.star_prompt_launches == start
            });
        }
    }

    /// The star prompt is due from the sixth start of the app and pops up
    /// once a session. Star on GitHub opens the project's page, only when
    /// pressed, and keeps the prompt from popping up again, as Dismiss
    /// does; after that, starts are no longer counted.
    ///
    /// parity: UPD-023
    #[gtk::test]
    fn the_star_prompt_pops_up_from_the_sixth_start_until_dismissed() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.context.record_launches();
        let prompt = test.window.star_prompt();

        count_starts(&test, 1..=5);
        test.window.offer_star_prompt();
        assert!(!prompt.is_visible(), "not before the sixth start");
        count_starts(&test, 6..=6);
        test.window.offer_star_prompt();
        assert!(prompt.is_visible(), "the sixth start pops it up");
        assert!(
            test.context.recorded_launches().is_empty(),
            "nothing opens by itself"
        );
        assert!(!test.context.star_prompt_due(), "once a session");

        prompt.click_star();
        assert!(!prompt.is_visible());
        assert_eq!(test.context.recorded_launches(), [REPOSITORY]);
        wait_until("the dismissal to be saved", || {
            test.context.settings_data().preferences.star_prompt_dismissed
        });
        test.context.count_launch_for_star_prompt();
        assert!(!test.context.star_prompt_due(), "never again");
        assert_eq!(test.context.settings_data().preferences.star_prompt_launches, 6);

        let other = TestWindow::open(&fixture.uri());
        count_starts(&other, 1..=6);
        other.window.offer_star_prompt();
        let prompt = other.window.star_prompt();
        prompt.click_dismiss();
        assert!(!prompt.is_visible());
        wait_until("the dismissal to be saved", || {
            other.context.settings_data().preferences.star_prompt_dismissed
        });
    }
}
