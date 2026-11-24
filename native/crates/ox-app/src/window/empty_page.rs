// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane's empty page: an empty or filtered-out folder, and a
//! location that could not be listed, with Try again.
//!
//! Ports the empty-state branch of `renderRows` in `v2.0.0:desktop/ui/app.js`.
//! A folder that is still being listed shows no page of its own: the
//! pane keeps its blank list, as Windows Explorer and Dolphin do (see
//! [`super::loading`]).

use gtk::glib;
use gtk::prelude::*;
use ox_core::entry::EntryError;
use ox_core::i18n::{format_message, gettext};
use ox_core::location::{is_server_location, is_smb_location, is_smb_server, split_location};
use ox_core::system_error::SystemError;

use crate::icons::{self, Icon};

use super::button_style::ButtonStyle;
use super::window_action::WindowAction;

/// The folder or network glyph above the title.
const STATE_GLYPH: i32 = 44;
/// Pixels between the glyph, the title, the message and Try again
/// (`.empty-state{gap:12px}`).
const PART_GAP: i32 = 12;
/// The widest the message gets before it wraps, in characters: about the
/// 460 pixels of `.empty-state p{max-width:460px}`.
const MESSAGE_WIDTH_CHARS: i32 = 65;

/// What the empty page says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EmptyState {
    /// The folder could not be listed: what to tell the user, the
    /// system's own error under Details when the message is not it, and a
    /// Try again button.
    Unavailable {
        /// The text under the title.
        message: String,
        /// The system's error, kept under Details for diagnosis.
        details: Option<String>,
        /// A folder on this computer, shown with a folder rather than the
        /// network.
        local: bool,
    },
    /// An archive opened like a folder could not be read (damaged,
    /// encrypted, a method the reader lacks): what to tell the user. It
    /// offers no Try again, which would fail the same way (ARC-026).
    ArchiveUnavailable(String),
    /// A search found nothing; the message says why (SRCH-013).
    NoMatches(String),
    /// The folder has no items; the Recycle Bin too, which says so as
    /// any empty folder does, as in Windows 11 (OPS-040).
    EmptyFolder,
    /// A server lists no shared folders.
    NoShares,
    /// The phone and camera list (`mtp://`) has no devices.
    NoDevices,
    /// Gallery found no photos, or no Pictures folder (HOME-023).
    NoPhotos,
}

impl EmptyState {
    /// What the page says about `uri`, which `error` kept from being
    /// listed. A share or server that refused the connection or dropped
    /// off the network is named in plain words, as Windows Explorer names
    /// it ("\\\\server\\share is unavailable."), with Windows' error code
    /// and words under Details ("Error code: 0x80070079", "The semaphore
    /// timeout period has expired."), and so is a drive whose driver
    /// stopped answering. Every other error is already said for people,
    /// and is shown as it is.
    pub(super) fn unavailable(uri: &str, error: &EntryError) -> EmptyState {
        // Only an archive that can't be read gets the archive page; a
        // folder or file a readable archive does not have (a typed or
        // stale address) is not found, as anywhere else.
        let is_missing_inside = matches!(error, EntryError::NotFound(_) | EntryError::NotDirectory(_));
        if ox_core::location::is_archive_location(uri) && !is_missing_inside {
            return EmptyState::ArchiveUnavailable(error.to_string());
        }
        let raw = match error {
            EntryError::Failed(raw) | EntryError::NotMounted(raw) | EntryError::NotConnected(raw)
                if is_server_location(uri) =>
            {
                raw
            }
            // A drive that stopped answering, in Windows' words for a
            // drive that is not there, with the system's error under
            // Details.
            EntryError::NotConnected(raw) => {
                let location = gtk::gio::File::for_uri(uri).parse_name().to_string();
                return EmptyState::Unavailable {
                    message: format_message(
                        "{location} is not accessible. The device is not ready.",
                        &[("location", &location)],
                    ),
                    details: (!raw.trim().is_empty()).then(|| code_details(raw, false)),
                    local: false,
                };
            }
            // A share mounted at a local path that went away keeps saying
            // it was disconnected, below.
            _ if is_local_location(uri) && error.to_string() != super::loading::disconnected_message() => {
                return local_unavailable(uri, error)
            }
            _ => {
                return EmptyState::Unavailable {
                    message: error.to_string(),
                    details: None,
                    local: false,
                }
            }
        };
        if *raw == super::loading::disconnected_message() {
            return EmptyState::Unavailable {
                message: raw.clone(),
                details: None,
                local: false,
            };
        }
        let location = if is_smb_location(uri) {
            ox_core::search::display_path(uri)
        } else {
            gtk::gio::File::for_uri(uri).parse_name().to_string()
        };
        EmptyState::Unavailable {
            message: format_message(
                "{location} is unavailable. Make sure you are connected to the network and the server is \
                 turned on, and then try again.",
                &[("location", &location)],
            ),
            details: (!raw.trim().is_empty()).then(|| network_details(raw)),
            local: false,
        }
    }

    /// What an empty listing of `uri` says: Dolphin's placeholders for a
    /// server without shares and the device list, and "This folder is
    /// empty" elsewhere, the Recycle Bin included.
    pub(super) fn empty_listing(uri: &str) -> EmptyState {
        if uri == ox_core::location::GALLERY_URI {
            EmptyState::NoPhotos
        } else if is_smb_server(uri) {
            EmptyState::NoShares
        } else if split_location(uri)
            .is_ok_and(|parts| parts.scheme.eq_ignore_ascii_case("mtp") && parts.authority.is_empty())
        {
            EmptyState::NoDevices
        } else {
            EmptyState::EmptyFolder
        }
    }
}

/// What Details says about a share that cannot be reached, as Windows'
/// "Network Error" says it under "See details": the error code, then its
/// words. A message that is not one of the system's errors is shown as it
/// is.
fn network_details(message: &str) -> String {
    code_details(message, true)
}

/// Windows' error code, then its words, for the system's error `message`,
/// as a share's or, when not `on_network`, a drive's Details say it.
fn code_details(message: &str, on_network: bool) -> String {
    let error = SystemError::from_sentence(message);
    let error = if on_network {
        error.map(SystemError::on_network)
    } else {
        error
    };
    match error {
        Some(error) => format_message(
            "Error code: {code}\n{message}",
            &[
                ("code", &format!("0x{:08X}", error.hresult())),
                ("message", &error.message()),
            ],
        ),
        None => message.to_owned(),
    }
}

/// The empty page's widgets.
#[derive(Debug)]
pub(super) struct EmptyPage {
    /// The page, centred in the folder pane.
    pub root: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    message: gtk::Label,
    /// The system's error, folded under Details.
    details: gtk::Expander,
    retry: gtk::Button,
}

/// Whether `uri` is a folder on this computer.
fn is_local_location(uri: &str) -> bool {
    uri.starts_with("file:")
}

/// What the page says about the folder on this computer at `uri`, which
/// `error` kept from being listed: a short sentence, as Windows says
/// "Location is not available", with the system's own text, which can
/// hold the whole path, under Details. A path past what Linux opens says
/// so in Windows' words for a name that is too long.
fn local_unavailable(uri: &str, error: &EntryError) -> EmptyState {
    let path = gtk::gio::File::for_uri(uri).path();
    let too_long = path.as_deref().is_some_and(|path| {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().len() >= PATH_MAX_BYTES
            || path.iter().any(|part| part.as_bytes().len() > NAME_MAX_BYTES)
    });
    let message = if too_long {
        ox_core::i18n::gettext("The file name or path is too long.")
    } else {
        match error {
            EntryError::NotFound(_) | EntryError::NotDirectory(_) => {
                ox_core::i18n::gettext("This folder can't be found. It may have been moved or deleted.")
            }
            EntryError::PermissionDenied(_) => ox_core::i18n::gettext("Access is denied."),
            _ => ox_core::i18n::gettext("This folder can't be opened."),
        }
    };
    let raw = error.to_string();
    EmptyState::Unavailable {
        message,
        details: (!raw.trim().is_empty()).then_some(raw),
        local: true,
    }
}

/// Linux's longest path (`PATH_MAX`, with its closing zero) and name
/// (`NAME_MAX`), in bytes.
const PATH_MAX_BYTES: usize = 4096;
const NAME_MAX_BYTES: usize = 255;

/// Clears `label`'s selection after its next frames, unless the pointer
/// selected it: GTK selects a focused selectable label's text whole when
/// the window becomes active or the label is mapped.
fn unselect_when_drawn(label: &gtk::Label) {
    for delay in [0_u64, 100, 400] {
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(delay),
            glib::clone!(
                #[weak]
                label,
                move || {
                    if !label.has_css_class(SELECTED_BY_POINTER) {
                        label.select_region(0, 0);
                    }
                }
            ),
        );
    }
}

/// Keeps `label`'s text unselected when it takes the keyboard: GTK
/// selects all of a selectable label's text then, so a page whose message
/// took the keyboard as it showed looked as if the user had selected it.
/// A drag or double-click still selects, so the text can be copied.
fn keep_unselected_on_focus(label: &gtk::Label) {
    let focus = gtk::EventControllerFocus::new();
    focus.connect_enter(glib::clone!(
        #[weak]
        label,
        move |_| {
            // GTK selects after the label took the keyboard.
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                label,
                move || {
                    if label.has_focus() && !label.has_css_class(SELECTED_BY_POINTER) {
                        label.select_region(0, 0);
                    }
                }
            ));
        }
    ));
    label.add_controller(focus);
    let click = gtk::GestureClick::new();
    click.connect_pressed(glib::clone!(
        #[weak]
        label,
        move |_, _, _, _| label.add_css_class(SELECTED_BY_POINTER)
    ));
    label.add_controller(click);
    let leave = gtk::EventControllerFocus::new();
    leave.connect_leave(glib::clone!(
        #[weak]
        label,
        move |_| label.remove_css_class(SELECTED_BY_POINTER)
    ));
    label.add_controller(leave);
}

/// Marks a label the pointer pressed on, whose selection is the user's.
const SELECTED_BY_POINTER: &str = "selected-by-pointer";

/// A centred, wrapping label. Wrapping keeps a narrow window at its size
/// when a folder is empty (`.empty-state{text-align:center}` wraps in
/// app.js too).
fn centred_text() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build()
}

impl EmptyPage {
    /// A page that says nothing yet.
    pub(super) fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(PART_GAP)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["empty-state"])
            .build();
        let icon = icons::image(Icon::Folder, STATE_GLYPH);
        let title = centred_text();
        title.add_css_class("empty-title");
        let message = centred_text();
        message.set_max_width_chars(MESSAGE_WIDTH_CHARS);
        message.set_selectable(true);
        keep_unselected_on_focus(&message);
        let details_text = gtk::Label::builder()
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(MESSAGE_WIDTH_CHARS)
            .xalign(0.0)
            .selectable(true)
            .css_classes(["dim-label"])
            .build();
        keep_unselected_on_focus(&details_text);
        let details = gtk::Expander::builder()
            .label(gettext("Details"))
            .child(&details_text)
            .halign(gtk::Align::Center)
            .visible(false)
            .build();
        let retry = gtk::Button::builder()
            .label(ox_core::i18n::gettext("Try again"))
            .action_name(WindowAction::Refresh.detailed_name())
            .halign(gtk::Align::Center)
            .css_classes([ButtonStyle::Bordered.css_class()])
            .visible(false)
            .build();
        root.append(&icon);
        root.append(&title);
        root.append(&message);
        root.append(&details);
        root.append(&retry);
        Self {
            root,
            icon,
            title,
            message,
            details,
            retry,
        }
    }

    /// Makes `area`, which holds the page, named by its title and
    /// described by its message, for screen readers.
    pub(super) fn label_area(&self, area: &impl IsA<gtk::Accessible>) {
        area.update_relation(&[
            gtk::accessible::Relation::LabelledBy(&[self.title.upcast_ref()]),
            gtk::accessible::Relation::DescribedBy(&[self.message.upcast_ref()]),
        ]);
    }

    /// Shows `state`, with the app.js wording (`renderRows`).
    pub(super) fn show(&self, state: &EmptyState) {
        let glyph = match state {
            // A folder on this computer gets a folder, as Windows' "Location
            // is not available"; only shares and servers get the network.
            EmptyState::Unavailable { local: true, .. } => Icon::Folder,
            EmptyState::Unavailable { .. } => Icon::Organization,
            EmptyState::ArchiveUnavailable(_) => Icon::FolderZip,
            EmptyState::NoPhotos => Icon::Image,
            _ => Icon::Folder,
        };
        icons::set_icon(&self.icon, glyph, STATE_GLYPH);
        let (title, message) = match state {
            EmptyState::Unavailable { message, .. } => (
                ox_core::i18n::gettext_static("This location is unavailable"),
                message.as_str(),
            ),
            EmptyState::ArchiveUnavailable(message) => (
                ox_core::i18n::gettext_static("This archive can't be opened"),
                message.as_str(),
            ),
            EmptyState::NoMatches(reason) => (
                ox_core::i18n::gettext_static("No matching items"),
                reason.as_str(),
            ),
            EmptyState::EmptyFolder => (
                ox_core::i18n::gettext_static("This folder is empty"),
                ox_core::i18n::gettext_static("Create a folder or paste files here."),
            ),
            EmptyState::NoShares => (ox_core::i18n::gettext_static("No shared folders found"), ""),
            EmptyState::NoDevices => (
                ox_core::i18n::gettext_static("No MTP-compatible devices found"),
                "",
            ),
            EmptyState::NoPhotos => (
                ox_core::i18n::gettext_static("No photos to show"),
                ox_core::i18n::gettext_static("Photos in your Pictures folder show here."),
            ),
        };
        self.title.set_text(title);
        self.message.set_text(message);
        // Only a message worth copying (an error, a search's reason) can
        // be selected; Gallery's hint would show selected when focused, and
        // an archive's reason would take the keyboard and show selected as
        // the archive opens.
        self.message.set_selectable(!matches!(
            state,
            EmptyState::NoPhotos | EmptyState::ArchiveUnavailable(_)
        ));
        self.message.set_visible(!message.is_empty());
        // A label that already had the keyboard selects its new text whole
        // once it is drawn again, which the focus guard does not see; the
        // new text starts unselected, as a page that just opened.
        self.message.remove_css_class(SELECTED_BY_POINTER);
        unselect_when_drawn(&self.message);
        let details = match state {
            EmptyState::Unavailable {
                details: Some(details),
                ..
            } => Some(details.as_str()),
            _ => None,
        };
        if let Some(text) = self.details.child().and_downcast::<gtk::Label>() {
            text.set_text(details.unwrap_or_default());
        }
        self.details.set_expanded(false);
        self.details.set_visible(details.is_some());
        self.retry
            .set_visible(matches!(state, EmptyState::Unavailable { .. }));
    }

    /// True while the page says the location is unavailable, which only
    /// that state offers Try again for.
    pub(super) fn is_unavailable(&self) -> bool {
        self.retry.get_visible()
    }

    /// The title shown, for tests.
    #[cfg(test)]
    pub(super) fn title(&self) -> String {
        self.title.text().to_string()
    }

    /// The message shown under the title, for tests.
    #[cfg(test)]
    pub(super) fn message(&self) -> String {
        self.message.text().to_string()
    }

    /// The icon shown, for tests.
    #[cfg(test)]
    pub(super) fn icon_name(&self) -> Option<String> {
        self.icon.icon_name().map(|name| name.to_string())
    }

    /// True when the message's text can be selected, or is, for tests.
    #[cfg(test)]
    pub(super) fn message_is_selectable(&self) -> bool {
        self.message.is_selectable() || self.message.selection_bounds().is_some()
    }

    /// True when a Try again button that refreshes is shown, for tests.
    #[cfg(test)]
    pub(super) fn offers_try_again(&self) -> bool {
        let retry = &self.retry;
        let refresh = WindowAction::Refresh.detailed_name();
        let runs_refresh = retry.action_name().as_deref() == Some(refresh.as_str());
        retry.is_visible() && runs_refresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-048
    #[test]
    fn empty_listings_say_what_is_missing_where() {
        assert_eq!(EmptyState::empty_listing("trash:///"), EmptyState::EmptyFolder);
        assert_eq!(EmptyState::empty_listing("smb://nas/"), EmptyState::NoShares);
        assert_eq!(
            EmptyState::empty_listing("smb://nas/Team"),
            EmptyState::EmptyFolder
        );
        assert_eq!(EmptyState::empty_listing("mtp:///"), EmptyState::NoDevices);
        assert_eq!(
            EmptyState::empty_listing("file:///tmp/empty"),
            EmptyState::EmptyFolder
        );
    }
}
