/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Columns whose published generations share storage with the column that goes on changing.
//!
//! A [`CowColumn`] keeps its rows in fixed-size chunks behind [`Arc`]s. A column holds whole
//! chunks, and a row it was never asked to hold reads as the default. Publishing clones the
//! vector of chunk pointers, so a [`ColumnSnapshot`] costs one reference count per chunk however
//! large the rows are. A write copies a chunk only when a snapshot still shares it, so the cost of
//! a generation follows the chunks written after it was published rather than the size of the
//! column. Both are `Send` and `Sync` when the row type is: the column is written through `&mut`,
//! and a snapshot never changes.

use std::sync::Arc;

/// A chunk starts on a cache line of its own, so rows do not share a line with the reference
/// counts or with another chunk.
#[derive(Clone)]
#[repr(align(64))]
struct Chunk<T, const CHUNK: usize>([T; CHUNK]);

pub(crate) struct CowColumn<T, const CHUNK: usize> {
    chunks: Vec<Arc<Chunk<T, CHUNK>>>,
    /// Whether each chunk is known to be unshared: the column made it writable after it last
    /// published, and nothing but publishing shares a chunk.
    unshared: Vec<bool>,
    written_since_publish: bool,
}

/// A generation of a [`CowColumn`], as it was when published. It does not see later writes.
pub(crate) struct ColumnSnapshot<T, const CHUNK: usize> {
    chunks: Vec<Arc<Chunk<T, CHUNK>>>,
}

impl<T, const CHUNK: usize> Default for CowColumn<T, CHUNK> {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            unshared: Vec::new(),
            written_since_publish: false,
        }
    }
}

impl<T, const CHUNK: usize> Default for ColumnSnapshot<T, CHUNK> {
    fn default() -> Self {
        Self { chunks: Vec::new() }
    }
}

impl<T, const CHUNK: usize> Clone for ColumnSnapshot<T, CHUNK> {
    fn clone(&self) -> Self {
        Self {
            chunks: self.chunks.clone(),
        }
    }
}

impl<T: Clone + Default, const CHUNK: usize> CowColumn<T, CHUNK> {
    const CHUNK_IS_NOT_EMPTY: () = assert!(CHUNK > 0);

    #[inline]
    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        Some(&self.chunks.get(index / CHUNK)?.0[index % CHUNK])
    }

    /// The row at `index`, for writing. Copies its chunk first if a snapshot shares it.
    #[inline]
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let chunk_index = index / CHUNK;
        let chunk = self.chunks.get_mut(chunk_index)?;
        self.written_since_publish = true;
        let rows = if self.unshared[chunk_index] {
            // SAFETY: The chunk has not been shared since `make_mut` below made it unique: only
            // `publish` clones a chunk, and it forgets which chunks are unshared. `&mut self`
            // keeps any other reference into the column from being live.
            unsafe { &mut *Arc::as_ptr(chunk).cast_mut() }
        } else {
            self.unshared[chunk_index] = true;
            Arc::make_mut(chunk)
        };
        Some(&mut rows.0[index % CHUNK])
    }

    /// Grows the column to hold at least `len` rows, the new ones default. A column never
    /// shrinks: rows are reset in place instead.
    pub(crate) fn grow_to(&mut self, len: usize) {
        let () = Self::CHUNK_IS_NOT_EMPTY;
        while self.chunks.len() * CHUNK < len {
            self.chunks.push(Arc::new(Chunk(std::array::from_fn(|_| T::default()))));
            self.unshared.push(true);
            self.written_since_publish = true;
        }
    }

    /// Whether the column changed since it last published.
    pub(crate) fn written_since_publish(&self) -> bool {
        self.written_since_publish
    }

    /// This generation of the column, sharing every chunk with it until the column writes one.
    pub(crate) fn publish(&mut self) -> ColumnSnapshot<T, CHUNK> {
        self.unshared.fill(false);
        self.written_since_publish = false;
        ColumnSnapshot {
            chunks: self.chunks.clone(),
        }
    }
}

impl<T, const CHUNK: usize> ColumnSnapshot<T, CHUNK> {
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        Some(&self.chunks.get(index / CHUNK)?.0[index % CHUNK])
    }
}

const _: () = {
    const fn assert_send_and_sync<T: Send + Sync>() {}
    assert_send_and_sync::<CowColumn<u64, 64>>();
    assert_send_and_sync::<ColumnSnapshot<u64, 64>>();
};

#[cfg(test)]
mod tests {
    use super::*;

    type Column = CowColumn<u32, 4>;

    fn column_of(values: &[u32]) -> Column {
        let mut column = Column::default();
        column.grow_to(values.len());
        for (index, &value) in values.iter().enumerate() {
            *column.get_mut(index).unwrap() = value;
        }
        column
    }

    fn rows(snapshot: &ColumnSnapshot<u32, 4>, count: usize) -> Vec<u32> {
        (0..count).map(|index| *snapshot.get(index).unwrap()).collect()
    }

    fn set(column: &mut Column, index: usize, value: u32) {
        *column.get_mut(index).unwrap() = value;
    }

    #[test]
    fn a_column_reads_back_what_was_written() {
        let mut column = column_of(&[1, 2, 3, 4, 5, 6]);
        set(&mut column, 4, 50);
        assert_eq!(
            (0..6).map(|index| *column.get(index).unwrap()).collect::<Vec<_>>(),
            [1, 2, 3, 4, 50, 6]
        );
    }

    #[test]
    fn a_column_holds_whole_chunks() {
        let mut column = column_of(&[1, 2, 3]);
        assert_eq!(column.get(3), Some(&0));
        assert_eq!(column.get(4), None);
        assert_eq!(column.get_mut(4), None);
        assert_eq!(column.publish().get(4), None);
    }

    #[test]
    fn growing_fills_with_defaults() {
        let mut column = column_of(&[7]);
        column.grow_to(9);
        assert_eq!(column.get(0), Some(&7));
        assert_eq!(column.get(11), Some(&0));
        assert_eq!(column.get(12), None);
        column.grow_to(3);
        assert_eq!(column.get(11), Some(&0));
    }

    #[test]
    fn a_snapshot_keeps_its_generation_while_the_column_changes() {
        let mut column = column_of(&[1, 2, 3, 4, 5, 6]);
        let first = column.publish();
        set(&mut column, 1, 20);
        column.grow_to(7);
        set(&mut column, 6, 7);
        let second = column.publish();
        set(&mut column, 5, 60);
        assert_eq!(rows(&first, 6), [1, 2, 3, 4, 5, 6]);
        assert_eq!(rows(&second, 7), [1, 20, 3, 4, 5, 6, 7]);
        assert_eq!(rows(&column.publish(), 7), [1, 20, 3, 4, 5, 60, 7]);
    }

    #[test]
    fn a_write_copies_only_the_chunk_a_snapshot_shares_and_only_once() {
        let mut column = column_of(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let snapshot = column.publish();
        set(&mut column, 5, 60);
        assert!(Arc::ptr_eq(&column.chunks[0], &snapshot.chunks[0]));
        assert!(!Arc::ptr_eq(&column.chunks[1], &snapshot.chunks[1]));
        let copied = Arc::as_ptr(&column.chunks[1]);
        set(&mut column, 6, 70);
        assert_eq!(Arc::as_ptr(&column.chunks[1]), copied);
        assert_eq!(rows(&snapshot, 8), [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn a_chunk_whose_snapshot_is_gone_is_written_in_place() {
        let mut column = column_of(&[1, 2, 3, 4]);
        drop(column.publish());
        let chunk = Arc::as_ptr(&column.chunks[0]);
        set(&mut column, 2, 30);
        assert_eq!(Arc::as_ptr(&column.chunks[0]), chunk);
    }

    #[test]
    fn a_column_knows_whether_it_changed_since_it_published() {
        let mut column = column_of(&[1, 2]);
        assert!(column.written_since_publish());
        drop(column.publish());
        assert!(!column.written_since_publish());
        column.get(0);
        assert!(!column.written_since_publish());
        set(&mut column, 0, 10);
        assert!(column.written_since_publish());
        drop(column.publish());
        column.grow_to(3);
        assert!(!column.written_since_publish());
        column.grow_to(5);
        assert!(column.written_since_publish());
    }

    #[test]
    fn a_snapshot_can_be_read_on_another_thread() {
        let mut column = column_of(&[1, 2, 3, 4, 5]);
        let snapshot = column.publish();
        let reader = std::thread::spawn(move || rows(&snapshot, 5));
        set(&mut column, 0, 10);
        assert_eq!(reader.join().unwrap(), [1, 2, 3, 4, 5]);
    }
}
