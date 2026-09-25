/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The layout tree update marks: what the DOM asks the layout tree build to rebuild. A mark is
//! written where the DOM changes and read by the next build, which retires it, so it is layout tree
//! state and the arena holds it, keyed by the style node identity the build walks by.

use crate::css::style::tree::StyleNodeID;

/// Which narrower rebuild the marks a node has collected so far still permit, as
/// `Node::LayoutTreeUpdateReuseReason` spells them. Nothing set means only a full rebuild will do.
pub(crate) mod layout_tree_update_reuse_reason {
    pub(crate) const CHILD_LIST_INSERTION: u8 = 1;
    pub(crate) const PSEUDO_ELEMENT_CHANGE: u8 = 2;
    pub(super) const ALL: u8 = CHILD_LIST_INSERTION | PSEUDO_ELEMENT_CHANGE;
}

/// The build has to rebuild what the node produces.
const NEEDS: u8 = 1 << 2;
/// A flat-tree descendant holds a mark: the chain the build climbs down to reach a node it has to
/// rebuild. Only an element, a shadow root and the document are ever on it.
const CHILD_NEEDS: u8 = 1 << 3;

/// One byte of marks per identity, in the element and the text index spaces apart. The low bits
/// are the reuse reasons, so no reason ever needs translating.
#[derive(Default)]
pub(crate) struct LayoutTreeUpdateMarks {
    elements: Vec<u8>,
    text: Vec<u8>,
}

impl LayoutTreeUpdateMarks {
    fn get(&self, node: StyleNodeID) -> u8 {
        let (column, index) = match node.element_index() {
            Some(index) => (&self.elements, index),
            None => (
                &self.text,
                node.text_index().expect("a style node is an element or a text node"),
            ),
        };
        column.get(index as usize).copied().unwrap_or(0)
    }

    fn update(&mut self, node: StyleNodeID, update: impl FnOnce(u8) -> u8) {
        let (column, index) = match node.element_index() {
            Some(index) => (&mut self.elements, index),
            None => (
                &mut self.text,
                node.text_index().expect("a style node is an element or a text node"),
            ),
        };
        let index = index as usize;
        let current = column.get(index).copied().unwrap_or(0);
        let updated = update(current);
        if updated == current {
            return;
        }
        if index >= column.len() {
            column.resize(index + 1, 0);
        }
        column[index] = updated;
    }

    /// Whether the layout tree build has to rebuild what this node produces.
    pub(crate) fn needs(&self, node: StyleNodeID) -> bool {
        self.get(node) & NEEDS != 0
    }

    /// Which narrower rebuilds the marks collected on this node still permit. See
    /// [`layout_tree_update_reuse_reason`].
    pub(crate) fn reuse_reasons(&self, node: StyleNodeID) -> u8 {
        self.get(node) & layout_tree_update_reuse_reason::ALL
    }

    /// Whether a flat-tree descendant holds a mark. A text node is never on the chain the mark
    /// climbs, so it answers no.
    pub(crate) fn child_needs(&self, node: StyleNodeID) -> bool {
        node.element_index().is_some() && self.get(node) & CHILD_NEEDS != 0
    }

    /// Fold one mark into the node's, answering whether its own bit changed. That answer is what
    /// tells the mark site it has a transition to widen from. Once a reason that forbids reuse
    /// arrives, a later one cannot narrow it back.
    pub(crate) fn merge(&mut self, node: StyleNodeID, value: bool, reuse_reason: u8) -> bool {
        let reuse_reason = reuse_reason & layout_tree_update_reuse_reason::ALL;
        let mut changed = false;
        self.update(node, |marks| {
            let reasons = marks & layout_tree_update_reuse_reason::ALL;
            let rest = marks & !(NEEDS | layout_tree_update_reuse_reason::ALL);
            if (marks & NEEDS != 0) == value {
                let merged = if reuse_reason == 0 || reasons == 0 {
                    0
                } else {
                    reasons | reuse_reason
                };
                return rest | (marks & NEEDS) | merged;
            }
            changed = true;
            rest | if value { NEEDS } else { 0 } | reuse_reason
        });
        changed
    }

    /// Record whether a flat-tree descendant holds a mark, answering what was recorded before. The
    /// mark's ancestor walk stops where the answer is already yes.
    pub(crate) fn set_child_needs(&mut self, node: StyleNodeID, value: bool) -> bool {
        if node.element_index().is_none() {
            return false;
        }
        let before = self.child_needs(node);
        self.update(node, |marks| {
            if value {
                marks | CHILD_NEEDS
            } else {
                marks & !CHILD_NEEDS
            }
        });
        before
    }

    /// Retire the marks the node holds, own and child alike: the build has just answered them, or
    /// the identity is retired and may name another node next.
    pub(crate) fn clear(&mut self, node: StyleNodeID) {
        self.update(node, |_| 0);
    }
}

#[cfg(test)]
mod tests {
    use super::layout_tree_update_reuse_reason::{CHILD_LIST_INSERTION, PSEUDO_ELEMENT_CHANGE};
    use super::*;

    #[test]
    fn a_reason_that_forbids_reuse_cannot_be_narrowed_again() {
        let mut marks = LayoutTreeUpdateMarks::default();
        let element = StyleNodeID::element(5);
        assert!(marks.merge(element, true, CHILD_LIST_INSERTION));
        assert!(!marks.merge(element, true, PSEUDO_ELEMENT_CHANGE));
        assert_eq!(
            marks.reuse_reasons(element),
            CHILD_LIST_INSERTION | PSEUDO_ELEMENT_CHANGE
        );
        assert!(!marks.merge(element, true, 0));
        assert_eq!(marks.reuse_reasons(element), 0);
        assert!(!marks.merge(element, true, CHILD_LIST_INSERTION));
        assert_eq!(marks.reuse_reasons(element), 0);
        assert!(marks.needs(element));
    }

    #[test]
    fn the_child_mark_answers_what_it_was_and_only_elements_hold_one() {
        let mut marks = LayoutTreeUpdateMarks::default();
        let element = StyleNodeID::element(3);
        let text = StyleNodeID::text(3);
        assert!(!marks.set_child_needs(element, true));
        assert!(marks.set_child_needs(element, true));
        assert!(marks.child_needs(element));
        assert!(!marks.set_child_needs(text, true));
        assert!(!marks.child_needs(text));
        assert!(marks.merge(text, true, 0));
        assert!(marks.needs(text) && !marks.needs(element));
        marks.clear(element);
        assert!(!marks.child_needs(element));
        assert!(marks.needs(text));
    }
}
