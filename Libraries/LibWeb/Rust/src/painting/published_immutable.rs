/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Census of paintable-row mutations after publication.
//!
//! `LIBWEB_VERIFY_PUBLISHED_IMMUTABLE` enables the verifier. It fingerprints the
//! paintable rows at each layout commit, records named mutation funnels, and
//! checks the rows before the next publication. Unset, the mutation hooks return
//! before inspecting the arena. Reports are log-only and each call site is
//! reported at most once per arena. `LIBWEB_VERIFY_PUBLISHED_IMMUTABLE_LOG`
//! names the log file; standard error is the fallback.

use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

#[derive(Default)]
struct PublishedArena {
    rows: HashMap<NodeSlotId, u64>,
    last_mutations: HashMap<NodeSlotId, &'static str>,
    reported: HashSet<&'static str>,
    in_publication: bool,
}

#[derive(Default)]
struct State {
    arenas: HashMap<usize, PublishedArena>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("LIBWEB_VERIFY_PUBLISHED_IMMUTABLE").as_deref(),
            Err(_) | Ok("0")
        )
    })
}

fn row_fingerprint(arena: &LayoutNodeArena, row: NodeSlotId) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", arena.paintable_rows().paintable_data(row)).hash(&mut hasher);
    arena.node_dom_paint_facts(row).hash(&mut hasher);
    format!("{:?}", arena.replaced_paint_facts(row)).hash(&mut hasher);
    arena.layer_image_paint_facts_for_verification(row).hash(&mut hasher);
    arena.paint_damage_of_row(row).hash(&mut hasher);
    hasher.finish()
}

fn fingerprints(arena: &LayoutNodeArena) -> HashMap<NodeSlotId, u64> {
    arena
        .published_paintable_rows()
        .into_iter()
        .map(|row| (row, row_fingerprint(arena, row)))
        .collect()
}

fn changed_rows(previous: &HashMap<NodeSlotId, u64>, current: &HashMap<NodeSlotId, u64>) -> Vec<NodeSlotId> {
    let mut changed: Vec<_> = previous
        .iter()
        .filter_map(|(row, fingerprint)| (current.get(row) != Some(fingerprint)).then_some(*row))
        .collect();
    changed.extend(current.keys().filter(|row| !previous.contains_key(row)).copied());
    changed.sort_unstable_by_key(|row| row.index);
    changed
}

fn report(call_site: &'static str, changed: &[NodeSlotId]) {
    let row_ids = changed
        .iter()
        .map(|row| format!("{}:{}", row.slot_index(), row.generation()))
        .collect::<Vec<_>>()
        .join(",");
    let report = format!(
        "PUBLISHED IMMUTABLE: {call_site}: {} row(s) changed without publication: {row_ids}\n",
        changed.len()
    );
    match std::env::var_os("LIBWEB_VERIFY_PUBLISHED_IMMUTABLE_LOG") {
        Some(path) => {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(report.as_bytes());
            }
        }
        None => eprint!("{report}"),
    }
}

fn verify(arena: &LayoutNodeArena, call_site: &'static str) {
    let key = arena as *const LayoutNodeArena as usize;
    let current = fingerprints(arena);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(published) = state.arenas.get_mut(&key) else {
            return;
        };
        let changed = changed_rows(&published.rows, &current);
        if changed.is_empty() {
            return;
        }
        let mut by_mutation = HashMap::<&'static str, Vec<NodeSlotId>>::new();
        for row in changed {
            by_mutation
                .entry(published.last_mutations.get(&row).copied().unwrap_or(call_site))
                .or_default()
                .push(row);
        }
        published.rows = current;
        for (mutation, rows) in by_mutation {
            if published.reported.insert(mutation) {
                report(mutation, &rows);
            }
        }
    });
}

pub(crate) fn note_row_mutation(arena: &LayoutNodeArena, row: NodeSlotId, call_site: &'static str) {
    if !enabled() {
        return;
    }
    let key = arena as *const LayoutNodeArena as usize;
    let in_publication = STATE.with(|state| {
        state
            .borrow()
            .arenas
            .get(&key)
            .is_some_and(|published| published.in_publication)
    });
    if in_publication {
        return;
    }
    STATE.with(|state| {
        if let Some(published) = state.borrow_mut().arenas.get_mut(&key) {
            published.last_mutations.entry(row).or_insert(call_site);
        }
    });
}

pub(crate) fn published(arena: &LayoutNodeArena) {
    if !enabled() {
        return;
    }
    let key = arena as *const LayoutNodeArena as usize;
    let rows = fingerprints(arena);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let published = state.arenas.entry(key).or_default();
        published.rows = rows;
        published.last_mutations.clear();
        published.in_publication = false;
    });
}

pub(crate) fn before_publication(arena: &LayoutNodeArena) {
    if !enabled() {
        return;
    }
    verify(arena, "next publication");
    let key = arena as *const LayoutNodeArena as usize;
    STATE.with(|state| {
        if let Some(published) = state.borrow_mut().arenas.get_mut(&key) {
            published.in_publication = true;
        }
    });
}

pub(crate) fn finish(arena: &LayoutNodeArena) {
    if !enabled() {
        return;
    }
    verify(arena, "arena destruction");
    let key = arena as *const LayoutNodeArena as usize;
    STATE.with(|state| {
        state.borrow_mut().arenas.remove(&key);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_rows_reports_changed_added_and_removed_rows() {
        let row_1 = NodeSlotId::new(1, 1);
        let row_2 = NodeSlotId::new(2, 1);
        let row_3 = NodeSlotId::new(3, 1);
        let previous = HashMap::from([(row_1, 10), (row_2, 20)]);
        let current = HashMap::from([(row_1, 11), (row_3, 30)]);
        assert_eq!(changed_rows(&previous, &current), vec![row_1, row_2, row_3]);
    }

    #[test]
    fn changed_rows_ignores_an_identical_publication() {
        let row = NodeSlotId::new(7, 2);
        let rows = HashMap::from([(row, 42)]);
        assert!(changed_rows(&rows, &rows).is_empty());
    }
}
