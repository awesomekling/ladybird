/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! https://drafts.csswg.org/css-anchor-position-1/#determining
//!
//! The anchor names the installed records of elements register in their tree scopes. Layout finds
//! an anchor by name in the layout arena, which the engine publishes these to, so the registry is
//! engine state: a record installed on the render side registers its names there without the
//! element.

use super::*;
use crate::css::retained_fly_string::RetainedUtf16FlyString;
use crate::layout::LayoutNodeArena;

#[derive(Default)]
pub(crate) struct AnchorNameRegistry {
    /// The names each element's installed record registers, and the tree scope it registered them in.
    by_element: HashMap<StyleNodeID, (TreeScopeID, Vec<RetainedUtf16FlyString>)>,
    /// The elements registered under a name in a tree scope. An element's place in the tree moves
    /// without its names moving, so they are put in tree order as they are published.
    by_name: HashMap<(TreeScopeID, usize), Vec<StyleNodeID>>,
    /// The names registration moved since they were last published. Putting a name's elements in
    /// tree order costs a sort, so a batch of registrations publishes each name it moved once.
    unpublished: Vec<(TreeScopeID, usize)>,
}

impl AnchorNameRegistry {
    /// An identity is minted again for another element: what the retired one registered goes
    /// with it. Its removal from the tree withdrew it from the arena already.
    pub(super) fn retire(&mut self, node: StyleNodeID) {
        let Some((tree_scope, names)) = self.by_element.remove(&node) else {
            return;
        };
        for name in names {
            let key = (tree_scope, name.raw());
            if let Some(elements) = self.by_name.get_mut(&key) {
                elements.retain(|&element| element != node);
                if elements.is_empty() {
                    self.by_name.remove(&key);
                }
            }
        }
    }
}

/// What registering an element's names changed about it.
#[derive(Clone, Copy, Default)]
pub(crate) struct AnchorNamesRegistered {
    pub(crate) had_names: bool,
    pub(crate) has_names: bool,
}

impl StyleEngine {
    /// Register the anchor names of the record `style_record` installs on `node` in place of the
    /// ones it registered before. The names that moved wait for `publish_anchor_names`. A zero
    /// record registers nothing: the element's style was discarded, or it left the tree.
    pub(crate) fn register_anchor_names(&mut self, node: StyleNodeID, style_record: u64) -> AnchorNamesRegistered {
        let names: Vec<RetainedUtf16FlyString> = if style_record == 0 {
            Vec::new()
        } else {
            self.style_record_payloads(style_record)
                .map(|payloads| {
                    let anchor = unsafe {
                        payloads[crate::css::computed_value_types::STYLE_GROUP_INDEX_ANCHOR]
                            .cast::<crate::css::computed_value_types::AnchorValues>()
                            .deref()
                    };
                    anchor.anchor_names.as_slice().to_vec()
                })
                .unwrap_or_default()
        };
        let tree_scope = if names.is_empty() || !self.retained.tree.is_live(node) {
            TreeScopeID::DOCUMENT
        } else {
            self.retained.tree.tree_scope(node)
        };
        let registry = &mut self.retained.anchor_names;
        let old = registry.by_element.remove(&node);
        let registered = AnchorNamesRegistered {
            had_names: old.is_some(),
            has_names: !names.is_empty(),
        };
        if let Some((old_scope, old_names)) = &old
            && *old_scope == tree_scope
            && *old_names == names
        {
            registry.by_element.insert(node, (tree_scope, names));
            return registered;
        }
        let moved = &mut registry.unpublished;
        if let Some((old_scope, old_names)) = old {
            for name in old_names {
                let key = (old_scope, name.raw());
                if let Some(elements) = registry.by_name.get_mut(&key) {
                    elements.retain(|&element| element != node);
                    if elements.is_empty() {
                        registry.by_name.remove(&key);
                    }
                }
                moved.push(key);
            }
        }
        if !names.is_empty() {
            for name in &names {
                let key = (tree_scope, name.raw());
                let elements = registry.by_name.entry(key).or_default();
                if !elements.contains(&node) {
                    elements.push(node);
                }
                moved.push(key);
            }
            registry.by_element.insert(node, (tree_scope, names));
        }
        registered
    }

    /// Publish the names registration moved since the last publication to `arena`.
    pub(crate) fn publish_anchor_names(&mut self, arena: &LayoutNodeArena) {
        let mut moved = std::mem::take(&mut self.retained.anchor_names.unpublished);
        moved.sort_unstable();
        moved.dedup();
        // The tree holds still while the names are published, so a parent's children are numbered once for all of them.
        let mut child_positions = HashMap::default();
        for key in moved {
            self.publish_anchor_name(arena, key, &mut child_positions);
        }
    }

    /// Publish the elements registered under a name in a tree scope to the arena, in tree order,
    /// named by the scope's shadow host, or by nothing for the document tree. `child_positions` holds
    /// each element's position among its siblings, for the parents numbered so far.
    fn publish_anchor_name(
        &self,
        arena: &LayoutNodeArena,
        (tree_scope, name): (TreeScopeID, usize),
        child_positions: &mut HashMap<StyleNodeID, u32>,
    ) {
        let scope_host = if tree_scope == TreeScopeID::DOCUMENT {
            0
        } else {
            // A scope whose host is already retired has left the tree with it, and nothing can
            // reach it any more. Naming it anyway would name the document.
            let Some(host) = self
                .scope_root(tree_scope)
                .and_then(|root| self.retained.tree.host_of(root))
            else {
                return;
            };
            host.raw()
        };
        let tree = &self.retained.tree;
        let elements = self
            .retained
            .anchor_names
            .by_name
            .get(&(tree_scope, name))
            .map_or(&[][..], Vec::as_slice);
        // An element's place in tree order is its root and the position among its siblings of each
        // ancestor on the way down to it. A retired element has no place any more and goes last.
        let mut keyed: Vec<(Vec<u32>, StyleNodeID)> = Vec::with_capacity(elements.len());
        for &element in elements {
            if !tree.is_live(element) {
                keyed.push((vec![u32::MAX], element));
                continue;
            }
            let mut key = Vec::with_capacity(tree.depth(element) as usize + 1);
            let mut node = element;
            while let Some(parent) = tree.parent(node) {
                if !child_positions.contains_key(&node) {
                    let mut position = 0;
                    let mut child = tree.first_element_child(parent);
                    while let Some(sibling) = child {
                        child_positions.insert(sibling, position);
                        position += 1;
                        child = tree.next_element_sibling(sibling);
                    }
                }
                key.push(child_positions.get(&node).copied().unwrap_or(u32::MAX));
                node = parent;
            }
            key.push(node.raw());
            key.reverse();
            keyed.push((key, element));
        }
        keyed.sort_by(|(first, _), (second, _)| first.cmp(second));
        let ordered: Vec<StyleNodeID> = keyed.into_iter().map(|(_, element)| element).collect();
        arena.set_anchor_name_elements(scope_host, name, &ordered);
    }
}
