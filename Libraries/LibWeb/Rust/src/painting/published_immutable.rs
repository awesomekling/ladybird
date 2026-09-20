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
    rows: HashMap<NodeSlotId, RowFingerprint>,
    last_mutations: HashMap<NodeSlotId, Mutation>,
    reported: HashSet<(&'static str, &'static str)>,
    // A publication can open inside another: draining the invalidation journal marks nodes for
    // repaint, and a mark made from inside a layout update drains again on the spot. Only the
    // outermost window verifies and re-fingerprints; an inner one that did so would report the
    // rows the outer window has already stopped attributing.
    publication_depth: u32,
    publication: Publication,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Publication {
    #[default]
    None,
    Layout,
    Journal,
}

#[derive(Clone, Copy)]
struct Mutation {
    call_site: &'static str,
    writer: &'static str,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RowFingerprint {
    paintable_data: u64,
    dom_paint_facts: u64,
    replaced_paint_facts: u64,
    layer_image_paint_facts: u64,
    paint_damage: u64,
}

fn fingerprint(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn row_fingerprint(arena: &LayoutNodeArena, row: NodeSlotId) -> RowFingerprint {
    RowFingerprint {
        paintable_data: fingerprint(&format!("{:?}", arena.paintable_rows().paintable_data(row))),
        dom_paint_facts: fingerprint(&arena.node_dom_paint_facts(row)),
        replaced_paint_facts: fingerprint(&format!("{:?}", arena.replaced_paint_facts(row))),
        layer_image_paint_facts: fingerprint(&arena.layer_image_paint_facts_for_verification(row)),
        paint_damage: fingerprint(&arena.paint_damage_of_row(row)),
    }
}

fn fingerprints(arena: &LayoutNodeArena) -> HashMap<NodeSlotId, RowFingerprint> {
    arena
        .published_paintable_rows()
        .into_iter()
        .map(|row| (row, row_fingerprint(arena, row)))
        .collect()
}

fn changed_rows(
    previous: &HashMap<NodeSlotId, RowFingerprint>,
    current: &HashMap<NodeSlotId, RowFingerprint>,
) -> Vec<NodeSlotId> {
    let mut changed: Vec<_> = previous
        .iter()
        .filter_map(|(row, fingerprint)| (current.get(row) != Some(fingerprint)).then_some(*row))
        .collect();
    changed.extend(current.keys().filter(|row| !previous.contains_key(row)).copied());
    changed.sort_unstable_by_key(|row| row.index);
    changed
}

fn unclassified_mutation(
    previous: Option<&RowFingerprint>,
    current: Option<&RowFingerprint>,
    fallback: &'static str,
) -> &'static str {
    let (Some(previous), Some(current)) = (previous, current) else {
        return "unclassified row membership";
    };
    if previous.paintable_data != current.paintable_data {
        return "unclassified paintable data";
    }
    if previous.dom_paint_facts != current.dom_paint_facts {
        return "unclassified DOM paint facts";
    }
    if previous.replaced_paint_facts != current.replaced_paint_facts {
        return "unclassified replaced paint facts";
    }
    if previous.layer_image_paint_facts != current.layer_image_paint_facts {
        return "unclassified layer image paint facts";
    }
    if previous.paint_damage != current.paint_damage {
        return "unclassified paint damage";
    }
    fallback
}

fn report(call_site: &'static str, writer: &'static str, changed: &[NodeSlotId]) {
    let row_ids = changed
        .iter()
        .map(|row| format!("{}:{}", row.slot_index(), row.generation()))
        .collect::<Vec<_>>()
        .join(",");
    let report = format!(
        "PUBLISHED IMMUTABLE: writer={writer}: {call_site}: {} row(s) changed without publication: {row_ids}\n",
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
        let mut by_mutation = HashMap::<(&'static str, &'static str), Vec<NodeSlotId>>::new();
        for row in changed {
            let unclassified = unclassified_mutation(published.rows.get(&row), current.get(&row), call_site);
            let mutation = published.last_mutations.get(&row).copied().unwrap_or(Mutation {
                call_site: unclassified,
                writer: "main-side direct",
            });
            by_mutation
                .entry((mutation.call_site, mutation.writer))
                .or_default()
                .push(row);
        }
        published.rows = current;
        for ((mutation, writer), rows) in by_mutation {
            if published.reported.insert((mutation, writer)) {
                report(mutation, writer, &rows);
            }
        }
    });
}

pub(crate) fn note_row_mutation(arena: &LayoutNodeArena, row: NodeSlotId, call_site: &'static str) {
    note_row_mutation_impl(arena, row, call_site, None);
}

pub(crate) fn note_row_mutation_with_writer(
    arena: &LayoutNodeArena,
    row: NodeSlotId,
    call_site: &'static str,
    writer: &'static str,
) {
    note_row_mutation_impl(arena, row, call_site, Some(writer));
}

fn note_row_mutation_impl(
    arena: &LayoutNodeArena,
    row: NodeSlotId,
    call_site: &'static str,
    explicit_writer: Option<&'static str>,
) {
    if !enabled() {
        return;
    }
    let key = arena as *const LayoutNodeArena as usize;
    let publication = STATE.with(|state| {
        state
            .borrow()
            .arenas
            .get(&key)
            .map_or(Publication::None, |published| published.publication)
    });
    if publication == Publication::Layout {
        return;
    }
    let writer = if publication == Publication::Journal {
        "journal drain"
    } else {
        explicit_writer
            .or_else(crate::painting::seal::current_pass_name)
            .unwrap_or("main-side direct")
    };
    STATE.with(|state| {
        if let Some(published) = state.borrow_mut().arenas.get_mut(&key) {
            if publication == Publication::Journal {
                if published.reported.insert((call_site, writer)) {
                    report(call_site, writer, &[row]);
                }
            } else {
                published
                    .last_mutations
                    .entry(row)
                    .or_insert(Mutation { call_site, writer });
            }
        }
    });
}

pub(crate) fn published(arena: &LayoutNodeArena) {
    if !enabled() {
        return;
    }
    let key = arena as *const LayoutNodeArena as usize;
    let closes_the_window = STATE.with(|state| match state.borrow_mut().arenas.get_mut(&key) {
        Some(published) => {
            published.publication_depth = published.publication_depth.saturating_sub(1);
            published.publication_depth == 0
        }
        // The first publication of an arena is what establishes its baseline.
        None => true,
    });
    if !closes_the_window {
        return;
    }
    let rows = fingerprints(arena);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let published = state.arenas.entry(key).or_default();
        published.rows = rows;
        published.last_mutations.clear();
        published.publication = Publication::None;
    });
}

fn begin_publication(arena: &LayoutNodeArena, call_site: &'static str, publication: Publication) {
    let key = arena as *const LayoutNodeArena as usize;
    let already_publishing = STATE.with(|state| {
        state
            .borrow()
            .arenas
            .get(&key)
            .is_some_and(|published| published.publication_depth != 0)
    });
    if !already_publishing {
        verify(arena, call_site);
    }
    STATE.with(|state| {
        if let Some(published) = state.borrow_mut().arenas.get_mut(&key) {
            published.publication_depth += 1;
            if published.publication_depth == 1 {
                published.publication = publication;
            }
        }
    });
}

pub(crate) fn before_publication(arena: &LayoutNodeArena) {
    if !enabled() {
        return;
    }
    begin_publication(arena, "next publication", Publication::Layout);
}

pub(crate) fn before_journal_publication(arena: &LayoutNodeArena) {
    if !enabled() {
        return;
    }
    begin_publication(arena, "invalidation journal publication", Publication::Journal);
}

pub(crate) fn after_journal_publication(arena: &LayoutNodeArena) {
    published(arena);
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
        let previous = HashMap::from([
            (
                row_1,
                RowFingerprint {
                    paintable_data: 10,
                    ..Default::default()
                },
            ),
            (
                row_2,
                RowFingerprint {
                    paintable_data: 20,
                    ..Default::default()
                },
            ),
        ]);
        let current = HashMap::from([
            (
                row_1,
                RowFingerprint {
                    paintable_data: 11,
                    ..Default::default()
                },
            ),
            (
                row_3,
                RowFingerprint {
                    paintable_data: 30,
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(changed_rows(&previous, &current), vec![row_1, row_2, row_3]);
    }

    #[test]
    fn changed_rows_ignores_an_identical_publication() {
        let row = NodeSlotId::new(7, 2);
        let rows = HashMap::from([(
            row,
            RowFingerprint {
                paintable_data: 42,
                ..Default::default()
            },
        )]);
        assert!(changed_rows(&rows, &rows).is_empty());
    }

    #[test]
    fn unclassified_mutation_names_the_changed_component() {
        let previous = RowFingerprint::default();
        let current = RowFingerprint {
            paint_damage: 1,
            ..Default::default()
        };
        assert_eq!(
            unclassified_mutation(Some(&previous), Some(&current), "fallback"),
            "unclassified paint damage"
        );
    }
}
