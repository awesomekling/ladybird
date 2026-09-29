/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What one thread builds and other threads read, freed by the thread that built it.
//!
//! A block the allocator hands one thread and another thread frees goes back to the first thread's heap only when that
//! thread collects the blocks others freed into it, which costs it a pass over every page they touched. The render owner
//! builds the rows the document and painting threads read, and a reader that let go of them last freed them there.
//!
//! A [`Lender`] shares what it builds as [`Lent`] references and keeps one of its own on each, until it holds the last
//! one: then it drops it itself, as it takes back what its readers let go of. A reader's reference is never the last,
//! so what a reader drops only lets go of it.

use std::ops::Deref;
use std::sync::Arc;

/// A reference on what a [`Lender`] shares. Only a lender makes one.
pub(crate) struct Lent<T>(Arc<T>);

impl<T> Clone for Lent<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Deref for Lent<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// An empty value, which no lender shares: a slot holds one until something is lent to it.
impl<T: Default> Default for Lent<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<T> Lent<T> {
    pub(crate) fn ptr_eq(this: &Self, other: &Self) -> bool {
        Arc::ptr_eq(&this.0, &other.0)
    }

    pub(crate) fn as_ptr(this: &Self) -> *const T {
        Arc::as_ptr(&this.0)
    }

    /// The reference as a handle for C++, which gives it back to [`Self::from_raw`].
    pub(crate) fn into_raw(this: Self) -> *const T {
        Arc::into_raw(this.0)
    }

    /// # Safety
    ///
    /// `raw` must come from [`Self::into_raw`], and be given back once.
    pub(crate) unsafe fn from_raw(raw: *const T) -> Self {
        // SAFETY: Guaranteed by the caller.
        Self(unsafe { Arc::from_raw(raw) })
    }
}

/// What a thread lent and still holds a reference on, which it drops once its readers let go of it.
pub(crate) struct Lender<T>(Vec<Arc<T>>);

impl<T> Default for Lender<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Lender<T> {
    /// Shares `value`.
    pub(crate) fn lend(&mut self, value: T) -> Lent<T> {
        let lent = Arc::new(value);
        self.0.push(Arc::clone(&lent));
        Lent(lent)
    }

    /// Drops what no reader holds any more. Nothing takes a new reference on what the lender holds the last one of.
    pub(crate) fn take_back_let_go(&mut self) {
        self.0.retain(|lent| Arc::strong_count(lent) > 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lender_drops_what_its_readers_let_go_of() {
        let mut lender = Lender::default();
        let first = lender.lend(1);
        let reader = first.clone();
        drop(first);
        let second = lender.lend(2);
        assert_eq!(lender.0.len(), 2, "a reader still holds the first");
        drop(reader);
        lender.take_back_let_go();
        assert_eq!(lender.0.len(), 1);
        assert_eq!(*second, 2);
    }
}
