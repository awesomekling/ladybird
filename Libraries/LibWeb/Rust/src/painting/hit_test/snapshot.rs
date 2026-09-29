/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread hit tests: a document's hit-test list, with the rows it is hit tested over.
//!
//! A [`HitTestSnapshot`] reads the [`RowSnapshot`] the document published last. Its paintable rows hold the list of the
//! last recording the document took in, with the structures a query derives from it built as it was recorded, and the
//! visual context tree its items convert points through, and every read a hit test makes of the rows, their layout
//! nodes and their styles is one the rows answer (see [`super::read`]). The rows are immutable and own all of it, so
//! the main thread hit tests them without reaching the arena or its owner, and a hit names no layout node: it names the
//! DOM node it is on as the host names one, and the rows it went through.
//!
//! A list outlives the rows it was recorded over: a scroll, a clip or a transform moves what a point hits through the
//! visual context tree and the committed rows, not through the list. So each hit test reads the latest rows, which
//! carry the latest list.

use crate::css::css_pixels::CssPixelPoint;
use crate::css::style::tree::StyleNodeID;
use crate::layout::FfiCssPixelPoint;
use crate::layout::node_data::{NodeFlag, NodeKind, NodeSlotId};
use crate::layout::row_reads::RowSnapshot;
use crate::painting::display_list::commands::ContextRef;
use crate::painting::geometry_read::GeometryRead;
use crate::painting::hit_test::HitTestList;
use crate::painting::published_frame::{PaintRead, PaintSource};
use crate::painting::visual_context::VisualContextTree;
use std::cell::RefCell;
use std::ffi::c_void;

/// A document's published rows, as a hit test reads them. Rows that hold no list hit nothing.
#[derive(Clone, Copy)]
pub(crate) struct HitTestSnapshot<'a> {
    /// The rows, which name the row each DOM node was bound to, which a hit's box finds the box of its element by.
    rows: &'a RowSnapshot,
}

impl<'a> HitTestSnapshot<'a> {
    pub(super) fn list(self) -> Option<&'a HitTestList> {
        self.rows.paintable.hit_test_list.as_deref()
    }

    /// Runs a query over the list, the visual context tree it converts points through and the rows it
    /// was recorded over, or answers `default` where the snapshot holds no list to query.
    pub(super) fn query<R>(
        &self,
        default: R,
        query: impl FnOnce(&HitTestList, &VisualContextTree, &PaintSource<'_>) -> R,
    ) -> R {
        let (Some(list), Some(tree)) = (self.list(), self.rows.paintable.visual_context_tree.as_deref()) else {
            return default;
        };
        if !list.spatial_indexes_built {
            return default;
        }
        let absolute_rects = RefCell::default();
        query(list, tree, &PaintSource::of_rows(self.rows, &absolute_rects))
    }

    /// Reads the rows the snapshot was published with, whether or not it holds a list.
    fn read_rows<R>(&self, read: impl FnOnce(&PaintSource<'_>) -> R) -> R {
        let absolute_rects = RefCell::default();
        read(&PaintSource::of_rows(self.rows, &absolute_rects))
    }

    /// Runs a query over the list and the rows it was recorded over.
    pub(super) fn read<R>(&self, default: R, read: impl FnOnce(&HitTestList, &PaintSource<'_>) -> R) -> R {
        let Some(list) = self.list() else {
            return default;
        };
        let absolute_rects = RefCell::default();
        read(list, &PaintSource::of_rows(self.rows, &absolute_rects))
    }
}

/// An item of a snapshot's list, as the host reads it: rows, which the host finds what they stand for.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiHitTestSnapshotItem {
    pub can_produce_caret_position: bool,
    pub paintable: NodeSlotId,
    pub hit_node: NodeSlotId,
    pub chrome_widget_kind: u8,
    pub caret_node: NodeSlotId,
    pub caret_rect: crate::layout::used_values::FfiCssPixelRect,
    pub context: ContextRef,
}

/// A DOM node, as the host names one (`DOM::NodeIdentity`): by its style node, or as the document.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FfiHitNodeIdentity {
    pub kind: FfiHitNodeIdentityKind,
    pub style_node: u32,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FfiHitNodeIdentityKind {
    #[default]
    None,
    StyleNode,
    Document,
}

impl FfiHitNodeIdentity {
    const DOCUMENT: Self = Self {
        kind: FfiHitNodeIdentityKind::Document,
        style_node: 0,
    };

    fn of_style_node(style_node: Option<StyleNodeID>) -> Self {
        style_node.map_or_else(Self::default, |style_node| Self {
            kind: FfiHitNodeIdentityKind::StyleNode,
            style_node: style_node.raw(),
        })
    }

    pub(super) fn is_none(self) -> bool {
        self.kind == FfiHitNodeIdentityKind::None
    }
}

/// What a hit on a snapshot's item resolves to: the DOM node an event there is dispatched to, the
/// box whose style admitted the hit, and where in its node the hit landed.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiHitTestSnapshotHit {
    pub node: FfiHitNodeIdentity,
    pub hit_box: FfiHitBox,
    /// The row of that box, which only scrolling still takes a layout node of.
    pub hit_node: NodeSlotId,
    pub has_index_in_node: bool,
    pub index_in_node: usize,
    pub is_text_fragment: bool,
}

impl Default for FfiHitTestSnapshotHit {
    fn default() -> Self {
        Self {
            node: FfiHitNodeIdentity::default(),
            hit_box: FfiHitBox::NONE,
            hit_node: NodeSlotId::INVALID,
            has_index_in_node: false,
            index_in_node: 0,
            is_text_fragment: false,
        }
    }
}

/// The DOM node a row an event is dispatched to stands for, as `Layout::Node::dom_node_identity()`
/// names it; for a row generated for a pseudo-element, the element it was generated for where
/// `allow_pseudo_fallback`.
pub(super) fn dispatch_identity(
    rows: &impl PaintRead,
    row: Option<NodeSlotId>,
    allow_pseudo_fallback: bool,
) -> FfiHitNodeIdentity {
    let Some(row) = row.filter(|row| rows.slot_is_live(*row)) else {
        return FfiHitNodeIdentity::default();
    };
    if rows.node_flags_if_live(row) & NodeFlag::Anonymous as u32 == 0 {
        // The document's row is the viewport.
        if rows.node_kind_if_live(row) == Some(NodeKind::Viewport) {
            return FfiHitNodeIdentity::DOCUMENT;
        }
        return FfiHitNodeIdentity::of_style_node(rows.node_style_node(row));
    }
    if allow_pseudo_fallback && rows.node_is_generated_for_pseudo_element(row) {
        return FfiHitNodeIdentity::of_style_node(rows.node_style_node(row));
    }
    FfiHitNodeIdentity::default()
}

impl HitTestSnapshot<'_> {
    /// https://html.spec.whatwg.org/multipage/image-maps.html#image-map-processing-model
    /// The `<area>` of the map an image is associated with that the point hits, which the image published
    /// onto its row.
    fn image_map_area_for_point(
        &self,
        rows: &impl PaintRead,
        image: NodeSlotId,
        local_point: CssPixelPoint,
    ) -> FfiHitNodeIdentity {
        // For historical reasons, the coordinates must be interpreted relative to the displayed image after any
        // stretching caused by the CSS 'width' and 'height' properties.
        let image_rect = crate::painting::paintable_geometry::absolute_rect_or_default(rows, image);
        let x = (local_point.x - image_rect.x).to_float();
        let y = (local_point.y - image_rect.y).to_float();
        let area = self.rows.paintable.image_map_areas.area_for_point(
            image,
            x,
            y,
            image_rect.width.to_double() as f32,
            image_rect.height.to_double() as f32,
        );
        FfiHitNodeIdentity::of_style_node(StyleNodeID::from_raw(area))
    }

    fn resolve_hit(&self, index: usize, local_point: CssPixelPoint) -> FfiHitTestSnapshotHit {
        self.read(FfiHitTestSnapshotHit::default(), |list, rows| {
            let Some(item) = list.items.get(index) else {
                debug_assert!(false, "an item the snapshot does not hold");
                return FfiHitTestSnapshotHit::default();
            };
            let mut hit_node = item.hit_node;
            // https://drafts.csswg.org/cssom-view/#dom-document-elementfrompoint
            // 2. If there is a box in the viewport that would be a target for hit testing at coordinates x,y, when
            //    applying the transforms that apply to the descendants of the viewport, return the associated element
            //    and terminate these steps.
            // 3. If the document has a root element, return the root element and terminate these steps.
            // AD-HOC: Our viewport refers to the document instead of the root element. The steps above imply that we
            //         should not hit test the viewport as a box, and report the root element as hit when we otherwise
            //         miss, so we correct those hits here. This is where both pointer event hit testing and
            //         elementFromPoint() converge.
            let mut node = FfiHitNodeIdentity::default();
            if rows.node_kind_if_live(item.paintable) == Some(NodeKind::Viewport) {
                let root = self.rows.paint_status.root_background_source.root_layout_node;
                if rows.paintable_row_is_populated(root) {
                    node = dispatch_identity(rows, Some(root), false);
                    hit_node = root;
                }
            }
            let resolved = list.resolve_hit(rows, index, local_point);
            if node.is_none() && rows.paintable_row_is_populated(item.paintable) {
                node = self.image_map_area_for_point(rows, item.paintable, local_point);
            }
            if node.is_none() {
                node = dispatch_identity(rows, resolved.dispatch, resolved.allow_pseudo_fallback);
            }
            if node.is_none() {
                node = dispatch_identity(rows, resolved.fallback_dispatch, false);
            }
            FfiHitTestSnapshotHit {
                node,
                hit_box: FfiHitBox::of(rows.slot_is_live(hit_node).then_some(hit_node)),
                hit_node,
                has_index_in_node: resolved.has_index_in_node,
                index_in_node: resolved.index_in_node,
                is_text_fragment: resolved.is_text_fragment,
            }
        })
    }
}

/// A box of a hit-test snapshot, which a hit went through. Only that snapshot's reads take it: it is not
/// a slot of the arena, so C++ can hand it to no arena entry.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FfiHitBox {
    pub index: u32,
}

impl FfiHitBox {
    const NONE: Self = Self {
        index: NodeSlotId::INVALID.index,
    };

    fn of(row: Option<NodeSlotId>) -> Self {
        row.map_or(Self::NONE, |row| Self { index: row.index })
    }

    fn row(self) -> NodeSlotId {
        NodeSlotId { index: self.index }
    }
}

/// What an event dispatched through a hit reads of a box of the snapshot, as it was published.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiHitBoxFacts {
    /// Whether layout committed the box. Nothing below is read of a box it did not commit.
    pub has_committed_box: bool,
    pub kind: NodeKind,
    /// The pseudo-element the box was generated for, encoded as `Layout::Node::encode_generated_for()`, or 0.
    pub generated_for: u8,
    /// The DOM node the box stands for, as `Layout::Node::dom_node_identity()` names it: none for an anonymous box.
    pub identity: FfiHitNodeIdentity,
    /// The element a box generated for a pseudo-element was generated for; none for any other box.
    pub generator: FfiHitNodeIdentity,
    pub parent: FfiHitBox,
    /// The navigable a navigable container's viewport box shows the content of, where it is hosted here.
    pub local_content_navigable: crate::painting::host::FfiCrossProcessId,
    pub accumulated_visual_context: ContextRef,
    pub absolute_rect: crate::layout::used_values::FfiCssPixelRect,
    /// The box's position whatever kind of box it is: its first piece's for an inline box.
    pub box_type_agnostic_position: FfiCssPixelPoint,
}

impl Default for FfiHitBoxFacts {
    fn default() -> Self {
        Self {
            has_committed_box: false,
            kind: NodeKind::Unset,
            generated_for: 0,
            identity: FfiHitNodeIdentity::default(),
            generator: FfiHitNodeIdentity::default(),
            parent: FfiHitBox::NONE,
            local_content_navigable: Default::default(),
            accumulated_visual_context: Default::default(),
            absolute_rect: Default::default(),
            box_type_agnostic_position: Default::default(),
        }
    }
}

impl HitTestSnapshot<'_> {
    /// The box the DOM node `identity` names was bound to: the viewport for the document.
    fn bound_box(&self, identity: FfiHitNodeIdentity) -> Option<NodeSlotId> {
        let row = match identity.kind {
            FfiHitNodeIdentityKind::None => return None,
            FfiHitNodeIdentityKind::Document => self.rows.viewport_row()?,
            FfiHitNodeIdentityKind::StyleNode => {
                let element = StyleNodeID::from_raw(identity.style_node)?;
                element.element_index()?;
                self.rows.bound_row(element)?
            }
        };
        self.read_rows(|rows| rows.slot_is_live(row).then_some(row))
    }

    fn box_facts(&self, row: NodeSlotId) -> FfiHitBoxFacts {
        self.read_rows(|rows| {
            let Some(kind) = rows.node_kind_if_live(row) else {
                return FfiHitBoxFacts::default();
            };
            let style_node = rows.node_style_node(row);
            let generated_for = rows.node_generated_for(row);
            let identity = if rows.node_flags_if_live(row) & NodeFlag::Anonymous as u32 != 0 {
                FfiHitNodeIdentity::default()
            } else if kind == NodeKind::Viewport {
                FfiHitNodeIdentity::DOCUMENT
            } else {
                FfiHitNodeIdentity::of_style_node(style_node)
            };
            let mut facts = FfiHitBoxFacts {
                kind,
                generated_for,
                identity,
                generator: if generated_for != 0 {
                    FfiHitNodeIdentity::of_style_node(style_node)
                } else {
                    FfiHitNodeIdentity::default()
                },
                parent: FfiHitBox::of(rows.node_parent_if_live(row)),
                ..FfiHitBoxFacts::default()
            };
            if !rows.paintable_row_is_populated(row) {
                return facts;
            }
            facts.has_committed_box = true;
            facts.local_content_navigable = rows
                .replaced_paint_facts(row)
                .and_then(|facts| facts.navigable_container())
                .map(|facts| facts.local_content_navigable)
                .unwrap_or_default();
            facts.accumulated_visual_context = rows.paintable_data(row).accumulated_visual_context;
            facts.absolute_rect = crate::painting::paintable_geometry::absolute_rect_or_default(rows, row).into();
            facts.box_type_agnostic_position =
                crate::painting::paintable_geometry::box_type_agnostic_position(rows, row).into();
            facts
        })
    }
}

/// SAFETY: `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
pub(super) unsafe fn snapshot_from_handle<'a>(snapshot: *const c_void) -> HitTestSnapshot<'a> {
    assert!(!snapshot.is_null(), "hit-test snapshot handle is null");
    HitTestSnapshot {
        rows: unsafe { &*snapshot.cast::<RowSnapshot>() },
    }
}

/// The document's rows as the owner published them last, with its hit-test list, as a snapshot the caller releases
/// with `hit_test_snapshot_release`. It is read without waiting for the owner: a script's hit test read the rows as of
/// what it sent when it laid the document out first.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_snapshot(arena: *mut c_void) -> *const c_void {
    // SAFETY: Guaranteed by the caller.
    crate::lent::Lent::into_raw(unsafe { crate::layout::row_reads::FrameRows::of(arena) }.into_shared()).cast()
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_release(snapshot: *const c_void) {
    assert!(!snapshot.is_null(), "hit-test snapshot handle is null");
    drop(unsafe { crate::lent::Lent::from_raw(snapshot.cast::<RowSnapshot>()) });
}

/// Visits the chrome widgets the document's hit-test list holds items of, as the rows published it, and answers the
/// list's generation (zero for none). The document visits a recording's list once it took the recording in.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and `visit` must accept `sink`
/// for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visit_hit_test_chrome_widgets(
    arena: *mut c_void,
    sink: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, NodeSlotId, u8),
) -> u64 {
    // SAFETY: Guaranteed by the caller. The list is shared, so the borrow of the rows ends before the host visits.
    let list = unsafe { RowSnapshot::published(arena) }.paintable.hit_test_list.clone();
    let Some(list) = list else {
        return 0;
    };
    for item in list.items.iter() {
        if item.chrome_widget_kind != crate::painting::hit_test::CHROME_WIDGET_NONE {
            // SAFETY: The C++ host consumes the visit synchronously.
            unsafe { visit(sink, item.paintable, item.chrome_widget_kind) };
        }
    }
    list.generation
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_find_topmost_item(
    snapshot: *const c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
) -> crate::painting::host::FfiTopmostItem {
    unsafe { snapshot_from_handle(snapshot) }.query(Default::default(), |list, tree, rows| {
        crate::painting::ffi::ffi_topmost(list.find_topmost_item(rows, tree, &callbacks, point.into()))
    })
}

/// Pushes the index of every item the point hits, topmost first.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and `push` must
/// accept `push_context` for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_all(
    snapshot: *const c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
    push_context: *mut c_void,
    push: unsafe extern "C" fn(*mut c_void, usize),
) {
    let indices = unsafe { snapshot_from_handle(snapshot) }.query(Vec::new(), |list, tree, rows| {
        list.hit_test_all(rows, tree, &callbacks, point.into())
    });
    for index in indices {
        // SAFETY: The C++ sink consumes the index synchronously.
        unsafe { push(push_context, index) };
    }
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and `index` an
/// item of its list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_item(snapshot: *const c_void, index: usize) -> FfiHitTestSnapshotItem {
    let snapshot = unsafe { snapshot_from_handle(snapshot) };
    // The host names items a query of the same snapshot answered with.
    let Some(item) = snapshot.list().and_then(|list| list.items.get(index)) else {
        debug_assert!(false, "an item the snapshot does not hold");
        return FfiHitTestSnapshotItem {
            can_produce_caret_position: false,
            paintable: NodeSlotId::INVALID,
            hit_node: NodeSlotId::INVALID,
            chrome_widget_kind: crate::painting::hit_test::CHROME_WIDGET_NONE,
            caret_node: NodeSlotId::INVALID,
            caret_rect: Default::default(),
            context: Default::default(),
        };
    };
    FfiHitTestSnapshotItem {
        can_produce_caret_position: item.can_produce_caret_position,
        paintable: item.paintable,
        hit_node: item.hit_node,
        chrome_widget_kind: item.chrome_widget_kind,
        caret_node: item.caret_node,
        caret_rect: item.caret_rect.into(),
        context: item.context,
    }
}

/// Resolves a hit on the item at `local_point` to the DOM node it names.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and `index` an
/// item of its list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_resolve_hit(
    snapshot: *const c_void,
    index: usize,
    local_point: FfiCssPixelPoint,
) -> FfiHitTestSnapshotHit {
    unsafe { snapshot_from_handle(snapshot) }.resolve_hit(index, local_point.into())
}

/// The box the DOM node `identity` names was bound to in the snapshot, or none.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_bound_box(
    snapshot: *const c_void,
    identity: FfiHitNodeIdentity,
) -> FfiHitBox {
    FfiHitBox::of(unsafe { snapshot_from_handle(snapshot) }.bound_box(identity))
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and `hit_box` a box it answered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_box_facts(snapshot: *const c_void, hit_box: FfiHitBox) -> FfiHitBoxFacts {
    unsafe { snapshot_from_handle(snapshot) }.box_facts(hit_box.row())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutNodeArena;
    use crate::painting::visual_context::{TransformData, TransformDataRole};
    use std::sync::Arc;

    fn tree() -> Option<Arc<VisualContextTree>> {
        Some(Arc::new(VisualContextTree::create(TransformData {
            matrix: libgfx_rust::FloatMatrix4x4::identity(),
            origin: Default::default(),
            sorting_context_root_index: None,
            flattens_inherited_transform: false,
            role: TransformDataRole::CssTransform,
            synthetic_plane: false,
            establishes_sorting_context: false,
        })))
    }

    /// A snapshot answers from the list and the visual context tree the rows it reads were published with, whatever
    /// the arena publishes after them.
    #[test]
    fn a_snapshot_answers_from_the_rows_it_reads() {
        let mut arena = LayoutNodeArena::new();
        let published_tree = tree();
        arena.paint_state().borrow_mut().visual_context.tree = published_tree.clone();
        *arena.hit_test_list.get_mut() = Some(Arc::new(HitTestList {
            generation: 1,
            spatial_indexes_built: true,
            ..Default::default()
        }));
        arena.publish_rows();
        let rows = arena.published_rows();
        let snapshot = HitTestSnapshot { rows: &rows };

        *arena.hit_test_list.get_mut() = Some(Arc::new(HitTestList {
            generation: 2,
            ..Default::default()
        }));
        arena.paint_state().borrow_mut().visual_context.tree = None;
        arena.publish_rows();

        let list = snapshot.list().expect("the snapshot holds its list");
        assert_eq!(list.generation, 1);
        assert!(snapshot.query(false, |_, tree, _| std::ptr::eq(
            tree,
            published_tree.as_deref().unwrap()
        )));
        let rows = arena.published_rows();
        assert_eq!(
            HitTestSnapshot { rows: &rows }.list().map(|list| list.generation),
            Some(2)
        );
    }

    /// A snapshot names the DOM node a row stands for as the host names it, from the style node the
    /// rows it reads published for the row, whatever the arena's rows name after the snapshot: the document
    /// for the viewport, which stands for the root element in a hit, and nothing for an anonymous
    /// row but where it was generated for a pseudo-element and may stand for its element.
    #[test]
    fn a_snapshot_names_the_dom_node_its_rows_published_for_a_row() {
        let mut arena = LayoutNodeArena::new();
        let viewport = arena.allocate_for_test().slot;
        arena.write_shape(viewport).set_kind(NodeKind::Viewport);
        arena.populate_paintable_row(viewport);
        let root = arena.allocate_for_test().slot;
        arena.write_shape(root).set_kind(NodeKind::BlockContainer);
        arena.populate_paintable_row(root);
        arena.set_style_node_for_test(root, StyleNodeID::from_raw(5));
        let pseudo = arena.allocate_for_test().slot;
        arena.write_shape(pseudo).set_kind(NodeKind::BlockContainer);
        arena.write_shape(pseudo).set_flags(NodeFlag::Anonymous as u32);
        arena
            .write_shape(pseudo)
            .set_generated_for(crate::layout::node_data::GENERATED_FOR_BEFORE);
        arena.set_style_node_for_test(pseudo, StyleNodeID::from_raw(5));
        arena.paint_state().borrow_mut().root_background_source =
            Some(crate::painting::host::FfiRootBackgroundSource {
                root_layout_node: root,
                ..Default::default()
            });
        arena.publish_rows();
        let snapshot = arena.published_rows();
        arena.set_style_node_for_test(root, StyleNodeID::from_raw(6));
        arena.publish_paint_tree();

        let element = FfiHitNodeIdentity {
            kind: FfiHitNodeIdentityKind::StyleNode,
            style_node: 5,
        };
        let absolute_rects = RefCell::default();
        let rows = PaintSource::of_rows(&snapshot, &absolute_rects);
        assert_eq!(snapshot.paint_status.root_background_source.root_layout_node, root);
        assert!(dispatch_identity(&rows, Some(root), false) == element);
        assert!(dispatch_identity(&rows, Some(viewport), false) == FfiHitNodeIdentity::DOCUMENT);
        assert!(dispatch_identity(&rows, Some(pseudo), false).is_none());
        assert!(dispatch_identity(&rows, Some(pseudo), true) == element);
        assert!(dispatch_identity(&rows, None, true).is_none());
    }
}
