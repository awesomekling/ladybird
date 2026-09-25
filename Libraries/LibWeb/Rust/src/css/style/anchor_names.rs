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
    /// ones it registered before, and publish the names that moved to `arena`. A zero record
    /// registers nothing: the element's style was discarded, or it left the tree.
    pub(crate) fn register_anchor_names(
        &mut self,
        arena: Option<&LayoutNodeArena>,
        node: StyleNodeID,
        style_record: u64,
    ) -> AnchorNamesRegistered {
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
        let mut moved = Vec::new();
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
        if let Some(arena) = arena {
            moved.sort_unstable();
            moved.dedup();
            for key in moved {
                self.publish_anchor_name(arena, key);
            }
        }
        registered
    }

    /// Publish the elements registered under a name in a tree scope to the arena, in tree order,
    /// named by the scope's shadow host, or by nothing for the document tree.
    fn publish_anchor_name(&self, arena: &LayoutNodeArena, (tree_scope, name): (TreeScopeID, usize)) {
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
        let mut ordered: Vec<StyleNodeID> = Vec::new();
        if let Some(elements) = self.retained.anchor_names.by_name.get(&(tree_scope, name)) {
            for &element in elements {
                let index = ordered
                    .iter()
                    .position(|&existing| {
                        tree.is_live(element)
                            && tree.is_live(existing)
                            && tree.precedes_in_tree_order(element, existing)
                    })
                    .unwrap_or(ordered.len());
                ordered.insert(index, element);
            }
        }
        arena.set_anchor_name_elements(scope_host, name, &ordered);
    }
}
