// SPDX-License-Identifier: AGPL-3.0-only
//! A fixed number of slots for work that runs off the main thread, so a
//! quick scroll through thousands of pictures decodes or makes only a few
//! at a time. A request waits for a slot; dropping it while it waits, as
//! a row scrolled away does, leaves the queue at once.

use gtk::gio;
use gtk::prelude::*;

/// A pool of slots. Taking one waits until one is free.
#[derive(Debug)]
pub(crate) struct Slots {
    free: async_channel::Sender<()>,
    taken: async_channel::Receiver<()>,
}

/// A slot; it is free again when dropped.
#[derive(Debug)]
pub(crate) struct Slot(async_channel::Sender<()>);

/// Cancels a worker's I/O when its caller stops waiting for it.
struct CancelOnDrop(gio::Cancellable);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

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

    /// Runs blocking work in a slot held by the worker itself. Dropping
    /// the future cancels its I/O, but cannot release its slot until the
    /// worker actually exits: GIO's blocking tasks outlive their futures.
    pub(crate) async fn blocking<T, F>(&self, work: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce(&gio::Cancellable) -> T + Send + 'static,
    {
        let slot = self.take().await;
        let cancellation = gio::Cancellable::new();
        let _cancel_on_drop = CancelOnDrop(cancellation.clone());
        gio::spawn_blocking(move || {
            let _slot = slot;
            (!cancellation.is_cancelled()).then(|| work(&cancellation))
        })
        .await
        .ok()
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::mpsc;
    use std::task::{Context, Waker};
    use std::time::Duration;

    use super::*;

    /// A scrolled-away request cancels its worker, without making room
    /// for another worker before the first actually stops.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn cancelling_a_request_keeps_its_slot_until_the_worker_stops() {
        let pool = Slots::new(1);
        let (started, running) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut request = Box::pin(pool.blocking(move |cancel| {
            started.send(cancel.clone()).expect("the caller is waiting");
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }));
        assert!(request
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        let cancellation = running
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker started");

        drop(request);

        assert!(cancellation.is_cancelled());
        assert!(pool.taken.try_recv().is_err(), "the worker still owns its slot");
        release.send(()).expect("the worker is still waiting");
        let slot = gtk::glib::MainContext::default().block_on(pool.take());
        drop(slot);
        assert_eq!(pool.taken.len(), 1, "the finished worker returned one slot");
    }
}
