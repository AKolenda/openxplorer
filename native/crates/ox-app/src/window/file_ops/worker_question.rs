// SPDX-License-Identifier: AGPL-3.0-only
//! Questions an operation's worker thread asks the user. The transfer
//! engine asks from its worker and waits; each question crosses to the main
//! loop through a channel, opens a dialog there, and the answer travels
//! back.

use std::future::Future;

use gtk::glib;

use crate::window::BrowserWindow;

/// A question from the worker and where its answer goes.
type Question<Q, A> = (Q, async_channel::Sender<A>);

/// The blocking function a worker calls to ask `ask` over `window` and wait
/// for the answer. A question the window can no longer show, because it
/// was closed or the dialog dropped its answer, is answered with
/// `unanswered`. The main-loop side ends when the worker drops the
/// function.
pub(super) fn worker_question<Q, A, F, Fut>(
    window: &BrowserWindow,
    unanswered: A,
    ask: F,
) -> impl Fn(&Q) -> A + Send + Sync + 'static
where
    Q: Clone + Send + 'static,
    A: Copy + Send + Sync + 'static,
    F: Fn(BrowserWindow, Q) -> Fut + 'static,
    Fut: Future<Output = A>,
{
    let (questions, question_queue) = async_channel::unbounded::<Question<Q, A>>();
    glib::spawn_future_local(glib::clone!(
        #[weak]
        window,
        async move {
            while let Ok((question, reply)) = question_queue.recv().await {
                let answer = ask(window.clone(), question).await;
                let _ = reply.send(answer).await;
            }
        }
    ));
    move |question: &Q| {
        let (reply, answer) = async_channel::bounded(1);
        if questions.send_blocking((question.clone(), reply)).is_err() {
            return unanswered;
        }
        answer.recv_blocking().unwrap_or(unanswered)
    }
}
