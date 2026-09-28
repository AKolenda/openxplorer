// SPDX-License-Identifier: AGPL-3.0-only
//! "Open with": choosing an installed application for a file or folder.
//!
//! Ports `openWithDialog` in `desktop/ui/app.js` (OPEN-011, OPEN-012):
//! "Find an application" filters the list by name, "Show all installed
//! applications" (ticked for folders) lists every application, and
//! "Always use this app for this file type" (never for folders) makes the
//! choice the default. The default, or the first application that can
//! open the item, is chosen when the list appears, and Open stays
//! disabled until one is chosen.
//!
//! [`OpenWithDialog`] is a `GtkWindow` subclass laid out by the template
//! `resources/ui/open-with-dialog.ui`.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::applications::{
    list_applications_in_background, prepare_launch, ApplicationChoice, ApplicationList, ApplicationScope,
    DefaultChoice, OpenWithError, PreparedLaunch,
};
use crate::icons::{self, Icon};

/// The glyph of each application row (app.js drew its grid icon at 25).
const ROW_GLYPH: i32 = 24;

/// The item Open with was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenWithSubject {
    /// Its location.
    pub(crate) uri: String,
    /// Its name, which the dialog shows.
    pub(crate) name: String,
    /// It is a folder: every application is listed, and its default is
    /// never changed.
    pub(crate) is_folder: bool,
}

/// Starts the chosen application: the desktop in the app, a recorder in
/// tests, which must never start a real application.
pub(crate) type Launcher =
    Box<dyn Fn(&str, &PreparedLaunch, DefaultChoice) -> Result<&'static str, OpenWithError>>;

/// Shows a message in the window that opened the dialog.
type Report = Box<dyn Fn(&str)>;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::{Launcher, OpenWithSubject, Report};
    use crate::integration::applications::ApplicationList;

    /// Private state of [`super::OpenWithDialog`].
    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/open-with-dialog.ui")]
    pub(crate) struct OpenWithDialog {
        #[template_child]
        pub(super) item_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) filter_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub(super) application_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub(super) show_all_check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub(super) make_default_check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub(super) status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) open_button: TemplateChild<gtk::Button>,
        /// The item, set when the dialog is made.
        pub(super) subject: OnceCell<OpenWithSubject>,
        /// Starts the chosen application.
        pub(super) launcher: OnceCell<Launcher>,
        /// Shows the result in the window.
        pub(super) report: OnceCell<Report>,
        /// The applications as last listed.
        pub(super) applications: RefCell<Option<ApplicationList>>,
        /// The desktop ID of each row shown, in row order.
        pub(super) shown_ids: RefCell<Vec<String>>,
        /// The chosen application's desktop ID.
        pub(super) chosen: RefCell<Option<String>>,
    }

    impl std::fmt::Debug for OpenWithDialog {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("OpenWithDialog")
                .field("subject", &self.subject)
                .field("chosen", &self.chosen)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OpenWithDialog {
        const NAME: &'static str = "OxOpenWithDialog";
        type Type = super::OpenWithDialog;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
            dialog.init_template();
        }
    }

    impl ObjectImpl for OpenWithDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().connect_controls();
        }
    }

    impl WidgetImpl for OpenWithDialog {}
    impl WindowImpl for OpenWithDialog {}
}

glib::wrapper! {
    /// The Open with dialog.
    pub(crate) struct OpenWithDialog(ObjectSubclass<imp::OpenWithDialog>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager;
}

impl OpenWithDialog {
    /// Opens the dialog for `subject` over `parent` and lists the
    /// applications. `launcher` starts the chosen one; `report` shows the
    /// outcome in the window.
    pub(crate) fn present_for(
        parent: &impl IsA<gtk::Window>,
        subject: OpenWithSubject,
        launcher: Launcher,
        report: impl Fn(&str) + 'static,
    ) -> Self {
        let dialog: Self = glib::Object::builder().property("transient-for", parent).build();
        let imp = dialog.imp();
        imp.item_label.set_text(&subject.name);
        imp.make_default_check.set_visible(!subject.is_folder);
        let is_folder = subject.is_folder;
        imp.subject.set(subject).expect("a new dialog has no subject");
        assert!(imp.launcher.set(launcher).is_ok(), "a new dialog has no launcher");
        let report: Report = Box::new(report);
        assert!(imp.report.set(report).is_ok(), "a new dialog has no report");
        dialog.present();
        if is_folder {
            // A folder lists every application; ticking the box lists them.
            imp.show_all_check.set_active(true);
        } else {
            dialog.fetch();
        }
        dialog
    }

    fn subject(&self) -> &OpenWithSubject {
        self.imp()
            .subject
            .get()
            .expect("OpenWithDialog::present_for sets the subject")
    }

    fn connect_controls(&self) {
        let imp = self.imp();
        imp.filter_entry.connect_changed(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.show_applications()
        ));
        imp.show_all_check.connect_toggled(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.fetch()
        ));
        imp.application_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_, row| dialog.choose_row(row)
        ));
        imp.cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.close()
        ));
        imp.open_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.open_chosen()
        ));
        self.add_controller(crate::modal::escape_closes());
    }

    /// Lists the applications again ("Finding installed applications…").
    fn fetch(&self) {
        let imp = self.imp();
        imp.status_label.set_text("Finding installed applications…");
        let scope = if imp.show_all_check.is_active() {
            ApplicationScope::AllInstalled
        } else {
            ApplicationScope::Recommended
        };
        let uri = self.subject().uri.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let listed = list_applications_in_background(uri, scope).await;
                dialog.show_list(listed);
            }
        ));
    }

    fn show_list(&self, listed: Result<ApplicationList, OpenWithError>) {
        let imp = self.imp();
        let list = match listed {
            Ok(list) => list,
            Err(error) => {
                imp.status_label.set_text(&error.to_string());
                return;
            }
        };
        let preselected = list.preselected().map(|choice| choice.id.clone());
        imp.chosen.replace(preselected);
        imp.applications.replace(Some(list));
        imp.status_label.set_text("Choose an installed application.");
        self.show_applications();
    }

    /// Shows the applications whose name has the filter text, the chosen
    /// one selected (`render`).
    fn show_applications(&self) {
        let imp = self.imp();
        let list = &*imp.application_list;
        list.remove_all();
        let query = imp.filter_entry.text().to_lowercase();
        let applications = imp.applications.borrow();
        let choices = applications.iter().flat_map(|list| list.choices.iter());
        let visible: Vec<&ApplicationChoice> = choices
            .filter(|choice| choice.name.to_lowercase().contains(&query))
            .collect();
        // Recorded before the rows exist: selecting a row looks its
        // application up here.
        let shown_ids = visible.iter().map(|choice| choice.id.clone()).collect();
        imp.shown_ids.replace(shown_ids);
        let chosen = imp.chosen.borrow().clone();
        for choice in &visible {
            let row = application_row(choice);
            list.append(&row);
            if chosen.as_deref() == Some(choice.id.as_str()) {
                list.select_row(Some(&row));
            }
        }
        if visible.is_empty() && applications.is_some() {
            list.append(&empty_row());
        }
        imp.open_button.set_sensitive(imp.chosen.borrow().is_some());
    }

    fn choose_row(&self, row: Option<&gtk::ListBoxRow>) {
        let imp = self.imp();
        let Some(row) = row else {
            return;
        };
        let index = usize::try_from(row.index()).ok();
        let id = index.and_then(|index| imp.shown_ids.borrow().get(index).cloned());
        if id.is_some() {
            imp.chosen.replace(id);
            imp.open_button.set_sensitive(true);
        }
    }

    /// Open: checks the chosen application again, launches it and closes;
    /// a failure stays in the dialog.
    fn open_chosen(&self) {
        let imp = self.imp();
        let Some(app_id) = imp.chosen.borrow().clone() else {
            return;
        };
        let default = if imp.make_default_check.is_active() && !self.subject().is_folder {
            DefaultChoice::MakeDefault
        } else {
            DefaultChoice::Keep
        };
        imp.open_button.set_sensitive(false);
        let uri = self.subject().uri.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let prepared = prepare_launch(uri, app_id.clone()).await;
                dialog.finish_open(&app_id, prepared, default);
            }
        ));
    }

    fn finish_open(
        &self,
        app_id: &str,
        prepared: Result<PreparedLaunch, OpenWithError>,
        default: DefaultChoice,
    ) {
        let imp = self.imp();
        let launcher = imp.launcher.get().expect("present_for sets the launcher");
        let launched = prepared.and_then(|prepared| launcher(app_id, &prepared, default));
        match launched {
            Ok(message) => {
                if let Some(report) = imp.report.get() {
                    report(message);
                }
                self.close();
            }
            Err(error) => {
                imp.status_label.set_text(&error.to_string());
                imp.open_button.set_sensitive(true);
            }
        }
    }

    /// The names of the listed applications, for tests.
    #[cfg(test)]
    pub(crate) fn shown_names(&self) -> Vec<String> {
        let applications = self.imp().applications.borrow();
        let ids = self.imp().shown_ids.borrow();
        let choices = applications.iter().flat_map(|list| list.choices.iter());
        choices
            .filter(|choice| ids.contains(&choice.id))
            .map(|choice| choice.name.clone())
            .collect()
    }

    /// The status line, for tests.
    #[cfg(test)]
    pub(crate) fn status(&self) -> String {
        self.imp().status_label.text().to_string()
    }

    /// The chosen application's desktop ID, for tests.
    #[cfg(test)]
    pub(crate) fn chosen(&self) -> Option<String> {
        self.imp().chosen.borrow().clone()
    }

    /// Types `text` into "Find an application", for tests.
    #[cfg(test)]
    pub(crate) fn type_filter(&self, text: &str) {
        self.imp().filter_entry.set_text(text);
    }

    /// Whether "Always use this app" shows, and Open is enabled, for tests.
    #[cfg(test)]
    pub(crate) fn controls_state(&self) -> (bool, bool) {
        let imp = self.imp();
        (
            imp.make_default_check.is_visible(),
            imp.open_button.is_sensitive(),
        )
    }

    /// Clicks Open, for tests.
    #[cfg(test)]
    pub(crate) fn click_open(&self) {
        self.imp().open_button.emit_clicked();
    }
}

/// The row of one application: its glyph, its name and why it is offered.
/// An application that cannot open the item is shown but cannot be chosen.
fn application_row(choice: &ApplicationChoice) -> gtk::ListBoxRow {
    let name = gtk::Label::builder().label(&choice.name).xalign(0.0).build();
    name.add_css_class("app-name");
    let note = gtk::Label::builder().label(choice.note()).xalign(0.0).build();
    note.add_css_class("app-note");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.append(&name);
    text.append(&note);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&icons::image(Icon::Apps, ROW_GLYPH));
    content.append(&text);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.add_css_class("app-choice");
    row.set_sensitive(choice.is_available);
    row
}

/// The row shown when the filter matches nothing.
fn empty_row() -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some("No matching installed applications."));
    label.add_css_class("apps-empty");
    gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(false)
        .selectable(false)
        .build()
}
