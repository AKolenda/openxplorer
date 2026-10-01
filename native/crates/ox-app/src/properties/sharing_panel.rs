// SPDX-License-Identifier: AGPL-3.0-only
//! The Sharing tab of Properties: share a local folder on the network
//! with Samba user shares (NET-035).
//!
//! Dolphin's Properties → Share tab, refined to the dialog's fields: share
//! this folder, its share name and comment, guest access and read-only
//! access, and Apply. The tab reads the folder's share when it is first
//! shown and again after each change, always off the main thread
//! ([`ox_core::network::Usershares`]). Where Samba or user shares are
//! missing, it says why and every control is off. The Flatpak has no tab:
//! Samba's `net` on the host cannot be run from the sandbox.
//! `OpenXplorer` never changes permissions on another server.

use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::Sandbox;
use ox_core::network::{Usershare, UsershareError, Usershares};

use crate::dialog::{check_row, labelled_entry, note, quiet_text};
use crate::window::ButtonStyle;

/// The note under the controls.
const SHARING_NOTE: &str = "Sharing uses Samba user shares on this computer. OpenXplorer never changes \
                            permissions on other servers.";

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

/// The controls of the tab.
struct SharingControls {
    folder: PathBuf,
    usershares: Usershares,
    state: gtk::Label,
    share: gtk::CheckButton,
    name: gtk::Entry,
    comment: gtk::Entry,
    guests: gtk::CheckButton,
    read_only: gtk::CheckButton,
    apply: gtk::Button,
    /// The share name Samba knows the folder by, to stop sharing it.
    shared_as: std::cell::RefCell<Option<String>>,
}

/// The Sharing tab of the local folder `folder`, using `usershares`.
pub(super) fn sharing_panel(folder: PathBuf, usershares: Usershares) -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let state = quiet_text("Checking whether this folder is shared…");
    panel.append(&state);
    let share = check_row("Share this folder", false);
    panel.append(&share);
    let folder_name = folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name = labelled_entry(&panel, "Share name", &folder_name);
    let comment = labelled_entry(&panel, "Comment", "");
    let guests = check_row("Allow guests (no account needed)", false);
    let read_only = check_row("Others can only read", true);
    panel.append(&guests);
    panel.append(&read_only);
    let apply = gtk::Button::builder()
        .label("Apply")
        .halign(gtk::Align::Start)
        .css_classes([ButtonStyle::Bordered.css_class()])
        .build();
    panel.append(&apply);
    panel.append(&note(SHARING_NOTE));
    let controls = Rc::new(SharingControls {
        folder,
        usershares,
        state,
        share,
        name,
        comment,
        guests,
        read_only,
        apply,
        shared_as: std::cell::RefCell::default(),
    });
    controls.set_sensitive(false);
    let pressed = Rc::clone(&controls);
    controls.apply.connect_clicked(move |_| pressed.apply());
    // Read when first shown, so Properties of any folder runs no `net`
    // until the tab is opened.
    let first_shown = std::cell::Cell::new(false);
    panel.connect_map(move |_| {
        if !first_shown.replace(true) {
            controls.reload();
        }
    });
    panel
}

impl SharingControls {
    /// Reads the folder's share off the main thread and shows it.
    fn reload(self: &Rc<Self>) {
        let (usershares, folder) = (self.usershares.clone(), self.folder.clone());
        let controls = Rc::clone(self);
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || usershares.share_of(&folder)).await;
            let read = read.unwrap_or_else(|_| Err(UsershareError::Refused("The check stopped.".into())));
            controls.show(read);
        });
    }

    /// Shows the folder's share, or why sharing is unavailable.
    fn show(&self, read: Result<Option<Usershare>, UsershareError>) {
        match read {
            Ok(Some(share)) => {
                self.state
                    .set_text(&format!("Shared as \\\\{}\\{}", glib::host_name(), share.name));
                self.share.set_active(true);
                self.name.set_text(&share.name);
                self.comment.set_text(&share.comment);
                self.guests.set_active(share.allows_guests);
                self.read_only.set_active(share.is_read_only);
                self.shared_as.replace(Some(share.name));
                self.set_sensitive(true);
            }
            Ok(None) => {
                self.state.set_text("This folder is not shared.");
                self.share.set_active(false);
                self.shared_as.replace(None);
                self.set_sensitive(true);
            }
            Err(error) => {
                self.state.set_text(&error.to_string());
                self.set_sensitive(false);
            }
        }
    }

    /// Shares the folder as the controls say, or stops sharing it.
    fn apply(self: &Rc<Self>) {
        let usershares = self.usershares.clone();
        let wanted = self.share.is_active().then(|| Usershare {
            name: self.name.text().trim().to_owned(),
            path: self.folder.clone(),
            comment: self.comment.text().into(),
            allows_guests: self.guests.is_active(),
            is_read_only: self.read_only.is_active(),
        });
        let shared_as = self.shared_as.borrow().clone();
        self.set_sensitive(false);
        let controls = Rc::clone(self);
        glib::spawn_future_local(async move {
            let changed =
                gio::spawn_blocking(move || change_share(&usershares, wanted.as_ref(), shared_as)).await;
            match changed.unwrap_or_else(|_| Err(UsershareError::Refused("The change stopped.".into()))) {
                Ok(()) => controls.reload(),
                Err(error) => {
                    controls.state.set_text(&error.to_string());
                    controls.set_sensitive(true);
                }
            }
        });
    }

    fn set_sensitive(&self, is_sensitive: bool) {
        for widget in [
            self.share.upcast_ref::<gtk::Widget>(),
            self.name.upcast_ref(),
            self.comment.upcast_ref(),
            self.guests.upcast_ref(),
            self.read_only.upcast_ref(),
            self.apply.upcast_ref(),
        ] {
            widget.set_sensitive(is_sensitive);
        }
    }
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

    /// A fake `net` in `folder` that lists `listing` and logs its calls.
    fn fake_net(folder: &Path, listing: &str) -> Usershares {
        let program = folder.join("net");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncat <<'EOF'\n{listing}\nEOF\n",
            folder.join("calls").display()
        );
        fs::write(&program, script).expect("the fake net");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        Usershares::with_program(program)
    }

    /// parity: NET-035
    #[gtk::test]
    fn the_tab_shows_the_share_and_applies_a_change() {
        let temporary = tempfile::tempdir().expect("a temporary folder");
        let listing = "[Projects]\npath=/srv/Projects\ncomment=Team\nusershare_acl=Everyone:R,\nguest_ok=n";
        let panel = sharing_panel("/srv/Projects".into(), fake_net(temporary.path(), listing));
        let window = gtk::Window::builder().child(&panel).build();
        window.present();
        let checks = descendants::<gtk::CheckButton>(&panel);
        wait_until("the share is read", || checks[0].is_active());
        let entries = descendants::<gtk::Entry>(&panel);
        assert_eq!(entries[1].text().as_str(), "Team");
        assert!(checks[2].is_active(), "read only");

        checks[2].set_active(false);
        descendants::<gtk::Button>(&panel)
            .into_iter()
            .find(|button| button.label().as_deref() == Some("Apply"))
            .expect("Apply")
            .emit_clicked();

        let calls = || fs::read_to_string(temporary.path().join("calls")).unwrap_or_default();
        wait_until("the share is changed", || calls().contains("usershare add"));
        assert!(calls().contains("usershare add Projects /srv/Projects Team Everyone:F guest_ok=n"));
        window.destroy();
    }
}
