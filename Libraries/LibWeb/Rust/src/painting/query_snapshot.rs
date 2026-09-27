/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a document publishes for the main thread to answer geometry reads from.
//!
//! A [`QuerySnapshot`] is the committed geometry of a document as it was when the document
//! published it: its paintable rows, the shape of its layout tree with the facts of each node's
//! style that a geometry read looks at, which box each element is bound to, and how a rect is
//! converted to viewport space. It is immutable and owns all of it, through the copy-on-write
//! generations the arena's columns publish, so it is read on the main thread while the arena goes
//! on changing, and outlives nothing it names. It keeps no style record alive: the facts it reads
//! of a style were copied out of it when the node was published.
//!
//! Reads go through [`GeometryRead`], which is all a snapshot answers. It holds no arena and does
//! not dereference to one, so a read written against it cannot reach the render side's state, and
//! the compiler says so. Whether a snapshot still describes the document is the holder's question:
//! the document drops the snapshot it holds as anything is written to its render inputs, so one it
//! still holds does.

use crate::cow_column::ColumnSnapshot;
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::css_enums::positioning;
use crate::css::css_pixels::CssPixelRect;
use crate::css::style::tree::StyleNodeID;
use crate::layout::fragment_tree::FragmentLink;
use crate::layout::node_data::{NodeFlag, NodeKind, NodeSlotId, PaintNode};
use crate::layout::node_facts::{self, QueryFacts};
use crate::layout::tree_shape::{PublishedShape, PublishedStyle};
use crate::layout::{BOUND_ELEMENT_ROWS_PER_CHUNK, LayoutNodeArena, SLOTS_PER_CHUNK};
use crate::painting::client_rects;
use crate::painting::geometry_read::GeometryRead;
use crate::painting::paintable_data::PaintableData;
use crate::painting::paintable_geometry;
use crate::painting::paintable_rows::CommittedSideDataRef;
use crate::painting::published_frame::PublishedRows;
use crate::painting::rect_to_viewport_transform::RectToViewportTransform;
use crate::painting::visual_context::VisualContextTree;
use libgfx_rust::FloatPoint;
use std::sync::Arc;

/// How a snapshot converts a box's rects to viewport space.
enum ViewportConversion {
    /// The document had no committed viewport box: rects are converted as they are.
    Identity,
    /// The accumulated visual contexts were not up to date: only a rect that no transform, sticky
    /// offset or scroll offset moves converts, as it is.
    UntransformedOnly { viewport_scroll_offset_is_zero: bool },
    /// Through the document's visual context tree and the scroll offsets its scroll state held.
    VisualContexts {
        tree: Option<Arc<VisualContextTree>>,
        device_scroll_offsets: Box<[FloatPoint]>,
        device_pixels_per_css_pixel: f32,
    },
}

/// What the main thread passes the arena for a snapshot's viewport conversion: whether the
/// document has a committed viewport box, whether its accumulated visual contexts are up to date,
/// and its scroll state as the main thread's paint state holds it.
#[repr(C)]
pub struct FfiQuerySnapshotViewport {
    pub has_committed_viewport_box: bool,
    pub visual_contexts_are_up_to_date: bool,
    pub viewport_scroll_offset_is_zero: bool,
    pub device_scroll_offsets: *const FloatPoint,
    pub device_scroll_offsets_len: usize,
    pub device_pixels_per_css_pixel: f32,
}

pub(crate) struct QuerySnapshot {
    rows: PublishedRows,
    nodes: ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK>,
    /// Every node's style, which the snapshot keeps alive for the facts it reads of it.
    styles: ColumnSnapshot<PublishedStyle, SLOTS_PER_CHUNK>,
    element_rows: ColumnSnapshot<NodeSlotId, BOUND_ELEMENT_ROWS_PER_CHUNK>,
    viewport_conversion: ViewportConversion,
}

// A snapshot is read on the main thread while the arena is written wherever its owner runs: it
// holds no cell, no borrow and no handle of the arena, and no reference to what it does not own.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<QuerySnapshot>();
};

impl LayoutNodeArena {
    /// Publishes the document's committed geometry as a snapshot, or none if its rows cannot be published.
    pub(crate) fn publish_query_snapshot(&mut self, viewport: &FfiQuerySnapshotViewport) -> Option<QuerySnapshot> {
        let viewport_conversion = if !viewport.has_committed_viewport_box {
            ViewportConversion::Identity
        } else if !viewport.visual_contexts_are_up_to_date {
            ViewportConversion::UntransformedOnly {
                viewport_scroll_offset_is_zero: viewport.viewport_scroll_offset_is_zero,
            }
        } else {
            // SAFETY: The caller passes the scroll offsets its paint state holds, alive for the call.
            let offsets = unsafe {
                libcompositing_rust::ffi::ffi_slice(viewport.device_scroll_offsets, viewport.device_scroll_offsets_len)
            };
            ViewportConversion::VisualContexts {
                tree: self.paint_state().borrow().visual_context.tree.clone(),
                device_scroll_offsets: offsets.into(),
                device_pixels_per_css_pixel: viewport.device_pixels_per_css_pixel,
            }
        };
        // The snapshot names no slot to anyone, so it does not keep freed slots from being reused.
        let PublishedShape { nodes, styles, .. } = self.publish_paint_tree();
        Some(QuerySnapshot {
            rows: self.publish_paintable_rows_for_query()?,
            nodes,
            styles,
            element_rows: self.publish_bound_element_rows(),
            viewport_conversion,
        })
    }
}

impl QuerySnapshot {
    fn node(&self, id: NodeSlotId) -> Option<&PaintNode> {
        if id.is_invalid() {
            return None;
        }
        self.nodes
            .get(id.slot_index() as usize)
            .filter(|node| node.generation != 0 && node.generation == id.generation())
    }

    /// What a geometry query reads of the style of the node in a live slot.
    fn facts(&self, id: NodeSlotId) -> Option<QueryFacts> {
        let node = self.node(id)?;
        let style = self
            .styles
            .get(id.slot_index() as usize)
            .and_then(|style| style.0.as_deref());
        Some(QueryFacts::of_style(
            node,
            style.map(|record| ComputedValuesView::new(&record.payloads.as_ffi().groups)),
        ))
    }

    /// The box the element is bound to: its layout node.
    pub(crate) fn element_box(&self, element: StyleNodeID) -> Option<NodeSlotId> {
        let row = *self.element_rows.get(element.element_index()? as usize)?;
        self.node(row).is_some().then_some(row)
    }

    /// https://drafts.csswg.org/css-tables-3/#table-wrapper-box
    /// The element's principal box: the table wrapper box of a table, which contains its caption
    /// boxes, and the box the element is bound to otherwise.
    pub(crate) fn principal_box(&self, element: StyleNodeID) -> Option<NodeSlotId> {
        let row = self.element_box(element)?;
        if self.facts(row)?.is_table_inside()
            && let Some(parent) = self.node_parent_if_live(row)
            && self.node_kind_if_live(parent) == Some(NodeKind::TableWrapper)
        {
            return Some(parent);
        }
        Some(row)
    }

    /// The computed value of the box's `position`.
    pub(crate) fn position(&self, id: NodeSlotId) -> u8 {
        self.facts(id).map_or(positioning::STATIC, |facts| facts.position())
    }

    /// Whether the box is positioned, as painting decides it.
    pub(crate) fn is_positioned(&self, id: NodeSlotId) -> bool {
        self.facts(id).is_some_and(|facts| facts.is_positioned())
    }

    /// Whether the box establishes an absolute and a fixed positioning containing block.
    pub(crate) fn establishes_positioning_containing_blocks(&self, id: NodeSlotId) -> (bool, bool) {
        let Some(node) = self.node(id) else {
            return (false, false);
        };
        if !node_facts::kind_is_box(node.kind) {
            return (false, false);
        }
        (
            node_facts::has_flag(node, NodeFlag::EstablishesAbsolutePositionContainingBlock),
            node_facts::has_flag(node, NodeFlag::EstablishesFixedPositionContainingBlock),
        )
    }

    /// https://www.w3.org/TR/css-position-3/#fixed-positioning-containing-block
    pub(crate) fn any_ancestor_establishes_a_fixed_position_containing_block(&self, id: NodeSlotId) -> bool {
        let mut ancestor = self.containing_block(id);
        while let Some(block) = ancestor {
            if self.establishes_positioning_containing_blocks(block).1 {
                return true;
            }
            ancestor = self.containing_block(block);
        }
        false
    }

    /// The box whose content box a node is laid out against, found by walking its ancestors as
    /// the arena's own walk does.
    fn containing_block(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        let node = self.node(id)?;
        let position = if node_facts::kind_is_text(node.kind) {
            positioning::STATIC
        } else {
            self.facts(id)?.position()
        };
        if position != positioning::ABSOLUTE && position != positioning::FIXED {
            let mut ancestor = self.node_parent_if_live(id);
            while let Some(candidate) = ancestor {
                if self.facts(candidate)?.forms_containing_block_for_children() {
                    return Some(candidate);
                }
                ancestor = self.node_parent_if_live(candidate);
            }
            return None;
        }
        let is_fixed_position = position == positioning::FIXED;
        let establishes_containing_block = node_facts::containing_block_establishment_flag(is_fixed_position) as u32;
        let mut current = id;
        while let Some(ancestor) = self.node_parent_if_live(current) {
            current = ancestor;
            if self.node_kind_if_live(current).is_some_and(node_facts::kind_is_box)
                && self.node_flags_if_live(current) & establishes_containing_block != 0
            {
                return Some(current);
            }
        }
        // A fixed-position box with no ancestor establishing its containing block is laid out
        // against the root.
        is_fixed_position.then_some(current)
    }

    /// The border box of the box's first fragment, relative to the initial containing block and
    /// ignoring transforms.
    pub(crate) fn absolute_border_box_rect(&self, id: NodeSlotId) -> CssPixelRect {
        if !self.paintable_row_is_populated(id) {
            return CssPixelRect::default();
        }
        paintable_geometry::absolute_border_box_rect(self, id)
    }

    /// The padding box of the box's first fragment, relative to the initial containing block and
    /// ignoring transforms.
    pub(crate) fn absolute_padding_box_rect(&self, id: NodeSlotId) -> CssPixelRect {
        if !self.paintable_row_is_populated(id) {
            return CssPixelRect::default();
        }
        paintable_geometry::absolute_padding_box_rect(self, id)
    }

    /// Visits the box's client rects in viewport space. `false` if the snapshot cannot convert them:
    /// the box may be transformed or scrolled, and the visual contexts were not up to date.
    pub(crate) fn for_each_client_rect(&self, id: NodeSlotId, push_rect: impl FnMut(CssPixelRect)) -> bool {
        let Some(transform) = self.rect_to_viewport_transform(id) else {
            return false;
        };
        client_rects::for_each_client_rect(self, id, transform.as_ref(), push_rect);
        true
    }

    /// https://drafts.csswg.org/cssom-view/#dom-element-getboundingclientrect
    pub(crate) fn bounding_client_rect(&self, id: NodeSlotId) -> Option<CssPixelRect> {
        let transform = self.rect_to_viewport_transform(id)?;
        Some(client_rects::bounding_client_rect(self, id, transform.as_ref()))
    }

    /// How the box's rects convert to viewport space: `Some(None)` as they are.
    fn rect_to_viewport_transform(&self, id: NodeSlotId) -> Option<Option<RectToViewportTransform<'_>>> {
        match &self.viewport_conversion {
            ViewportConversion::Identity => Some(None),
            ViewportConversion::UntransformedOnly {
                viewport_scroll_offset_is_zero,
            } => self
                .rects_are_untransformed(id, *viewport_scroll_offset_is_zero)
                .then_some(None),
            ViewportConversion::VisualContexts {
                tree,
                device_scroll_offsets,
                device_pixels_per_css_pixel,
            } => Some(tree.as_deref().map(|visual_context_tree| RectToViewportTransform {
                visual_context_tree,
                scroll_offsets: device_scroll_offsets,
                device_pixels_per_css_pixel: *device_pixels_per_css_pixel,
            })),
        }
    }

    /// Whether nothing moves the box's rects from where layout put them, as
    /// [`client_rects::can_compute_client_rects_without_visual_context_update`] decides it, from the
    /// facts the snapshot has of each style. A style that may transform counts as one that does.
    fn rects_are_untransformed(&self, id: NodeSlotId, viewport_scroll_offset_is_zero: bool) -> bool {
        let mut current = id;
        while let Some(node) = self.node(current) {
            let kind = node.kind;
            if node_facts::kind_is_svg_box(kind) || kind == NodeKind::SVGSVGBox || kind == NodeKind::SVGForeignObjectBox
            {
                return false;
            }
            if node_facts::has_flag(node, NodeFlag::HasStyle)
                && self
                    .facts(current)
                    .is_some_and(|facts| facts.may_transform() || facts.position() == positioning::STICKY)
            {
                return false;
            }
            let compensates_for_scroll =
                NodeFlag::CompensatesForHorizontalScroll as u32 | NodeFlag::CompensatesForVerticalScroll as u32;
            if node.flags & compensates_for_scroll != 0 {
                return false;
            }
            // A scroll container's contents move, but its own border box does not.
            if current != id {
                let scroll_offset_is_zero = if kind == NodeKind::Viewport {
                    viewport_scroll_offset_is_zero
                } else {
                    !node_facts::has_flag(node, NodeFlag::HasScrollOffset)
                };
                if !scroll_offset_is_zero && self.paintable_row_is_populated(current) {
                    return false;
                }
            }
            current = node.parent;
        }
        true
    }
}

impl GeometryRead for QuerySnapshot {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.rows.paintable_data(id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        self.rows.paintable_row_is_populated(id)
    }

    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R {
        self.rows.with_committed_fragment_link(id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Published(self.rows.committed_side_data(id))
    }

    fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind> {
        self.node(id).map(|node| node.kind)
    }

    fn node_flags_if_live(&self, id: NodeSlotId) -> u32 {
        self.node(id).map_or(0, |node| node.flags)
    }

    fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        let parent = self.node(id)?.parent;
        (!parent.is_invalid()).then_some(parent)
    }

    fn node_is_fragmented_inline(&self, id: NodeSlotId) -> bool {
        self.facts(id).is_some_and(|facts| facts.is_fragmented_inline())
    }

    fn memoized_absolute_rect(&self, _id: NodeSlotId) -> Option<CssPixelRect> {
        None
    }

    fn memoize_absolute_rect(&self, _id: NodeSlotId, _rect: CssPixelRect) {}
}

/// A box of a query snapshot. Only that snapshot's reads take it: it is not a slot of the arena, so
/// C++ can hand it to no arena entry.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiQueryBox {
    pub index: u32,
}

impl FfiQueryBox {
    fn of(id: Option<NodeSlotId>) -> Self {
        Self {
            index: id.unwrap_or(NodeSlotId::INVALID).index,
        }
    }

    fn id(self) -> NodeSlotId {
        NodeSlotId { index: self.index }
    }
}

/// What an offset read asks of a snapshot's box.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FfiQueryBoxFacts {
    /// Whether layout committed the box.
    pub has_committed_box: bool,
    /// The computed value of the box's `position`.
    pub position: u8,
    /// Whether painting counts the box as positioned: a position other than static, or a z-index on a
    /// flex or grid item.
    pub is_positioned: bool,
    pub establishes_an_absolute_positioning_containing_block: bool,
    pub establishes_a_fixed_positioning_containing_block: bool,
}

/// SAFETY: `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`.
unsafe fn snapshot_from_handle<'a>(snapshot: *const std::ffi::c_void) -> &'a QuerySnapshot {
    assert!(!snapshot.is_null(), "query snapshot handle is null");
    unsafe { &*snapshot.cast::<QuerySnapshot>() }
}

/// Hands a published snapshot to C++, which releases it with `query_snapshot_release`.
pub(crate) fn into_handle(snapshot: QuerySnapshot) -> *const std::ffi::c_void {
    Arc::into_raw(Arc::new(snapshot)).cast()
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_release(snapshot: *const std::ffi::c_void) {
    assert!(!snapshot.is_null(), "query snapshot handle is null");
    drop(unsafe { Arc::from_raw(snapshot.cast::<QuerySnapshot>()) });
}

/// The box the element with `style_node` is bound to, or none.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_element_box(snapshot: *const std::ffi::c_void, style_node: u32) -> FfiQueryBox {
    let snapshot = unsafe { snapshot_from_handle(snapshot) };
    FfiQueryBox::of(StyleNodeID::from_raw(style_node).and_then(|style_node| snapshot.element_box(style_node)))
}

/// The principal box of the element with `style_node`, or none.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_principal_box(
    snapshot: *const std::ffi::c_void,
    style_node: u32,
) -> FfiQueryBox {
    let snapshot = unsafe { snapshot_from_handle(snapshot) };
    FfiQueryBox::of(StyleNodeID::from_raw(style_node).and_then(|style_node| snapshot.principal_box(style_node)))
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, and `query_box` a
/// box it answered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_box_facts(
    snapshot: *const std::ffi::c_void,
    query_box: FfiQueryBox,
) -> FfiQueryBoxFacts {
    let snapshot = unsafe { snapshot_from_handle(snapshot) };
    let id = query_box.id();
    let (absolute, fixed) = snapshot.establishes_positioning_containing_blocks(id);
    FfiQueryBoxFacts {
        has_committed_box: snapshot.paintable_row_is_populated(id),
        position: snapshot.position(id),
        is_positioned: snapshot.paintable_row_is_populated(id) && snapshot.is_positioned(id),
        establishes_an_absolute_positioning_containing_block: absolute,
        establishes_a_fixed_positioning_containing_block: fixed,
    }
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, and `query_box` a
/// box it answered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_any_ancestor_establishes_a_fixed_position_containing_block(
    snapshot: *const std::ffi::c_void,
    query_box: FfiQueryBox,
) -> bool {
    unsafe { snapshot_from_handle(snapshot) }.any_ancestor_establishes_a_fixed_position_containing_block(query_box.id())
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, and `query_box` a
/// box it answered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_absolute_border_box_rect(
    snapshot: *const std::ffi::c_void,
    query_box: FfiQueryBox,
) -> crate::layout::used_values::FfiCssPixelRect {
    unsafe { snapshot_from_handle(snapshot) }
        .absolute_border_box_rect(query_box.id())
        .into()
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, and `query_box` a
/// box it answered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_absolute_padding_box_rect(
    snapshot: *const std::ffi::c_void,
    query_box: FfiQueryBox,
) -> crate::layout::used_values::FfiCssPixelRect {
    unsafe { snapshot_from_handle(snapshot) }
        .absolute_padding_box_rect(query_box.id())
        .into()
}

/// Pushes the box's client rects in viewport space. `false`, with nothing pushed, if the snapshot
/// cannot convert them.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, `query_box` a box it
/// answered, and `push_rect` must accept `context` for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_client_rects(
    snapshot: *const std::ffi::c_void,
    query_box: FfiQueryBox,
    context: *mut std::ffi::c_void,
    push_rect: unsafe extern "C" fn(*mut std::ffi::c_void, crate::layout::used_values::FfiCssPixelRect),
) -> bool {
    unsafe { snapshot_from_handle(snapshot) }.for_each_client_rect(query_box.id(), |rect| {
        // SAFETY: The consumer copies the plain-data rect synchronously.
        unsafe { push_rect(context, rect.into()) };
    })
}

/// Writes the box's bounding client rect to `rect`. `false`, with nothing written, if the snapshot
/// cannot convert it.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_query_snapshot`, `query_box` a box it
/// answered, and `rect` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query_snapshot_bounding_client_rect(
    snapshot: *const std::ffi::c_void,
    query_box: FfiQueryBox,
    rect: *mut crate::layout::used_values::FfiCssPixelRect,
) -> bool {
    let Some(bounding_rect) = unsafe { snapshot_from_handle(snapshot) }.bounding_client_rect(query_box.id()) else {
        return false;
    };
    unsafe { rect.write(bounding_rect.into()) };
    true
}
