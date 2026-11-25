// SPDX-License-Identifier: AGPL-3.0-only
//! A fixed number of slots for work that runs off the main thread, so a
//! quick scroll through thousands of pictures decodes or makes only a few
//! at a time. A request waits for a slot; dropping it while it waits, as
//! a row scrolled away does, leaves the queue at once.

use std::cell::Cell;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};

/// How often a request for an item off the screen looks again whether
/// its turn has come.
const TURN_POLL: Duration = Duration::from_millis(20);

/// A pool of slots. Taking one waits until one is free.
#[derive(Debug)]
pub(crate) struct Slots {
    free: async_channel::Sender<()>,
    taken: async_channel::Receiver<()>,
    /// How many requests for items on the screen wait for a slot, which
    /// requests for items off it let go first ([`Self::take_in_turn`]).
    on_screen_waiting: Cell<usize>,
    /// How many requests for items off the screen wait in the queue: twice
    /// as many as there are slots, so a slot freed is taken at once.
    off_screen_queued: Cell<usize>,
    /// How many slots there are.
    count: usize,
}

/// Counts a request while it waits in the queue.
struct Waiting<'a>(&'a Cell<usize>);

impl<'a> Waiting<'a> {
    fn new(count: &'a Cell<usize>) -> Self {
        count.set(count.get() + 1);
        Self(count)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
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
        Self {
            free,
            taken,
            on_screen_waiting: Cell::new(0),
            off_screen_queued: Cell::new(0),
            count,
        }
    }

    /// Waits for a free slot, the items on the screen first: a request
    /// whose item `on_screen` says is off it joins the queue only while no
    /// request for an item on it waits, and while fewer requests off it
    /// than twice the slots wait there, so the tiles in view get their
    /// thumbnails made before the many GTK binds around them, and a slot
    /// freed is still taken at once. It looks again every [`TURN_POLL`],
    /// as the view may scroll meanwhile.
    pub(crate) async fn take_in_turn(&self, on_screen: &dyn Fn() -> bool) -> Slot {
        loop {
            if on_screen() {
                let _waiting = Waiting::new(&self.on_screen_waiting);
                return self.take().await;
            }
            if self.on_screen_waiting.get() == 0 && self.off_screen_queued.get() < 2 * self.count {
                let _waiting = Waiting::new(&self.off_screen_queued);
                return self.take().await;
            }
            glib::timeout_future(TURN_POLL).await;
        }
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
        self.blocking_in_turn(&|| true, work).await
    }

    /// Runs blocking work as [`Self::blocking`] does, in a slot taken in
    /// turn ([`Self::take_in_turn`]).
    pub(crate) async fn blocking_in_turn<T, F>(&self, on_screen: &dyn Fn() -> bool, work: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce(&gio::Cancellable) -> T + Send + 'static,
    {
        let slot = self.take_in_turn(on_screen).await;
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

    /// A request for a tile on the screen takes a slot before the
    /// requests for tiles off it that wait outside the queue, however
    /// early they asked; twice as many of those as there are slots wait in
    /// the queue, so a slot freed is never left idle.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn tiles_on_the_screen_take_slots_first() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let pool = Rc::new(Slots::new(1));
        let context = glib::MainContext::default();
        let held = context.block_on(pool.take());
        let order = Rc::new(RefCell::new(Vec::new()));
        let ask = |name: &'static str, on_screen: bool| {
            let (pool, order) = (Rc::clone(&pool), Rc::clone(&order));
            let asked = glib::spawn_future_local(async move {
                let slot = pool.take_in_turn(&move || on_screen).await;
                order.borrow_mut().push(name);
                drop(slot);
            });
            for _ in 0..3 {
                context.iteration(false);
            }
            asked
        };
        let first = ask("off the screen, queued first", false);
        let second = ask("off the screen, queued second", false);
        let outside = ask("off the screen, outside", false);
        let on = ask("on the screen", true);
        assert_eq!(
            pool.off_screen_queued.get(),
            2,
            "two wait in the queue for the one slot"
        );
        drop(held);
        context.block_on(async {
            let _ = first.await;
            let _ = second.await;
            let _ = on.await;
            let _ = outside.await;
        });

        assert_eq!(
            *order.borrow(),
            [
                "off the screen, queued first",
                "off the screen, queued second",
                "on the screen",
                "off the screen, outside"
            ]
        );
    }
}
