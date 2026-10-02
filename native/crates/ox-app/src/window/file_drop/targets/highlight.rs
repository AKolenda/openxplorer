// SPDX-License-Identifier: AGPL-3.0-only
//! The highlight that shows where a drop would go (DND-011, DND-014,
//! TAB-018): a folder row or the whole folder view, the sidebar's row or
//! Quick access line, a crumb, or a tab. Holding a drag over a tab for
//! 800 ms shows it, as Windows Explorer does. No highlight means no drop.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::spot::DropSpot;
use super::spring::folder_to_open;
use super::DropZone;
use crate::window::file_drop::DropDestination;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// How long a drag must stay over a tab before the tab is shown.
const TAB_HOVER_DELAY: Duration = Duration::from_millis(800);

/// The CSS class of the folder view while a drop would go into the
/// folder it shows (`#file-scroll.file-drop-active`).
pub(super) const VIEW_DROP_CLASS: &str = "file-drop-active";

impl BrowserWindow {
    /// Highlights `spot` in `zone`, or nothing there, and opens the
    /// folder under it when the drag stays there.
    pub(super) fn show_drop_spot(&self, zone: DropZone, spot: Option<&DropSpot>) {
        self.open_folder_after_hover(spot.and_then(folder_to_open));
        match zone {
            DropZone::FolderView => self.show_folder_view_spot(spot),
            DropZone::Sidebar => {
                let sidebar_spot = match spot {
                    Some(DropSpot::Sidebar(spot)) => Some(spot),
                    _ => None,
                };
                self.sidebar().show_drop_spot(sidebar_spot);
            }
            DropZone::Breadcrumbs => {
                let crumb = match spot {
                    Some(DropSpot::Crumb(folder)) => Some(folder.as_str()),
                    _ => None,
                };
                self.address_bar().highlight_crumb(crumb);
            }
            DropZone::CrumbMenu => {
                let folder = match spot {
                    Some(DropSpot::Crumb(folder)) => Some(folder.as_str()),
                    _ => None,
                };
                self.highlight_drag_crumb_menu(folder);
            }
            DropZone::Tabs => {
                let tab = match spot {
                    Some(DropSpot::Tab { id, .. }) => Some(*id),
                    _ => None,
                };
                self.tab_strip().highlight_drop_tab(tab);
                self.show_tab_after_hover(tab);
            }
        }
    }

    /// Highlights the folder view's row, or the whole view, of `spot`,
    /// and says which program a drop there opens.
    fn show_folder_view_spot(&self, spot: Option<&DropSpot>) {
        let pane = self.folder_pane();
        let (row, whole_view, program) = match spot {
            Some(DropSpot::FolderView { destination, row }) => {
                let program = match destination {
                    DropDestination::Program(program) => Some(program.name.clone()),
                    DropDestination::Folder(_)
                    | DropDestination::Volume(_)
                    | DropDestination::QuickAccess { .. }
                    | DropDestination::RecycleBin
                    | DropDestination::NewTabs => None,
                };
                (*row, row.is_none(), program)
            }
            _ => (None, false, None),
        };
        pane.owners().show_drop_target(row);
        let view = pane.view_widget();
        if whole_view {
            view.add_css_class(VIEW_DROP_CLASS);
        } else {
            view.remove_css_class(VIEW_DROP_CLASS);
        }
        let hint = program
            .map(|name| ox_core::i18n::format_message("Open with {name}", &[("name", &(name).to_string())]));
        pane.show_drag_hint(hint.as_deref());
    }

    /// Shows tab `id` once a drag has stayed over it for
    /// [`TAB_HOVER_DELAY`]; a drag that leaves it, or `None`, stops that.
    fn show_tab_after_hover(&self, id: Option<TabId>) {
        let waiting = self
            .imp()
            .tab_hover
            .borrow()
            .as_ref()
            .map(|(waiting, _)| *waiting);
        if waiting == id {
            return;
        }
        if let Some((_, timer)) = self.imp().tab_hover.take() {
            timer.remove();
        }
        let Some(id) = id else {
            return;
        };
        let timer = glib::timeout_add_local_once(
            TAB_HOVER_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().tab_hover.replace(None);
                    window.switch_tab(id);
                }
            ),
        );
        self.imp().tab_hover.replace(Some((id, timer)));
    }
}
