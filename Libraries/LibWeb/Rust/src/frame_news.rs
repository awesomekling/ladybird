/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the render owner tells the main thread of the frames it sent: the frame's news.
//!
//! The main thread sends a rendering update's frame and goes on ([`FrameSent`], which holds nothing to wait on). The
//! owner posts what the frame left for the main thread into the main thread's [`NewsInbox`] as a [`FrameNews`], and
//! the main thread adopts it only at an [`AdoptionPoint`]: at the top of its event loop ([`TaskBoundary`]), or where a
//! join of the frame in flight took the frame back ([`LegacyJoin`], which goes away with the joins).

use crate::flight::{FfiFlightOutcome, FlightRan};
use crate::stage_thread::FrameOwns;
use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::mpsc::{Receiver, Sender, channel};

/// The number of a frame the main thread sent, in the order it sent them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FrameSeq(u64);

/// What the owner left the main thread of one frame it ran.
pub(crate) enum FrameNews {
    /// The frame's flight ended: where, and what its stages left.
    FlightEnded {
        seq: FrameSeq,
        outcome: FfiFlightOutcome,
        ran: FlightRan,
    },
}

/// That the main thread sent a frame. There is nothing in it to wait on: what the frame leaves comes as news, which
/// the main thread adopts at an [`AdoptionPoint`].
pub(crate) struct FrameSent(PhantomData<*const ()>);

/// Where the owner posts the news of a frame: to the inbox of the main thread that sent it.
pub(crate) struct NewsSender(Sender<FrameOwns<FrameNews>>);

impl NewsSender {
    pub(crate) fn post(&self, news: FrameNews) {
        // SAFETY: What a frame left is the frame's, which the main thread reaches only as it adopts the news.
        let news = unsafe { FrameOwns::new(news) };
        // The main thread keeps its inbox for as long as it runs.
        let _ = self.0.send(news);
    }
}

/// The main thread's inbox of frame news.
pub(crate) struct NewsInbox {
    news: Receiver<FrameOwns<FrameNews>>,
    sender: Sender<FrameOwns<FrameNews>>,
}

thread_local! {
    // On a main thread, the inbox of the news of the frames it sent.
    static INBOX: NewsInbox = {
        let (sender, news) = channel();
        NewsInbox { news, sender }
    };
    // On a main thread, the number of the last frame it sent.
    static LAST_SENT: Cell<u64> = const { Cell::new(0) };
}

mod sealed {
    pub trait Sealed {}
}

/// Where the main thread may adopt the news of the frames it sent: only there does what a frame left reach it.
pub(crate) trait AdoptionPoint: sealed::Sealed {}

/// The top of the main thread's event loop, before its next task: where a frame's news is adopted as a rule.
pub(crate) struct TaskBoundary(PhantomData<*const ()>);

impl sealed::Sealed for TaskBoundary {}
impl AdoptionPoint for TaskBoundary {}

impl TaskBoundary {
    /// Only the event loop's step 1 mints one, through the frame scheduler's consume of a finished frame.
    fn at_step_one() -> Self {
        Self(PhantomData)
    }
}

/// A join of the frame in flight, which took the frame back before the main thread went on. Only the joins that wait
/// for the frame mint one, and it goes away with them.
pub(crate) struct LegacyJoin(PhantomData<*const ()>);

impl sealed::Sealed for LegacyJoin {}
impl AdoptionPoint for LegacyJoin {}

impl LegacyJoin {
    pub(crate) fn waiting_for_the_frame() -> Self {
        Self(PhantomData)
    }
}

/// Numbers the next frame the calling main thread sends, and says where the owner posts its news.
pub(crate) fn send_frame() -> (FrameSent, FrameSeq, NewsSender) {
    let seq = FrameSeq(LAST_SENT.with(|last| {
        last.set(last.get() + 1);
        last.get()
    }));
    let sender = INBOX.with(|inbox| NewsSender(inbox.sender.clone()));
    (FrameSent(PhantomData), seq, sender)
}

/// Adopts, at `at`, the news of the frames the calling main thread sent that the owner has posted, in the order it
/// posted them.
pub(crate) fn adopt_news(_at: &impl AdoptionPoint) {
    while let Some(news) = INBOX.with(|inbox| inbox.news.try_recv().ok()) {
        match news.into_inner() {
            FrameNews::FlightEnded { seq, outcome, ran } => crate::flight::adopt_flight_end(seq, outcome, ran),
        }
    }
    // A flight whose run panicked posted nothing: it ends where it began.
    crate::flight::end_flights_without_news();
}

/// Takes the main thread's frame in flight back at the top of its event loop, once every stage of it has finished,
/// and adopts its news. Returns whether there was a finished frame to take. Never waits.
#[unsafe(no_mangle)]
pub extern "C" fn rust_frame_news_take_finished_frame() -> bool {
    let at = TaskBoundary::at_step_one();
    crate::stage_thread::take_finished_frame(&at)
}
