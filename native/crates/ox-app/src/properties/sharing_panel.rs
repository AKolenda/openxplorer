// SPDX-License-Identifier: AGPL-3.0-only
//! The Sharing tab of a local folder (NET-035), laid out as Windows 11's,
//! over Samba user shares.
//!
//! "Network File and Folder Sharing" shows the folder, whether it is
//! shared and its network path, with Share...; "Advanced Sharing" opens the
//! dialog where the share is set up: Share this folder, the share name,
//! comments, guest access and read-only access, applied by its OK or
//! Apply. "Password Protection" says who may connect. The tab reads the
//! folder's share when it is first shown and again after each change,
//! always off the main thread ([`ox_core::network::Usershares`]). Where
//! Samba or user shares are missing, it says why and its buttons are off.
//! The Flatpak has no tab: Samba's `net` on the host cannot be run from the
//! sandbox. `OpenXplorer` never changes permissions on another server.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::Sandbox;
use ox_core::network::{Usershare, UsershareError, Usershares};

use super::window::{group, paragraph, push_button, ApplyFuture, Pending, PropertiesWindow, Rows};
use crate::dialog::check_row;
use crate::icons::{self, Art, ArtImage, Icon};

/// The size of the folder's picture in the first group.
const FOLDER_ART: i32 = 32;

/// Samba's `net`, which the Sharing tab runs; `None` in the Flatpak,
/// which cannot run the host's. The app's own tests never read the
/// computer's shares: they get a command that does not exist, so the tab
/// says sharing is unavailable.
pub(crate) fn system_usershares(sandbox: Sandbox) -> Option<Usershares> {
    if sandbox == Sandbox::Flatpak {
        None
    } else if cfg!(test) {
        Some(Usershares::with_program("/nonexistent/openxplorer-test-net"))
    } else {
        Some(Usershares::default())
    }
}

/// The folder, its share as last read, and the tab's labels.
struct SharingState {
    /// The folder, which a rename on General changes.
    folder: RefCell<PathBuf>,
    /// The folder's name in the first group.
    folder_label: gtk::Label,
    usershares: Usershares,
    /// The share as last read; `None` when the folder is not shared.
    share: RefCell<Option<Usershare>>,
    /// Whether the last read worked.
    is_available: Cell<bool>,
    state: gtk::Label,
    path: gtk::Label,
    buttons: [gtk::Button; 2],
}

/// The Sharing tab of the local folder `folder`, using `usershares`, and
/// what points it at the folder's new path after a rename.
pub(super) fn sharing_panel(folder: PathBuf, usershares: Usershares) -> (gtk::Box, Rc<dyn Fn(PathBuf)>) {
    let page = super::window::page();
    let folder_name = folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let (network, content) = group(&ox_core::i18n::gettext("Network File and Folder Sharing"));
    let identity = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    identity.append(&ArtImage::new(Art::Folder, FOLDER_ART));
    let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
    names.set_hexpand(true);
    // A long name ends in "…" inside the window's fixed width and height.
    let folder_label = gtk::Label::builder()
        .label(&folder_name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    names.append(&folder_label);
    let state = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Checking whether this folder is shared…"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["properties-text"])
        .build();
    state.set_accessible_role(gtk::AccessibleRole::Status);
    names.append(&state);
    identity.append(&names);
    content.append(&identity);
    let path_rows = Rows::new();
    let path = path_rows.add(&ox_core::i18n::gettext("Network Path"), "");
    content.append(path_rows.widget());
    let share = push_button(&ox_core::i18n::gettext("Share..."));
    content.append(&share);
    page.append(&network);

    let (advanced_group, content) = group(&ox_core::i18n::gettext("Advanced Sharing"));
    content.append(&paragraph(&ox_core::i18n::gettext(
        "Set custom permissions, create multiple shares, and set other advanced sharing options.",
    )));
    let advanced = gtk::Button::builder()
        .halign(gtk::Align::Start)
        .css_classes(["properties-button"])
        .build();
    let advanced_label = ox_core::i18n::gettext("Advanced Sharing...");
    let advanced_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    advanced_content.append(&icons::image(Icon::ShieldLock, 16));
    advanced_content.append(&gtk::Label::new(Some(&advanced_label)));
    advanced.set_child(Some(&advanced_content));
    advanced.update_property(&[gtk::accessible::Property::Label(&advanced_label)]);
    content.append(&advanced);
    page.append(&advanced_group);

    let (password, content) = group(&ox_core::i18n::gettext("Password Protection"));
    content.append(&paragraph(&ox_core::i18n::gettext(
        "People must have a user account and password for this computer to access shared folders, unless \
         the share allows guests.",
    )));
    content.append(&paragraph(&ox_core::i18n::gettext(
        "To change this setting, use Advanced Sharing.",
    )));
    page.append(&password);

    let sharing = Rc::new(SharingState {
        folder: RefCell::new(folder),
        folder_label,
        usershares,
        share: RefCell::new(None),
        is_available: Cell::new(false),
        state,
        path,
        buttons: [share.clone(), advanced.clone()],
    });
    sharing.set_sensitive(false);
    for button in [&share, &advanced] {
        let sharing = Rc::clone(&sharing);
        button.connect_clicked(move |button| {
            if let Some(window) = PropertiesWindow::of(button) {
                open_advanced_sharing(&window, &sharing);
            }
        });
    }
    // Read when first shown, so Properties of any folder runs no `net`
    // until the tab is opened.
    let first_shown = Rc::new(Cell::new(false));
    let shown = Rc::clone(&first_shown);
    let read = Rc::clone(&sharing);
    page.connect_map(move |_| {
        if !shown.replace(true) {
            read.reload();
        }
    });
    let follow = move |folder: PathBuf| {
        let name = folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        sharing.folder_label.set_text(&name);
        sharing.folder.replace(folder);
        if first_shown.get() {
            sharing.reload();
        }
    };
    (page, Rc::new(follow))
}

impl SharingState {
    /// Reads the folder's share off the main thread and shows it.
    fn reload(self: &Rc<Self>) {
        let (usershares, folder) = (self.usershares.clone(), self.folder.borrow().clone());
        let sharing = Rc::clone(self);
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || usershares.share_of(&folder)).await;
            let read = read.unwrap_or_else(|_| {
                Err(UsershareError::Refused(
                    ox_core::i18n::gettext_static("The check stopped.").into(),
                ))
            });
            sharing.show(read);
        });
    }

    /// Shows the folder's share, or why sharing is unavailable.
    fn show(&self, read: Result<Option<Usershare>, UsershareError>) {
        match read {
            Ok(Some(share)) => {
                self.state.set_text(&ox_core::i18n::gettext("Shared"));
                self.path.set_text(&ox_core::i18n::format_message(
                    "\\\\{value1}\\{name}",
                    &[("value1", glib::host_name().as_ref()), ("name", &share.name)],
                ));
                self.share.replace(Some(share));
                self.is_available.set(true);
            }
            Ok(None) => {
                self.state.set_text(&ox_core::i18n::gettext("Not Shared"));
                self.path.set_text(&ox_core::i18n::gettext("Not Shared"));
                self.share.replace(None);
                self.is_available.set(true);
            }
            Err(error) => {
                self.state.set_text(&error.to_string());
                self.path.set_text("");
                self.is_available.set(false);
            }
        }
        self.set_sensitive(self.is_available.get());
    }

    fn set_sensitive(&self, is_sensitive: bool) {
        for button in &self.buttons {
            button.set_sensitive(is_sensitive);
        }
    }
}

/// Advanced Sharing's controls, held until OK or Apply.
struct ShareSettings {
    sharing: Rc<SharingState>,
    /// The share as the dialog opened on it, or as its last Apply made it.
    initial: RefCell<Option<Usershare>>,
    share: gtk::CheckButton,
    name: gtk::Entry,
    comment: gtk::Entry,
    guests: gtk::CheckButton,
    read_only: gtk::CheckButton,
}

impl ShareSettings {
    /// The share the controls describe; `None` to stop sharing.
    fn wanted(&self) -> Option<Usershare> {
        self.share.is_active().then(|| Usershare {
            name: self.name.text().trim().to_owned(),
            path: self.sharing.folder.borrow().clone(),
            comment: self.comment.text().into(),
            allows_guests: self.guests.is_active(),
            is_read_only: self.read_only.is_active(),
        })
    }
}

impl Pending for ShareSettings {
    fn has_changes(&self) -> bool {
        self.wanted() != *self.initial.borrow()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            let usershares = self.sharing.usershares.clone();
            let wanted = self.wanted();
            let shared_as = self.initial.borrow().as_ref().map(|share| share.name.clone());
            let made = wanted.clone();
            let changed =
                gio::spawn_blocking(move || change_share(&usershares, made.as_ref(), shared_as)).await;
            let result = changed.unwrap_or_else(|_| {
                Err(UsershareError::Refused(
                    ox_core::i18n::gettext_static("The change stopped.").into(),
                ))
            });
            self.sharing.reload();
            // What was made is where a later Apply or OK starts from.
            if result.is_ok() {
                self.initial.replace(wanted);
            }
            result.map_err(|error| error.to_string())
        })
    }
}

/// Opens Advanced Sharing over `parent`, on the share as last read.
fn open_advanced_sharing(parent: &PropertiesWindow, sharing: &Rc<SharingState>) -> PropertiesWindow {
    let dialog = PropertiesWindow::dialog_of(parent, &ox_core::i18n::gettext("Advanced Sharing"));
    let page = super::window::page();
    let initial = sharing.share.borrow().clone();
    let folder_name = sharing
        .folder
        .borrow()
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let share = check_row(&ox_core::i18n::gettext("Share this folder"), initial.is_some());
    page.append(&share);
    let (settings, content) = group(&ox_core::i18n::gettext("Settings"));
    let rows = Rows::new();
    let field = |text: &str| gtk::Entry::builder().text(text).hexpand(true).build();
    let name = field(
        initial
            .as_ref()
            .map_or(folder_name.as_str(), |share| share.name.as_str()),
    );
    rows.add_control(&ox_core::i18n::gettext("Share name"), &name);
    let comment = field(initial.as_ref().map_or("", |share| share.comment.as_str()));
    rows.add_control(&ox_core::i18n::gettext("Comments"), &comment);
    content.append(rows.widget());
    let guests = check_row(
        &ox_core::i18n::gettext("Allow guests (no account needed)"),
        initial.as_ref().is_some_and(|share| share.allows_guests),
    );
    let read_only = check_row(
        &ox_core::i18n::gettext("Others can only read"),
        initial.as_ref().is_none_or(|share| share.is_read_only),
    );
    content.append(&guests);
    content.append(&read_only);
    page.append(&settings);
    dialog.add_tab(
        "advanced-sharing",
        &ox_core::i18n::gettext("Advanced Sharing"),
        &page,
    );
    let settings = Rc::new(ShareSettings {
        sharing: Rc::clone(sharing),
        initial: RefCell::new(initial),
        share,
        name,
        comment,
        guests,
        read_only,
    });
    let follow = {
        let settings = Rc::downgrade(&settings);
        let window = dialog.downgrade();
        move || {
            if let Some(settings) = settings.upgrade() {
                let is_shared = settings.share.is_active();
                for widget in [
                    settings.name.upcast_ref::<gtk::Widget>(),
                    settings.comment.upcast_ref(),
                    settings.guests.upcast_ref(),
                    settings.read_only.upcast_ref(),
                ] {
                    widget.set_sensitive(is_shared);
                }
            }
            if let Some(window) = window.upgrade() {
                window.changed();
            }
        }
    };
    follow();
    for check in [&settings.share, &settings.guests, &settings.read_only] {
        let follow = follow.clone();
        check.connect_toggled(move |_| follow());
    }
    for entry in [&settings.name, &settings.comment] {
        let follow = follow.clone();
        entry.connect_changed(move |_| follow());
    }
    dialog.add_pending(settings);
    dialog.present();
    dialog
}

/// Makes Samba's share of the folder `wanted`: shares it (renaming drops
/// the old share), or stops sharing it for `None`.
fn change_share(
    usershares: &Usershares,
    wanted: Option<&Usershare>,
    shared_as: Option<String>,
) -> Result<(), UsershareError> {
    let renamed = match (wanted, &shared_as) {
        (Some(share), Some(old)) => &share.name != old,
        (None, Some(_)) => true,
        _ => false,
    };
    if let Some(share) = wanted {
        usershares.share(share)?;
    }
    if let (true, Some(old)) = (renamed, shared_as) {
        usershares.unshare(&old)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;
    use crate::test_support::harness::{descendants, wait_until};

    /// A fake `net` in `folder` that lists `listing` and logs its calls,
    /// slowly.
    fn fake_net(folder: &Path, listing: &str) -> Usershares {
        let program = folder.join("net");
        // Like Samba's on a slow computer, it answers half a second after
        // it is asked, so a change is still running when the call shows.
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nsleep 0.5\ncat <<'EOF'\n{listing}\nEOF\n",
            folder.join("calls").display()
        );
        fs::write(&program, script).expect("the fake net");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        Usershares::with_program(program)
    }

    /// The tab shows the share as Windows does, and Advanced Sharing
    /// changes it on OK.
    ///
    /// parity: NET-035
    #[gtk::test]
    fn the_tab_shows_the_share_and_advanced_sharing_changes_it() {
        let temporary = tempfile::tempdir().expect("a temporary folder");
        let listing = "[Projects]\npath=/srv/Projects\ncomment=Team\nusershare_acl=Everyone:R,\nguest_ok=n";
        let (panel, _) = sharing_panel("/srv/Projects".into(), fake_net(temporary.path(), listing));
        let window = PropertiesWindow::new("Projects Properties", None, None);
        window.add_tab("sharing", "Sharing", &panel);
        window.present();
        let shown = || {
            descendants::<gtk::Label>(&panel)
                .iter()
                .map(|label| label.text().to_string())
                .collect::<Vec<_>>()
        };
        wait_until("the share is read", || shown().contains(&"Shared".to_owned()));
        let host = glib::host_name();
        assert!(
            shown().contains(&format!("\\\\{host}\\Projects")),
            "{:?}",
            shown()
        );

        let advanced = descendants::<gtk::Button>(&panel)
            .into_iter()
            .find(|button| {
                button.accessible_role() == gtk::AccessibleRole::Button && button.label().is_none()
            })
            .expect("Advanced Sharing...");
        advanced.emit_clicked();
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|toplevel| toplevel.downcast::<PropertiesWindow>().ok())
            .find(|dialog| dialog.title_text() == "Advanced Sharing")
            .expect("Advanced Sharing opened");
        let entries = descendants::<gtk::Entry>(&dialog);
        assert_eq!(entries[1].text().as_str(), "Team");
        let read_only = descendants::<gtk::CheckButton>(&dialog)
            .into_iter()
            .find(|check| check.label().as_deref() == Some("Others can only read"))
            .expect("read only");
        assert!(read_only.is_active());
        read_only.set_active(false);
        assert!(dialog.apply_button().is_sensitive());
        dialog.apply_button().emit_clicked();
        let calls = || fs::read_to_string(temporary.path().join("calls")).unwrap_or_default();
        wait_until("the share is changed", || calls().contains("usershare add"));
        assert!(calls().contains("usershare add Projects /srv/Projects Team Everyone:F guest_ok=n"));
        // Apply greys every button while the change runs: the change is
        // made once OK can be pressed again.
        wait_until("the change to be made", || {
            dialog.ok_button().is_sensitive() && !dialog.apply_button().is_sensitive()
        });

        // OK after Apply has nothing left to change.
        dialog.ok_button().emit_clicked();
        wait_until("Advanced Sharing to close", || dialog.is_closed());
        assert_eq!(
            calls().matches("usershare add").count(),
            1,
            "the change is made once"
        );
        window.destroy();
    }
}
