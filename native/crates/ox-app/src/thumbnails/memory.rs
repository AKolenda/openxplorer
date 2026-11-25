// SPDX-License-Identifier: AGPL-3.0-only
//! The previews shown lately, kept decoded: a folder visited again, or a
//! row scrolled back into view, shows its thumbnails on the frame it
//! first draws, as Windows does from its cache, without reading or
//! decoding anything. The least recently shown go first once the kept
//! pictures pass their budget, and a preview of another version of its
//! file (another time or size) is never handed out.

use std::collections::{BTreeMap, HashMap};

use gtk::gdk;
use gtk::prelude::*;

/// A kept preview's item and the edge it was asked for.
pub(super) type Key = (String, i32);

/// A kept preview, with the version of its file it shows.
#[derive(Debug)]
struct Kept {
    texture: gdk::Texture,
    modified: Option<u64>,
    size: Option<u64>,
    /// When it was last handed out, which orders the eviction.
    used: u64,
    /// What its pixels take.
    bytes: usize,
}

/// The previews kept, within a budget of bytes.
#[derive(Debug)]
pub(super) struct Memory {
    kept: HashMap<Key, Kept>,
    /// The keys by when they were last used, oldest first.
    by_use: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
    budget: usize,
}

impl Memory {
    /// An empty memory that keeps previews of at most `budget` bytes.
    pub(super) fn new(budget: usize) -> Self {
        Self {
            kept: HashMap::new(),
            by_use: BTreeMap::new(),
            clock: 0,
            bytes: 0,
            budget,
        }
    }

    /// The preview kept for `key`, made from the version of the file
    /// modified at `modified` and `size` bytes long; one of another version
    /// is dropped.
    pub(super) fn get(
        &mut self,
        key: &Key,
        modified: Option<u64>,
        size: Option<u64>,
    ) -> Option<gdk::Texture> {
        let current = self.kept.get(key).map(|kept| {
            kept.modified == modified && (kept.size.is_none() || size.is_none() || kept.size == size)
        })?;
        if !current {
            self.remove(key);
            return None;
        }
        self.clock += 1;
        let clock = self.clock;
        let kept = self.kept.get_mut(key)?;
        self.by_use.remove(&kept.used);
        kept.used = clock;
        self.by_use.insert(clock, key.clone());
        Some(kept.texture.clone())
    }

    /// Keeps `texture` for `key`, made from the version modified at
    /// `modified` and `size` bytes long, dropping the least recently used
    /// previews until the rest fit the budget.
    pub(super) fn put(&mut self, key: Key, texture: gdk::Texture, modified: Option<u64>, size: Option<u64>) {
        self.remove(&key);
        let bytes = texture_bytes(&texture);
        if bytes > self.budget {
            return;
        }
        while self.bytes + bytes > self.budget {
            let Some((_, oldest)) = self.by_use.pop_first() else {
                break;
            };
            if let Some(kept) = self.kept.remove(&oldest) {
                self.bytes -= kept.bytes;
            }
        }
        self.clock += 1;
        self.by_use.insert(self.clock, key.clone());
        self.bytes += bytes;
        self.kept.insert(
            key,
            Kept {
                texture,
                modified,
                size,
                used: self.clock,
                bytes,
            },
        );
    }

    /// Forgets every preview of the item at `uri`, at any size.
    pub(super) fn forget(&mut self, uri: &str) {
        let keys: Vec<Key> = self
            .kept
            .keys()
            .filter(|(seen, _)| seen == uri)
            .cloned()
            .collect();
        for key in keys {
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &Key) {
        if let Some(kept) = self.kept.remove(key) {
            self.by_use.remove(&kept.used);
            self.bytes -= kept.bytes;
        }
    }

    /// Lets go of every kept preview.
    pub(super) fn clear(&mut self) {
        self.kept.clear();
        self.by_use.clear();
        self.bytes = 0;
    }

    /// How many previews are kept.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.kept.len()
    }

    /// What the kept previews' pixels take.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
}

/// What a texture's pixels take, at four bytes a pixel.
fn texture_bytes(texture: &gdk::Texture) -> usize {
    let pixels = i64::from(texture.width()) * i64::from(texture.height());
    usize::try_from(pixels.saturating_mul(4)).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use gtk::gdk_pixbuf::{Colorspace, Pixbuf};

    use super::*;

    fn texture(edge: i32) -> gdk::Texture {
        let pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, edge, edge).expect("a picture");
        pixbuf.fill(0x3366_99ff);
        gdk::Texture::for_pixbuf(&pixbuf)
    }

    fn key(name: &str) -> Key {
        (format!("file:///home/demo/Pictures/{name}.jpg"), 96)
    }

    /// The least recently shown previews go first once the budget is
    /// passed; one shown again stays.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn the_least_recently_shown_previews_go_first() {
        // Room for three previews of 10 × 10 pixels.
        let mut memory = Memory::new(3 * 400);
        for name in ["a", "b", "c"] {
            memory.put(key(name), texture(10), Some(1), Some(10));
        }
        assert!(memory.get(&key("a"), Some(1), Some(10)).is_some());

        memory.put(key("d"), texture(10), Some(1), Some(10));

        assert_eq!(memory.len(), 3);
        assert!(
            memory.get(&key("b"), Some(1), Some(10)).is_none(),
            "the oldest went"
        );
        for name in ["a", "c", "d"] {
            assert!(
                memory.get(&key(name), Some(1), Some(10)).is_some(),
                "{name} stays"
            );
        }
        // One larger than the whole budget is not kept, and drops nothing.
        memory.put(key("e"), texture(100), Some(1), Some(10));
        assert_eq!(memory.len(), 3);
    }

    /// A preview of another version of its file is dropped rather than
    /// shown: the file changed since.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_changed_file_drops_its_kept_preview() {
        let mut memory = Memory::new(1 << 20);
        memory.put(key("a"), texture(10), Some(1), Some(10));
        assert!(
            memory.get(&key("a"), Some(2), Some(10)).is_none(),
            "modified again"
        );
        assert_eq!(memory.len(), 0);
        memory.put(key("a"), texture(10), Some(1), Some(10));
        assert!(memory.get(&key("a"), Some(1), Some(11)).is_none(), "another size");
        memory.put(key("a"), texture(10), Some(1), Some(10));
        memory.put((key("a").0, 256), texture(20), Some(1), Some(10));
        memory.forget(&key("a").0);
        assert_eq!(memory.len(), 0, "every size of a moved file is forgotten");
    }
}
