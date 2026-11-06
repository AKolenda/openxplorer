// SPDX-License-Identifier: AGPL-3.0-only
//! "Open with": choosing an installed application for a file or folder,
//! laid out as Windows 11's app chooser (right-click > Open with > Choose
//! another app).
//!
//! "Select an app to open this .txt file" heads the dialog. The current
//! default comes first, then "Suggested apps", the applications registered
//! for the item's type, then "More apps", an expander that lists every
//! other installed application (OPEN-011's "Show all installed
//! applications", open for folders). "Find an application" filters all of
//! them by name (OPEN-011). "Just once" opens the item with the chosen
//! application, as does double-clicking it; "Always" also makes it the
//! default for the file's type (OPEN-012) and is never offered for folders.
//! Nothing changes a default otherwise.
//!
//! "Choose an app on your PC" picks a program file: an installed launcher
//! (`.desktop`) is chosen in the list, and an executable becomes the
//! command line, which "Use a custom command…" also opens (OPEN-014). A
//! command runs just once: it never becomes a default.
//!
//! [`OpenWithDialog`] is a `GtkWindow` subclass laid out by the template
//! `resources/ui/open-with-dialog.ui`.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::applications::{
    application_image, list_applications_in_background, prepare_launch, prepare_target, program_file,
    ApplicationChoice, ApplicationList, ApplicationScope, DefaultChoice, OpenWithError, PreparedLaunch,
    ProgramFile,
};
use super::custom_command::{run_custom_command, CustomCommand};
use crate::icons::{self, Icon};

/// The glyph of each application row (app.js drew its grid icon at 25).
const ROW_GLYPH: i32 = 24;

/// The item Open with was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenWithSubject {
    /// Its location.
    pub(crate) uri: String,
    /// Its name, which the heading's extension comes from and screen
    /// readers read as the dialog's description.
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

/// The lists of the dialog, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    /// "Current default".
    Default,
    /// "Suggested apps": registered for the item's type.
    Suggested,
    /// "More apps": every other installed application.
    More,
}

impl Section {
    const ALL: [Self; 3] = [Self::Default, Self::Suggested, Self::More];

    fn index(self) -> usize {
        self as usize
    }

    /// The section `choice` belongs in.
    fn of(choice: &ApplicationChoice) -> Self {
        if choice.is_default {
            Self::Default
        } else if choice.is_recommended {
            Self::Suggested
        } else {
            Self::More
        }
    }
}

/// The dialog's heading for an item named `name`: "Select an app to open
/// this .txt file", "… this file" without an extension (a hidden file's
/// leading dot is not one), "… this folder" for a folder.
pub(crate) fn heading(name: &str, is_folder: bool) -> String {
    if is_folder {
        return ox_core::i18n::gettext("Select an app to open this folder");
    }
    let extension = name
        .rfind('.')
        .filter(|&dot| dot > 0 && dot + 1 < name.len())
        .map(|dot| &name[dot..]);
    match extension {
        Some(extension) => ox_core::i18n::format_message(
            "Select an app to open this {extension} file",
            &[("extension", extension)],
        ),
        None => ox_core::i18n::gettext("Select an app to open this file"),
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::{Launcher, OpenWithSubject, Report};
    use crate::integration::applications::ApplicationList;

    /// Private state of [`super::OpenWithDialog`].
    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/open-with-dialog.ui")]
    pub(crate) struct OpenWithDialog {
        /// The scrolling body, capped to the parent window's height.
        #[template_child]
        pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub(super) heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) filter_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub(super) default_heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) default_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub(super) suggested_heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) suggested_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub(super) more_expander: TemplateChild<gtk::Expander>,
        #[template_child]
        pub(super) more_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub(super) choose_program_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) custom_command_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) command_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub(super) command_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub(super) terminal_check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub(super) keep_open_check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub(super) status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub(super) just_once_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) always_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        /// The item, set when the dialog is made.
        pub(super) subject: OnceCell<OpenWithSubject>,
        /// Starts the chosen application.
        pub(super) launcher: OnceCell<Launcher>,
        /// Shows the result in the window.
        pub(super) report: OnceCell<Report>,
        /// The applications as last listed.
        pub(super) applications: RefCell<Option<ApplicationList>>,
        /// The applications are being listed.
        pub(super) loading: Cell<bool>,
        /// The last list holds every installed application.
        pub(super) listed_all: Cell<bool>,
        /// The desktop IDs of the rows of each section, in row order.
        pub(super) shown_ids: RefCell<[Vec<String>; 3]>,
        /// The chosen application's desktop ID.
        pub(super) chosen: RefCell<Option<String>>,
        /// An installed launcher picked with "Choose an app on your PC"
        /// that is chosen once every application is listed.
        pub(super) picked: RefCell<Option<String>>,
        /// An application is starting: the buttons wait.
        pub(super) busy: Cell<bool>,
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
            crate::i18n::translate_template(&*self.obj(), "open-with-dialog.ui");
            self.obj().connect_controls();
        }
    }

    impl WidgetImpl for OpenWithDialog {
        /// Fits the dialog to its parent window before its first frame, as
        /// it is realized when it shows.
        fn realize(&self) {
            crate::modal::fit_to_parent(&*self.obj(), &self.scroller);
            self.parent_realize();
        }
    }

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
        crate::window::follow_text_size_keys(&dialog);
        let imp = dialog.imp();
        imp.heading.set_text(&heading(&subject.name, subject.is_folder));
        dialog.update_property(&[gtk::accessible::Property::Description(&subject.name)]);
        imp.always_button.set_visible(!subject.is_folder);
        let is_folder = subject.is_folder;
        imp.subject.set(subject).expect("a new dialog has no subject");
        assert!(imp.launcher.set(launcher).is_ok(), "a new dialog has no launcher");
        let report: Report = Box::new(report);
        assert!(imp.report.set(report).is_ok(), "a new dialog has no report");
        dialog.present();
        if is_folder {
            // A folder lists every application; opening More apps lists them.
            imp.more_expander.set_expanded(true);
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

    fn list(&self, section: Section) -> gtk::ListBox {
        let imp = self.imp();
        match section {
            Section::Default => imp.default_list.get(),
            Section::Suggested => imp.suggested_list.get(),
            Section::More => imp.more_list.get(),
        }
    }

    fn connect_controls(&self) {
        let imp = self.imp();
        imp.filter_entry.connect_changed(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.show_applications()
        ));
        imp.more_expander.connect_expanded_notify(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |expander| {
                if expander.is_expanded() && !dialog.imp().listed_all.get() {
                    dialog.fetch();
                }
            }
        ));
        for section in Section::ALL {
            let list = self.list(section);
            list.connect_row_selected(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |_, row| dialog.choose_row(section, row)
            ));
            // Double-click or Enter on an application: Just once.
            list.connect_row_activated(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |_, row| {
                    dialog.choose_row(section, Some(row));
                    if dialog.imp().chosen.borrow().is_some() {
                        dialog.open_chosen(DefaultChoice::Keep);
                    }
                }
            ));
        }
        imp.choose_program_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.choose_program()
        ));
        imp.custom_command_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.show_command(None)
        ));
        imp.command_entry.connect_changed(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.update_buttons()
        ));
        imp.command_entry.connect_activate(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.open_chosen(DefaultChoice::Keep)
        ));
        imp.terminal_check
            .bind_property("active", &*imp.keep_open_check, "sensitive")
            .sync_create()
            .build();
        imp.cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.close()
        ));
        imp.just_once_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.open_chosen(DefaultChoice::Keep)
        ));
        imp.always_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.open_chosen(DefaultChoice::MakeDefault)
        ));
        self.add_controller(crate::modal::escape_closes());
    }

    /// Shows `text` under the lists, or hides the line for `None`.
    fn set_status(&self, text: Option<&str>) {
        let label = &self.imp().status_label;
        label.set_text(text.unwrap_or_default());
        label.set_visible(text.is_some());
    }

    /// Lists the applications again ("Finding installed applications…"):
    /// every installed one while More apps is open.
    fn fetch(&self) {
        let imp = self.imp();
        imp.loading.set(true);
        self.set_status(Some(&ox_core::i18n::gettext("Finding installed applications…")));
        let scope = if imp.more_expander.is_expanded() {
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
                dialog.show_list(listed, scope);
            }
        ));
    }

    fn show_list(&self, listed: Result<ApplicationList, OpenWithError>, scope: ApplicationScope) {
        let imp = self.imp();
        imp.loading.set(false);
        let list = match listed {
            Ok(list) => list,
            Err(error) => {
                self.set_status(Some(&error.to_string()));
                return;
            }
        };
        imp.listed_all
            .set(scope == ApplicationScope::AllInstalled || list.is_folder);
        let can_open = |id: &str| {
            list.choices
                .iter()
                .any(|choice| choice.id == id && choice.is_available)
        };
        let mut status = None;
        let chosen = if let Some(picked) = imp.picked.take() {
            if !can_open(&picked) {
                status = Some(OpenWithError::Unavailable.to_string());
            }
            Some(picked).filter(|id| can_open(id))
        } else {
            imp.chosen.borrow().clone().filter(|id| can_open(id))
        };
        let commanding = imp.command_box.is_visible();
        let chosen = match chosen {
            Some(chosen) => Some(chosen),
            None if commanding => None,
            None => list.preselected().map(|choice| choice.id.clone()),
        };
        imp.chosen.replace(chosen);
        let first_listing = imp.applications.replace(Some(list)).is_none();
        self.set_status(status.as_deref());
        self.show_applications();
        if first_listing {
            // The keyboard starts on the chosen application, as in Windows.
            let selected = Section::ALL
                .into_iter()
                .find_map(|section| self.list(section).selected_row());
            if let Some(row) = selected {
                row.grab_focus();
            }
        }
    }

    /// Shows the applications whose name has the filter text in their
    /// sections, the chosen one selected (`render`).
    fn show_applications(&self) {
        let imp = self.imp();
        for section in Section::ALL {
            self.list(section).remove_all();
        }
        let query = imp.filter_entry.text().to_lowercase();
        let applications = imp.applications.borrow();
        let choices = applications.iter().flat_map(|list| list.choices.iter());
        let mut sections: [Vec<&ApplicationChoice>; 3] = Default::default();
        for choice in choices.filter(|choice| choice.name.to_lowercase().contains(&query)) {
            sections[Section::of(choice).index()].push(choice);
        }
        // Recorded before the rows exist: selecting a row looks its
        // application up here.
        let shown_ids = sections
            .each_ref()
            .map(|shown| shown.iter().map(|choice| choice.id.clone()).collect());
        imp.shown_ids.replace(shown_ids);
        let chosen = imp.chosen.borrow().clone();
        for section in Section::ALL {
            let list = self.list(section);
            for choice in &sections[section.index()] {
                let row = application_row(choice);
                list.append(&row);
                if chosen.as_deref() == Some(choice.id.as_str()) {
                    list.select_row(Some(&row));
                }
            }
        }
        let has_default = !sections[Section::Default.index()].is_empty();
        imp.default_heading.set_visible(has_default);
        imp.default_list.set_visible(has_default);
        let has_suggested = !sections[Section::Suggested.index()].is_empty();
        imp.suggested_heading.set_visible(has_suggested);
        if sections.iter().all(Vec::is_empty) && applications.is_some() {
            imp.suggested_list.append(&empty_row());
        }
        self.update_buttons();
    }

    /// The typed command, if the command line is open and not empty, and
    /// how to run it.
    fn custom_command(&self) -> Option<CustomCommand> {
        let imp = self.imp();
        if !imp.command_box.is_visible() {
            return None;
        }
        let text = imp.command_entry.text().trim().to_owned();
        (!text.is_empty()).then(|| CustomCommand {
            text,
            in_terminal: imp.terminal_check.is_active(),
            keep_open: imp.terminal_check.is_active() && imp.keep_open_check.is_active(),
        })
    }

    /// Just once works with an application chosen or a command typed;
    /// Always only with an application, for a file.
    fn update_buttons(&self) {
        let imp = self.imp();
        let idle = !imp.busy.get();
        let has_app = imp.chosen.borrow().is_some();
        let has_command = self.custom_command().is_some();
        imp.just_once_button
            .set_sensitive(idle && (has_app || has_command));
        let is_folder = imp.subject.get().is_some_and(|subject| subject.is_folder);
        imp.always_button
            .set_sensitive(idle && has_app && !has_command && !is_folder);
    }

    /// Chooses the application of `row` in `section`, and clears the
    /// selection of the other lists and the command line.
    fn choose_row(&self, section: Section, row: Option<&gtk::ListBoxRow>) {
        let imp = self.imp();
        let Some(row) = row else {
            return;
        };
        let index = usize::try_from(row.index()).ok();
        let id = index.and_then(|index| imp.shown_ids.borrow()[section.index()].get(index).cloned());
        if id.is_none() {
            return;
        }
        imp.chosen.replace(id);
        for other in Section::ALL.into_iter().filter(|&other| other != section) {
            self.list(other).unselect_all();
        }
        imp.command_box.set_visible(false);
        self.update_buttons();
    }

    /// Opens the command line with `text`, or as it was, instead of an
    /// application, and puts the keyboard there.
    fn show_command(&self, text: Option<&str>) {
        let imp = self.imp();
        imp.chosen.replace(None);
        for section in Section::ALL {
            self.list(section).unselect_all();
        }
        if let Some(text) = text {
            imp.command_entry.set_text(text);
        }
        imp.command_box.set_visible(true);
        imp.command_entry.grab_focus();
        imp.command_entry.set_position(-1);
        self.update_buttons();
    }

    /// Choose an app on your PC: asks for a program file, then chooses
    /// it ([`Self::use_program`]).
    fn choose_program(&self) {
        let programs = gtk::FileFilter::new();
        programs.set_name(Some(&ox_core::i18n::gettext("Programs")));
        for mime_type in [
            "application/x-desktop",
            "application/x-executable",
            "application/x-pie-executable",
            "application/x-sharedlib",
            "application/x-shellscript",
        ] {
            programs.add_mime_type(mime_type);
        }
        let everything = gtk::FileFilter::new();
        everything.set_name(Some(&ox_core::i18n::gettext("All files")));
        everything.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&programs);
        filters.append(&everything);
        let picker = gtk::FileDialog::builder()
            .title(ox_core::i18n::gettext("Choose an app on your PC"))
            .modal(true)
            .filters(&filters)
            .initial_folder(&gio::File::for_path("/usr/bin"))
            .build();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let Ok(file) = picker.open_future(Some(&dialog)).await else {
                    return;
                };
                match file.path() {
                    Some(path) => dialog.use_program(program_file(&path)),
                    None => dialog.set_status(Some(&ox_core::i18n::gettext(
                        "Choose a program file or an app launcher (.desktop).",
                    ))),
                }
            }
        ));
    }

    /// Chooses a picked program: an installed launcher in the lists, which
    /// opens More apps when it is not listed yet, or an executable as the
    /// command line.
    fn use_program(&self, program: Result<ProgramFile, String>) {
        let imp = self.imp();
        match program {
            Err(message) => self.set_status(Some(&message)),
            Ok(ProgramFile::Command(command)) => {
                self.set_status(None);
                self.show_command(Some(&command));
            }
            Ok(ProgramFile::Installed(id)) => {
                self.set_status(None);
                imp.command_box.set_visible(false);
                imp.filter_entry.set_text("");
                let listed = imp.applications.borrow().as_ref().and_then(|list| {
                    let choice = list.choices.iter().find(|choice| choice.id == id)?;
                    choice.is_available.then(|| Section::of(choice))
                });
                if let Some(section) = listed {
                    imp.chosen.replace(Some(id));
                    if section == Section::More {
                        imp.more_expander.set_expanded(true);
                    }
                    self.show_applications();
                } else if imp.listed_all.get() {
                    self.set_status(Some(&OpenWithError::Unavailable.to_string()));
                } else {
                    imp.picked.replace(Some(id));
                    imp.more_expander.set_expanded(true);
                }
            }
        }
    }

    /// Just once or Always: checks the chosen application again, launches
    /// it and closes; a failure stays in the dialog. A command runs just
    /// once.
    fn open_chosen(&self, default: DefaultChoice) {
        let imp = self.imp();
        if imp.busy.get() {
            return;
        }
        if let Some(command) = self.custom_command() {
            if default == DefaultChoice::Keep {
                self.run_command(command);
            }
            return;
        }
        let Some(app_id) = imp.chosen.borrow().clone() else {
            return;
        };
        let default = if self.subject().is_folder {
            DefaultChoice::Keep
        } else {
            default
        };
        self.set_busy(true);
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

    fn set_busy(&self, busy: bool) {
        self.imp().busy.set(busy);
        self.update_buttons();
    }

    /// Runs the typed command with the item and closes; a failure stays
    /// in the dialog (OPEN-014).
    fn run_command(&self, command: CustomCommand) {
        self.set_busy(true);
        let uri = self.subject().uri.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let ran = prepare_target(uri)
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|prepared| run_custom_command(&command, &prepared));
                match ran {
                    Ok(()) => {
                        if let Some(report) = dialog.imp().report.get() {
                            report(ox_core::i18n::gettext_static("Ran the command."));
                        }
                        dialog.close();
                    }
                    Err(message) => {
                        dialog.set_status(Some(&message));
                        dialog.set_busy(false);
                    }
                }
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
                self.set_status(Some(&error.to_string()));
                self.set_busy(false);
            }
        }
    }
}

/// What tests read and do.
#[cfg(test)]
impl OpenWithDialog {
    /// The names of the listed applications, in their sections' order.
    pub(crate) fn shown_names(&self) -> Vec<String> {
        Section::ALL
            .into_iter()
            .flat_map(|section| self.section_names(section))
            .collect()
    }

    /// The names of the applications listed in `section`.
    pub(crate) fn section_names(&self, section: Section) -> Vec<String> {
        let applications = self.imp().applications.borrow();
        let Some(list) = applications.as_ref() else {
            return Vec::new();
        };
        let ids = &self.imp().shown_ids.borrow()[section.index()];
        ids.iter()
            .filter_map(|id| list.choices.iter().find(|choice| &choice.id == id))
            .map(|choice| choice.name.clone())
            .collect()
    }

    /// The applications are listed.
    pub(crate) fn is_listed(&self) -> bool {
        !self.imp().loading.get() && self.imp().applications.borrow().is_some()
    }

    /// The heading.
    pub(crate) fn heading_text(&self) -> String {
        self.imp().heading.text().to_string()
    }

    /// The status line, empty while it is hidden.
    pub(crate) fn status(&self) -> String {
        self.imp().status_label.text().to_string()
    }

    /// The chosen application's desktop ID.
    pub(crate) fn chosen(&self) -> Option<String> {
        self.imp().chosen.borrow().clone()
    }

    /// The name of the application whose row has the keyboard.
    pub(crate) fn focused_name(&self) -> Option<String> {
        let focus = gtk::prelude::RootExt::focus(self)?;
        let row = focus.downcast::<gtk::ListBoxRow>().ok()?;
        let section = Section::ALL
            .into_iter()
            .find(|&section| row.parent().as_ref() == Some(self.list(section).upcast_ref()))?;
        let index = usize::try_from(row.index()).ok()?;
        self.section_names(section).get(index).cloned()
    }

    /// Types `text` into "Find an application".
    pub(crate) fn type_filter(&self, text: &str) {
        self.imp().filter_entry.set_text(text);
    }

    /// Whether Always shows, and Just once and Always are enabled.
    pub(crate) fn controls_state(&self) -> (bool, bool, bool) {
        let imp = self.imp();
        (
            imp.always_button.is_visible(),
            imp.just_once_button.is_sensitive(),
            imp.always_button.is_sensitive(),
        )
    }

    /// Opens or closes More apps.
    pub(crate) fn expand_more_apps(&self, expanded: bool) {
        self.imp().more_expander.set_expanded(expanded);
    }

    /// Selects the application named `name`, as a click does.
    pub(crate) fn select_named(&self, name: &str) {
        let (section, row) = self.row_named(name);
        self.list(section).select_row(Some(&row));
    }

    /// Double-clicks the application named `name`.
    pub(crate) fn activate_named(&self, name: &str) {
        let (_, row) = self.row_named(name);
        WidgetExt::activate(&row);
    }

    fn row_named(&self, name: &str) -> (Section, gtk::ListBoxRow) {
        Section::ALL
            .into_iter()
            .find_map(|section| {
                let index = self
                    .section_names(section)
                    .iter()
                    .position(|shown| shown == name)?;
                let row = self.list(section).row_at_index(i32::try_from(index).ok()?)?;
                Some((section, row))
            })
            .unwrap_or_else(|| panic!("{name} is listed"))
    }

    /// Clicks Just once.
    pub(crate) fn click_just_once(&self) {
        self.imp().just_once_button.emit_clicked();
    }

    /// Clicks Always.
    pub(crate) fn click_always(&self) {
        self.imp().always_button.emit_clicked();
    }

    /// Opens "Use a custom command…" and types `text` as the command.
    pub(crate) fn type_command(&self, text: &str) {
        self.imp().custom_command_button.emit_clicked();
        self.imp().command_entry.set_text(text);
    }
}

/// The row of one application: its own icon (the generic glyph without
/// one) and its name, as Windows lists them. An application that cannot
/// open the item is shown but cannot be chosen, and says why.
fn application_row(choice: &ApplicationChoice) -> gtk::ListBoxRow {
    let name = gtk::Label::builder().label(&choice.name).xalign(0.0).build();
    name.add_css_class("app-name");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    if let Some(note) = choice.note() {
        let note = gtk::Label::builder().label(note).xalign(0.0).build();
        note.add_css_class("app-note");
        text.append(&note);
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = application_image(choice.icon.as_deref(), ROW_GLYPH)
        .unwrap_or_else(|| icons::image(Icon::Apps, ROW_GLYPH));
    content.append(&icon);
    content.append(&text);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.add_css_class("app-choice");
    row.set_sensitive(choice.is_available);
    row
}

/// The row shown when the filter matches nothing.
fn empty_row() -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some(&ox_core::i18n::gettext(
        "No matching installed applications.",
    )));
    label.add_css_class("apps-empty");
    gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(false)
        .selectable(false)
        .build()
}

#[cfg(test)]
mod tests {
    use super::heading;

    /// The heading names the extension as Windows 11's chooser does.
    ///
    /// parity: OPEN-011
    #[test]
    fn the_heading_names_the_files_extension() {
        assert_eq!(
            heading("Notes.txt", false),
            "Select an app to open this .txt file"
        );
        assert_eq!(
            heading("archive.tar.gz", false),
            "Select an app to open this .gz file"
        );
        assert_eq!(heading("Makefile", false), "Select an app to open this file");
        assert_eq!(heading(".bashrc", false), "Select an app to open this file");
        assert_eq!(heading("trailing.", false), "Select an app to open this file");
        assert_eq!(heading("Documents", true), "Select an app to open this folder");
    }
}
