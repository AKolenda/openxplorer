// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties window: a top-level window of its own, as Windows 11's
//! Properties are, with its tabs and OK, Cancel and Apply at the bottom.
//!
//! Each Properties window stands alone: it never blocks the browser
//! window or another Properties window, and stays open while the user
//! works elsewhere. A tab that changes something registers a [`Pending`]
//! change; Apply is greyed until one of them has something to apply. OK
//! applies, then closes; Cancel and Escape close without applying, and
//! closing the window stops the work its tabs started (a folder being
//! counted, versions being looked up).
//!
//! What a tab asks of the browser window that opened it (open a snapshot
//! in a tab, restore a copy, Open with, rename) it asks through that
//! window's methods ([`PropertiesWindow::opener`]). The window takes none
//! of the browser window's actions or keyboard shortcuts, so Ctrl+F, F11
//! and the others do nothing here; F5 looks again in the tab shown, where
//! it can. Closing the window while OK or Apply runs waits for the change:
//! the window closes once it is made, and stays open with the message when
//! it fails. The same window, modal and without tabs, is the Security tab's
//! Edit and Advanced dialogs and Advanced Sharing.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::transfer::Cancellation;

use crate::icons::{self, Art, ArtImage, Icon};
use crate::window::BrowserWindow;

/// The width of a Properties window, as Windows 11 draws it at 100 %.
const WINDOW_WIDTH: i32 = 400;
/// The space around the tabs and their page (`.properties-notebook`).
const PAGE_MARGIN: i32 = 11;
/// The size of the item's picture in the title bar.
const TITLE_ART: i32 = 16;
/// The room kept above and below a window as tall as its screen, for the
/// desktop's panels.
const SCREEN_MARGIN: i32 = 48;
/// The close glyph of the title bar.
const CLOSE_GLYPH: i32 = 12;

/// What F5 runs, with the name of the tab shown.
type RefreshHandler = Box<dyn Fn(&str)>;

/// The future a pending change's [`Pending::apply`] returns: `Err` with
/// the message to show when the change could not be made.
pub(crate) type ApplyFuture = Pin<Box<dyn Future<Output = Result<(), String>>>>;

/// A change a tab holds until OK or Apply.
pub(crate) trait Pending {
    /// Whether there is anything to apply.
    fn has_changes(&self) -> bool;
    /// Makes the change. Called only when [`Self::has_changes`] is true.
    fn apply(self: Rc<Self>) -> ApplyFuture;
}

mod imp {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::transfer::Cancellation;

    use super::Pending;
    use crate::window::BrowserWindow;

    /// Private state of [`super::PropertiesWindow`].
    #[derive(Default)]
    pub(crate) struct PropertiesWindow {
        /// The title bar's text.
        pub(super) title: gtk::Label,
        /// The title bar's picture.
        pub(super) art: gtk::Box,
        /// The tabs.
        pub(super) notebook: gtk::Notebook,
        /// Scrolls the tabs when they are taller than the screen.
        pub(super) scroller: gtk::ScrolledWindow,
        /// How tall the window may be, once it is realized on a screen.
        pub(super) tallest: Cell<Option<i32>>,
        /// OK, Cancel and Apply.
        pub(super) ok: gtk::Button,
        pub(super) cancel: gtk::Button,
        pub(super) apply: gtk::Button,
        /// The changes the tabs hold.
        pub(super) pending: RefCell<Vec<Rc<dyn Pending>>>,
        /// How many changes at the end of `pending` are made last.
        pub(super) made_last: Cell<usize>,
        /// Work to stop when the window closes.
        pub(super) work: RefCell<Vec<Cancellation>>,
        /// Run when the window closes.
        pub(super) on_closed: RefCell<Vec<Box<dyn Fn()>>>,
        /// Run once OK or Apply finished, whether or not every change was
        /// made, so the tabs read the item again.
        pub(super) on_applied: RefCell<Vec<Box<dyn Fn()>>>,
        /// Run by F5, with the name of the tab shown.
        pub(super) on_refresh: RefCell<Vec<super::RefreshHandler>>,
        /// Set when the window was asked to close while a change ran.
        pub(super) close_after_apply: Cell<bool>,
        /// The browser window that opened it.
        pub(super) opener: glib::WeakRef<BrowserWindow>,
        /// True while OK or Apply runs.
        pub(super) is_applying: Cell<bool>,
        /// True once the window closed.
        pub(super) is_closed: Cell<bool>,
        /// True once the window was first shown.
        pub(super) has_shown: Cell<bool>,
        /// True when it shows a mount point as a drive, which has its own
        /// window beside the folder's (PROP-004).
        pub(super) shows_drive: Cell<bool>,
    }

    impl std::fmt::Debug for PropertiesWindow {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("PropertiesWindow")
                .field("title", &self.title.text())
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PropertiesWindow {
        const NAME: &'static str = "OxPropertiesWindow";
        type Type = super::PropertiesWindow;
        type ParentType = gtk::Window;
    }

    impl ObjectImpl for PropertiesWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for PropertiesWindow {
        /// Finds how tall the window may be on its screen.
        fn realize(&self) {
            self.obj().fit_to_screen();
            self.parent_realize();
        }

        /// Tabs that outgrew the screen scroll from the next frame on.
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let outgrown = self.tallest.get().is_some_and(|tallest| height > tallest);
            if outgrown && self.scroller.parent().is_none() {
                let window = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(window) = window.upgrade() {
                        window.scroll_tabs();
                    }
                });
            }
        }

        /// Starts with the tab row focused, as Windows does, so no
        /// selectable value starts selected.
        fn map(&self) {
            self.parent_map();
            // A dialog with one tab hides the tab row, which then takes no
            // focus: OK has it instead.
            if !self.has_shown.replace(true) && !self.notebook.grab_focus() {
                self.ok.grab_focus();
            }
        }
    }

    impl WindowImpl for PropertiesWindow {
        fn close_request(&self) -> glib::Propagation {
            // A change being made finishes first: the window closes after
            // it, or stays to show why it failed.
            if self.is_applying.get() {
                self.close_after_apply.set(true);
                return glib::Propagation::Stop;
            }
            if !self.is_closed.replace(true) {
                for work in self.work.take() {
                    work.cancel();
                }
                for closed in self.on_closed.take() {
                    closed();
                }
            }
            self.parent_close_request()
        }
    }
}

glib::wrapper! {
    /// A Properties window, or one of its dialogs.
    pub(crate) struct PropertiesWindow(ObjectSubclass<imp::PropertiesWindow>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager;
}

impl PropertiesWindow {
    /// A window titled `title`, with `art` in its title bar, opened from
    /// `opener`. Add tabs with [`Self::add_tab`], then present it.
    pub(crate) fn new(title: &str, art: Option<Art>, opener: Option<&BrowserWindow>) -> Self {
        let window: Self = glib::Object::new();
        window.set_title_text(title);
        if let Some(art) = art {
            window.imp().art.append(&ArtImage::new(art, TITLE_ART));
        }
        // The window joins no application, so the browser window's
        // keyboard shortcuts never reach it.
        if let Some(opener) = opener {
            window.imp().opener.set(Some(opener));
        }
        window
    }

    /// A modal dialog of `parent` (the Security tab's Edit and Advanced,
    /// Advanced Sharing): the same frame, without a tab row when it has one
    /// tab.
    pub(crate) fn dialog_of(parent: &PropertiesWindow, title: &str) -> Self {
        let dialog = Self::new(title, None, parent.opener().as_ref());
        dialog.set_transient_for(Some(parent));
        dialog.set_modal(true);
        dialog.set_destroy_with_parent(true);
        dialog.add_css_class("ox-properties-dialog");
        dialog
    }

    fn build(&self) {
        let imp = self.imp();
        self.set_resizable(false);
        self.set_default_width(WINDOW_WIDTH);
        self.set_hide_on_close(false);
        self.add_css_class("ox");
        self.add_css_class("ox-properties");
        self.set_titlebar(Some(&self.title_bar()));
        crate::window::follow_text_size_keys(self);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        imp.notebook.add_css_class("properties-notebook");
        imp.notebook.set_scrollable(false);
        imp.notebook.set_vexpand(true);
        // The tabs move into the scroller only once they outgrow the screen
        // (Self::scroll_tabs): a window that holds a scroller no longer
        // grows to the rows a tab reads after it shows.
        imp.scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        imp.scroller.set_propagate_natural_height(true);
        imp.scroller.set_vexpand(true);
        column.append(&imp.notebook);
        column.append(&self.footer());
        // Windows' Properties keep one width for every item: a long name,
        // location or value wraps or ends in "…" instead of widening it.
        self.set_child(Some(&super::fixed_width::FixedWidth::new(&column, WINDOW_WIDTH)));
        self.add_controller(crate::modal::escape_closes());
        self.add_controller(self.refresh_key());
        self.update_relation(&[gtk::accessible::Relation::LabelledBy(&[imp.title.upcast_ref()])]);
    }

    /// F5: looks again in the tab shown, before any other widget takes it.
    fn refresh_key(&self) -> gtk::ShortcutController {
        let window = self.downgrade();
        let action = gtk::CallbackAction::new(move |_, _| {
            if let Some(window) = window.upgrade() {
                let shown = window.selected_tab().unwrap_or_default();
                for refresh in window.imp().on_refresh.borrow().iter() {
                    refresh(&shown);
                }
            }
            glib::Propagation::Stop
        });
        let trigger = gtk::KeyvalTrigger::new(gtk::gdk::Key::F5, gtk::gdk::ModifierType::empty());
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        shortcuts
    }

    /// Runs `refresh` on F5, with the name of the tab shown.
    pub(crate) fn connect_refresh(&self, refresh: impl Fn(&str) + 'static) {
        self.imp().on_refresh.borrow_mut().push(Box::new(refresh));
    }

    /// Runs `applied` each time OK or Apply finished, whether or not every
    /// change was made.
    pub(crate) fn connect_applied(&self, applied: impl Fn() + 'static) {
        self.imp().on_applied.borrow_mut().push(Box::new(applied));
    }

    /// The item's picture, the title and the close button, as a handle the
    /// window is dragged by.
    fn title_bar(&self) -> gtk::WindowHandle {
        let imp = self.imp();
        let bar = gtk::Box::builder()
            .css_classes(["ox-titlebar", "properties-titlebar"])
            .build();
        imp.art.set_valign(gtk::Align::Center);
        imp.art.add_css_class("properties-title-art");
        bar.append(&imp.art);
        imp.title.set_xalign(0.0);
        imp.title.set_hexpand(true);
        imp.title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // A long name ends in "…" in the title bar, as in Windows, rather
        // than widening the window: the label asks for no more than a
        // character and takes the bar's room.
        imp.title.set_max_width_chars(1);
        imp.title.add_css_class("properties-title");
        bar.append(&imp.title);
        let close = gtk::Button::builder()
            .child(&icons::image(Icon::Dismiss, CLOSE_GLYPH))
            .css_classes(["caption", "close"])
            .focus_on_click(false)
            .tooltip_text(ox_core::i18n::gettext("Close"))
            .build();
        close.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext("Close"))]);
        close.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.close()
        ));
        bar.append(&close);
        gtk::WindowHandle::builder().child(&bar).build()
    }

    /// OK, Cancel and Apply, from the right, as Windows places them.
    fn footer(&self) -> gtk::Box {
        let imp = self.imp();
        let footer = gtk::Box::builder()
            .css_classes(["properties-footer"])
            .halign(gtk::Align::End)
            .build();
        for (button, label) in [
            (&imp.ok, ox_core::i18n::gettext("OK")),
            (&imp.cancel, ox_core::i18n::gettext("Cancel")),
            (&imp.apply, ox_core::i18n::gettext("Apply")),
        ] {
            button.set_label(&label);
            button.add_css_class("properties-button");
            footer.append(button);
        }
        imp.ok.add_css_class("suggested-action");
        imp.apply.set_sensitive(false);
        self.set_default_widget(Some(&imp.ok));
        imp.ok.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.press_ok()
        ));
        imp.cancel.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.close()
        ));
        imp.apply.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.press_apply()
        ));
        footer
    }

    /// Shows `art` in the title bar.
    pub(crate) fn set_art(&self, art: Art) {
        let holder = &self.imp().art;
        while let Some(child) = holder.first_child() {
            holder.remove(&child);
        }
        holder.append(&ArtImage::new(art, TITLE_ART));
    }

    /// Sets the title bar's text and the window's title.
    pub(crate) fn set_title_text(&self, title: &str) {
        self.imp().title.set_text(title);
        self.set_title(Some(title));
    }

    /// The title bar's text.
    pub(crate) fn title_text(&self) -> String {
        self.imp().title.text().to_string()
    }

    /// Makes `opener` the browser window this one acts through, as when the
    /// tab that opened it moved to `opener` and its window closed.
    pub(crate) fn set_opener(&self, opener: &BrowserWindow) {
        self.imp().opener.set(Some(opener));
    }

    /// The browser window that opened this one, while it is open.
    pub(crate) fn opener(&self) -> Option<BrowserWindow> {
        self.imp().opener.upgrade()
    }

    /// Marks the window as showing a mount point as a drive, not as the
    /// folder it is: each has its own window, as in Windows.
    pub(crate) fn set_shows_drive(&self, shows_drive: bool) {
        self.imp().shows_drive.set(shows_drive);
    }

    /// Whether the window shows a mount point as a drive.
    pub(crate) fn shows_drive(&self) -> bool {
        self.imp().shows_drive.get()
    }

    /// The Properties window `widget` is in.
    pub(crate) fn of(widget: &impl IsA<gtk::Widget>) -> Option<Self> {
        widget.root().and_downcast::<Self>()
    }

    /// Adds the tab `label`, named `name`, showing `page`.
    pub(crate) fn add_tab(&self, name: &str, label: &str, page: &impl IsA<gtk::Widget>) {
        let notebook = &self.imp().notebook;
        page.add_css_class("properties-page");
        // Windows' Properties keep their width; a page's wrapped rows are
        // measured at it, never at the width of a short tab row.
        page.set_size_request(WINDOW_WIDTH - 2 * PAGE_MARGIN - 2, -1);
        page.set_widget_name(name);
        let tab = gtk::Label::new(Some(label));
        notebook.append_page(page, Some(&tab));
        notebook.set_show_tabs(notebook.n_pages() > 1);
        notebook.set_show_border(true);
    }

    /// Keeps the window on the screen beside its parent or opener: tabs
    /// taller than that screen, as at a large text size, scroll instead of
    /// pushing OK, Cancel and Apply off it.
    fn fit_to_screen(&self) {
        let beside = self.transient_for().or_else(|| self.opener().map(Cast::upcast));
        let monitor = beside
            .and_then(|window| window.surface())
            .and_then(|surface| surface.display().monitor_at_surface(&surface));
        if let Some(monitor) = monitor {
            self.fit_to_height(monitor.geometry().height() - 2 * SCREEN_MARGIN);
        }
    }

    /// Lets the window be at most `height` tall: taller tabs scroll.
    pub(crate) fn fit_to_height(&self, height: i32) {
        self.imp().tallest.set(Some(height));
    }

    /// Holds the window at its tallest, its tabs scrolling inside it.
    fn scroll_tabs(&self) {
        let imp = self.imp();
        let Some(tallest) = imp.tallest.get() else {
            return;
        };
        let Some(column) = imp.notebook.parent().and_downcast::<gtk::Box>() else {
            return;
        };
        column.remove(&imp.notebook);
        imp.scroller.set_child(Some(&imp.notebook));
        column.prepend(&imp.scroller);
        crate::modal::cap_body(self.upcast_ref(), &imp.scroller, tallest);
        imp.scroller
            .set_min_content_height(imp.scroller.max_content_height());
    }

    /// The scrolling around the tabs.
    #[cfg(test)]
    pub(crate) fn scroller(&self) -> gtk::ScrolledWindow {
        self.imp().scroller.clone()
    }

    /// Leaves Apply out, for a dialog that has only OK and Cancel, as
    /// Windows' Change Icon has.
    pub(crate) fn hide_apply(&self) {
        self.imp().apply.set_visible(false);
    }

    /// Presses OK: makes the changes, then closes the window.
    pub(crate) fn accept(&self) {
        self.press_ok();
    }

    /// Shows the tab row although there is one tab, as Windows' Edit...
    /// dialog shows its Security tab.
    pub(crate) fn show_single_tab(&self) {
        self.imp().notebook.set_show_tabs(true);
    }

    /// Shows the tab named `name`, if there is one.
    pub(crate) fn select_tab(&self, name: &str) {
        let notebook = &self.imp().notebook;
        let found = (0..notebook.n_pages()).find(|page| {
            notebook
                .nth_page(Some(*page))
                .is_some_and(|child| child.widget_name() == name)
        });
        if let Some(page) = found {
            notebook.set_current_page(Some(page));
        }
    }

    /// The name of the tab shown.
    pub(crate) fn selected_tab(&self) -> Option<String> {
        let notebook = &self.imp().notebook;
        let page = notebook.nth_page(notebook.current_page())?;
        Some(page.widget_name().to_string())
    }

    /// The tabs' labels, left to right.
    #[cfg(test)]
    pub(crate) fn tab_labels(&self) -> Vec<String> {
        let notebook = &self.imp().notebook;
        (0..notebook.n_pages())
            .filter_map(|page| notebook.nth_page(Some(page)))
            .filter_map(|page| notebook.tab_label_text(&page))
            .map(String::from)
            .collect()
    }

    /// The page of the tab named `name`.
    pub(crate) fn tab_page(&self, name: &str) -> Option<gtk::Widget> {
        let notebook = &self.imp().notebook;
        (0..notebook.n_pages())
            .filter_map(|page| notebook.nth_page(Some(page)))
            .find(|page| page.widget_name() == name)
    }

    /// Runs `shown` each time another tab is shown, with its name.
    pub(crate) fn connect_tab_shown(&self, shown: impl Fn(&str) + 'static) {
        self.imp().notebook.connect_switch_page(move |_, page, _| {
            shown(page.widget_name().as_str());
        });
    }

    /// Registers a change a tab holds until OK or Apply.
    pub(crate) fn add_pending(&self, pending: Rc<dyn Pending>) {
        let imp = self.imp();
        let mut changes = imp.pending.borrow_mut();
        let at = changes.len() - imp.made_last.get();
        changes.insert(at, pending);
    }

    /// Registers a change made after every other, as Location's, which
    /// moves the folder's files once the other tabs changed them.
    pub(crate) fn add_pending_last(&self, pending: Rc<dyn Pending>) {
        let imp = self.imp();
        imp.pending.borrow_mut().push(pending);
        imp.made_last.set(imp.made_last.get() + 1);
    }

    /// Greys Apply, or not, after a tab's control changed.
    pub(crate) fn changed(&self) {
        let imp = self.imp();
        imp.apply
            .set_sensitive(!imp.is_applying.get() && self.has_changes());
    }

    /// Whether any tab holds a change.
    pub(crate) fn has_changes(&self) -> bool {
        self.imp()
            .pending
            .borrow()
            .iter()
            .any(|pending| pending.has_changes())
    }

    /// Stops `work` when the window closes.
    pub(crate) fn track_work(&self, work: Cancellation) {
        if self.imp().is_closed.get() {
            work.cancel();
        } else {
            self.imp().work.borrow_mut().push(work);
        }
    }

    /// Runs `closed` once, when the window closes.
    pub(crate) fn connect_closed_once(&self, closed: impl Fn() + 'static) {
        self.imp().on_closed.borrow_mut().push(Box::new(closed));
    }

    /// True once the window closed.
    pub(crate) fn is_closed(&self) -> bool {
        self.imp().is_closed.get()
    }

    fn press_ok(&self) {
        // OK while a change is being made closes the window once it is
        // made, as closing it then does; the change is not made twice.
        if self.imp().is_applying.get() {
            self.imp().close_after_apply.set(true);
            return;
        }
        let window = self.clone();
        glib::spawn_future_local(async move {
            if window.apply_changes().await {
                window.close();
            }
        });
    }

    fn press_apply(&self) {
        let window = self.clone();
        glib::spawn_future_local(async move {
            window.apply_changes().await;
        });
    }

    /// Applies every change in tab order, stopping at the first that
    /// fails, whose message is shown. True when nothing failed.
    pub(crate) async fn apply_changes(&self) -> bool {
        let imp = self.imp();
        if imp.is_applying.replace(true) {
            return false;
        }
        self.set_buttons_sensitive(false);
        let pending: Vec<Rc<dyn Pending>> = imp.pending.borrow().clone();
        let mut failure = None;
        for change in pending.iter().filter(|change| change.has_changes()) {
            if let Err(message) = Rc::clone(change).apply().await {
                failure = Some(message);
                break;
            }
        }
        imp.is_applying.set(false);
        self.set_buttons_sensitive(true);
        for applied in imp.on_applied.borrow().iter() {
            applied();
        }
        let close_after = imp.close_after_apply.replace(false);
        if let Some(message) = failure {
            self.changed();
            self.show_error(&message);
            return false;
        }
        self.changed();
        if close_after {
            self.close();
        }
        true
    }

    fn set_buttons_sensitive(&self, sensitive: bool) {
        let imp = self.imp();
        imp.ok.set_sensitive(sensitive);
        imp.cancel.set_sensitive(sensitive);
        imp.apply.set_sensitive(sensitive && self.has_changes());
    }

    /// Shows `message` in a message box over the window, as Windows shows
    /// a failed change, with OK.
    pub(crate) fn show_error(&self, message: &str) {
        // A closed window shows nothing more.
        if self.is_closed() {
            return;
        }
        let window = self.clone();
        let message = message.to_owned();
        glib::spawn_future_local(async move {
            crate::dialog::show_message(&window, &window.title_text(), &message).await;
        });
    }

    /// Apply, for tests.
    #[cfg(test)]
    pub(crate) fn apply_button(&self) -> gtk::Button {
        self.imp().apply.clone()
    }

    /// OK, for tests.
    #[cfg(test)]
    pub(crate) fn ok_button(&self) -> gtk::Button {
        self.imp().ok.clone()
    }

    /// The labels of the footer buttons shown, for tests.
    #[cfg(test)]
    pub(crate) fn button_labels(&self) -> Vec<String> {
        let imp = self.imp();
        [&imp.ok, &imp.cancel, &imp.apply]
            .iter()
            .filter(|button| button.is_visible())
            .filter_map(|button| button.label())
            .map(String::from)
            .collect()
    }
}

/// A divider line between groups of rows, as Windows draws them.
pub(crate) fn divider() -> gtk::Separator {
    gtk::Separator::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["properties-divider"])
        .build()
}

/// A grid of names and values in two columns (`.properties-rows`).
#[derive(Debug, Clone)]
pub(crate) struct Rows {
    grid: gtk::Grid,
    line: std::rc::Rc<std::cell::Cell<i32>>,
}

impl Default for Rows {
    fn default() -> Self {
        Self::new()
    }
}

impl Rows {
    pub(crate) fn new() -> Self {
        let grid = gtk::Grid::builder().css_classes(["properties-rows"]).build();
        Self {
            grid,
            line: std::rc::Rc::default(),
        }
    }

    /// The grid.
    pub(crate) fn widget(&self) -> &gtk::Grid {
        &self.grid
    }

    /// Adds `name:` and the selectable `value`; returns the value.
    pub(crate) fn add(&self, name: &str, value: &str) -> gtk::Label {
        let value = gtk::Label::builder()
            .label(value)
            .xalign(0.0)
            .hexpand(true)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["property-value"])
            .build();
        self.add_control(name, &value);
        value
    }

    /// Adds `name:` and the selectable `value` on one line, ending in "…"
    /// when it is wider than the window, with the whole value as its
    /// tooltip: Windows' one-line read-only fields, such as Location and
    /// Object name, which keep the window's size. Selecting it copies
    /// the whole value.
    pub(crate) fn add_line(&self, name: &str, value: &str) -> gtk::Label {
        let value = gtk::Label::builder()
            .label(value)
            .xalign(0.0)
            .hexpand(true)
            .selectable(true)
            .single_line_mode(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .tooltip_text(value)
            .css_classes(["property-value"])
            .build();
        self.add_control(name, &value);
        value
    }

    /// Adds `name:` before `control`.
    pub(crate) fn add_control(&self, name: &str, control: &impl IsA<gtk::Widget>) -> gtk::Label {
        let line = self.line.get();
        let label = gtk::Label::builder()
            .label(ox_core::i18n::format_message("{name}:", &[("name", name)]))
            .xalign(0.0)
            .yalign(0.0)
            .valign(gtk::Align::Start)
            .css_classes(["property-name"])
            .build();
        self.grid.attach(&label, 0, line, 1, 1);
        self.grid.attach(control, 1, line, 1, 1);
        control
            .upcast_ref::<gtk::Widget>()
            .update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
        self.line.set(line + 1);
        label
    }
}

/// A group box with a title, as Windows' Sharing and Customize tabs draw
/// them (`.properties-group`).
pub(crate) fn group(title: &str) -> (gtk::Frame, gtk::Box) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["properties-group-content"])
        .build();
    let frame = gtk::Frame::builder()
        .label(title)
        .child(&content)
        .css_classes(["properties-group"])
        .build();
    (frame, content)
}

/// A wrapped paragraph of plain text.
pub(crate) fn paragraph(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(48)
        .css_classes(["properties-text"])
        .build()
}

/// A Windows push button, `Change...` or `Edit...`.
pub(crate) fn push_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .halign(gtk::Align::Start)
        .css_classes(["properties-button"])
        .build()
}

/// A page of a tab: a vertical box with the tab's padding.
pub(crate) fn page() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build()
}

/// A Windows push button with `glyph` and `label`, for the Location tab,
/// the mount assistant and the permissions of several items.
pub(crate) fn glyph_button(label: &str, glyph: Icon) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icons::image(glyph, 16));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Start)
        .css_classes(["properties-button"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}
