// SPDX-License-Identifier: AGPL-3.0-only
//! The document types New offers while an application opens them: the
//! OpenDocument types, and Rich Text Document, which Windows 11 lists only
//! with WordPad (VM check 2026-11-22).
//!
//! Windows' New menu lists a file type once a program registers it: with
//! Microsoft Office, "Microsoft Word Document" and the others; with
//! LibreOffice, its OpenDocument types. Here New offers "OpenDocument
//! Presentation", "OpenDocument Spreadsheet" and "OpenDocument Text"
//! while the desktop has an application for the type, LibreOffice above
//! all, each with that application's icon. The desktop's application
//! database is read when a New menu is built, as the item menus read it
//! for their applications.

use ox_core::ops::NewDocument;

use crate::icons::Icon;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;

/// The New menu's name of `document`, a type an application must open.
fn document_label(document: NewDocument) -> Option<&'static str> {
    let label = match document {
        NewDocument::OpenDocumentPresentation => crate::i18n::message_id("OpenDocument Presentation"),
        NewDocument::OpenDocumentSpreadsheet => crate::i18n::message_id("OpenDocument Spreadsheet"),
        NewDocument::OpenDocumentText => crate::i18n::message_id("OpenDocument Text"),
        NewDocument::RichTextDocument => crate::i18n::message_id("Rich Text Document"),
        NewDocument::BitmapImage | NewDocument::TextDocument | NewDocument::CompressedFolder => return None,
    };
    Some(ox_core::i18n::gettext_static(label))
}

/// The application that opens a document's type.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentApplication {
    /// Its icon, serialized as GIO writes one, when it has one.
    icon: Option<String>,
}

/// The documents an application must open that one does now, in New's
/// order, each with that application's icon.
pub(in crate::window) fn offered_documents() -> Vec<(NewDocument, Option<String>)> {
    NewDocument::ALL
        .into_iter()
        .filter_map(|document| {
            let application = application_for(document.content_type()?)?;
            Some((document, application.icon))
        })
        .collect()
}

/// Whether New offers `document` now.
pub(in crate::window) fn is_offered(document: NewDocument) -> bool {
    document.content_type().is_none()
        || offered_documents()
            .iter()
            .any(|(offered, _)| *offered == document)
}

/// The New menu's items for the documents an application opens now:
/// each makes its document ([`WindowAction::NewOpenDocument`], whose
/// target is the extension) and shows its application's icon.
pub(in crate::window) fn document_entries() -> Vec<MenuEntry> {
    offered_documents()
        .into_iter()
        .filter_map(|(document, icon)| {
            let label = document_label(document)?;
            let item = if document == NewDocument::RichTextDocument {
                MenuItem::new(label, Icon::Document, WindowAction::NewRichTextDocument)
            } else {
                MenuItem::with_text_target(
                    label,
                    Icon::Document,
                    WindowAction::NewOpenDocument,
                    document.extension(),
                )
            };
            Some(item.with_application_icon(icon.as_deref()).into())
        })
        .collect()
}

/// The application that opens `content_type`, or `None` when none does.
fn application_for(content_type: &str) -> Option<DocumentApplication> {
    #[cfg(test)]
    {
        pretended_application_for(content_type)
    }
    #[cfg(not(test))]
    {
        installed_application_for(content_type)
    }
}

/// The icon of the desktop's application for `content_type`: its default,
/// else the first that says it opens the type, as GIO picks one.
#[cfg_attr(test, allow(dead_code, reason = "tests read it in their own test only"))]
fn installed_application_for(content_type: &str) -> Option<DocumentApplication> {
    let application = gtk::gio::AppInfo::default_for_type(content_type, false)?;
    Some(DocumentApplication {
        icon: ox_core::integration::ApplicationInfo::icon(&application),
    })
}

#[cfg(test)]
thread_local! {
    /// The content types tests pretend an application opens, with its
    /// icon; none unless a test says so, whatever is installed.
    static PRETENDED: std::cell::RefCell<Vec<(&'static str, &'static str)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// [`application_for`] in tests: what [`pretend_document_applications`]
/// says.
#[cfg(test)]
fn pretended_application_for(content_type: &str) -> Option<DocumentApplication> {
    PRETENDED.with(|pretended| {
        pretended
            .borrow()
            .iter()
            .find(|(pretended_type, _)| *pretended_type == content_type)
            .map(|(_, icon)| DocumentApplication {
                icon: Some((*icon).to_owned()),
            })
    })
}

/// Pretends, for tests, that applications open the content types of
/// `applications`, each with its icon, until the guard is dropped.
#[cfg(test)]
pub(crate) fn pretend_document_applications(
    applications: &[(&'static str, &'static str)],
) -> PretendedApplications {
    PRETENDED.with(|pretended| pretended.replace(applications.to_vec()));
    PretendedApplications
}

/// Ends [`pretend_document_applications`] when dropped.
#[cfg(test)]
pub(crate) struct PretendedApplications;

#[cfg(test)]
impl Drop for PretendedApplications {
    fn drop(&mut self) {
        PRETENDED.with(|pretended| pretended.borrow_mut().clear());
    }
}
