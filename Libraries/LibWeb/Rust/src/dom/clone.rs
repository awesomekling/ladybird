/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Cloning a node and its descendants.
//! https://dom.spec.whatwg.org/#concept-node-clone
//!
//! Rust runs "clone a node": it walks the subtree, copies the attributes of each element, and appends each copy to the
//! copy of its parent. C++ makes each copy, which is "clone a single node" but for the attributes, runs the cloning
//! steps of other specifications, and attaches a shadow root to the copy of a shadow host, through the clone hooks,
//! which keep the document, the fallback registry and a thrown exception in a DOM::CloneScope.
//!
//! The spec clones the descendants by recursing into each child. The walk here clones them in the same order, and runs
//! the same steps for each, without recursing, so a deep tree takes no stack: it keeps each node whose children it clones
//! and its copy in a list that C++ roots, see rooted_nodes.rs, and runs step 6 for a node once it has cloned all of its
//! children, as the recursion does when the call for it returns.

use super::attributes::copy_attributes;
use super::hooks;
use super::hooks::CloneScope;
use super::node::NodeCell;
use super::node_core::NodeType;
use super::node_ref::NodeRef;
use super::node_ref::NullableNodeRef;
use super::node_ref::RawNullableNodeRef;
use super::pre_insert::PreInsertValidity;
use super::pre_insert::pre_insert;
use super::rooted_nodes::RootedNodeListCore;
use super::rooted_nodes::RootedNodes;
use crate::stage::MainThread;

crate::stage::main_thread_ffi_entries!(without_host);

/// Why cloning a node threw.
enum CloneError {
    /// A step that C++ runs threw, and the scope of the clone holds the exception.
    Threw,
    /// Appending a copy to its parent threw this exception.
    Append(PreInsertValidity),
}

/// "Clone a node" given `node`, the document of `scope`, `subtree`, `parent`, and the fallback registry of `scope` where
/// `uses_fallback_registry` says so, or null. `copies` holds the nodes whose children the walk clones and their copies,
/// above the nodes it holds as the call starts, which it holds again as it returns.
/// https://dom.spec.whatwg.org/#concept-node-clone
fn clone_a_node(
    main_thread: &mut MainThread<'_>,
    scope: &mut CloneScope,
    node: NodeRef,
    subtree: bool,
    parent: Option<NodeRef>,
    uses_fallback_registry: bool,
    copies: &mut RootedNodes<'_>,
) -> Result<NodeRef, CloneError> {
    // To clone a node given a node node and an optional document document (default node's node document), boolean
    // subtree (default false), node-or-null parent (default null), and null or a CustomElementRegistry object
    // fallbackRegistry (default null):

    // 1. Assert: node is not a document or node is document.
    // NB: C++ asserts this as it picks the document.

    // 2. Let copy be the result of cloning a single node given node, document, and fallbackRegistry.
    // 3. Run any cloning steps defined for node in other applicable specifications and pass node, copy, and subtree as
    //    parameters.
    // 4. If parent is non-null, then append copy to parent.
    let copy = clone_and_append(main_thread, scope, node, subtree, parent, uses_fallback_registry)?;

    // 5. If subtree is true, then for each child of node's children, in tree order: clone a node given child with
    //    document set to document, subtree set to subtree, parent set to copy, and fallbackRegistry set to
    //    fallbackRegistry.
    if subtree {
        clone_descendants(main_thread, scope, node, copy, uses_fallback_registry, copies)?;
    }

    // 6. If node is an element, node is a shadow host, and node's shadow root's clonable is true:
    clone_shadow_root(main_thread, scope, node, copy, copies)?;

    // 7. Return copy.
    Ok(copy)
}

/// Steps 2 to 4 of cloning `node`, see clone_a_node(). Answers the copy.
#[inline(always)]
fn clone_and_append(
    main_thread: &mut MainThread<'_>,
    scope: &mut CloneScope,
    node: NodeRef,
    subtree: bool,
    parent: Option<NodeRef>,
    uses_fallback_registry: bool,
) -> Result<NodeRef, CloneError> {
    // 2. Let copy be the result of cloning a single node given node, document, and fallbackRegistry.
    // NB: The callback runs step 3 too for a node that is no element, whose copy has no attributes to copy first.
    let copy =
        hooks::clone_single_node(main_thread, scope, node, uses_fallback_registry, subtree).ok_or(CloneError::Threw)?;
    if node.is_element() {
        // https://dom.spec.whatwg.org/#clone-a-single-node
        // 2.5. For each attribute of node's attribute list:
        //      1. Let copyAttribute be the result of cloning a single node given attribute, document, and null.
        //      2. Append copyAttribute to copy.
        copy_attributes(main_thread, node, copy);

        // 3. Run any cloning steps defined for node in other applicable specifications and pass node, copy, and
        //    subtree as parameters.
        if !hooks::run_element_cloning_steps(main_thread, scope, node, copy, subtree) {
            return Err(CloneError::Threw);
        }
    }

    // 4. If parent is non-null, then append copy to parent.
    if let Some(parent) = parent {
        // https://dom.spec.whatwg.org/#concept-node-append
        // To append a node to a parent, pre-insert node into parent before null.
        // NB: Inserting a DocumentFragment inserts its children, which a copy has none of yet.
        let nodes: &[NodeRef] = match copy.node_type() {
            NodeType::DOCUMENT_FRAGMENT_NODE => &[],
            _ => &[copy],
        };
        match pre_insert(main_thread, copy, parent, None, nodes) {
            PreInsertValidity::Valid => {}
            validity => return Err(CloneError::Append(validity)),
        }
    }
    Ok(copy)
}

/// Step 5 of cloning `node`, whose copy is `copy`: clones each descendant of `node`, in tree order, as the recursion of
/// step 5 does, with subtree set to true.
fn clone_descendants(
    main_thread: &mut MainThread<'_>,
    scope: &mut CloneScope,
    node: NodeRef,
    copy: NodeRef,
    uses_fallback_registry: bool,
    copies: &mut RootedNodes<'_>,
) -> Result<(), CloneError> {
    // NB: Above `base`, the list holds `node` and each descendant whose children are being cloned, each followed by its
    //     copy, so the last one is the copy of the parent of the next child to clone. The walk goes back up to the
    //     nodes it holds there, not to their parents, so it goes on as the recursion does where a step changes the
    //     tree: with the next sibling that each node it cloned has after its step 5.
    let base = copies.len();
    copies.push(node);
    copies.push(copy);
    let mut child = node.first_child();
    loop {
        if let Some(current) = child {
            let current_copy = clone_and_append(
                main_thread,
                scope,
                current,
                true,
                Some(copies.last()),
                uses_fallback_registry,
            )?;
            // NB: Step 5 for current clones its children before step 6 for it runs.
            if let Some(first_child) = current.first_child() {
                copies.push(current);
                copies.push(current_copy);
                child = Some(first_child);
                continue;
            }
            clone_shadow_root(main_thread, scope, current, current_copy, copies)?;
            child = current.next_sibling();
            continue;
        }

        // NB: Every child of parent is cloned, which ends step 5 for it.
        let length = copies.len();
        let (parent, parent_copy) = (copies.get(length - 2), copies.get(length - 1));
        if length - 2 == base {
            copies.truncate(base);
            return Ok(());
        }
        clone_shadow_root(main_thread, scope, parent, parent_copy, copies)?;
        copies.truncate(length - 2);
        child = parent.next_sibling();
    }
}

/// Step 6 of cloning `node`, whose copy is `copy`.
#[inline(always)]
fn clone_shadow_root(
    main_thread: &mut MainThread<'_>,
    scope: &mut CloneScope,
    node: NodeRef,
    copy: NodeRef,
    copies: &mut RootedNodes<'_>,
) -> Result<(), CloneError> {
    // 6. If node is an element, node is a shadow host, and node's shadow root's clonable is true:
    if node.shadow_root().is_none() {
        return Ok(());
    }
    clone_clonable_shadow_root(main_thread, scope, node, copy, copies)
}

#[inline(never)]
fn clone_clonable_shadow_root(
    main_thread: &mut MainThread<'_>,
    scope: &mut CloneScope,
    node: NodeRef,
    copy: NodeRef,
    copies: &mut RootedNodes<'_>,
) -> Result<(), CloneError> {
    // 1. Assert: copy is not a shadow host.
    // 2. Let shadowRootRegistry be node's shadow root's custom element registry.
    // 3. If shadowRootRegistry is a global custom element registry, then set shadowRootRegistry to document's effective
    //    global custom element registry.
    // 4. Attach a shadow root with copy, node's shadow root's mode, true, node's shadow root's serializable, node's
    //    shadow root's delegates focus, node's shadow root's slot assignment, and shadowRootRegistry.
    // 5. Set copy's shadow root's declarative to node's shadow root's declarative.
    // 6. Set copy's shadow root's keep custom element registry null to node's shadow root's keep custom element
    //    registry null.
    let Some(shadow_root_copy) =
        hooks::attach_shadow_root_copy(main_thread, scope, node, copy).map_err(|()| CloneError::Threw)?
    else {
        return Ok(());
    };

    // 7. For each child of node's shadow root's children, in tree order: clone a node given child with document set to
    //    document, subtree set to true, and parent set to copy's shadow root.
    // NB: These leave fallbackRegistry at its default, null.
    let shadow_root = node.shadow_root().expect("the node is a shadow host");
    clone_descendants(main_thread, scope, shadow_root, shadow_root_copy, false, copies)
}

/// "Clone a node" given `node` with the document and the fallback registry of `scope`, `subtree` and `parent`, for
/// DOM::Node. `copies` is the core of an empty DOM::RootedNodeList that the caller makes for the call. Sets `copy` to
/// the copy and answers PreInsertValidity::Valid where nothing throws. Answers which DOMException to throw where
/// appending a copy throws, and leaves `copy` null where a step that C++ runs throws, whose exception `scope` then
/// holds.
/// https://dom.spec.whatwg.org/#concept-node-clone
///
/// # Safety
///
/// The nodes must be live, `scope` a DOM::CloneScope and `copies` the core of a DOM::RootedNodeList that live for the
/// call, and the call must be made on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_clone_a_node(
    node: &NodeCell,
    subtree: bool,
    parent: RawNullableNodeRef,
    scope: &mut CloneScope,
    copies: &RootedNodeListCore,
    copy: &mut NullableNodeRef,
) -> PreInsertValidity {
    // SAFETY: Guaranteed by the caller.
    let mut main_thread = unsafe { main_thread() };
    // SAFETY: The node is live.
    let node = unsafe { NodeRef::from_cell(node, &main_thread) };
    // SAFETY: The list lives for the call, and only this code uses it.
    let mut copies = unsafe { RootedNodes::from_core(copies) };
    match clone_a_node(&mut main_thread, scope, node, subtree, parent.into(), true, &mut copies) {
        Ok(node_copy) => {
            *copy = Some(node_copy).into();
            PreInsertValidity::Valid
        }
        Err(CloneError::Threw) => PreInsertValidity::Valid,
        Err(CloneError::Append(validity)) => validity,
    }
}
