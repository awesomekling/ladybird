/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Recording the elements whose style a size query container's new box moves.

use std::collections::HashSet;

use super::{StyleEngineState, StyleNodeID};

/// What the host learned about size container queries while it computed styles: which elements
/// were asked about, which elements asked, and which containers had no box to answer with yet.
#[derive(Default)]
pub(super) struct SizeContainerQueryFacts {
    /// Elements some size query or container-relative unit resolved against. `container-type` is
    /// set far more widely than it is asked about, so a container outside this set has no
    /// dependent below it to find.
    queried_containers: HashSet<StyleNodeID>,
    /// Elements whose style some size query or container-relative unit decided.
    dependents: HashSet<StyleNodeID>,
    /// Containers a style computation asked about before they had a committed box. The layout
    /// that gives them one is where their dependents move.
    needing_evaluation_after_layout: HashSet<StyleNodeID>,
    /// Elements the dependent walks have visited, for the style invalidation counters.
    scan_visits: u64,
}

impl SizeContainerQueryFacts {
    pub(super) fn retire(&mut self, node: StyleNodeID) {
        self.queried_containers.remove(&node);
        self.dependents.remove(&node);
        self.needing_evaluation_after_layout.remove(&node);
    }
}

impl StyleEngineState {
    pub(crate) fn note_size_query_container(&mut self, node: StyleNodeID) {
        self.retained.size_container_queries.queried_containers.insert(node);
    }

    pub(crate) fn note_style_depends_on_size_container_query(&mut self, node: StyleNodeID) {
        self.retained.size_container_queries.dependents.insert(node);
    }

    pub(crate) fn note_size_container_needs_evaluation_after_layout(&mut self, node: StyleNodeID) {
        self.retained
            .size_container_queries
            .needing_evaluation_after_layout
            .insert(node);
    }

    #[must_use]
    pub fn has_size_containers_needing_evaluation_after_layout(&self) -> bool {
        !self
            .retained
            .size_container_queries
            .needing_evaluation_after_layout
            .is_empty()
    }

    /// The elements the dependent walks have visited, optionally starting the count again.
    pub(crate) fn size_query_container_scan_visits(&mut self, reset: bool) -> u64 {
        let visits = &mut self.retained.size_container_queries.scan_visits;
        if reset { std::mem::take(visits) } else { *visits }
    }

    /// A layout gave the containers that had no box when they were asked about one; their
    /// dependents are computed again against it.
    pub(crate) fn evaluate_size_containers_needing_evaluation_after_layout(&mut self) {
        let containers = std::mem::take(&mut self.retained.size_container_queries.needing_evaluation_after_layout);
        for container in containers {
            // A container that has left the document has no dependents left to move.
            if self.retained.tree.is_live(container) {
                self.size_container_content_size_changed(container);
            }
        }
    }

    /// What `container` answers to the queries below it changed: its content box moved along an
    /// axis its container type queries, or its scroll state moved. Every element whose style a
    /// size query or container-relative unit decided below it, and the container itself for its
    /// own pseudo-elements, is recorded to compute again.
    pub(crate) fn size_container_content_size_changed(&mut self, container: StyleNodeID) {
        let facts = &self.retained.size_container_queries;
        if !facts.queried_containers.contains(&container) {
            return;
        }

        let mut changed = Vec::new();
        // The container's own pseudo-elements select it as their query container too, and their
        // styles are computed again with the element's.
        if facts.dependents.contains(&container) {
            changed.push(container);
        }

        let mut visits = 0;
        let mut stack = Vec::new();
        self.push_size_query_flat_tree_children(container, &mut stack);
        while let Some(node) = stack.pop() {
            if node.is_text() {
                continue;
            }
            visits += 1;
            if self.retained.size_container_queries.dependents.contains(&node) {
                changed.push(node);
            }
            self.push_size_query_flat_tree_children(node, &mut stack);
        }
        self.retained.size_container_queries.scan_visits += visits;

        for node in changed {
            self.record_container_query_input(node);
        }
    }

    /// The children of `node` in the flat tree, pushed in reverse so the walk pops them in order:
    /// a host's are its shadow tree's, a slot's are the nodes assigned to it, and everything else's
    /// are its own children. This is the inverse of the walk that selects a query container, which
    /// is what makes it the right one for finding that container's dependents.
    fn push_size_query_flat_tree_children(&self, node: StyleNodeID, stack: &mut Vec<StyleNodeID>) {
        let tree = &self.retained.tree;
        let parent = match tree.shadow_root_of(node) {
            Some(shadow_root) => shadow_root,
            None => {
                if self.retained.facts.is_slot(node) {
                    let assigned = tree.assigned_nodes_of(node);
                    if !assigned.is_empty() {
                        stack.extend(assigned.iter().rev().copied());
                        return;
                    }
                }
                node
            }
        };
        let start = stack.len();
        let mut next = tree.first_element_child(parent);
        while let Some(child) = next {
            next = tree.next_element_sibling(child);
            stack.push(child);
        }
        stack[start..].reverse();
    }
}
