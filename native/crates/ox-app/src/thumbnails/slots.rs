// SPDX-License-Identifier: AGPL-3.0-only
//! A fixed number of slots for work that runs off the main thread, so a
//! quick scroll through thousands of pictures decodes or makes only a few
//! at a time. A request waits for a slot; dropping it while it waits, as
//! a row scrolled away does, leaves the queue at once.

/// A pool of slots. Taking one waits until one is free.
#[derive(Debug)]
pub(crate) struct Slots {
    free: async_channel::Sender<()>,
    taken: async_channel::Receiver<()>,
}

/// A slot; it is free again when dropped.
#[derive(Debug)]
pub(crate) struct Slot(async_channel::Sender<()>);

impl Drop for Slot {
    fn drop(&mut self) {
        // The pool's receiver lives as long as the thread's pools do.
        let _ = self.0.try_send(());
    }
}

impl Slots {
    /// A pool of `count` free slots.
    pub(crate) fn new(count: usize) -> Self {
        let (free, taken) = async_channel::bounded(count);
        for _ in 0..count {
            let _ = free.try_send(());
        }
        Self { free, taken }
    }

    /// Waits for a free slot.
    pub(crate) async fn take(&self) -> Slot {
        // The pool keeps a sender, so the channel never closes.
        let _ = self.taken.recv().await;
        Slot(self.free.clone())
    }
}
