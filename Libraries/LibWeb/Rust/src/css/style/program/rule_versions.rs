/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::{
    ActivationPredicateID, CascadeLayerID, DeclarationBlockID, RuleID, RuleKind, RuleVersion, SelectorProgramID,
    StyleScopeID,
};
use crate::css::style::capacity::ShallowCapacityBytes;
use crate::css::style::fast_hash::fast_hasher;
use crate::css::style::index::StyleAtomID;
use crate::css::style::memory::{MemoryCategory, MemoryController, MemoryLease};
use crate::css::style::weak_pool::WeakPool;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

const RULE_VERSIONS_PER_PAGE: usize = 128;

/// A rule version as a page stores it, each identity it may lack as `NONE`, so that a version takes 32 bytes rather
/// than 48.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PackedRuleVersion {
    rule: RuleID,
    declared_name: u32,
    selector_program: u32,
    declaration_block: u32,
    activation_predicate: u32,
    layer: CascadeLayerID,
    scope: StyleScopeID,
    kind: RuleKind,
}

const _: () = assert!(size_of::<PackedRuleVersion>() == 32);

const NONE: u32 = u32::MAX;

impl PackedRuleVersion {
    fn pack(version: RuleVersion) -> Self {
        let pack = |id: Option<u32>| {
            assert_ne!(id, Some(NONE), "a rule version identity must not be the absent one");
            id.unwrap_or(NONE)
        };
        Self {
            rule: version.rule,
            declared_name: pack(version.declared_name.map(|atom| atom.0)),
            selector_program: pack(version.selector_program.map(|program| program.0)),
            declaration_block: pack(version.declaration_block.map(|block| block.0)),
            activation_predicate: pack(version.activation_predicate.map(|predicate| predicate.0)),
            layer: version.layer,
            scope: version.scope,
            kind: version.kind,
        }
    }

    fn unpack(self) -> RuleVersion {
        let unpack = |id: u32| (id != NONE).then_some(id);
        RuleVersion {
            rule: self.rule,
            kind: self.kind,
            declared_name: unpack(self.declared_name).map(StyleAtomID),
            selector_program: unpack(self.selector_program).map(SelectorProgramID),
            declaration_block: unpack(self.declaration_block).map(DeclarationBlockID),
            activation_predicate: unpack(self.activation_predicate).map(ActivationPredicateID),
            layer: self.layer,
            scope: self.scope,
        }
    }
}

const EMPTY_VERSION: PackedRuleVersion = PackedRuleVersion {
    rule: RuleID(0),
    declared_name: NONE,
    selector_program: NONE,
    declaration_block: NONE,
    activation_predicate: NONE,
    layer: CascadeLayerID::UNLAYERED,
    scope: StyleScopeID::NONE,
    kind: RuleKind::Style,
};

struct RuleVersionPage {
    values: [PackedRuleVersion; RULE_VERSIONS_PER_PAGE],
    shared_hash: Option<u64>,
    _memory: MemoryLease,
}

impl RuleVersionPage {
    fn new(values: [PackedRuleVersion; RULE_VERSIONS_PER_PAGE], memory_controller: &mut MemoryController) -> Self {
        let mut memory = MemoryLease::new(MemoryCategory::RuleProgram);
        memory.resize_required_to(memory_controller, size_of::<Self>() as u64);
        Self {
            values,
            shared_hash: None,
            _memory: memory,
        }
    }
}

fn rule_version_pages() -> MutexGuard<'static, WeakPool<RuleVersionPage>> {
    // Every engine interns through the same pool, from whichever thread runs it. Pages never reach
    // back into the pool when they are dropped, so the lock is only held while interning.
    static RULE_VERSION_PAGES: OnceLock<Mutex<WeakPool<RuleVersionPage>>> = OnceLock::new();
    RULE_VERSION_PAGES.get_or_init(Mutex::default).lock().unwrap()
}

/// Rule versions contain only numeric identities. Equal pages are interned through the process's
/// pool rather than copied.
#[derive(Clone, Default)]
pub(super) struct RuleVersionTable {
    pages: Vec<Arc<RuleVersionPage>>,
    len: usize,
    needs_sharing: bool,
}

impl RuleVersionTable {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn push(&mut self, value: RuleVersion) {
        if self.len.is_multiple_of(RULE_VERSIONS_PER_PAGE) {
            self.pages.push(Arc::new(RuleVersionPage::new(
                [EMPTY_VERSION; RULE_VERSIONS_PER_PAGE],
                &mut rule_version_pages().memory,
            )));
        }
        let index = self.len;
        self.len = self.len.checked_add(1).expect("rule version space exhausted");
        self.needs_sharing = true;
        self.set(index, value);
    }

    pub(super) fn get(&self, index: usize) -> RuleVersion {
        assert!(index < self.len);
        self.pages[index / RULE_VERSIONS_PER_PAGE].values[index % RULE_VERSIONS_PER_PAGE].unpack()
    }

    pub(super) fn set(&mut self, index: usize, value: RuleVersion) {
        assert!(index < self.len);
        let value = PackedRuleVersion::pack(value);
        if self.pages[index / RULE_VERSIONS_PER_PAGE].values[index % RULE_VERSIONS_PER_PAGE] == value {
            return;
        }
        let page = &mut self.pages[index / RULE_VERSIONS_PER_PAGE];
        if Arc::get_mut(page).is_none() {
            rule_version_pages().make_private(page.shared_hash, page, |page, memory| {
                RuleVersionPage::new(page.values, memory)
            });
        }
        let page = Arc::get_mut(page).expect("a private page has one owner");
        page.shared_hash = None;
        page.values[index % RULE_VERSIONS_PER_PAGE] = value;
        self.needs_sharing = true;
    }

    pub(super) fn share(&mut self) {
        if !self.needs_sharing {
            return;
        }
        let mut pool = rule_version_pages();
        for page in &mut self.pages {
            if page.shared_hash.is_some() {
                continue;
            }
            let mut hasher = fast_hasher();
            page.values.hash(&mut hasher);
            let hash = hasher.finish();
            if let Some(found) = pool.find(hash, |candidate| candidate.values == page.values) {
                *page = found;
                continue;
            }
            // A clone of the table, as a fork of the style engine, holds the unpublished page too, and publishes a
            // copy of its own.
            if Arc::get_mut(page).is_none() {
                *page = Arc::new(RuleVersionPage::new(page.values, &mut pool.memory));
            }
            Arc::get_mut(page).expect("a private page has one owner").shared_hash = Some(hash);
            pool.insert(hash, page);
        }
        self.needs_sharing = false;
    }
}

impl ShallowCapacityBytes for RuleVersionTable {
    fn shallow_capacity_bytes(&self) -> u64 {
        // Page allocations are charged to the pool's ledger, including private pages.
        self.pages.shallow_capacity_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_pages_share_and_edits_detach_only_the_changed_page() {
        let make_table = || {
            let mut table = RuleVersionTable::default();
            for index in 0..RULE_VERSIONS_PER_PAGE * 2 + 1 {
                table.push(RuleVersion::new(RuleID(index as u32), RuleKind::Style));
            }
            table.share();
            table
        };
        let first = make_table();
        let mut second = make_table();
        for (left, right) in first.pages.iter().zip(&second.pages) {
            assert!(Arc::ptr_eq(left, right));
        }
        let index = RULE_VERSIONS_PER_PAGE + 7;
        let mut changed = second.get(index);
        changed.kind = RuleKind::Media;
        second.set(index, changed);
        assert_eq!(first.get(index).kind, RuleKind::Style);
        assert_eq!(second.get(index).kind, RuleKind::Media);
        assert!(Arc::ptr_eq(&first.pages[0], &second.pages[0]));
        assert!(!Arc::ptr_eq(&first.pages[1], &second.pages[1]));
        second.share();
        second.push(RuleVersion::new(RuleID(second.len() as u32), RuleKind::Style));
        assert_eq!(first.len(), RULE_VERSIONS_PER_PAGE * 2 + 1);
        assert_eq!(second.len(), first.len() + 1);
        drop(second);
        let third = make_table();
        for (left, right) in first.pages.iter().zip(&third.pages) {
            assert!(Arc::ptr_eq(left, right));
        }
    }

    #[test]
    fn a_clone_shares_the_unpublished_pages_it_holds_with_the_original() {
        let mut table = RuleVersionTable::default();
        for index in 0..RULE_VERSIONS_PER_PAGE + 1 {
            table.push(RuleVersion::new(RuleID(0x5ee7_0000 + index as u32), RuleKind::Style));
        }
        let mut clone = table.clone();
        clone.share();
        table.share();
        for (left, right) in table.pages.iter().zip(&clone.pages) {
            assert!(Arc::ptr_eq(left, right));
        }
    }
}
