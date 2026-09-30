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

    pub(crate) fn retain(&mut self, keep: impl FnMut(&K, &mut V) -> bool) {
        let before = self.entries.len();
        self.entries.retain(keep);
        self.moved |= self.entries.len() != before;
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

/// A table the engine writes for the host's drain and never reads, which it hands its home by moving it: a table the
/// engine started over replaces the home's, and what the engine wrote into it since goes into the home's.
pub(crate) struct HandedTable<K, V> {
    entries: HashMap<K, V>,
    replaces: bool,
}

impl<K, V> Default for HandedTable<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::default(),
            replaces: false,
        }
    }
}

impl<K, V> Deref for HandedTable<K, V> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<K: Eq + Hash, V> HandedTable<K, V> {
    /// Starts the table over with `entries`.
    pub(crate) fn replace(&mut self, entries: impl IntoIterator<Item = (K, V)>) {
        self.entries.clear();
        self.entries.extend(entries);
        self.replaces = true;
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        self.entries.insert(key, value);
    }

    /// Follows `later`, what was written to the table since.
    pub(crate) fn follow(&mut self, later: Self) {
        if later.replaces {
            *self = later;
        } else {
            self.entries.extend(later.entries);
        }
    }
}

/// A table whose writes the engine's home follows key by key: each write names its key, and whoever reaches the engine
/// hands the home what the table holds now under each key written as it is done with the engine.
pub(crate) struct FollowedTable<K, V> {
    entries: HashMap<K, V>,
    written: Vec<K>,
}

impl<K, V> Default for FollowedTable<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::default(),
            written: Vec::new(),
        }
    }
}

impl<K, V> Deref for FollowedTable<K, V> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<K: Eq + Hash + Copy, V> FollowedTable<K, V> {
    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.written.push(key);
        self.entries.insert(key, value)
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let removed = self.entries.remove(key);
        if removed.is_some() {
            self.written.push(*key);
        }
        removed
    }

    /// Writes every entry in place.
    pub(crate) fn for_each_value_mut(&mut self, mut write: impl FnMut(&mut V)) {
        for (key, value) in &mut self.entries {
            self.written.push(*key);
            write(value);
        }
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        let written = &mut self.written;
        self.entries.retain(|key, value| {
            let kept = keep(key, value);
            if !kept {
                written.push(*key);
            }
            kept
        });
    }

    /// Takes the entry the host took from its copy already.
    pub(crate) fn take_taken_by_host(&mut self, key: &K) -> Option<V> {
        self.entries.remove(key)
    }

    /// The keys written since the home last followed the table.
    pub(crate) fn take_written(&mut self) -> Vec<K> {
        std::mem::take(&mut self.written)
    }
}
