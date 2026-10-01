// SPDX-License-Identifier: AGPL-3.0-only
//! Emblems over a listed item's icon: the link glyph on a symbolic link,
//! as Dolphin and Windows (its shortcut arrow) mark one, and the lock on
//! an item GIO reports as not writable, the parity behaviour. Dolphin's
//! `KFileItem::overlays` locks items that are not readable instead; an item
//! the user cannot change is the more useful warning in a file manager
//! that writes. Mounted network locations carry the green network bar
//! instead ([`super::Art::Network`]). Search results carry no emblems:
//! the search index does not record links or write access.
//!
//! An emblem is a real bundled glyph on a small plate in the window's
//! background colour, so it reads on any artwork. The link sits in the
//! bottom-left corner, where Windows draws the shortcut arrow; the lock
//! in the top-right corner, clear of the zip badge in the bottom-right.

use gtk::prelude::*;
use ox_core::entry::Entry;

use super::Icon;

/// The CSS class of every emblem, for the skin's plate.
const EMBLEM_CLASS: &str = "emblem";

/// The emblems an item's icon carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Emblems {
    /// The item is a symbolic link.
    pub(crate) link: bool,
    /// The user cannot change the item.
    pub(crate) read_only: bool,
}

impl Emblems {
    /// The emblems of a listed item. Shares in a server listing and items
    /// in the Recycle Bin carry none: they are not the user's files to
    /// write, and a lock on every one of them would say nothing.
    pub(crate) fn for_entry(entry: &Entry) -> Self {
        let is_plain_item = !entry.is_virtual && entry.trash_orig_path.is_none();
        Self {
            link: is_plain_item && entry.is_symlink,
            read_only: is_plain_item && entry.can_write == Some(false),
        }
    }

    /// True when the icon carries no emblem.
    pub(crate) fn is_empty(self) -> bool {
        !self.link && !self.read_only
    }
}

/// An emblem's edge for an icon `size` pixels high: half the icon, and
/// never under 8 pixels, so the glyph stays legible on 16-pixel rows.
const fn emblem_size(size: i32) -> i32 {
    let half = (size + 1) / 2;
    if half < 8 {
        8
    } else {
        half
    }
}

/// The two emblem images, laid over an icon once an item needs one.
#[derive(Debug)]
pub(super) struct EmblemPieces {
    /// The link glyph, bottom left.
    pub(super) link: gtk::Image,
    /// The lock, top right.
    pub(super) lock: gtk::Image,
}

impl EmblemPieces {
    pub(super) fn new() -> Self {
        Self {
            link: emblem_image(Icon::Link, gtk::Align::Start, gtk::Align::End),
            lock: emblem_image(Icon::ShieldLock, gtk::Align::End, gtk::Align::Start),
        }
    }

    /// Shows the emblems of `emblems` over an icon `size` pixels high.
    pub(super) fn show(&self, emblems: Emblems, size: i32) {
        for (image, is_shown) in [(&self.link, emblems.link), (&self.lock, emblems.read_only)] {
            image.set_pixel_size(emblem_size(size));
            image.set_visible(is_shown);
        }
    }
}

/// A hidden emblem image of `icon`, aligned to its corner.
fn emblem_image(icon: Icon, halign: gtk::Align, valign: gtk::Align) -> gtk::Image {
    let image = super::decorative_image();
    image.set_icon_name(Some(icon.name()));
    image.set_halign(halign);
    image.set_valign(valign);
    image.add_css_class(EMBLEM_CLASS);
    image.set_visible(false);
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::{Art, ArtImage};
    use crate::test_support::{file_entry, folder_entry};

    /// A symbolic link carries the link glyph in the bottom-left corner and
    /// an item the user cannot write the lock in the top-right; a share
    /// in a server listing and a deleted item carry none, and an image
    /// shown again for a plain file hides them.
    ///
    /// parity: LOOK-017
    #[gtk::test]
    fn links_and_read_only_items_carry_their_emblems() {
        let mut link = folder_entry("Projects");
        link.is_symlink = true;
        let mut read_only = file_entry("hosts");
        read_only.can_write = Some(false);
        let mut share = folder_entry("Media");
        share.is_virtual = true;
        share.can_write = Some(false);
        let mut deleted = file_entry("old.txt");
        deleted.can_write = Some(false);
        deleted.trash_orig_path = Some("/home/user/old.txt".into());

        let both = Emblems {
            link: true,
            read_only: true,
        };
        assert_eq!(
            Emblems::for_entry(&link),
            Emblems {
                link: true,
                ..Emblems::default()
            }
        );
        assert_eq!(
            Emblems::for_entry(&read_only),
            Emblems {
                read_only: true,
                ..Emblems::default()
            }
        );
        assert!(Emblems::for_entry(&share).is_empty());
        assert!(Emblems::for_entry(&deleted).is_empty());
        assert!(Emblems::for_entry(&file_entry("notes.txt")).is_empty());

        let image = ArtImage::new(Art::Folder, 16);
        image.set_emblems(both, 16);
        let pieces = image.emblem_pieces().expect("the emblems were made");
        assert_eq!(pieces.link.icon_name().as_deref(), Some(Icon::Link.name()));
        assert_eq!(pieces.lock.icon_name().as_deref(), Some(Icon::ShieldLock.name()));
        assert!(pieces.link.is_visible() && pieces.lock.is_visible());
        assert_eq!(pieces.link.pixel_size(), 8);
        assert_eq!(
            (pieces.link.halign(), pieces.link.valign()),
            (gtk::Align::Start, gtk::Align::End)
        );
        assert_eq!(
            (pieces.lock.halign(), pieces.lock.valign()),
            (gtk::Align::End, gtk::Align::Start)
        );
        assert!(pieces.link.has_css_class(EMBLEM_CLASS));

        image.set_emblems(Emblems::default(), 48);
        assert!(!pieces.link.is_visible() && !pieces.lock.is_visible());
        assert_eq!(emblem_size(48), 24);
    }
}
