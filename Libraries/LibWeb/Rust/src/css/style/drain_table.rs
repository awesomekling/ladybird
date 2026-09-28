/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! A table the engine keeps for the host's drain of a style transaction, which the engine's home keeps a copy of: the
//! main thread reads and takes an entry there without asking the render owner, and tells the engine what it took.

use super::fast_hash::FastMap as HashMap;
use std::hash::Hash;
use std::ops::Deref;

/// Every write but a take the host made in its copy already moves the table, and whoever reaches the engine hands the
/// home a new copy of a moved table as it is done with the engine.
pub(crate) struct DrainTable<K, V> {
    entries: HashMap<K, V>,
    moved: bool,
}

impl<K, V> Default for DrainTable<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::default(),
            moved: false,
        }
    }
}

impl<K, V> Deref for DrainTable<K, V> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<K: Eq + Hash, V: Default> DrainTable<K, V> {
    pub(crate) fn entry_or_default(&mut self, key: K) -> &mut V {
        self.moved = true;
        self.entries.entry(key).or_default()
    }
}

impl<K: Eq + Hash, V> DrainTable<K, V> {
    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.moved = true;
        self.entries.insert(key, value)
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let removed = self.entries.remove(key);
        self.moved |= removed.is_some();
        removed
    }

    pub(crate) fn clear(&mut self) {
        self.moved |= !self.entries.is_empty();
        self.entries.clear();
    }

    /// Takes the entry the host took from its copy already.
    pub(crate) fn take_taken_by_host(&mut self, key: &K) -> Option<V> {
        self.entries.remove(key)
    }

    /// Whether anything but the host's takes moved the table since the home's copy was made, which a new copy
    /// follows.
    pub(crate) fn take_moved(&mut self) -> bool {
        std::mem::take(&mut self.moved)
    }
}
