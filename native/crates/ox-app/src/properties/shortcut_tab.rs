// SPDX-License-Identifier: AGPL-3.0-only
//! The Shortcut tab of a symbolic link or a launcher (a `.desktop` file),
//! after General as Windows places a shortcut's (PROP-004).
//!
//! It names the target's type and the folder it is in (Target type and
//! Target location), then Target: the path a link points to, or the
//! command a launcher runs (or the address of a launcher of a link). A
//! launcher adds Start in, the folder its command starts in, and Comment.
//! Open File Location shows the target selected in its folder, in the
//! browser window, as Windows' button does.
//!
//! The fields are held until OK or Apply ([`ShortcutEdits`]). A new Target
//! points the link at it: a new link is made beside it and renamed over it,
//! so the link is never missing. A launcher's changed keys are written
//! into the same file, which keeps its other keys, its translations, its
//! comments and its permission to run. Linux links have no Start in,
//! Comment, icon or shortcut key, so a link shows Target alone.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::metadata::ItemProperties;
use super::window::{
    divider, give_access_key, page, push_button, with_access_key, ApplyFuture, Pending, PropertiesWindow,
    Rows,
};
use crate::icons::{Art, ArtImage};

/// The group of a launcher's keys.
const DESKTOP_ENTRY: &str = "Desktop Entry";

/// What a Shortcut tab describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShortcutKind {
    /// A symbolic link.
    Link,
    /// A launcher that runs a command (`Type=Application`).
    Application,
    /// A launcher that opens an address (`Type=Link`).
    Address,
}

impl ShortcutKind {
    /// The kind of the item at `uri`: a local symbolic link, or a local
    /// `.desktop` file of an application or a link; `None` for anything
    /// else, which has no Shortcut tab.
    pub(super) fn of(uri: &str) -> Option<Self> {
        let path = gio::File::for_uri(uri).path()?;
        if std::fs::symlink_metadata(&path).ok()?.file_type().is_symlink() {
            return Some(Self::Link);
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("desktop") {
            return None;
        }
        let keys = read_launcher(&path)?;
        match keys.string(DESKTOP_ENTRY, "Type").ok()?.as_str() {
            "Application" => Some(Self::Application),
            "Link" => Some(Self::Address),
            _ => None,
        }
    }

    /// The launcher key Target holds.
    fn target_key(&self) -> &'static str {
        match self {
            Self::Address => "URL",
            Self::Link | Self::Application => "Exec",
        }
    }
}

/// The launcher at `path`, with its comments and translations.
fn read_launcher(path: &Path) -> Option<glib::KeyFile> {
    let keys = glib::KeyFile::new();
    keys.load_from_file(
        path,
        glib::KeyFileFlags::KEEP_COMMENTS | glib::KeyFileFlags::KEEP_TRANSLATIONS,
    )
    .ok()?;
    Some(keys)
}

/// The Shortcut tab: its page, filled once the item is read.
pub(super) struct ShortcutPanel {
    page: gtk::Box,
    kind: ShortcutKind,
    /// The fields shown; the tab is filled again after OK or Apply.
    edits: Rc<RefCell<Option<Rc<ShortcutEdits>>>>,
}

impl ShortcutPanel {
    /// The tab of a shortcut of `kind`, whose changes `window` makes on
    /// OK or Apply.
    pub(super) fn new(window: &PropertiesWindow, kind: ShortcutKind) -> Self {
        let edits = Rc::new(RefCell::new(None));
        window.add_pending(Rc::new(ShownEdits(Rc::clone(&edits))));
        Self {
            page: page(),
            kind,
            edits,
        }
    }

    pub(super) fn page(&self) -> &gtk::Box {
        &self.page
    }

    /// Fills the tab with the item `properties` describes, editable when
    /// `editable`, registering its changes with `window` the first time.
    pub(super) fn fill(&self, window: &PropertiesWindow, properties: &ItemProperties, editable: bool) {
        let Some(path) = gio::File::for_uri(&properties.entry.uri).path() else {
            return;
        };
        let facts = match self.kind {
            ShortcutKind::Link => link_facts(&path, properties.link_target.as_deref().unwrap_or_default()),
            ShortcutKind::Application | ShortcutKind::Address => launcher_facts(&path, &self.kind),
        };
        let Some(facts) = facts else {
            return;
        };
        while let Some(child) = self.page.first_child() {
            self.page.remove(&child);
        }
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["properties-header"])
            .build();
        header.append(&ArtImage::new(
            Art::for_entry(&properties.entry),
            super::general_tab::HEADER_ART_SIZE,
        ));
        header.append(
            &gtk::Label::builder()
                .label(&properties.entry.name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .selectable(true)
                .build(),
        );
        self.page.append(&header);
        self.page.append(&divider());

        let about = Rows::new();
        let target_type = about.add(&ox_core::i18n::gettext("Target type"), "");
        let target_location = about.add(&ox_core::i18n::gettext("Target location"), "");
        self.page.append(about.widget());
        self.page.append(&divider());

        let fields = Rows::new();
        let target = field(
            &fields,
            &ox_core::i18n::gettext("Target"),
            't',
            &facts.target,
            editable,
        );
        let launcher = (self.kind != ShortcutKind::Link).then(|| {
            let start_in = (self.kind == ShortcutKind::Application).then(|| {
                field(
                    &fields,
                    &ox_core::i18n::gettext("Start in"),
                    's',
                    &facts.start_in,
                    editable,
                )
            });
            let comment = field(
                &fields,
                &ox_core::i18n::gettext("Comment"),
                'o',
                &facts.comment,
                editable,
            );
            (start_in, comment)
        });
        self.page.append(fields.widget());
        self.page.append(&divider());

        let open = push_button(&ox_core::i18n::gettext("Open File Location"));
        give_access_key(&open, 'f');
        open.set_sensitive(false);
        self.page.append(&open);

        let shown = Shown {
            target_type,
            target_location,
            open: open.clone(),
            located: Rc::new(RefCell::new(None)),
        };
        let located = Rc::clone(&shown.located);
        let opener = window.downgrade();
        open.connect_clicked(move |_| {
            let place = located.borrow().clone();
            if let Some((uri, opener)) = place.zip(opener.upgrade().and_then(|window| window.opener())) {
                opener.show_item_in_folder(&uri);
            }
        });
        shown.locate(facts.located.clone(), self.kind.clone());

        let (start_in, comment) = match launcher {
            Some((start_in, comment)) => (start_in, Some(comment)),
            None => (None, None),
        };
        let edits = Rc::new(ShortcutEdits {
            window: window.downgrade(),
            path,
            kind: self.kind.clone(),
            facts,
            target,
            start_in,
            comment,
        });
        edits.connect_controls();
        self.edits.replace(Some(edits));
    }
}

/// A text field after `name:` with `text`, its name underlining `key`.
fn field(rows: &Rows, name: &str, key: char, text: &str, editable: bool) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .text(text)
        .hexpand(true)
        .editable(editable)
        .can_focus(true)
        // Enter presses OK, as in the name field and Windows' dialogs.
        .activates_default(true)
        .css_classes(["properties-field"])
        .build();
    let label = rows.add_control(name, &entry);
    // Level with the field's text.
    label.set_valign(gtk::Align::Center);
    label.set_yalign(0.5);
    label.set_text_with_mnemonic(&with_access_key(&label.text(), key));
    label.set_mnemonic_widget(Some(&entry));
    entry
}

/// What the tab shows of the shortcut, as read from it.
#[derive(Debug, Clone, Default)]
struct Facts {
    /// The link's target as stored, or the launcher's command or address.
    target: String,
    /// A launcher's `Path`.
    start_in: String,
    /// A launcher's `Comment`.
    comment: String,
    /// The target's path, which Target type, Target location and Open File
    /// Location describe; `None` when there is none to find (an address).
    located: Option<PathBuf>,
}

/// The link at `path`, pointing at `target`.
fn link_facts(path: &Path, target: &str) -> Option<Facts> {
    let stored = std::fs::read_link(path).ok()?;
    let shown = if target.is_empty() {
        stored.to_string_lossy().into_owned()
    } else {
        target.to_owned()
    };
    let located = path
        .parent()
        .map_or_else(|| stored.clone(), |folder| folder.join(&stored));
    Some(Facts {
        target: shown,
        located: Some(located),
        ..Facts::default()
    })
}

/// The launcher at `path`.
fn launcher_facts(path: &Path, kind: &ShortcutKind) -> Option<Facts> {
    let keys = read_launcher(path)?;
    let text = |key: &str| {
        keys.string(DESKTOP_ENTRY, key)
            .map(String::from)
            .unwrap_or_default()
    };
    let target = text(kind.target_key());
    let located = (*kind == ShortcutKind::Application)
        .then(|| program_of(&target))
        .flatten();
    Some(Facts {
        target,
        start_in: text("Path"),
        comment: text("Comment"),
        located,
    })
}

/// The program a launcher's command runs: its first word, found on `PATH`
/// when it names no folder.
fn program_of(command: &str) -> Option<PathBuf> {
    let words = glib::shell_parse_argv(command).ok()?;
    let first = words.first()?;
    glib::find_program_in_path(first)
}

/// The rows the target's facts fill in, once it is read.
struct Shown {
    target_type: gtk::Label,
    target_location: gtk::Label,
    open: gtk::Button,
    /// The target's location, for Open File Location.
    located: Rc<RefCell<Option<String>>>,
}

impl Shown {
    /// Reads the target at `path` off the main thread and shows its type
    /// and folder; an address is a link to a web page or a place.
    fn locate(self, path: Option<PathBuf>, kind: ShortcutKind) {
        let Some(path) = path else {
            if kind == ShortcutKind::Address {
                self.target_type.set_text(&ox_core::i18n::gettext("Address"));
            }
            return;
        };
        let file = gio::File::for_path(&path);
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .map_or_else(|| String::from("/"), |name| name.to_string_lossy().into_owned());
        glib::spawn_future_local(async move {
            let read = file
                .query_info_future(
                    ox_core::entry::ATTRIBUTES,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await;
            let Ok(info) = read else {
                self.target_type
                    .set_text(&ox_core::i18n::gettext("The target cannot be found."));
                return;
            };
            let entry = ox_core::entry::entry_from_info(&file, &info);
            let type_label = if kind == ShortcutKind::Application {
                ox_core::i18n::gettext("Application")
            } else {
                entry.type_label.clone()
            };
            self.target_type.set_text(&type_label);
            self.target_location.set_text(&folder);
            self.located.replace(Some(entry.uri));
            self.open.set_sensitive(true);
        });
    }
}

/// The Target, Start in and Comment as the user left them, until OK or
/// Apply.
pub(super) struct ShortcutEdits {
    window: glib::WeakRef<PropertiesWindow>,
    /// The link or launcher.
    path: PathBuf,
    kind: ShortcutKind,
    /// What the fields started from.
    facts: Facts,
    target: gtk::Entry,
    start_in: Option<gtk::Entry>,
    comment: Option<gtk::Entry>,
}

impl ShortcutEdits {
    fn connect_controls(self: &Rc<Self>) {
        let fields = [Some(&self.target), self.start_in.as_ref(), self.comment.as_ref()];
        for field in fields.into_iter().flatten() {
            let window = self.window.clone();
            field.connect_changed(move |_| {
                if let Some(window) = window.upgrade() {
                    window.changed();
                }
            });
        }
    }

    /// The fields' text: Target, Start in and Comment.
    fn wanted(&self) -> (String, Option<String>, Option<String>) {
        (
            self.target.text().trim().to_owned(),
            self.start_in.as_ref().map(|field| field.text().trim().to_owned()),
            self.comment.as_ref().map(|field| field.text().trim().to_owned()),
        )
    }

    fn has_changes(&self) -> bool {
        let facts = &self.facts;
        let (target, start_in, comment) = self.wanted();
        target != facts.target.trim()
            || start_in.is_some_and(|text| text != facts.start_in.trim())
            || comment.is_some_and(|text| text != facts.comment.trim())
    }

    /// Makes the change off the main thread.
    fn apply(&self) -> ApplyFuture {
        let (target, start_in, comment) = self.wanted();
        let path = self.path.clone();
        let kind = self.kind.clone();
        let facts = self.facts.clone();
        Box::pin(async move {
            if target.is_empty() {
                return Err(ox_core::i18n::gettext("Type the target the shortcut opens."));
            }
            let made = gio::spawn_blocking(move || match kind {
                ShortcutKind::Link => repoint_link(&path, &target),
                ShortcutKind::Application | ShortcutKind::Address => {
                    let change = LauncherChange {
                        target: (target != facts.target.trim()).then_some(target),
                        start_in: start_in.filter(|text| *text != facts.start_in.trim()),
                        comment: comment.filter(|text| *text != facts.comment.trim()),
                    };
                    write_launcher(&path, &kind, &change)
                }
            })
            .await
            .unwrap_or_else(|_| Err(ox_core::i18n::gettext("The shortcut could not be changed.")));
            made
        })
    }
}

/// The tab's change, as the window holds it: the fields shown now, which
/// are new each time the tab is filled again.
struct ShownEdits(Rc<RefCell<Option<Rc<ShortcutEdits>>>>);

impl Pending for ShownEdits {
    fn has_changes(&self) -> bool {
        self.0.borrow().as_ref().is_some_and(|edits| edits.has_changes())
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        match self.0.borrow().as_ref() {
            Some(edits) => edits.apply(),
            None => Box::pin(async { Ok(()) }),
        }
    }
}

/// Points the link at `path` to `target`: a new link beside it, renamed
/// over it, so the link is never missing.
fn repoint_link(path: &Path, target: &str) -> Result<(), String> {
    let folder = path.parent().ok_or_else(|| failure(path))?;
    let name = path.file_name().ok_or_else(|| failure(path))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".ox-{}", std::process::id()));
    let temporary = folder.join(temporary_name);
    std::os::unix::fs::symlink(target, &temporary).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        error.to_string()
    })
}

/// The keys of a launcher to change; `None` keeps a key as it is.
#[derive(Debug, Default)]
struct LauncherChange {
    target: Option<String>,
    start_in: Option<String>,
    comment: Option<String>,
}

/// Writes `change` into the launcher at `path`, read again first so the
/// keys another program changed meanwhile are kept. The same file is
/// rewritten, so it keeps its permissions and anything the desktop records
/// about it, such as whether it may run.
fn write_launcher(path: &Path, kind: &ShortcutKind, change: &LauncherChange) -> Result<(), String> {
    let keys = read_launcher(path).ok_or_else(|| failure(path))?;
    let set = |key: &str, value: &str| {
        if value.is_empty() {
            let _ = keys.remove_key(DESKTOP_ENTRY, key);
        } else {
            keys.set_string(DESKTOP_ENTRY, key, value);
        }
    };
    if let Some(target) = &change.target {
        set(kind.target_key(), target);
    }
    if let Some(start_in) = &change.start_in {
        set("Path", start_in);
    }
    if let Some(comment) = &change.comment {
        set("Comment", comment);
    }
    std::fs::write(path, keys.to_data().as_bytes()).map_err(|error| error.to_string())
}

/// The message for a shortcut that could not be read.
fn failure(path: &Path) -> String {
    ox_core::i18n::format_message(
        "{name} could not be changed.",
        &[(
            "name",
            &path
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A changed key is written in place; the others, the comments and
    /// the translations stay, and an emptied key is removed.
    #[test]
    fn a_launcher_keeps_what_was_not_changed() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let path = folder.path().join("Example.desktop");
        std::fs::write(
            &path,
            "[Desktop Entry]\n# Fictional\nType=Application\nName=Example\nName[fr]=Exemple\n\
             Exec=true\nPath=/tmp\nComment=Old\n",
        )
        .expect("a launcher");
        let change = LauncherChange {
            target: Some("true --new".into()),
            start_in: Some(String::new()),
            comment: None,
        };

        write_launcher(&path, &ShortcutKind::Application, &change).expect("written");

        let written = std::fs::read_to_string(&path).expect("the launcher");
        assert!(written.contains("Exec=true --new\n"), "{written}");
        assert!(!written.contains("Path="), "{written}");
        assert!(written.contains("Comment=Old\n"), "{written}");
        assert!(written.contains("Name[fr]=Exemple\n"), "{written}");
        assert!(written.contains("# Fictional\n"), "{written}");
    }

    /// A link is pointed elsewhere by a new link renamed over it, and a
    /// launcher's program is found on `PATH`.
    #[test]
    fn a_link_is_pointed_elsewhere_and_a_program_is_found() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let link = folder.path().join("link");
        std::os::unix::fs::symlink("first", &link).expect("a link");

        repoint_link(&link, "second").expect("pointed elsewhere");

        assert_eq!(std::fs::read_link(&link).expect("a link"), Path::new("second"));
        assert_eq!(std::fs::read_dir(folder.path()).expect("listed").count(), 1);
        assert!(program_of("sh -c 'echo example'").is_some_and(|program| program.is_absolute()));
        assert_eq!(program_of(""), None);
    }
}
