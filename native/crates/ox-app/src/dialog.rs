// SPDX-License-Identifier: AGPL-3.0-only
//! The app's dialog: one [`DialogFrame`] (title, message, body, error line
//! and buttons, built from the same fields), shown in one of two hosts.
//!
//! Ports `showModal`, `closeModal`, `showMessage` and `textField` of
//! `v2.0.0:desktop/ui/app.js`, refined to the `ContentDialog` of `WinUI`
//! (`native/docs/ui-spec.md` §4.11).
//!
//! - [`Dialog`] puts the frame in a modal window of its own over the
//!   browser window, for every question and form: the desktop dims and
//!   attaches it, and nothing behind it takes input until it is answered.
//!   [`Dialog::with_title_bar`] gives it a title bar with a close button,
//!   as Windows draws Format and Map network drive: a
//!   window that can be moved anywhere and is modal to its window alone.
//! - [`DialogLayer`] shows the frame on a dimmed layer inside the browser
//!   window, under its title bar, for a dialog that leaves the tab strip
//!   usable and stays in front across tab switches: Restore a copy, the
//!   archive dialogs and result messages. Properties is a window of its
//!   own ([`crate::properties::PropertiesWindow`]).
//!
//! Both hosts follow the same rules: Escape dismisses the dialog, the
//! content behind it takes no focus or clicks, the dialog is never taller
//! than its window less a margin (its body scrolls instead) and its first
//! frame is drawn at the size it keeps.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `frame` | [`DialogFrame`]: one dialog's title, body, error and buttons |
//! | `fields` | The fields, notes and rows dialog bodies are built from |
//! | `window` | [`Dialog`]: the frame in a modal window, and its answers |
//! | `layer` | [`DialogLayer`]: the frame on a layer inside the window |

mod fields;
mod frame;
mod layer;
mod window;

pub(crate) use fields::{check_row, labelled_entry, note, quiet_text, PropertyGrid};
pub(crate) use frame::{DialogFrame, DialogWidth};
pub(crate) use layer::DialogLayer;
pub(crate) use window::{show_message, Dialog, DialogButton};
