/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::abspos_inputs::AbsposLayoutInputs;
use super::formatting_context::DerivedBaselines;
use super::formatting_context::LayoutHost;
use super::formatting_context::LayoutMode;
use super::geometry::AvailableSize;
use super::geometry::AvailableSpace;
use super::host_tables::HostTables;
use super::rendered_text::{FfiTextSourceRange, RenderedTextBoundary, TextContent, TextFragments};
use super::svg_formatting_context::FfiSvgAttributeFacts;

/// How many interned names one SVG element's publication can name.
const PUBLISHED_REFERENCE_ATOM_COUNT: usize = 5;
use super::tree_builder::FfiLayoutTreeBuildOutcome;
use super::update_layout::FfiLayoutTreeBuildStats;
use super::used_values::SizeConstraint;
use crate::css::style::bridge::ElementBoxKind;
use crate::css::style::fast_hash::{FastMap as HashMap, FastSet as HashSet};
use crate::css::style::tree::StyleNodeID;
use crate::css::style::{
    PublishedBoxFacts, PublishedTextSource, StyleEngine, TextStyleParentFacts,
    layout_style::{AnonymousStyleKind, AnonymousStyleOverrides, DerivedStyleRecord, LayoutStyle},
};
use crate::layout::ComputedValuesView;
use crate::layout::CssPixels;
use crate::layout::FfiReplacedContentFacts;
use crate::layout::node_data::{
    AncestorFact, DomPaintFact, FfiNodeConstructionFacts, FfiNodeLink, FfiStylePayloads, MAX_NODE_SLOT_COUNT, NodeData,
    NodeFlag, NodeKind, NodeSlotId, ShellId, StylePayloadsRef,
};
use crate::layout::used_values::FfiCssPixelPoint;
use std::cell::Cell;
use std::cell::RefCell;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::thread;

mod main_thread_entries;

pub(crate) use main_thread_entries::MainThreadFfiEntry;

pub(crate) const SLOTS_PER_CHUNK: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IntrinsicSizeCacheKey {
    pub(crate) measured_at_inline_size: Option<CssPixels>,
    pub(crate) measured_at_block_size: Option<CssPixels>,
    pub(crate) percentage_basis_inline_size: Option<CssPixels>,
    pub(crate) percentage_basis_block_size: Option<CssPixels>,
    pub(crate) quirks_mode_percentage_basis_block_size: Option<CssPixels>,
}

impl IntrinsicSizeCacheKey {
    fn with_percentage_block_bases_masked(self) -> Self {
        Self {
            percentage_basis_block_size: None,
            quirks_mode_percentage_basis_block_size: None,
            ..self
        }
    }

    fn with_percentage_inline_basis_masked(self) -> Self {
        Self {
            percentage_basis_inline_size: None,
            ..self
        }
    }

    fn masked_for(self, dependencies: IntrinsicMeasurementDependencies) -> Self {
        let mut key = self;
        if !dependencies.percentage_block_size {
            key = key.with_percentage_block_bases_masked();
        }
        if !dependencies.percentage_inline_basis {
            key = key.with_percentage_inline_basis_masked();
        }
        key
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IntrinsicMeasurementDependencies {
    pub(crate) percentage_block_size: bool,
    pub(crate) percentage_inline_basis: bool,
}

pub(crate) trait IntrinsicMeasurement: Copy {
    fn dependencies(&self) -> IntrinsicMeasurementDependencies;
}

// Measurements are stored under their key with every basis they never observed masked out. A
// probe that had to mask a basis must skip entries that observed it: a measurement made without
// that basis shares the masked key's shape.
fn intrinsic_cache_lookup<V: IntrinsicMeasurement>(
    map: &HashMap<IntrinsicSizeCacheKey, V>,
    key: IntrinsicSizeCacheKey,
) -> Option<V> {
    if let Some(value) = map.get(&key) {
        return Some(*value);
    }
    let block_masked = key.with_percentage_block_bases_masked();
    let inline_masked = key.with_percentage_inline_basis_masked();
    let masking_block_changes_key = block_masked != key;
    let masking_inline_changes_key = inline_masked != key;
    let candidates = [
        (block_masked, masking_block_changes_key, true, false),
        (inline_masked, masking_inline_changes_key, false, true),
        (
            block_masked.with_percentage_inline_basis_masked(),
            masking_block_changes_key && masking_inline_changes_key,
            true,
            true,
        ),
    ];
    candidates.into_iter().filter(|(_, applies, _, _)| *applies).find_map(
        |(candidate, _, block_was_masked, inline_was_masked)| {
            let value = *map.get(&candidate)?;
            let dependencies = value.dependencies();
            let observed_a_masked_basis = (block_was_masked && dependencies.percentage_block_size)
                || (inline_was_masked && dependencies.percentage_inline_basis);
            (!observed_a_masked_basis).then_some(value)
        },
    )
}

fn intrinsic_cache_store<V: IntrinsicMeasurement>(
    map: &mut HashMap<IntrinsicSizeCacheKey, V>,
    key: IntrinsicSizeCacheKey,
    value: V,
) {
    map.insert(key.masked_for(value.dependencies()), value);
}

impl Hash for IntrinsicSizeCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn hash_optional<H: Hasher>(value: Option<CssPixels>, state: &mut H) {
            match value {
                Some(value) => {
                    true.hash(state);
                    value.raw_value().hash(state);
                }
                None => false.hash(state),
            }
        }

        hash_optional(self.measured_at_inline_size, state);
        hash_optional(self.measured_at_block_size, state);
        hash_optional(self.percentage_basis_inline_size, state);
        hash_optional(self.percentage_basis_block_size, state);
        hash_optional(self.quirks_mode_percentage_basis_block_size, state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TableCellMeasurementKey {
    pub(crate) layout_mode: LayoutMode,
    pub(crate) available_space: AvailableSpace,
    pub(crate) content_inline_size: CssPixels,
    pub(crate) content_block_size: CssPixels,
    pub(crate) has_definite_inline_size: bool,
    pub(crate) has_definite_block_size: bool,
    pub(crate) inline_size_constraint: SizeConstraint,
    pub(crate) block_size_constraint: SizeConstraint,
    pub(crate) uses_collapsing_borders_model: bool,
    pub(crate) adopt_automatic_content_block_size: bool,
}

impl Hash for TableCellMeasurementKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn hash_available_size<H: Hasher>(size: AvailableSize, state: &mut H) {
            match size {
                AvailableSize::Definite(value) => {
                    0u8.hash(state);
                    value.raw_value().hash(state);
                }
                AvailableSize::Indefinite => 1u8.hash(state),
                AvailableSize::MinContent => 2u8.hash(state),
                AvailableSize::MaxContent => 3u8.hash(state),
            }
        }

        (self.layout_mode as u8).hash(state);
        hash_available_size(self.available_space.inline_size, state);
        hash_available_size(self.available_space.block_size, state);
        self.content_inline_size.raw_value().hash(state);
        self.content_block_size.raw_value().hash(state);
        self.has_definite_inline_size.hash(state);
        self.has_definite_block_size.hash(state);
        (self.inline_size_constraint as u8).hash(state);
        (self.block_size_constraint as u8).hash(state);
        self.uses_collapsing_borders_model.hash(state);
        self.adopt_automatic_content_block_size.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TableCellMeasurement {
    pub(crate) automatic_content_block_size: CssPixels,
    pub(crate) baselines: DerivedBaselines,
    pub(crate) depends_on_percentage_block_size: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntrinsicSizeCacheKind {
    MinContentInline,
    MaxContentInline,
    MinContentBlock,
    MaxContentBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IntrinsicInlineSizeMeasurement {
    pub(crate) automatic_content_inline_size: CssPixels,
    pub(crate) min_content_inline_size_from_max_content_layout: Option<CssPixels>,
    // A dedicated inline-size query does not produce block sizes or baselines.
    pub(crate) layout: Option<IntrinsicInlineMeasurementLayout>,
    pub(crate) depends_on_percentage_block_size: bool,
    pub(crate) depends_on_percentage_inline_basis: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IntrinsicInlineMeasurementLayout {
    pub(crate) available_block_size: AvailableSize,
    pub(crate) content_inline_size: CssPixels,
    pub(crate) content_block_size: CssPixels,
    pub(crate) automatic_content_block_size: CssPixels,
    pub(crate) uses_collapsing_borders_model: bool,
    pub(crate) is_collapsed_borders_table_box: bool,
    pub(crate) has_first_baseline: bool,
    pub(crate) first_baseline: CssPixels,
    pub(crate) has_last_baseline: bool,
    pub(crate) last_baseline: CssPixels,
}

impl IntrinsicMeasurement for IntrinsicInlineSizeMeasurement {
    fn dependencies(&self) -> IntrinsicMeasurementDependencies {
        IntrinsicMeasurementDependencies {
            percentage_block_size: self.depends_on_percentage_block_size,
            percentage_inline_basis: self.depends_on_percentage_inline_basis,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IntrinsicBlockSizeMeasurement {
    pub(crate) size: CssPixels,
    pub(crate) depends_on_percentage_block_size: bool,
    pub(crate) depends_on_percentage_inline_basis: bool,
}

impl IntrinsicMeasurement for IntrinsicBlockSizeMeasurement {
    fn dependencies(&self) -> IntrinsicMeasurementDependencies {
        IntrinsicMeasurementDependencies {
            percentage_block_size: self.depends_on_percentage_block_size,
            percentage_inline_basis: self.depends_on_percentage_inline_basis,
        }
    }
}

#[derive(Default)]
struct IntrinsicSizeMaps {
    // This is a subtree fact and shares the intrinsic cache's epoch so descendant changes invalidate it.
    inline_size_depends_on_block_size: Option<bool>,
    min_content_inline_size: HashMap<IntrinsicSizeCacheKey, IntrinsicInlineSizeMeasurement>,
    max_content_inline_size: HashMap<IntrinsicSizeCacheKey, IntrinsicInlineSizeMeasurement>,
    min_content_block_size: HashMap<IntrinsicSizeCacheKey, IntrinsicBlockSizeMeasurement>,
    max_content_block_size: HashMap<IntrinsicSizeCacheKey, IntrinsicBlockSizeMeasurement>,
    table_cell_measurements: HashMap<TableCellMeasurementKey, TableCellMeasurement>,
}

impl IntrinsicSizeMaps {
    fn block_sizes(
        &self,
        kind: IntrinsicSizeCacheKind,
    ) -> Option<&HashMap<IntrinsicSizeCacheKey, IntrinsicBlockSizeMeasurement>> {
        match kind {
            IntrinsicSizeCacheKind::MinContentInline | IntrinsicSizeCacheKind::MaxContentInline => None,
            IntrinsicSizeCacheKind::MinContentBlock => Some(&self.min_content_block_size),
            IntrinsicSizeCacheKind::MaxContentBlock => Some(&self.max_content_block_size),
        }
    }

    fn block_sizes_mut(
        &mut self,
        kind: IntrinsicSizeCacheKind,
    ) -> Option<&mut HashMap<IntrinsicSizeCacheKey, IntrinsicBlockSizeMeasurement>> {
        match kind {
            IntrinsicSizeCacheKind::MinContentInline | IntrinsicSizeCacheKind::MaxContentInline => None,
            IntrinsicSizeCacheKind::MinContentBlock => Some(&mut self.min_content_block_size),
            IntrinsicSizeCacheKind::MaxContentBlock => Some(&mut self.max_content_block_size),
        }
    }

    fn inline_measurements(
        &self,
        kind: IntrinsicSizeCacheKind,
    ) -> Option<&HashMap<IntrinsicSizeCacheKey, IntrinsicInlineSizeMeasurement>> {
        match kind {
            IntrinsicSizeCacheKind::MinContentInline => Some(&self.min_content_inline_size),
            IntrinsicSizeCacheKind::MaxContentInline => Some(&self.max_content_inline_size),
            IntrinsicSizeCacheKind::MinContentBlock | IntrinsicSizeCacheKind::MaxContentBlock => None,
        }
    }

    fn inline_measurements_mut(
        &mut self,
        kind: IntrinsicSizeCacheKind,
    ) -> Option<&mut HashMap<IntrinsicSizeCacheKey, IntrinsicInlineSizeMeasurement>> {
        match kind {
            IntrinsicSizeCacheKind::MinContentInline => Some(&mut self.min_content_inline_size),
            IntrinsicSizeCacheKind::MaxContentInline => Some(&mut self.max_content_inline_size),
            IntrinsicSizeCacheKind::MinContentBlock | IntrinsicSizeCacheKind::MaxContentBlock => None,
        }
    }
}

#[derive(Default)]
struct IntrinsicSizeCacheSlot {
    generation: u8,
    epoch: u16,
    sizes: Option<Box<IntrinsicSizeMaps>>,
}

/// The intrinsic sizes layout measured, kept from one pass to the next and indexed by slot. Each
/// entry is stamped with its row's generation and intrinsic cache epoch, and invalidation bumps the
/// epoch on the row. The caches are the layout stage's scratch: the arena only records the slots
/// whose entries must go before the next pass reads them.
#[derive(Default)]
pub(crate) struct IntrinsicSizeCaches {
    slots: RefCell<Vec<IntrinsicSizeCacheSlot>>,
}

impl IntrinsicSizeCaches {
    /// Drops the entries of `slots`, which the arena recorded since the last pass.
    pub(crate) fn drop_slots(&self, slots: Vec<u32>) {
        let mut caches = self.slots.borrow_mut();
        for index in slots {
            if let Some(slot) = caches.get_mut(index as usize) {
                *slot = IntrinsicSizeCacheSlot::default();
            }
        }
    }

    pub(crate) fn intrinsic_block_size_cache_get(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
    ) -> Option<IntrinsicBlockSizeMeasurement> {
        assert!(
            matches!(
                kind,
                IntrinsicSizeCacheKind::MinContentBlock | IntrinsicSizeCacheKind::MaxContentBlock
            ),
            "block size cache kind must use the block axis"
        );

        let (index, metadata) = arena.slot_for_data(data);
        let caches = self.slots.borrow();
        let slot = caches.get(index as usize)?;
        if slot.generation != metadata.generation || slot.epoch != data.intrinsic_cache_epoch.get() {
            return None;
        }
        let map = slot
            .sizes
            .as_ref()?
            .block_sizes(kind)
            .expect("block size cache kind must use the block axis");
        intrinsic_cache_lookup(map, key)
    }

    fn with_intrinsic_size_maps_mut(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        callback: impl FnOnce(&mut IntrinsicSizeMaps),
    ) {
        let (index, metadata) = arena.slot_for_data(data);
        let mut caches = self.slots.borrow_mut();
        if caches.len() <= index as usize {
            caches.resize_with(index as usize + 1, IntrinsicSizeCacheSlot::default);
        }
        let slot = &mut caches[index as usize];
        if slot.generation != metadata.generation || slot.epoch != data.intrinsic_cache_epoch.get() {
            *slot = IntrinsicSizeCacheSlot {
                generation: metadata.generation,
                epoch: data.intrinsic_cache_epoch.get(),
                sizes: Some(Box::default()),
            };
        }
        callback(slot.sizes.get_or_insert_with(Box::default));
    }

    pub(crate) fn intrinsic_block_size_cache_put(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
        value: IntrinsicBlockSizeMeasurement,
    ) {
        self.with_intrinsic_size_maps_mut(arena, data, |maps| {
            let map = maps
                .block_sizes_mut(kind)
                .expect("block size cache kind must use the block axis");
            intrinsic_cache_store(map, key, value);
        });
    }

    pub(crate) fn intrinsic_inline_size_measurement_cache_get(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
    ) -> Option<IntrinsicInlineSizeMeasurement> {
        assert!(
            matches!(
                kind,
                IntrinsicSizeCacheKind::MinContentInline | IntrinsicSizeCacheKind::MaxContentInline
            ),
            "inline measurement cache kind must use the inline axis"
        );

        let (index, metadata) = arena.slot_for_data(data);
        let caches = self.slots.borrow();
        let slot = caches.get(index as usize)?;
        if slot.generation != metadata.generation || slot.epoch != data.intrinsic_cache_epoch.get() {
            return None;
        }
        let map = slot
            .sizes
            .as_ref()?
            .inline_measurements(kind)
            .expect("inline measurement cache kind must use the inline axis");
        intrinsic_cache_lookup(map, key)
    }

    pub(crate) fn intrinsic_inline_size_depends_on_block_size(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        compute: impl FnOnce() -> bool,
    ) -> bool {
        let (index, metadata) = arena.slot_for_data(data);
        {
            let caches = self.slots.borrow();
            if let Some(slot) = caches.get(index as usize)
                && slot.generation == metadata.generation
                && slot.epoch == data.intrinsic_cache_epoch.get()
                && let Some(value) = slot
                    .sizes
                    .as_ref()
                    .and_then(|sizes| sizes.inline_size_depends_on_block_size)
            {
                return value;
            }
        }

        let value = compute();
        self.with_intrinsic_size_maps_mut(arena, data, |maps| {
            maps.inline_size_depends_on_block_size = Some(value);
        });
        value
    }

    pub(crate) fn intrinsic_inline_size_measurement_cache_put(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
        value: IntrinsicInlineSizeMeasurement,
    ) {
        self.with_intrinsic_size_maps_mut(arena, data, |maps| {
            let map = maps
                .inline_measurements_mut(kind)
                .expect("inline measurement cache kind must use the inline axis");
            intrinsic_cache_store(map, key, value);
        });
    }

    pub(crate) fn table_cell_measurement_cache_get(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        key: TableCellMeasurementKey,
    ) -> Option<TableCellMeasurement> {
        let (index, metadata) = arena.slot_for_data(data);
        let caches = self.slots.borrow();
        let slot = caches.get(index as usize)?;
        if slot.generation != metadata.generation || slot.epoch != data.intrinsic_cache_epoch.get() {
            return None;
        }
        slot.sizes.as_ref()?.table_cell_measurements.get(&key).copied()
    }

    pub(crate) fn table_cell_measurement_cache_put(
        &self,
        arena: &LayoutNodeArena,
        data: &NodeData,
        key: TableCellMeasurementKey,
        value: TableCellMeasurement,
    ) {
        self.with_intrinsic_size_maps_mut(arena, data, |maps| {
            maps.table_cell_measurements.insert(key, value);
        });
    }
}

#[derive(Clone, Copy, Default)]
struct DefaultScrollShiftAnchorSlot {
    generation: u8,
    anchor: NodeSlotId,
}

#[derive(Default)]
struct TextNodeSlot {
    generation: u8,
    state: Option<Box<TextNodeState>>,
}

#[derive(Default)]
struct TextNodeState {
    source_range: Option<FfiTextSourceRange>,
    first_letter: NodeSlotId,
    content: Option<TextContent>,
    /// What a generated text row renders. Generated content has no DOM text node behind it, so
    /// the build stamps the characters it resolved on the row rather than reading them back out
    /// of the host object it asked to be allocated.
    generated_text: Option<ak::Utf16String>,
}

#[derive(Default)]
struct ReplacedContentFactsSlot {
    generation: u8,
    facts: Option<FfiReplacedContentFacts>,
}

// NodeData is sized to one cache line; the aligned chunk keeps every densely-strided slot
// line-aligned, and per-slot bookkeeping lives in a parallel array so it stays that way.
#[repr(align(64))]
pub(crate) struct Chunk {
    slots: [NodeData; SLOTS_PER_CHUNK],
}

fn new_chunk() -> Box<Chunk> {
    // SAFETY: Every slot is written with NodeData::default() before the chunk is exposed. The
    // chunk is built in place on the heap because it is far too large for the stack.
    unsafe {
        let mut chunk = Box::<Chunk>::new_uninit();
        let slots = &raw mut (*chunk.as_mut_ptr()).slots;
        for offset in 0..SLOTS_PER_CHUNK {
            (&raw mut (*slots)[offset]).write(NodeData::default());
        }
        chunk.assume_init()
    }
}

#[derive(Clone, Copy, Default)]
struct SlotMetadata {
    generation: u8,
    occupied: bool,
}

#[derive(Clone, Copy)]
struct ChunkAddress {
    start: usize,
    chunk_index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AncestorInvalidation {
    StructuralChange,
    ContentChange,
}

pub(crate) type ShellFactory = (*mut c_void, unsafe extern "C" fn(*mut c_void, NodeSlotId, NodeKind));

/// How the host learns what boxes a DOM node has. The identity is 0 for the document, which has
/// none of its own. The callback must not reenter the arena: it runs while the arena is changing
/// the bindings it would read.
pub(crate) type BoxPresenceHost = (*mut c_void, unsafe extern "C" fn(*mut c_void, u32, u8));

/// How a shell hears that its row's style changed: the shell, the new record and its payloads, and
/// whether the shell attaches its style resources again.
pub(crate) type ShellStyleChangedHost = (
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, *mut c_void, u64, *const c_void, bool),
);

/// A row is bound to the node.
pub const BOX_PRESENCE_HAS_LAYOUT_BOX: u8 = 1 << 0;
/// The bound row has a committed box.
pub const BOX_PRESENCE_HAS_COMMITTED_BOX: u8 = 1 << 1;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiStyleRecordHostCallbacks {
    pub style_engine: *mut c_void,
    pub context: *mut c_void,
    pub shell_style_changed: unsafe extern "C" fn(*mut c_void, *mut c_void, u64, *const c_void, bool),
}

fn style_payloads_equal_in_layout_affecting_groups(a: *const c_void, b: *const c_void) -> bool {
    if a == b {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }
    // SAFETY: A non-null style pointer addresses the engine's group pointer array, which
    // FfiStylePayloads mirrors exactly.
    let (a, b) = unsafe { (&*a.cast::<FfiStylePayloads>(), &*b.cast::<FfiStylePayloads>()) };
    (0..a.groups.len()).all(|group_index| {
        !crate::css::computed_values::style_group_affects_layout(group_index)
            || a.groups[group_index] == b.groups[group_index]
            || crate::css::computed_values::style_group_payloads_equal(
                group_index,
                a.groups[group_index],
                b.groups[group_index],
            )
    })
}

/// The image resources a tree build owes the host for a row it stamped, which the host attaches
/// once the frame the build ran in is over.
#[derive(Clone, Copy)]
pub(crate) enum OwedImageResources {
    /// The row's style resources: the images its style names, and the paint facts that follow.
    StyleResources { owns_content_replacement_image: bool },
    /// The provider for an image a pseudo-element's generated content names, and the image box's
    /// style resources.
    GeneratedImage {
        generator: StyleNodeID,
        pseudo_element: super::tree_builder::FfiPseudoElement,
        item: super::tree_builder::FfiGeneratedContentItem,
        pseudo_element_box: NodeSlotId,
    },
}

#[must_use]
pub(crate) struct FreedSubtree {
    shells: Vec<ShellId>,
    rows_with_owned_image_provider: Vec<NodeSlotId>,
    rows_with_image_observers: Vec<NodeSlotId>,
    paintable_row_resets: Vec<crate::painting::paintable_rows::PaintableRowReset>,
    arena_pinned_style_records: Vec<u64>,
    style_engine: *mut c_void,
}

/// Who hears that a shell's style changed: the host at once, which only the main thread can ask,
/// or the handbacks of the span the change is made in (a tree build's, or a layout pass's), which
/// a main-thread payer pays once that work is over.
#[derive(Clone, Copy)]
pub(crate) enum ShellStyleChangeNotice<'a> {
    Now(&'a crate::stage::MainThread<'a>),
    Handback,
}

/// One thing the arena owes the host: a node's box presence, or an object of a row's that the host
/// owns the memory of and that the row has let go of. The arena only queues these; a main-thread
/// caller pays them, and a tree build returns them as part of its output.
enum HostHandback {
    /// The node whose boxes changed, named the way the box presence host names it. The bits are
    /// read when the handback is paid, so a node a build changes several times is told once.
    BoxPresence(u32),
    Shell(ShellId),
    /// A row's host objects, named by the row. The host tables hold the objects themselves, and
    /// the payer looks them up before it pays anything, which is where the arena let go of them.
    OwnedImageProvider(NodeSlotId),
    ImageObservers(NodeSlotId),
    OwnedImageProviderDetach(NodeSlotId),
    PaintableRowReset(crate::painting::paintable_rows::PaintableRowReset),
    /// A shell whose row's style changed while the tree build ran. The host is handed the style
    /// the row has when this is paid, and nothing if the row has gone by then.
    ShellStyleChanged {
        slot: NodeSlotId,
        shell: ShellId,
        attach_resources: bool,
    },
}

/// What the arena owes the host, in the order it let go of it.
#[derive(Default)]
pub(crate) struct HostHandbacks {
    handbacks: Vec<HostHandback>,
    nodes_with_box_presence: HashSet<u32>,
}

impl HostHandbacks {
    fn push(&mut self, handback: HostHandback) {
        match handback {
            HostHandback::BoxPresence(node) => {
                if !self.nodes_with_box_presence.insert(node) {
                    return;
                }
            }
            HostHandback::OwnedImageProvider(_)
            | HostHandback::ImageObservers(_)
            | HostHandback::OwnedImageProviderDetach(_)
            | HostHandback::PaintableRowReset(_)
            | HostHandback::Shell(_)
            | HostHandback::ShellStyleChanged { .. } => {}
        }
        self.handbacks.push(handback);
    }
}

/// The node a layout row can be bound to: an element or text node, named by its identity, a
/// pseudo-element, which has no identity of its own and is named by its generator's identity and
/// its kind, or the document, which is bound to a viewport row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoundNode {
    Identity(StyleNodeID),
    PseudoElement(StyleNodeID, u8),
    Document,
}

impl BoundNode {
    /// The node a row carrying `style_node` can be bound to: the node itself, or the pseudo-element
    /// the row was generated for.
    fn of(style_node: StyleNodeID, generated_for: u8) -> Self {
        if generated_for == 0 {
            BoundNode::Identity(style_node)
        } else {
            BoundNode::PseudoElement(style_node, generated_for)
        }
    }
}

impl FreedSubtree {
    #[cfg(test)]
    pub(crate) fn shell_count(&self) -> usize {
        self.shells.len()
    }

    #[cfg(test)]
    pub(crate) fn arena_pinned_style_record_count(&self) -> usize {
        self.arena_pinned_style_records.len()
    }

    #[cfg(test)]
    pub(crate) fn destroy_shells_and_invoke_callbacks(self) {
        let main_thread = crate::stage::MainThread::for_test();
        for shell in self.shells {
            crate::layout::tree_mutation::destroy_shell(&main_thread, shell.host_object(&main_thread));
        }
        assert!(
            self.rows_with_owned_image_provider.is_empty() && self.rows_with_image_observers.is_empty(),
            "a test arena has no host to own image objects"
        );
        for reset in self.paintable_row_resets {
            reset.invoke_callback_on_main_thread(&main_thread);
        }
        Self::unpin_arena_pinned_style_records(self.style_engine, self.arena_pinned_style_records);
    }

    fn unpin_arena_pinned_style_records(style_engine: *mut c_void, arena_pinned_style_records: Vec<u64>) {
        if !style_engine.is_null() {
            for style_record in arena_pinned_style_records {
                // SAFETY: Registration and unregistration keep the style engine live.
                unsafe { &mut *style_engine.cast::<StyleEngine>() }.unpin_layout_style_record(style_record);
            }
        }
    }
}

const MAXIMUM_PRE_ORDER_LABEL_STRIDE: u64 = 1 << 32;

/// One row for each StyleNodeID, indexed by the identity's dense index within its kind, so element
/// and text identities each cost one entry per node of their own kind.
#[derive(Default)]
struct RowsByStyleNode {
    elements: Vec<NodeSlotId>,
    texts: Vec<NodeSlotId>,
}

impl RowsByStyleNode {
    fn head(&self, style_node: StyleNodeID) -> NodeSlotId {
        let (rows, index) = match style_node.element_index() {
            Some(index) => (&self.elements, index),
            None => (&self.texts, style_node.text_index().unwrap()),
        };
        rows.get(index as usize).copied().unwrap_or(NodeSlotId::INVALID)
    }

    fn head_mut(&mut self, style_node: StyleNodeID) -> &mut NodeSlotId {
        let (rows, index) = match style_node.element_index() {
            Some(index) => (&mut self.elements, index),
            None => (&mut self.texts, style_node.text_index().unwrap()),
        };
        let index = index as usize;
        if rows.len() <= index {
            rows.resize(index + 1, NodeSlotId::INVALID);
        }
        &mut rows[index]
    }
}

/// Where an element sits in the shadow-including tree, as the tree build last saw it: the identity
/// of its shadow-including parent element, or 0, and whether the step to that parent crossed a
/// shadow root, which makes the parent the host of the tree scope the element is in.
#[derive(Clone, Copy, Default)]
pub(crate) struct ShadowIncludingParent {
    pub(crate) element: u32,
    pub(crate) parent_is_a_shadow_root: bool,
}

/// What one step of the shadow-including walk that clears stale layout boxes needs to know about
/// a node: whether its boxes belong to someone else, and where the walk goes next.
#[derive(Clone, Copy)]
pub(crate) struct StaleWalkFacts {
    pub(crate) rendered_in_top_layer: bool,
    pub(crate) shadow_root: Option<StyleNodeID>,
    pub(crate) first_dom_child: Option<StyleNodeID>,
    pub(crate) next_dom_sibling: Option<StyleNodeID>,
}

/// The arena's link to the style engine it mirrors.
///
/// A stage reaches the engine through the arena only while the document thread, which owns both,
/// waits for the stage: the tree build walks the style mirror and pins the records it stamps, and
/// layout and recording look up SVG references and published styles by identity. Those are some
/// thirty-five engine reads and writes, too many to publish into the arena as rows, so the arena
/// carries the engine along as a `&mut StyleEngine` would be carried: the link is `Send` exactly
/// when the engine is.
#[derive(Clone, Copy)]
struct StyleEngineLink(*mut c_void);

// SAFETY: The link stands for an exclusive borrow of the engine, which the compiler checks is
// `Send`. The engine is only reached through it by whoever holds the arena exclusively, while the
// engine's owning thread waits for that stage, and the handoff orders those accesses.
unsafe impl Send for StyleEngineLink where StyleEngine: Send {}

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<StyleEngine>();
};

pub(crate) struct LayoutNodeArena {
    chunks: Vec<Box<Chunk>>,
    chunks_by_address: Vec<ChunkAddress>,
    slot_metadata: Vec<SlotMetadata>,
    style_records: Vec<Cell<u64>>,
    style_records_pinned_by_arena: Vec<Cell<bool>>,
    /// The style record a row's host has pinned for readers that outlive the row's place in the
    /// tree - a detached box is read until its row is freed - or zero for a row with no such pin.
    /// A pin is counted, so this is a pin of its own beside the arena's rather than a share of it,
    /// and it names the record it took rather than whichever record the row holds when it goes.
    style_records_pinned_by_host: Vec<Cell<u64>>,
    /// The StyleNodeID of the element or text node each row is bound to, or of the element it is
    /// generated for. Rows carrying one
    /// identity are chained through `next_rows_with_same_style_node` from
    /// `first_rows_by_style_node`, so retiring an identity reaches rows the element no longer
    /// points at before the identity can be reused.
    style_nodes: Vec<Cell<Option<StyleNodeID>>>,
    next_rows_with_same_style_node: Vec<Cell<NodeSlotId>>,
    first_rows_by_style_node: RefCell<RowsByStyleNode>,
    /// The row each element or text node is bound to: the row its layout node is. Carrying the
    /// identity does not make a row bound, since rows for other referencers, first-letter slices
    /// and rows awaiting a rebuild carry it too.
    bound_rows_by_style_node: RefCell<RowsByStyleNode>,
    /// The principal box each pseudo-element is bound to, keyed by its generator's identity and its
    /// kind. The generated content inside the box carries the same pair but is never bound.
    bound_pseudo_element_rows: RefCell<HashMap<(StyleNodeID, u8), NodeSlotId>>,
    /// The scroll offset each pseudo-element holds, keyed by its generator's identity and its
    /// kind. A pseudo-element has no identity of its own and its box is replaced whenever its
    /// subtree is rebuilt, so the offset is held against the pair that outlives both, and a newly
    /// bound box reads it here instead of asking the DOM cell that used to store it.
    pseudo_element_scroll_offsets: HashMap<(StyleNodeID, u8), FfiCssPixelPoint>,
    /// The scroll offset each element holds, keyed by its identity. The element's own box is
    /// replaced whenever its subtree is rebuilt, so a newly stamped row reads the offset here
    /// rather than off the element. Zero is the absence of an entry, which is nearly every
    /// element; a text identity never publishes one, and so reads zero.
    element_scroll_offsets: HashMap<StyleNodeID, FfiCssPixelPoint>,
    /// The identities sitting in the user agent shadow tree of the focused text control, which is
    /// what a caret and a selection are painted inside. At most one control is focused, so this
    /// holds one control's shadow tree and is empty the rest of the time.
    identities_in_focused_text_control: HashSet<StyleNodeID>,
    /// The rows that own an image provider, for a row whose image comes from its style rather than
    /// from a DOM element. The provider is made for the row and is of no use without it, so the
    /// arena hands it back when the row is freed, rather than leaving it on a shell that the arena
    /// materialises and destroys on its own schedule. The providers are in the host tables.
    rows_with_owned_image_provider: RefCell<HashSet<NodeSlotId>>,
    /// The rows that hold the set of image observers their style asks for. Like the provider a row
    /// owns, the set is made for the row and is of no use without it, so the arena hands it back
    /// when the row is freed. The sets are in the host tables.
    rows_with_image_observers: RefCell<HashSet<NodeSlotId>>,
    /// Where each element sits in the shadow-including tree, as the tree build last saw it, indexed
    /// by the element's dense index. An element's DOM parent only changes when it is inserted or
    /// removed, and either one makes the tree build visit it again, so the fact keeps up with the
    /// tree without being recomputed.
    shadow_including_parent_elements: RefCell<Vec<ShadowIncludingParent>>,
    /// The elements carrying each anchor name, in tree order, keyed by the tree scope the name is
    /// registered in - the identity of its shadow host, or 0 for the document tree - and the name's
    /// interned representation. The document keeps the registry the names go into and republishes a
    /// name's list whenever it changes, so an anchor query during layout reads it instead of asking.
    anchor_name_elements: RefCell<HashMap<(u32, usize), Vec<StyleNodeID>>>,
    /// The viewport row the document is bound to. The document has no identity of its own.
    bound_viewport_row: Cell<NodeSlotId>,
    /// The style engine whose mirror the arena's rows are built from, or null before the document
    /// registers it.
    style_engine: Cell<StyleEngineLink>,
    /// Whether the host listens for box presence. The callback itself is in the host tables,
    /// which only the main thread reaches; this says whether a change is worth handing back.
    host_hears_box_presence: Cell<bool>,
    /// What the arena owes the host and has not handed over yet.
    host_handbacks: RefCell<HostHandbacks>,
    /// How many spans that pay the host, or the tree build's, are open.
    host_handback_spans: Cell<u32>,
    /// Rows whose committed box appeared or went away since the host last heard. A commit changes
    /// them with the arena borrowed for writing, so the host hears about them when it drains this.
    rows_with_changed_committed_box: RefCell<Vec<NodeSlotId>>,
    /// Whether the document is an SVG file decoded as an image, which is fixed for its lifetime.
    document_is_decoded_svg: Cell<bool>,
    /// Depth of synchronous layout passes, including their commits, on the stack.
    active_layout_pass_depth: Cell<u32>,
    /// Whether any fragment-cache epoch changed during the outermost active layout pass. Geometry that the pass laid
    /// out before the change may not match the box's current epoch.
    fragment_cache_epoch_changed_during_layout_pass: Cell<bool>,
    /// The viewport the last layout tree build placed, invalid once that box is freed.
    layout_root: Cell<NodeSlotId>,
    /// The subtree roots the last layout tree build rebuilt, waiting for the partial relayout
    /// plan that follows it. A full layout pass covers every one of them, so its commit clears
    /// them.
    pending_rebuilt_subtree_roots: RefCell<Vec<NodeSlotId>>,
    pending_layout_tree_update_escaped_rebuild_roots: Cell<bool>,
    /// Whether a layout tree build placed content showing the `list-item` counter's value inside a
    /// list owner whose item counters are stale, since the reconciliation after the last build.
    stale_list_item_counter_rendered: Cell<bool>,
    update_layout_running: Cell<bool>,
    /// Every box must be recreated by the next layout tree build; set when the tree is torn down
    /// or a build finds a box it cannot place among rebuilt roots, cleared by the full pass.
    needs_full_layout_tree_update: Cell<bool>,
    partial_layout_count: Cell<u64>,
    full_layout_count: Cell<u64>,
    layout_tree_build_stats: Cell<FfiLayoutTreeBuildStats>,
    pre_order_labels: Vec<Cell<u64>>,
    pre_order_relabel_count: Cell<u64>,
    free_list: Vec<u32>,
    next_index: u32,
    live_count: u32,
    /// The slots whose intrinsic size caches must be dropped before the next layout pass reads
    /// them: freed slots, and rows whose cache epoch wrapped.
    intrinsic_size_caches_to_drop: RefCell<Vec<u32>>,
    table_cell_measurement_cache_misses: Cell<u64>,
    intrinsic_measurements: Cell<u64>,
    intrinsic_inline_measurements: Cell<u64>,
    default_scroll_shift_anchors: RefCell<Vec<DefaultScrollShiftAnchorSlot>>,
    any_default_scroll_shift_anchor_ever_stored: Cell<bool>,
    text_nodes: Vec<TextNodeSlot>,
    pub(super) searchable_text: Option<Vec<super::text_queries::MappedText>>,
    replaced_content_facts: Vec<ReplacedContentFactsSlot>,
    raw_table_column_spans: HashMap<NodeSlotId, u32>,
    /// Rows whose last style attach loaded the `<image>` values its style holds. Only such a row
    /// keeps image observers and cursor style values on its shell, so only such a row has anything
    /// for a later attach to clear. Nearly no row is ever in here.
    style_image_resources_attached: RefCell<HashSet<NodeSlotId>>,
    replaced_paint_facts: RefCell<HashMap<NodeSlotId, crate::painting::replaced_paint_facts::ReplacedPaintFacts>>,
    layer_image_paint_facts:
        RefCell<HashMap<NodeSlotId, Vec<crate::painting::layer_image_paint_facts::LayerImagePaintFactsEntry>>>,
    svg_paint_resources: crate::painting::svg_paint_resources::SvgPaintResources,
    /// The SVG presentation attributes the document published for an element, keyed by the
    /// element's style node rather than by a row. An SVG element that draws nothing itself - the
    /// `<path>` inside a `<defs>` a `<textPath>` follows - has no row at all, and one that is a
    /// mask, a clip or a pattern has a row per referencing element; both fall out of keying the
    /// column by the element rather than by the box.
    svg_attribute_facts: HashMap<StyleNodeID, Box<FfiSvgAttributeFacts>>,
    svg_points: HashMap<StyleNodeID, Arc<[super::svg_formatting_context::FfiFloatPoint]>>,
    /// The counter styles each tree scope registers. The rule cache that settles them is C++'s,
    /// and the document owns the result because a fallback chain is followed from the scope the
    /// counter is used in, not from the scope the style was written in.
    counter_styles: RefCell<crate::css::counter_representation::CounterStyleRegistry>,
    /// The CSS counters set of every element and pseudo-element the tree build resolved one for.
    counters_sets: RefCell<super::counters::CountersSets>,
    /// The generated content of every pseudo-element the tree build gave a box.
    generated_content: RefCell<super::generated_content::GeneratedContent>,
    /// The rows built for one DOM node, chained into a ring through the rows themselves; a row that
    /// is the only one built for its node links to nothing. The chain lives on the rows rather than
    /// under a key so that it survives the node's identity being retired and re-issued.
    next_rows_built_for_same_node: Vec<Cell<NodeSlotId>>,
    fc_run_cache_store: super::fc_run_cache::FcRunCacheArenaStore,
    pub(super) layout_trace: super::trace::LayoutTrace,
    pub(crate) paintable_rows: crate::painting::paintable_rows::PaintableRowStore,
    paint_state: RefCell<crate::painting::paint_state::PaintState>,
    // The list the last recording produced. Each published generation of the rows pins the list
    // and the visual context tree as they were when it was published.
    pub(crate) hit_test_list: RefCell<Option<std::sync::Arc<crate::painting::hit_test::HitTestList>>>,
    // Where the recording stage's workspace waits between runs. It is not a column: nothing
    // outside a running recording may reach it, and it is no part of the committed paint state.
    recording_scratch: crate::painting::record::scratch::RecordingScratchSlot,
    pub(crate) scrollable_overflow: crate::painting::scrollable_overflow::ScrollableOverflowState,
    pub(crate) partial_relayout_boundary_roots: RefCell<Vec<NodeSlotId>>,
    nodes_with_layout_update_flags: RefCell<Vec<NodeSlotId>>,
    layout_update_flag_node_indices: RefCell<HashMap<NodeSlotId, usize>>,
    #[cfg(test)]
    layout_update_flag_ancestor_visits: Cell<u64>,
    pub(super) pending_attached_subtree_roots: RefCell<Vec<NodeSlotId>>,
    /// Boxes whose child lists gained children since the last layout tree build, held back from
    /// layout invalidation until the build shows what the new children are.
    pub(crate) deferred_child_list_insertion_parents: RefCell<Vec<(NodeSlotId, NodeSlotId)>>,
    /// Layout inputs for new absolutely positioned boxes that the build confined to themselves.
    /// They stand in for the committed inputs such a box does not have yet.
    pub(crate) confined_abspos_layout_inputs: RefCell<HashMap<NodeSlotId, AbsposLayoutInputs>>,
    pub(crate) inline_boxes_lifted_out_of: RefCell<HashMap<NodeSlotId, NodeSlotId>>,
    pub(crate) out_of_flow_positioning_contained: RefCell<HashMap<NodeSlotId, u32>>,
    /// Attribution of pending updates for partial relayout. Invariant: every update recorded
    /// since the last layout pass is either attributed to a boundary in the root set above, or
    /// this escape bit is set. Partial relayout may only run while the bit is clear; a full
    /// layout pass re-derives every fact boundary qualification depends on, so it clears the bit.
    pub(crate) pending_updates_escape_partial_relayout: Cell<bool>,
    pub(crate) boxes_needing_scrollable_overflow_recalculation: RefCell<Vec<NodeSlotId>>,
    pub(crate) needs_full_scrollable_overflow_recalculation: Cell<bool>,
    text_nodes_enrolled_for_content_sync: RefCell<HashSet<NodeSlotId>>,
    /// The rows a running tree build owes the host a shell once the build is over, as the shell's
    /// construction tells the host something about the row.
    shells_owed_to_host: RefCell<Vec<NodeSlotId>>,
    /// The image resources the tree builds owe the host for the rows they stamped, in the order the
    /// builds came to owe them, until the frame the builds ran in takes them.
    image_resources_owed_to_host: RefCell<Vec<(NodeSlotId, OwedImageResources)>>,
    /// The image boxes among those rows that own the provider of the image they show. Until the host
    /// hands a box its provider, the box has no image.
    image_boxes_awaiting_owned_provider: RefCell<HashSet<NodeSlotId>>,
    /// The document's style, handed to a build that may build the viewport before it starts, and
    /// pinned until the viewport's row takes it or the build ends without one.
    published_document_style: Cell<Option<DerivedStyleRecord>>,
    nodes_enrolled_for_replaced_content_facts_sync: RefCell<Vec<NodeSlotId>>,
    /// What the running pass has to tell the document, waiting for the commit that delivers it.
    messages_reported_during_pass: RefCell<Vec<super::commit::FfiCommitMessage>>,
    pub(crate) layout_style_snapshots: std::sync::Arc<super::style_snapshot::LayoutStyleSnapshotStore>,
    owner_thread: thread::ThreadId,
}

impl LayoutNodeArena {
    pub(crate) fn new() -> Self {
        Self {
            chunks: Vec::new(),
            chunks_by_address: Vec::new(),
            slot_metadata: Vec::new(),
            style_records: Vec::new(),
            style_records_pinned_by_arena: Vec::new(),
            style_records_pinned_by_host: Vec::new(),
            style_nodes: Vec::new(),
            next_rows_with_same_style_node: Vec::new(),
            first_rows_by_style_node: RefCell::new(RowsByStyleNode::default()),
            bound_rows_by_style_node: RefCell::new(RowsByStyleNode::default()),
            bound_pseudo_element_rows: RefCell::new(HashMap::default()),
            pseudo_element_scroll_offsets: HashMap::default(),
            element_scroll_offsets: HashMap::default(),
            identities_in_focused_text_control: HashSet::default(),
            rows_with_owned_image_provider: RefCell::new(HashSet::default()),
            rows_with_image_observers: RefCell::new(HashSet::default()),
            shadow_including_parent_elements: RefCell::new(Vec::new()),
            anchor_name_elements: RefCell::new(HashMap::default()),
            bound_viewport_row: Cell::new(NodeSlotId::INVALID),
            style_engine: Cell::new(StyleEngineLink(std::ptr::null_mut())),
            host_hears_box_presence: Cell::new(false),
            host_handbacks: RefCell::new(HostHandbacks::default()),
            host_handback_spans: Cell::new(0),
            rows_with_changed_committed_box: RefCell::new(Vec::new()),
            document_is_decoded_svg: Cell::new(false),
            active_layout_pass_depth: Cell::new(0),
            fragment_cache_epoch_changed_during_layout_pass: Cell::new(false),
            layout_root: Cell::new(NodeSlotId::INVALID),
            pending_rebuilt_subtree_roots: RefCell::new(Vec::new()),
            pending_layout_tree_update_escaped_rebuild_roots: Cell::new(false),
            stale_list_item_counter_rendered: Cell::new(false),
            update_layout_running: Cell::new(false),
            needs_full_layout_tree_update: Cell::new(false),
            partial_layout_count: Cell::new(0),
            full_layout_count: Cell::new(0),
            layout_tree_build_stats: Cell::new(FfiLayoutTreeBuildStats::default()),
            pre_order_labels: Vec::new(),
            pre_order_relabel_count: Cell::new(0),
            free_list: Vec::new(),
            next_index: 0,
            live_count: 0,
            intrinsic_size_caches_to_drop: RefCell::new(Vec::new()),
            table_cell_measurement_cache_misses: Cell::new(0),
            intrinsic_measurements: Cell::new(0),
            intrinsic_inline_measurements: Cell::new(0),
            default_scroll_shift_anchors: RefCell::new(Vec::new()),
            any_default_scroll_shift_anchor_ever_stored: Cell::new(false),
            text_nodes: Vec::new(),
            searchable_text: None,
            replaced_content_facts: Vec::new(),
            raw_table_column_spans: HashMap::default(),
            style_image_resources_attached: RefCell::new(HashSet::default()),
            replaced_paint_facts: RefCell::new(HashMap::default()),
            layer_image_paint_facts: RefCell::new(HashMap::default()),
            svg_paint_resources: crate::painting::svg_paint_resources::SvgPaintResources::default(),
            svg_attribute_facts: HashMap::default(),
            svg_points: HashMap::default(),
            counter_styles: RefCell::new(crate::css::counter_representation::CounterStyleRegistry::default()),
            counters_sets: RefCell::new(super::counters::CountersSets::default()),
            generated_content: RefCell::new(super::generated_content::GeneratedContent::default()),
            next_rows_built_for_same_node: Vec::new(),
            fc_run_cache_store: super::fc_run_cache::FcRunCacheArenaStore::default(),
            layout_trace: super::trace::LayoutTrace::default(),
            paintable_rows: crate::painting::paintable_rows::PaintableRowStore::default(),
            paint_state: RefCell::new(crate::painting::paint_state::PaintState::default()),
            hit_test_list: RefCell::new(None),
            recording_scratch: crate::painting::record::scratch::RecordingScratchSlot::default(),
            scrollable_overflow: Default::default(),
            partial_relayout_boundary_roots: RefCell::new(Vec::new()),
            nodes_with_layout_update_flags: RefCell::new(Vec::new()),
            layout_update_flag_node_indices: RefCell::new(HashMap::default()),
            #[cfg(test)]
            layout_update_flag_ancestor_visits: Cell::new(0),
            pending_attached_subtree_roots: RefCell::new(Vec::new()),
            deferred_child_list_insertion_parents: RefCell::new(Vec::new()),
            confined_abspos_layout_inputs: RefCell::new(HashMap::default()),
            inline_boxes_lifted_out_of: RefCell::new(HashMap::default()),
            out_of_flow_positioning_contained: RefCell::new(HashMap::default()),
            pending_updates_escape_partial_relayout: Cell::new(false),
            boxes_needing_scrollable_overflow_recalculation: RefCell::new(Vec::new()),
            needs_full_scrollable_overflow_recalculation: Cell::new(false),
            text_nodes_enrolled_for_content_sync: RefCell::new(HashSet::default()),
            shells_owed_to_host: RefCell::new(Vec::new()),
            image_resources_owed_to_host: RefCell::new(Vec::new()),
            image_boxes_awaiting_owned_provider: RefCell::new(HashSet::default()),
            published_document_style: Cell::new(None),
            nodes_enrolled_for_replaced_content_facts_sync: RefCell::new(Vec::new()),
            messages_reported_during_pass: RefCell::new(Vec::new()),
            layout_style_snapshots: Default::default(),
            owner_thread: thread::current().id(),
        }
    }

    /// Drops one node's cached intrinsic sizes outright, for the wrap of its epoch:
    /// entries are stamped with the epoch they were measured under, so a stamp reused
    /// after a full lap would match a pre-wrap entry.
    pub(crate) fn drop_intrinsic_size_cache(&self, data: &NodeData) {
        let (index, _) = self.slot_for_data(data);
        self.intrinsic_size_caches_to_drop.borrow_mut().push(index);
    }

    /// The slots whose intrinsic size caches the next layout pass must drop before it reads any.
    pub(crate) fn take_intrinsic_size_caches_to_drop(&self) -> Vec<u32> {
        std::mem::take(&mut *self.intrinsic_size_caches_to_drop.borrow_mut())
    }

    pub(crate) fn reset_cached_intrinsic_sizes(&self, node: NodeSlotId) {
        let data = self.data(node);
        let bumped_epoch = data.intrinsic_cache_epoch.get().wrapping_add(1);
        data.intrinsic_cache_epoch.set(bumped_epoch);
        if bumped_epoch == 0 {
            self.drop_intrinsic_size_cache(data);
        }
    }

    pub(crate) fn fc_run_cache_store(&self) -> &super::fc_run_cache::FcRunCacheArenaStore {
        &self.fc_run_cache_store
    }

    pub(crate) fn end_layout_pass(&self) {
        self.sweep_stale_fc_run_cache_entries();
    }

    /// Drops entries whose slot or epoch no longer matches.
    /// Checks only entries invalidated or stored since the previous sweep. Stale entries
    /// survive until commit so inline layout can reuse their undamaged line prefixes.
    fn sweep_stale_fc_run_cache_entries(&self) {
        self.fc_run_cache_store.sweep_pending_entries(|slot, validity| {
            let Some(metadata) = self.slot_metadata.get(slot as usize) else {
                return false;
            };
            if !metadata.occupied || metadata.generation != validity.slot_generation {
                return false;
            }
            let id = NodeSlotId::new(slot, metadata.generation);
            self.data(id).fragment_cache_epoch.get() == validity.fragment_cache_epoch
        });
    }

    pub(crate) fn assert_owner_thread(&self) {
        debug_assert_eq!(self.owner_thread, crate::stage_thread::acting_thread());
    }

    // Freshly created chunks are default-initialized and free() resets slots on release, so
    // allocate() always hands out clean NodeData without writing it again.
    pub(crate) fn allocate(&mut self, construction_facts: FfiNodeConstructionFacts) -> NodeSlotId {
        let slot = self.allocate_unbound();
        self.bind_shell(slot, construction_facts);
        slot
    }

    pub(crate) fn allocate_unbound(&mut self) -> NodeSlotId {
        self.allocate_slot()
    }

    pub(crate) fn bind_shell(&self, slot: NodeSlotId, construction_facts: FfiNodeConstructionFacts) {
        assert!(
            self.slot_is_live(slot),
            "layout node arena bound a shell to a dead slot"
        );
        let data = self.data(slot);
        assert!(
            data.shell.get().is_none(),
            "layout node arena bound a second shell to a slot"
        );
        data.kind.set(construction_facts.kind);
        data.shell.set(ShellId::of_host_object(construction_facts.shell));
        let element_facts = self.element_construction_facts(StyleNodeID::from_raw(construction_facts.style_node));
        data.flags.set(super::node_facts::construction_flags(
            &construction_facts,
            element_facts,
        ));
        data.dom_paint_facts.set(construction_facts.dom_paint_facts);
        self.set_node_style_node(slot, StyleNodeID::from_raw(construction_facts.style_node));
        self.enroll_node_for_replaced_content_facts_sync_if_eligible(slot);
    }

    #[cfg(test)]
    pub(crate) fn allocate_for_test(&mut self) -> NodeAllocation {
        NodeAllocation {
            slot: self.allocate_slot(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_style_node_for_test(&self, slot: NodeSlotId, style_node: Option<StyleNodeID>) {
        self.set_node_style_node(slot, style_node);
    }

    pub(crate) fn enroll_node_for_replaced_content_facts_sync_if_eligible(&self, node: NodeSlotId) {
        let data = self.data(node);
        if !super::node_facts::node_may_have_replaced_content_facts_including_size_containment(data) {
            return;
        }
        let mut enrolled_nodes = self.nodes_enrolled_for_replaced_content_facts_sync.borrow_mut();
        if !enrolled_nodes.contains(&node) {
            enrolled_nodes.push(node);
        }
    }

    fn allocate_slot(&mut self) -> NodeSlotId {
        self.assert_owner_thread();

        let index = if let Some(index) = self.free_list.pop() {
            index
        } else {
            let index = self.next_index;
            assert!(
                index < MAX_NODE_SLOT_COUNT,
                "layout node arena exhausted its 24-bit slot index space"
            );
            if (index as usize).is_multiple_of(SLOTS_PER_CHUNK) {
                let chunk = new_chunk();
                let start = (&raw const chunk.slots) as usize;
                let chunk_index = self.chunks.len();
                let insertion_index = self.chunks_by_address.partition_point(|address| address.start < start);
                self.chunks_by_address
                    .insert(insertion_index, ChunkAddress { start, chunk_index });
                self.chunks.push(chunk);
            }
            self.slot_metadata.push(SlotMetadata::default());
            self.style_records.push(Cell::new(0));
            self.style_records_pinned_by_arena.push(Cell::new(false));
            self.style_records_pinned_by_host.push(Cell::new(0));
            self.style_nodes.push(Cell::new(None));
            self.next_rows_with_same_style_node.push(Cell::new(NodeSlotId::INVALID));
            self.next_rows_built_for_same_node.push(Cell::new(NodeSlotId::INVALID));
            self.pre_order_labels.push(Cell::new(0));
            self.next_index = self
                .next_index
                .checked_add(1)
                .expect("layout node arena exhausted its slot ID space");
            index
        };

        self.live_count = self
            .live_count
            .checked_add(1)
            .expect("layout node arena live count overflowed");

        let metadata = self.metadata_mut(index);
        assert!(!metadata.occupied, "layout node arena allocated a live slot");
        metadata.generation = metadata
            .generation
            .checked_add(1)
            .expect("retired layout node arena slot was reused");
        metadata.occupied = true;
        let generation = metadata.generation;
        self.data_mut(index).slot_generation.set(generation);

        NodeSlotId::new(index, generation)
    }

    pub(crate) fn free_subtree(&mut self, root: NodeSlotId) -> FreedSubtree {
        self.assert_owner_thread();

        assert!(!root.is_invalid(), "invalid layout node arena slot ID");
        self.assert_node_is_unlinked_from_parent(root);
        if self.layout_root.get() == root {
            self.layout_root.set(NodeSlotId::INVALID);
            self.pending_rebuilt_subtree_roots.get_mut().clear();
            self.pending_layout_tree_update_escaped_rebuild_roots.set(false);
        }
        if !self.scrollable_overflow.non_child_boxes.borrow().is_empty() {
            self.scrollable_overflow.contained_boxes_dirty.set(true);
        }
        if self.scrollable_overflow.viewport.get() == Some(root) {
            self.scrollable_overflow.viewport.set(None);
            self.scrollable_overflow.non_child_boxes.borrow_mut().clear();
            self.scrollable_overflow.full_layout_commit.set(false);
            self.scrollable_overflow.geometry_changed.set(false);
            self.scrollable_overflow.scrollability_changed.set(false);
            self.boxes_needing_scrollable_overflow_recalculation
                .borrow_mut()
                .clear();
            self.needs_full_scrollable_overflow_recalculation.set(false);
        }
        let mut slots_in_pre_order = Vec::new();
        self.for_each_node_in_layout_subtree_in_pre_order(root, |slot| slots_in_pre_order.push(slot));

        let mut shells = Vec::with_capacity(slots_in_pre_order.len());
        let mut rows_with_owned_image_provider = Vec::new();
        let mut rows_with_image_observers = Vec::new();
        let mut paintable_row_resets = Vec::new();
        let mut arena_pinned_style_records = Vec::new();
        for slot in slots_in_pre_order {
            shells.extend(self.data(slot).shell.get());
            if self.rows_with_owned_image_provider.get_mut().remove(&slot) {
                rows_with_owned_image_provider.push(slot);
            }
            if self.rows_with_image_observers.get_mut().remove(&slot) {
                rows_with_image_observers.push(slot);
            }
            if self.style_records_pinned_by_arena[slot.slot_index() as usize].get() {
                arena_pinned_style_records.push(self.style_records[slot.slot_index() as usize].get());
            }
            let host_pinned_style_record = self.style_records_pinned_by_host[slot.slot_index() as usize].get();
            if host_pinned_style_record != 0 {
                arena_pinned_style_records.push(host_pinned_style_record);
            }
            self.unlink_children_of_node_being_freed(slot);
            if let Some(reset) = self.free_unlinked_slot(slot) {
                paintable_row_resets.push(reset);
            }
        }
        FreedSubtree {
            shells,
            rows_with_owned_image_provider,
            rows_with_image_observers,
            paintable_row_resets,
            arena_pinned_style_records,
            style_engine: self.style_engine.get().0,
        }
    }

    fn assert_node_is_unlinked_from_parent(&self, id: NodeSlotId) {
        let data = self.data(id);
        assert!(
            data.parent.get().is_invalid()
                && data.previous_sibling.get().is_invalid()
                && data.next_sibling.get().is_invalid(),
            "layout node arena freed a slot that is still linked under a parent"
        );
    }

    fn unlink_children_of_node_being_freed(&self, id: NodeSlotId) {
        let data = self.data(id);
        loop {
            let child = data.first_child.get();
            if child.is_invalid() {
                break;
            }
            self.unlink_child(id, child);
        }
    }

    fn free_unlinked_slot(&mut self, id: NodeSlotId) -> Option<crate::painting::paintable_rows::PaintableRowReset> {
        self.searchable_text = None;
        let index = id.slot_index();
        let id_generation = id.generation();
        let should_reuse = {
            let metadata = self.metadata_mut(index);
            assert!(metadata.occupied, "layout node arena freed an unused slot");
            assert_eq!(
                metadata.generation, id_generation,
                "layout node arena freed a stale slot generation"
            );
            metadata.generation != u8::MAX
        };

        let paintable_row_reset = self.prepare_paintable_row_freed_reset(index);
        if let Some(reset) = paintable_row_reset {
            self.paintable_row_freed(reset);
        }
        self.inline_boxes_lifted_out_of.get_mut().remove(&id);
        self.out_of_flow_positioning_contained.get_mut().remove(&id);
        self.pre_order_labels[index as usize].set(0);
        self.metadata_mut(index).occupied = false;
        self.forget_row_sharing_dom_node(id);
        self.unbind_row(id);
        self.set_node_style_node(id, None);
        self.style_records[index as usize].set(0);
        self.style_records_pinned_by_arena[index as usize].set(false);
        self.style_records_pinned_by_host[index as usize].set(0);

        self.intrinsic_size_caches_to_drop.get_mut().push(index);
        if let Some(slot) = self.default_scroll_shift_anchors.get_mut().get_mut(index as usize) {
            *slot = DefaultScrollShiftAnchorSlot::default();
        }
        self.paintable_rows.reset_committed_fragment_link_slot(index);
        if let Some(slot) = self.text_nodes.get_mut(index as usize) {
            *slot = TextNodeSlot::default();
        }
        self.text_nodes_enrolled_for_content_sync.get_mut().remove(&id);
        if let Some(slot) = self.replaced_content_facts.get_mut(index as usize) {
            *slot = ReplacedContentFactsSlot::default();
        }
        self.fc_run_cache_store.remove_entry(index);
        self.remove_layout_update_flag_node(id);
        self.raw_table_column_spans.remove(&id);
        self.style_image_resources_attached.get_mut().remove(&id);
        self.image_boxes_awaiting_owned_provider.get_mut().remove(&id);
        self.replaced_paint_facts.get_mut().remove(&id);
        self.layer_image_paint_facts.get_mut().remove(&id);
        self.svg_paint_resources.forget_slot(id);
        self.paint_state.get_mut().selection_pseudo_styles.remove(&id);
        let data = self.data_mut(index);
        debug_assert!(
            data.parent.get().is_invalid()
                && data.first_child.get().is_invalid()
                && data.last_child.get().is_invalid()
                && data.previous_sibling.get().is_invalid()
                && data.next_sibling.get().is_invalid(),
            "layout node arena freed a slot that is still linked into a tree"
        );
        *data = NodeData::default();

        self.live_count = self
            .live_count
            .checked_sub(1)
            .expect("layout node arena live count underflowed");
        if should_reuse {
            self.free_list.push(index);
        }
        paintable_row_reset
    }

    pub(crate) fn data(&self, id: NodeSlotId) -> &NodeData {
        assert!(!id.is_invalid(), "invalid layout node arena slot ID");
        let index = id.slot_index() as usize;
        let chunk = self
            .chunks
            .get(index / SLOTS_PER_CHUNK)
            .expect("invalid layout node arena slot ID");
        let data = &chunk.slots[index % SLOTS_PER_CHUNK];
        assert_eq!(
            data.slot_generation.get(),
            id.generation(),
            "layout node arena read a stale or unused slot"
        );
        data
    }

    pub(crate) fn set_node_generated_for(&self, id: NodeSlotId, generated_for: u8, generator: Option<StyleNodeID>) {
        self.assert_owner_thread();
        let data = self.data(id);
        data.generated_for.set(generated_for);
        self.set_node_style_node(id, generator);
    }

    /// Names the DOM node a row is bound to the way a commit message does: by its style node, or
    /// by 0 for the document, which is what the viewport row is bound to. An anonymous row names
    /// no DOM node, and there is nothing to tell the document about one.
    pub(crate) fn commit_message_style_node(&self, id: NodeSlotId) -> Option<u32> {
        if !self.slot_is_live(id) {
            return None;
        }
        match self.node_style_node(id) {
            Some(style_node) => Some(style_node.raw()),
            None if self.data(id).kind.get() == NodeKind::Viewport => Some(0),
            None => None,
        }
    }

    /// Leaves a message about `id` for the document, to be delivered with the pass's commit. A row
    /// that names no DOM node has nothing to tell, and its message is dropped.
    pub(crate) fn report_to_document(&self, id: NodeSlotId, kind: super::commit::FfiCommitMessageKind) {
        let Some(style_node) = self.commit_message_style_node(id) else {
            return;
        };
        self.messages_reported_during_pass
            .borrow_mut()
            .push(super::commit::FfiCommitMessage {
                style_node,
                other_style_node: 0,
                kind,
                pending_face: 0,
                pending_face_has_been_retried: false,
            });
    }

    pub(crate) fn take_messages_reported_during_pass(&self) -> Vec<super::commit::FfiCommitMessage> {
        std::mem::take(&mut *self.messages_reported_during_pass.borrow_mut())
    }

    pub(crate) fn node_style_node(&self, id: NodeSlotId) -> Option<StyleNodeID> {
        if !self.slot_is_live(id) {
            return None;
        }
        self.style_nodes[id.slot_index() as usize].get()
    }

    fn set_node_style_node(&self, id: NodeSlotId, style_node: Option<StyleNodeID>) {
        let index = id.slot_index() as usize;
        let previous = self.style_nodes[index].get();
        if previous == style_node {
            return;
        }
        let generated_for = self.data(id).generated_for.get();
        let mut was_bound = false;
        if let Some(previous) = previous {
            self.with_bound_row_entry_of(BoundNode::of(previous, generated_for), |bound_row| {
                if *bound_row == id {
                    *bound_row = NodeSlotId::INVALID;
                    was_bound = true;
                }
            });
        }
        let mut first_rows = self.first_rows_by_style_node.borrow_mut();
        if let Some(previous) = previous {
            let next = self.next_rows_with_same_style_node[index].replace(NodeSlotId::INVALID);
            let head = first_rows.head_mut(previous);
            if *head == id {
                *head = next;
            } else {
                let mut row = *head;
                loop {
                    let link = &self.next_rows_with_same_style_node[row.slot_index() as usize];
                    if link.get() == id {
                        link.set(next);
                        break;
                    }
                    row = link.get();
                }
            }
        }
        self.style_nodes[index].set(style_node);
        if let Some(style_node) = style_node {
            let head = first_rows.head_mut(style_node);
            self.next_rows_with_same_style_node[index].set(*head);
            *head = id;
            if was_bound {
                self.with_bound_row_entry_of(BoundNode::of(style_node, generated_for), |bound_row| *bound_row = id);
            }
        }
        drop(first_rows);
        if let Some(previous) = previous {
            self.notify_box_presence(BoundNode::of(previous, generated_for));
        }
        if let Some(style_node) = style_node {
            self.notify_box_presence(BoundNode::of(style_node, generated_for));
        }
    }

    /// The row the element or text node with `style_node` is bound to, if any.
    pub(crate) fn bound_row(&self, style_node: StyleNodeID) -> NodeSlotId {
        self.bound_rows_by_style_node.borrow().head(style_node)
    }

    pub(crate) fn bound_viewport_row(&self) -> NodeSlotId {
        self.bound_viewport_row.get()
    }

    /// Everything the host did as it configured a pseudo-element's principal box: what the box is
    /// generated for, the unique node id it answers for, and the binding that makes it the
    /// pseudo-element's box rather than one of its own contents.
    ///
    /// The box was built out of the pseudo-element's own style record and has published it
    /// already, so nothing rebinds the record here.
    pub(crate) fn stamp_pseudo_element_box(&self, slot: NodeSlotId, generator: StyleNodeID, generated_for: u8) {
        self.set_node_generated_for(slot, generated_for, Some(generator));
        // A pseudo-element's box answers for its generator, which the mirror publishes the
        // document's name for. Reading it from the generator's own box would answer nothing for a
        // `display: contents` element, which has no box and still has pseudo-elements.
        let generator_unique_node_id = self.with_style_store(|engine| engine.element_unique_node_id(generator));
        self.unique_node_ids().publish(slot, generator_unique_node_id);

        let previously_bound = self.bound_pseudo_element_row(generator, generated_for);
        if previously_bound != slot {
            self.clear_pseudo_element_box(generator, generated_for);
        }
        self.set_node_flag(slot, NodeFlag::IsPseudoElementPrincipalBox, true);
        self.bind_row(slot);
        // The box becomes the pseudo-element's box here, which is when it starts holding the
        // offset the pseudo-element has been scrolled to.
        let is_the_pseudo_elements_box = self.bound_pseudo_element_row(generator, generated_for) == slot;
        let scroll_offset = if is_the_pseudo_elements_box {
            self.pseudo_element_scroll_offset(generator, generated_for)
        } else {
            FfiCssPixelPoint::default()
        };
        self.set_node_flag(
            slot,
            NodeFlag::HasScrollOffset,
            is_the_pseudo_elements_box && scroll_offset != FfiCssPixelPoint::default(),
        );
        self.scroll_offsets().publish(slot, scroll_offset.into());
    }

    /// Holds a style record for the rest of a layout tree build. The build releases them all
    /// together, since letting go of one it has stopped looking at buys nothing before it ends.
    pub(crate) fn pin_style_record_for_build(&self, record: u64) {
        self.with_style_engine(|engine| engine.pin_layout_style_record(record));
    }

    /// Releases a pin [`Self::pin_style_record_for_build`] took.
    pub(crate) fn release_style_record_pinned_for_build(&self, record: u64) {
        self.with_style_engine(|engine| engine.unpin_layout_style_record(record));
    }

    /// The reasons the node's layout tree update mark permits reusing its box, if any.
    pub(crate) fn layout_tree_update_reuse_reasons(&self, node: StyleNodeID) -> u8 {
        self.with_style_store(|engine| engine.tree().layout_tree_update_reuse_reasons(node))
    }

    /// Retires the tree update marks a node gives up along with its stale box. A shadow root has
    /// no box of its own, so the mark it gives up is its host's as well; only the node's own
    /// child mark goes, as the host may still have other children to update.
    pub(crate) fn retire_layout_tree_update_marks_of_cleared_node(&self, node: StyleNodeID) {
        self.with_style_engine(|engine| {
            let mut current = node;
            while engine.merge_layout_tree_update_mark(current, false, 0) {
                let Some(host) = engine.tree().host_of(current) else {
                    break;
                };
                current = host;
            }
            engine.set_child_needs_layout_tree_update(node, false);
        });
    }

    /// Whether any element holds a box for one of its pseudo-elements, which is what says a walk
    /// over one element's pseudo-element boxes has anything to look at.
    pub(crate) fn has_pseudo_element_boxes(&self) -> bool {
        !self.bound_pseudo_element_rows.borrow().is_empty()
    }

    /// The pseudo-element of kind `generated_for` on the element with `generator` gives up the box
    /// it holds, which is what a build does before it decides whether the pseudo-element gets one.
    pub(crate) fn clear_pseudo_element_box(&self, generator: StyleNodeID, generated_for: u8) {
        let bound = self.bound_pseudo_element_row(generator, generated_for);
        if bound.is_invalid() {
            return;
        }
        // The outgoing box keeps its style readable for as long as the host holds it.
        if super::tree_builder::node_kind_is_node_with_style(self.data(bound).kind.get()) {
            let style_record = self.style_records[bound.slot_index() as usize].get();
            if style_record != 0 {
                self.pin_node_style_record_for_host(bound, style_record);
            }
        }
        self.set_node_flag(bound, NodeFlag::IsPseudoElementPrincipalBox, false);
        self.unbind_row(bound);
    }

    /// The row the pseudo-element of kind `generated_for` on the element with `generator` is bound
    /// to, if any.
    pub(crate) fn bound_pseudo_element_row(&self, generator: StyleNodeID, generated_for: u8) -> NodeSlotId {
        self.bound_pseudo_element_rows
            .borrow()
            .get(&(generator, generated_for))
            .copied()
            .unwrap_or(NodeSlotId::INVALID)
    }

    /// The node a row can be bound to, if any.
    fn bound_node_of(&self, id: NodeSlotId) -> Option<BoundNode> {
        if let Some(style_node) = self.style_nodes[id.slot_index() as usize].get() {
            return Some(BoundNode::of(style_node, self.data(id).generated_for.get()));
        }
        if self.data(id).kind.get() == NodeKind::Viewport {
            return Some(BoundNode::Document);
        }
        None
    }

    /// The node `id` is bound to, when it is that node's row: its style node and the kind of the
    /// pseudo-element it was generated for, or no style node for the document's viewport.
    pub(crate) fn bound_node_name(&self, id: NodeSlotId) -> Option<(Option<StyleNodeID>, u8)> {
        let (bound_row, name) = match self.bound_node_of(id)? {
            BoundNode::Identity(style_node) => (self.bound_row(style_node), (Some(style_node), 0)),
            BoundNode::PseudoElement(generator, generated_for) => (
                self.bound_pseudo_element_row(generator, generated_for),
                (Some(generator), generated_for),
            ),
            BoundNode::Document => (self.bound_viewport_row(), (None, 0)),
        };
        (bound_row == id).then_some(name)
    }

    /// Runs `callback` on the row entry `node` is bound through.
    fn with_bound_row_entry_of<R>(&self, node: BoundNode, callback: impl FnOnce(&mut NodeSlotId) -> R) -> R {
        let style_node = match node {
            BoundNode::Identity(style_node) => style_node,
            BoundNode::PseudoElement(generator, generated_for) => {
                let mut bound_rows = self.bound_pseudo_element_rows.borrow_mut();
                let key = (generator, generated_for);
                let mut bound_row = bound_rows.get(&key).copied().unwrap_or(NodeSlotId::INVALID);
                let result = callback(&mut bound_row);
                if bound_row.is_invalid() {
                    bound_rows.remove(&key);
                } else {
                    bound_rows.insert(key, bound_row);
                }
                return result;
            }
            BoundNode::Document => {
                let mut bound_row = self.bound_viewport_row.get();
                let result = callback(&mut bound_row);
                self.bound_viewport_row.set(bound_row);
                return result;
            }
        };
        callback(self.bound_rows_by_style_node.borrow_mut().head_mut(style_node))
    }

    /// Binds the node `id` belongs to to `id`, replacing any row bound to it before.
    pub(crate) fn bind_row(&self, id: NodeSlotId) {
        self.assert_owner_thread();
        let Some(node) = self.bound_node_of(id) else {
            return;
        };
        self.with_bound_row_entry_of(node, |bound_row| *bound_row = id);
        self.notify_box_presence(node);
    }

    /// What a row the build stamped for a DOM node owes the node's other rows, and the node
    /// itself. The row that was bound joins the ring of rows built for the same node and keeps
    /// its style readable for as long as the host holds it; the stamped row becomes the node's.
    pub(crate) fn take_over_rows_of_bound_node(&self, slot: NodeSlotId) {
        self.assert_owner_thread();
        let Some(node) = self.bound_node_of(slot) else {
            return;
        };
        let previously_bound = self.with_bound_row_entry_of(node, |bound_row| *bound_row);
        if !previously_bound.is_invalid() {
            self.note_rows_share_dom_node(previously_bound, slot);
            // The outgoing box keeps its style readable for as long as the host holds it.
            if super::tree_builder::node_kind_is_node_with_style(self.data(previously_bound).kind.get()) {
                let style_record = self.style_records[previously_bound.slot_index() as usize].get();
                if style_record != 0 {
                    self.pin_node_style_record_for_host(previously_bound, style_record);
                }
            }
        }
        self.bind_row(slot);
    }

    /// Leaves the node `id` is bound to without a bound row.
    pub(crate) fn unbind_row(&self, id: NodeSlotId) {
        self.assert_owner_thread();
        let Some(node) = self.bound_node_of(id) else {
            return;
        };
        self.with_bound_row_entry_of(node, |bound_row| {
            if *bound_row == id {
                *bound_row = NodeSlotId::INVALID;
            }
        });
        self.notify_box_presence(node);
    }

    /// Records a new identity for the element bound to `id`, on every row that shares its DOM node.
    pub(crate) fn set_style_node_of_rows_sharing_dom_node_with(&self, id: NodeSlotId, style_node: Option<StyleNodeID>) {
        self.assert_owner_thread();
        for row in self.rows_sharing_dom_node_with(id) {
            self.set_node_style_node(row, style_node);
        }
    }

    /// Records a new identity for the generator of a pseudo-element box, on the box and the
    /// generated content inside it.
    pub(crate) fn set_style_node_of_generated_subtree(&self, root: NodeSlotId, style_node: Option<StyleNodeID>) {
        self.assert_owner_thread();
        self.for_each_node_in_layout_subtree_in_pre_order(root, |row| {
            if self.data(row).generated_for.get() != 0 && !self.node_is_dom_backed(row) {
                self.set_node_style_node(row, style_node);
            }
        });
    }

    /// Records what the tree build saw of `element`'s place in the shadow-including tree.
    pub(crate) fn set_shadow_including_parent_element(&self, element: StyleNodeID, parent: ShadowIncludingParent) {
        let Some(index) = element.element_index() else {
            return;
        };
        let mut parents = self.shadow_including_parent_elements.borrow_mut();
        let index = index as usize;
        if parents.len() <= index {
            if parent.element == 0 {
                return;
            }
            parents.resize(index + 1, ShadowIncludingParent::default());
        }
        parents[index] = parent;
    }

    fn shadow_including_parent(&self, element: StyleNodeID) -> ShadowIncludingParent {
        let Some(index) = element.element_index() else {
            return ShadowIncludingParent::default();
        };
        self.shadow_including_parent_elements
            .borrow()
            .get(index as usize)
            .copied()
            .unwrap_or_default()
    }

    /// The shadow-including parent element of `element`: the DOM parent, or the shadow host when
    /// the DOM parent is a shadow root. Elements whose layout the tree build never reached, and the
    /// root element, have none.
    pub(crate) fn shadow_including_parent_element(&self, element: StyleNodeID) -> Option<StyleNodeID> {
        StyleNodeID::from_raw(self.shadow_including_parent(element).element)
    }

    /// The host of the shadow root `element` is in, or none when it is in the document tree. The
    /// shadow root itself has no identity, so the tree scope is named by its host.
    pub(crate) fn tree_scope_host(&self, element: StyleNodeID) -> Option<StyleNodeID> {
        let mut ancestor = Some(element);
        while let Some(current) = ancestor {
            let parent = self.shadow_including_parent(current);
            if parent.parent_is_a_shadow_root {
                return StyleNodeID::from_raw(parent.element);
            }
            ancestor = StyleNodeID::from_raw(parent.element);
        }
        None
    }

    /// Replaces the elements registered under `anchor_name` in the tree scope hosted by
    /// `scope_host`, in tree order. An empty list forgets the name.
    pub(crate) fn set_anchor_name_elements(&self, scope_host: u32, anchor_name: usize, elements: &[StyleNodeID]) {
        let mut names = self.anchor_name_elements.borrow_mut();
        if elements.is_empty() {
            names.remove(&(scope_host, anchor_name));
        } else {
            names.insert((scope_host, anchor_name), elements.to_vec());
        }
    }

    /// The last element in tree order registered under `anchor_name` in the tree scope hosted by
    /// `scope_host` that `is_acceptable` accepts.
    pub(crate) fn last_element_with_anchor_name(
        &self,
        scope_host: Option<StyleNodeID>,
        anchor_name: usize,
        mut is_acceptable: impl FnMut(StyleNodeID) -> bool,
    ) -> Option<StyleNodeID> {
        let names = self.anchor_name_elements.borrow();
        let elements = names.get(&(scope_host.map_or(0, StyleNodeID::raw), anchor_name))?;
        elements.iter().rev().copied().find(|&element| is_acceptable(element))
    }

    /// Clears a retired identity from every row still carrying it, including rows of a removed
    /// subtree that outlive the element's disconnection.
    pub(crate) fn forget_style_node(&mut self, style_node: StyleNodeID) {
        self.assert_owner_thread();
        self.set_shadow_including_parent_element(style_node, ShadowIncludingParent::default());
        // A retired shadow host takes its tree scope with it, and the host withdraws no names from
        // a scope it can no longer name.
        self.anchor_name_elements
            .borrow_mut()
            .retain(|&(scope_host, _), _| scope_host != style_node.raw());
        self.counters_sets.borrow_mut().forget(style_node);
        self.generated_content.borrow_mut().forget(style_node);
        self.pseudo_element_scroll_offsets
            .retain(|&(generator, _), _| generator != style_node);
        self.element_scroll_offsets.remove(&style_node);
        self.identities_in_focused_text_control.remove(&style_node);
        loop {
            let row = self.first_rows_by_style_node.borrow().head(style_node);
            if row.is_invalid() {
                return;
            }
            self.set_node_style_node(row, None);
        }
    }

    pub(crate) fn set_node_style(&self, id: NodeSlotId, style_record: u64, payloads: *const c_void) -> bool {
        self.assert_owner_thread();
        let data = self.data(id);
        data.style.set(StylePayloadsRef::new(payloads));
        self.set_node_flag(id, NodeFlag::FollowsPrincipalStyle, false);
        self.invalidate_overflow_after_style_change(id);
        let previous = self.style_records[id.slot_index() as usize].replace(style_record);
        if self.style_records_pinned_by_arena[id.slot_index() as usize].replace(false) {
            self.with_style_engine(|engine| engine.unpin_layout_style_record(previous));
        }
        self.enroll_text_children_for_content_sync(id);
        self.enroll_node_for_replaced_content_facts_sync_if_eligible(id);
        previous != style_record
    }

    pub(crate) fn enroll_node_for_svg_paint_resources_sync(&self, id: NodeSlotId) {
        use crate::painting::svg_paint_resources::SvgPaintResourceKind;
        let Some(style) = self.node_style_if_live(id) else {
            return;
        };
        let effects = style.effects();
        let mut kinds = 0;
        if crate::painting::css_filter::contains_url(&effects.filter) {
            kinds |= SvgPaintResourceKind::Filter.bit();
        }
        if crate::painting::css_filter::contains_url(&effects.backdrop_filter) {
            kinds |= SvgPaintResourceKind::BackdropFilter.bit();
        }
        if crate::painting::node_painting::is_svg_path(self.data(id).kind.get()) {
            let svg = style.inherited_svg();
            if svg.fill.kind == crate::css::computed_value_types::SVG_PAINT_URL {
                kinds |= SvgPaintResourceKind::Fill.bit();
            }
            if svg.stroke.kind == crate::css::computed_value_types::SVG_PAINT_URL {
                kinds |= SvgPaintResourceKind::Stroke.bit();
            }
        }
        self.svg_paint_resources.set_enrolled_kinds(id, kinds);
    }

    pub(crate) fn svg_paint_resources(&self) -> &crate::painting::svg_paint_resources::SvgPaintResources {
        &self.svg_paint_resources
    }

    pub(crate) fn node_style_record(&self, id: NodeSlotId) -> u64 {
        assert!(
            self.slot_is_live(id),
            "layout node arena read the style record of a dead slot"
        );
        self.style_records[id.slot_index() as usize].get()
    }

    pub(crate) fn node_style_record_is_pinned_by_arena(&self, id: NodeSlotId) -> bool {
        assert!(
            self.slot_is_live(id),
            "layout node arena read the style pin of a dead slot"
        );
        self.style_records_pinned_by_arena[id.slot_index() as usize].get()
    }

    /// Pin `record` for the host's readers of `slot`. A row holds at most one such pin; asking
    /// again while one is held keeps the one it has, as the shell flag this replaces did.
    pub(crate) fn pin_node_style_record_for_host(&self, id: NodeSlotId, record: u64) {
        self.assert_owner_thread();
        assert!(record != 0, "a row pinned a null style record for its host");
        if self.style_records_pinned_by_host[id.slot_index() as usize].get() != 0 {
            return;
        }
        self.style_records_pinned_by_host[id.slot_index() as usize].set(record);
        self.with_style_engine(|engine| engine.pin_layout_style_record(record));
    }

    /// Release the pin the host holds on `slot`'s style record, if it holds one.
    pub(crate) fn release_node_style_record_pin_for_host(&self, id: NodeSlotId) {
        self.assert_owner_thread();
        let record = self.style_records_pinned_by_host[id.slot_index() as usize].replace(0);
        if record == 0 {
            return;
        }
        self.with_style_engine(|engine| engine.unpin_layout_style_record(record));
    }

    /// The style record the host has pinned for `slot`, or zero.
    pub(crate) fn node_style_record_pinned_by_host(&self, id: NodeSlotId) -> u64 {
        self.style_records_pinned_by_host[id.slot_index() as usize].get()
    }

    pub(crate) fn set_style_engine(&self, style_engine: *mut c_void) {
        self.style_engine.set(StyleEngineLink(style_engine));
        if !style_engine.is_null() {
            // SAFETY: The registered style engine outlives this arena's live nodes.
            unsafe { &mut *style_engine.cast::<StyleEngine>() }
                .install_layout_style_snapshots(self.layout_style_snapshots.clone());
        }
    }

    pub(crate) fn set_document_is_decoded_svg(&self, is_decoded_svg: bool) {
        self.document_is_decoded_svg.set(is_decoded_svg);
    }

    pub(crate) fn document_is_decoded_svg(&self) -> bool {
        self.document_is_decoded_svg.get()
    }

    /// True while a synchronous layout pass, including its commit, is on the stack. Computed
    /// values must never be replaced in that window: the pass caches decoded style and borrows
    /// payload pointers that a replacement would invalidate under it.
    pub(crate) fn layout_pass_is_running(&self) -> bool {
        self.active_layout_pass_depth.get() > 0
    }

    /// Runs `stage` as a render stage: on the stage thread under `LIBWEB_STAGE_THREAD=lockstep`, here
    /// otherwise. The stage has the arena to itself while this thread waits for it.
    pub(crate) fn run_stage<R: Send>(&mut self, stage: impl FnOnce(&mut Self) -> R + Send) -> R {
        crate::stage_thread::run_stage(move || stage(self))
    }

    /// True while a render stage is on the stack: a layout pass, a layout tree build, or a paint
    /// pass. A host call made in that window is part of the stage, not a main-side read between
    /// stages.
    pub(crate) fn a_stage_is_running(&self) -> bool {
        self.layout_pass_is_running()
            || super::tree_build_seal::build_is_running()
            || crate::painting::seal::current_pass_name().is_some()
    }

    /// The writer the main side's scroll offset writes are attributed to.
    pub(crate) const SCROLL_OFFSETS_WRITER: &str = "scroll offsets";
    /// The writer the main side's selection state writes are attributed to.
    pub(crate) const SELECTION_WRITER: &str = "selection state";
    /// The writer the rendering update's compositor animation choices are attributed to.
    pub(crate) const COMPOSITOR_ELIGIBILITY_WRITER: &str = "compositor animation eligibility";
    /// The writer a DOM tree mutation's writes to the style mirror and the arena are attributed to.
    pub(crate) const DOM_TREE_MUTATION_WRITER: &str = "DOM tree mutation";

    /// The one door a main-side writer of render-owned state goes through. A write joins the
    /// frame in flight and lands after it, the way a main-side read of render state waits for it:
    /// a frame never sees half of a write, and nothing writes under a frame that is reading. The
    /// wait happens as the writer borrows the arena ([`Self::from_handle`] joins an overlapping
    /// stage), and the frame's last join has ended the update by the time it returns. What is left
    /// to check here is where waiting is impossible: a writer inside the frame itself, in a join
    /// or with the stages in lockstep, must not write while a stage is on the stack. Every write
    /// made inside the returned scope is attributed to `writer`.
    #[track_caller]
    pub(crate) fn join_frame_for_main_side_write(
        &self,
        writer: &'static str,
    ) -> crate::painting::published_immutable::WriterScope {
        assert!(
            !self.a_stage_is_running(),
            "{writer} were written by the main side while a render stage was running"
        );
        // FIXME: The wait for an overlapping stage happens as the writer borrows the arena, before the door is
        //        counted. Once the door itself waits for the frame in flight, pass the wait here.
        record_door_pass(writer, None);
        crate::painting::published_immutable::enter_writer(writer)
    }

    pub(crate) fn begin_active_layout_pass(&self) {
        let depth = self.active_layout_pass_depth.get();
        if depth == 0 {
            self.fragment_cache_epoch_changed_during_layout_pass.set(false);
        }
        self.active_layout_pass_depth.set(depth + 1);
    }

    pub(crate) fn end_active_layout_pass(&self, main_thread: &crate::stage::MainThread) {
        let depth = self.active_layout_pass_depth.get();
        assert!(depth > 0, "layout pass depth underflow");
        self.active_layout_pass_depth.set(depth - 1);
        if depth == 1 {
            self.layout_trace.name_owners(main_thread, self);
        }
    }

    /// Ends a pass whose host half the frame carries past the next pass. Its trace owners are
    /// named with those of the pass that ends the frame's run of passes, whose host half names
    /// them on the document thread.
    pub(crate) fn end_active_layout_pass_ahead_of_host(&self) {
        let depth = self.active_layout_pass_depth.get();
        assert!(depth > 0, "layout pass depth underflow");
        self.active_layout_pass_depth.set(depth - 1);
    }

    pub(crate) fn set_layout_root(&self, viewport: NodeSlotId) {
        self.layout_root.set(viewport);
    }

    pub(crate) fn layout_root(&self) -> NodeSlotId {
        self.layout_root.get()
    }

    /// Whether the tree the arena holds already reflects every pending update. A document without
    /// a layout root needs one built, so it is never up to date; the DOM-side half of the answer
    /// comes in as `document_needs_layout_tree_build`.
    pub(crate) fn layout_is_up_to_date(&self, document_needs_layout_tree_build: bool) -> bool {
        let layout_root = self.layout_root();
        if layout_root.is_invalid() {
            return false;
        }
        !self.node_needs_layout_update(layout_root)
            && !document_needs_layout_tree_build
            && !self.needs_full_layout_tree_update()
            && !self.has_partial_relayout_boundary_roots()
    }

    pub(crate) fn set_pending_rebuilt_subtree_roots(
        &self,
        roots: Vec<NodeSlotId>,
        layout_tree_update_escaped_rebuild_roots: bool,
    ) {
        *self.pending_rebuilt_subtree_roots.borrow_mut() = roots;
        self.pending_layout_tree_update_escaped_rebuild_roots
            .set(layout_tree_update_escaped_rebuild_roots);
    }

    pub(crate) fn take_pending_rebuilt_subtree_roots(&self) -> (Vec<NodeSlotId>, bool) {
        (
            std::mem::take(&mut *self.pending_rebuilt_subtree_roots.borrow_mut()),
            self.pending_layout_tree_update_escaped_rebuild_roots.replace(false),
        )
    }

    pub(crate) fn clear_pending_rebuilt_subtree_roots(&self) {
        self.pending_rebuilt_subtree_roots.borrow_mut().clear();
        self.pending_layout_tree_update_escaped_rebuild_roots.set(false);
    }

    /// Records whether the list owner's items were renumbered without its layout tree being
    /// rebuilt, which leaves the counters its build resolved stale.
    pub(crate) fn set_list_owner_has_stale_item_counters(&self, list_owner: StyleNodeID, value: bool) {
        self.with_style_engine(|engine| engine.set_list_owner_has_stale_item_counters(list_owner, value));
    }

    /// Content generated for `element` shows the `list-item` counter's value. Inside a list owner
    /// whose item counters are stale, that value is stale as well.
    pub(crate) fn note_list_item_counter_value_rendered(&self, element: StyleNodeID) {
        if self.stale_list_item_counter_rendered.get() {
            return;
        }
        let renders_stale_value = self.with_style_store(|engine| {
            let tree = engine.tree();
            let list_owners = tree.list_owners_with_stale_item_counters();
            if list_owners.is_empty() {
                return false;
            }
            // The element and its ancestor elements, up to the root of its tree.
            let mut candidate = element;
            loop {
                if list_owners.contains(&candidate) {
                    return true;
                }
                match tree.parent(candidate) {
                    Some(parent) if tree.host_of(parent).is_none() && !tree.is_relation_only(parent) => {
                        candidate = parent;
                    }
                    _ => return false,
                }
            }
        });
        if renders_stale_value {
            self.stale_list_item_counter_rendered.set(true);
        }
    }

    /// Settles the stale list owners after a layout tree build of the document `document` names,
    /// and answers with the ones whose layout trees have to be built again. A rebuilt subtree has
    /// re-resolved the counters of every stale owner inside it, and an owner that has left the
    /// document renders nothing. When the build showed a stale counter value, every owner left is
    /// rebuilt, and none of them stays stale.
    pub(crate) fn reconcile_stale_list_item_counters_after_tree_build(
        &self,
        document: StyleNodeID,
    ) -> Vec<StyleNodeID> {
        let stale_value_rendered = self.stale_list_item_counter_rendered.replace(false);
        if self.with_style_store(|engine| engine.tree().list_owners_with_stale_item_counters().is_empty()) {
            return Vec::new();
        }
        let layout_root = self.layout_root();
        let rebuilt_roots: Vec<StyleNodeID> = self
            .pending_rebuilt_subtree_roots
            .borrow()
            .iter()
            .filter(|&&root| self.node_is_dom_backed(root))
            .filter_map(|&root| match root == layout_root {
                true => Some(document),
                false => self.node_style_node(root),
            })
            .collect();
        self.with_style_engine(|engine| {
            engine.forget_list_owners_with_stale_item_counters(|tree, list_owner| {
                !tree.is_live(list_owner)
                    || rebuilt_roots
                        .iter()
                        .any(|&root| tree.is_in_dom_subtree_of(list_owner, root, document))
            });
            if !stale_value_rendered {
                return Vec::new();
            }
            let list_owners: Vec<StyleNodeID> = engine
                .tree()
                .list_owners_with_stale_item_counters()
                .iter()
                .copied()
                .collect();
            engine.forget_list_owners_with_stale_item_counters(|_, _| true);
            list_owners
        })
    }

    /// A document runs one layout update at a time; a nested request is a caller bug.
    pub(crate) fn begin_update_layout(&self) {
        assert!(
            !self.update_layout_running.replace(true),
            "a layout update is already running"
        );
    }

    pub(crate) fn end_update_layout(&self) {
        assert!(self.update_layout_running.replace(false), "no layout update is running");
    }

    pub(crate) fn update_layout_is_running(&self) -> bool {
        self.update_layout_running.get()
    }

    pub(crate) fn note_partial_layout(&self) {
        self.partial_layout_count.set(self.partial_layout_count.get() + 1);
    }

    pub(crate) fn note_full_layout(&self) {
        self.full_layout_count.set(self.full_layout_count.get() + 1);
    }

    pub(crate) fn partial_layout_count(&self) -> u64 {
        self.partial_layout_count.get()
    }

    pub(crate) fn full_layout_count(&self) -> u64 {
        self.full_layout_count.get()
    }

    pub(crate) fn record_layout_tree_build(&self, outcome: &FfiLayoutTreeBuildOutcome) {
        let stats = self.layout_tree_build_stats.get();
        self.layout_tree_build_stats.set(FfiLayoutTreeBuildStats {
            builds: stats.builds + 1,
            last_build_rebuilt_subtree_roots: outcome.rebuilt_subtree_root_count as u64,
            last_build_escaped_rebuild_roots: outcome.layout_tree_update_escaped_rebuild_roots,
        });
    }

    pub(crate) fn layout_tree_build_stats(&self) -> FfiLayoutTreeBuildStats {
        self.layout_tree_build_stats.get()
    }

    pub(crate) fn needs_full_layout_tree_update(&self) -> bool {
        self.needs_full_layout_tree_update.get()
    }

    pub(crate) fn set_needs_full_layout_tree_update(&self, value: bool) {
        self.needs_full_layout_tree_update.set(value);
    }

    fn style_engine(&self) -> *mut c_void {
        let style_engine = self.style_engine.get().0;
        assert!(!style_engine.is_null(), "layout node arena has no style record host");
        style_engine
    }

    // The engine outlives the arena's live nodes. No host callback runs while this
    // native style-store borrow is active; shell notifications follow publication.
    /// Borrows the style store for one read-only query. Nothing the query calls may reach back
    /// into the arena for another style-store read: this borrow stands for the whole query.
    pub(crate) fn with_style_store<T>(&self, query: impl FnOnce(&StyleEngine) -> T) -> T {
        let style_engine = self.style_engine();
        // SAFETY: As with `with_style_engine`, the engine outlives the arena's live nodes and no
        // host callback runs while the borrow is active.
        unsafe { query(&*style_engine.cast::<StyleEngine>()) }
    }

    /// The committed content box of the element `style_node` names changed along an axis its
    /// container type queries; the style engine records the dependents that change moves.
    pub(crate) fn record_size_container_content_size_change(&self, style_node: u32) {
        if let Some(style_node) = StyleNodeID::from_raw(style_node) {
            self.with_style_engine(|engine| engine.size_container_content_size_changed(style_node));
        }
    }

    /// A full layout has committed a box for every container that had none when a style
    /// computation asked about it; the style engine records the dependents of each.
    pub(crate) fn evaluate_size_containers_needing_evaluation_after_layout(&self) {
        self.with_style_engine(|engine| engine.evaluate_size_containers_needing_evaluation_after_layout());
    }

    fn with_style_engine<T>(&self, callback: impl FnOnce(&mut StyleEngine) -> T) -> T {
        let style_engine = self.style_engine();
        unsafe { callback(&mut *style_engine.cast::<StyleEngine>()) }
    }

    /// Everything one step of the stale-subtree walk reads out of the style mirror, taken in one
    /// borrow. The walk steps through every node below a rebuilt subtree, and asking the mirror
    /// four separate questions per node was four separate borrows of the style engine.
    pub(crate) fn stale_walk_facts(&self, style_node: StyleNodeID) -> StaleWalkFacts {
        self.with_style_store(|engine| {
            let tree = engine.tree();
            let owns_children = style_node.element_index().is_some();
            StaleWalkFacts {
                rendered_in_top_layer: owns_children
                    && engine.element_adjustment_facts(style_node)
                        & crate::css::style::bridge::element_adjustment_fact::RENDERED_IN_TOP_LAYER
                        != 0,
                shadow_root: owns_children.then(|| tree.shadow_root_of(style_node)).flatten(),
                first_dom_child: owns_children.then(|| tree.dom_children(style_node).next()).flatten(),
                next_dom_sibling: tree.next_sibling_in_dom_order(style_node),
            }
        })
    }

    /// The first child the style mirror's DOM child sequence holds for `style_node`, text nodes
    /// included. Only an element, a shadow root and the document own a sequence; a text node owns
    /// none. Nodes that can never have a box - a comment, a doctype, a processing instruction -
    /// hold no place in it, so a parent whose children are all of those answers nothing.
    pub(crate) fn first_dom_child(&self, style_node: Option<StyleNodeID>) -> Option<StyleNodeID> {
        let style_node = style_node?;
        style_node.element_index()?;
        self.with_style_engine(|engine| engine.tree().dom_children(style_node).next())
    }

    /// The next node after `style_node` in its parent's DOM child sequence.
    pub(crate) fn next_dom_sibling(&self, style_node: StyleNodeID) -> Option<StyleNodeID> {
        self.with_style_engine(|engine| engine.tree().next_sibling_in_dom_order(style_node))
    }

    /// The shadow root attached to the host `style_node` names, or nothing when it hosts none. A
    /// root is named the moment it is attached, so a host that has one is never answered for with
    /// nothing.
    pub(crate) fn shadow_root_of(&self, style_node: Option<StyleNodeID>) -> Option<StyleNodeID> {
        let style_node = style_node?;
        style_node.element_index()?;
        self.with_style_engine(|engine| engine.tree().shadow_root_of(style_node))
    }

    /// Whether the style mirror's DOM child sequence holds a child for `style_node`.
    pub(crate) fn has_dom_children(&self, style_node: Option<StyleNodeID>) -> bool {
        self.first_dom_child(style_node).is_some()
    }

    /// Whether the node has an element child, which is what `childElementCount` counts.
    pub(crate) fn has_dom_element_children(&self, style_node: Option<StyleNodeID>) -> bool {
        let Some(style_node) = style_node else {
            return false;
        };
        if style_node.element_index().is_none() {
            return false;
        }
        self.with_style_engine(|engine| engine.tree().first_element_child(style_node).is_some())
    }

    /// How many nodes the style mirror holds assigned to the slot `style_node` names. Anything that
    /// is not a slot with assigned nodes answers zero, which is also what a slot rendering its
    /// fallback content answers.
    pub(crate) fn assigned_node_count(&self, style_node: Option<StyleNodeID>) -> usize {
        let Some(style_node) = style_node else {
            return 0;
        };
        if style_node.element_index().is_none() {
            return 0;
        }
        self.with_style_engine(|engine| engine.tree().assigned_nodes_of(style_node).len())
    }

    /// The node assigned to the slot `style_node` names at `index`, in flat-tree order.
    pub(crate) fn assigned_node_at(&self, style_node: StyleNodeID, index: usize) -> StyleNodeID {
        self.with_style_engine(|engine| engine.tree().assigned_nodes_of(style_node)[index])
    }

    /// The document's top layer, in the order its members were added.
    pub(crate) fn top_layer_elements(&self) -> Vec<StyleNodeID> {
        self.with_style_engine(|engine| engine.top_layer_elements().to_vec())
    }

    /// The node above `style_node` in the flat tree, or nothing for a root and for a node the flat
    /// tree leaves out - a shadow host's unassigned light child, or a slot's fallback content while
    /// the slot has assigned nodes.
    pub(crate) fn flat_tree_parent(&self, style_node: Option<StyleNodeID>) -> Option<StyleNodeID> {
        let style_node = style_node?;
        style_node.element_index()?;
        self.with_style_engine(|engine| engine.tree().flat_tree_parent(style_node))
    }

    /// Retire the layout tree update marks `style_node` holds, own and child alike.
    pub(crate) fn clear_layout_tree_update_marks(&self, style_node: Option<StyleNodeID>) {
        let Some(style_node) = style_node else {
            return;
        };
        self.with_style_engine(|engine| engine.clear_layout_tree_update_marks(style_node));
    }

    /// Whether the style mirror holds a layout tree update mark on `style_node` itself. An
    /// anonymous row names no node and answers no.
    pub(crate) fn needs_layout_tree_update(&self, style_node: Option<StyleNodeID>) -> bool {
        let Some(style_node) = style_node else {
            return false;
        };
        self.with_style_engine(|engine| engine.needs_layout_tree_update(style_node))
    }

    /// The element above `element` in the shadow-including tree, and whether the step to it crossed
    /// a shadow root. The document is no element, so the document element answers none. Layout
    /// walks this ancestry to find the inline that establishes an absolutely positioned box's
    /// containing block, and the tree scope an anchor name resolves in.
    pub(crate) fn published_shadow_including_parent(&self, element: StyleNodeID) -> ShadowIncludingParent {
        self.with_style_engine(|engine| {
            let tree = engine.tree();
            let Some(parent) = tree.parent(element) else {
                return ShadowIncludingParent::default();
            };
            if let Some(host) = tree.host_of(parent) {
                return ShadowIncludingParent {
                    element: host.raw(),
                    parent_is_a_shadow_root: true,
                };
            }
            // The document stands in the tree without being styled, and is no element.
            if tree.is_relation_only(parent) {
                return ShadowIncludingParent::default();
            }
            ShadowIncludingParent {
                element: parent.raw(),
                parent_is_a_shadow_root: false,
            }
        })
    }

    /// Whether the element's published style record replaces its contents with a single image,
    /// which is what makes its box a replaced box rather than a container for its children.
    pub(crate) fn published_content_is_single_image(&self, style_node: Option<StyleNodeID>) -> bool {
        let Some(style_node) = style_node else {
            return false;
        };
        style_node.element_index().is_some()
            && self.with_style_engine(|engine| engine.element_content_is_single_image(style_node))
    }

    /// What the element above the text node in the flat tree publishes, in one borrow.
    pub(crate) fn text_style_parent_facts(&self, style_node: Option<StyleNodeID>) -> TextStyleParentFacts {
        let Some(style_node) = style_node else {
            return TextStyleParentFacts::default();
        };
        self.with_style_engine(|engine| engine.text_style_parent_facts(style_node))
    }

    /// Whether the style mirror holds a layout tree update mark on a flat-tree descendant of
    /// `style_node`. An anonymous row names no node and answers no.
    pub(crate) fn child_needs_layout_tree_update(&self, style_node: Option<StyleNodeID>) -> bool {
        let Some(style_node) = style_node else {
            return false;
        };
        self.with_style_engine(|engine| engine.child_needs_layout_tree_update(style_node))
    }

    /// The element type facts the style store holds for `style_node`. A text node, an anonymous
    /// row and the document hold none, and answer zero.
    pub(crate) fn element_adjustment_facts(&self, style_node: Option<StyleNodeID>) -> u32 {
        match style_node {
            Some(style_node) if style_node.element_index().is_some() => {
                self.with_style_engine(|engine| engine.element_adjustment_facts(style_node))
            }
            _ => 0,
        }
    }

    /// The facts a row built for `style_node` records, as `element_construction_fact` names them.
    /// A text node holds only which kind of tree it sits in; an anonymous row and the document
    /// hold none, and answer zero, as does a row built in an arena that names no style mirror.
    pub(crate) fn element_construction_facts(&self, style_node: Option<StyleNodeID>) -> u32 {
        match style_node {
            Some(style_node) => {
                let style_engine = self.style_engine.get().0;
                if style_engine.is_null() {
                    return 0;
                }
                // SAFETY: As with `with_style_engine`, the engine outlives the arena's live nodes
                // and no host callback runs while the borrow is active.
                unsafe { &*style_engine.cast::<StyleEngine>() }.element_construction_facts(style_node)
            }
            _ => 0,
        }
    }

    /// Which principal box the element asks for, before its computed style has a say. A text
    /// node, an anonymous row and an arena that names no style mirror ask for nothing in
    /// particular, which is what the computed display alone decides.
    pub(crate) fn element_box_kind(&self, style_node: Option<StyleNodeID>) -> ElementBoxKind {
        match style_node {
            Some(style_node) if style_node.element_index().is_some() => {
                let style_engine = self.style_engine.get().0;
                if style_engine.is_null() {
                    return ElementBoxKind::FromDisplay;
                }
                // SAFETY: As with `with_style_engine`, the engine outlives the arena's live nodes
                // and no host callback runs while the borrow is active.
                ElementBoxKind::from_raw(unsafe { &*style_engine.cast::<StyleEngine>() }.element_box_kind(style_node))
            }
            _ => ElementBoxKind::FromDisplay,
        }
    }

    /// A bit per pseudo-element kind the element's published style holds a record for, which is
    /// what says each of them exists at all. The style store settles no record for `::backdrop` or
    /// a highlight pseudo-element, so neither can be asked about this way.
    pub(crate) fn published_pseudo_record_mask(&self, style_node: Option<StyleNodeID>) -> u32 {
        match style_node {
            Some(style_node) if style_node.element_index().is_some() => {
                self.with_style_engine(|engine| engine.published_pseudo_record_mask(style_node))
            }
            _ => 0,
        }
    }

    /// Whether the element's published style holds a `::first-letter`. A text node, an
    /// anonymous row that names no element and the document have no record and answer no.
    pub(crate) fn has_published_first_letter_style(&self, style_node: Option<StyleNodeID>) -> bool {
        match style_node {
            Some(style_node) if style_node.element_index().is_some() => {
                self.with_style_engine(|engine| engine.has_published_first_letter_style(style_node))
            }
            _ => false,
        }
    }

    /// Whether the text node's data is nothing but ASCII whitespace. Anything that is not a text
    /// node has no data and answers no.
    pub(crate) fn text_is_ascii_whitespace(&self, style_node: Option<StyleNodeID>) -> bool {
        match style_node {
            Some(style_node) if style_node.is_text() => {
                self.with_style_engine(|engine| engine.text_is_ascii_whitespace(style_node))
            }
            _ => false,
        }
    }

    pub(crate) fn published_box_facts(&self, style_node: Option<StyleNodeID>) -> Option<PublishedBoxFacts> {
        let style_node = style_node?;
        style_node.element_index()?;
        self.with_style_engine(|engine| engine.element_published_box_facts(style_node))
    }

    pub(crate) fn derive_anonymous_style_record(
        &self,
        parent: u64,
        kind: AnonymousStyleKind,
        overrides: AnonymousStyleOverrides,
    ) -> DerivedStyleRecord {
        self.with_style_engine(|engine| LayoutStyle::anonymous(engine, parent, kind, overrides).intern(engine))
    }

    pub(crate) fn reinherit_anonymous_style_record(&self, record: u64, parent: u64) -> DerivedStyleRecord {
        self.with_style_engine(|engine| {
            let mut style = LayoutStyle::from_record(engine, record);
            style.inherit_from(engine, parent);
            style.intern(engine)
        })
    }

    pub(crate) fn update_layout_style(
        &self,
        node: NodeSlotId,
        notice: ShellStyleChangeNotice<'_>,
        update: impl FnOnce(&mut LayoutStyle),
    ) {
        let derived = self.with_style_engine(|engine| {
            let mut style = LayoutStyle::from_record(engine, self.node_style_record(node));
            update(&mut style);
            if style.is_unchanged() {
                return None;
            }
            Some(style.intern(engine))
        });
        if let Some(derived) = derived {
            self.set_node_flag(node, NodeFlag::FollowsPrincipalStyle, false);
            self.apply_reinherited_style_record(node, derived, notice);
        }
    }

    pub(crate) fn reset_table_box_style_used_by_wrapper(&self, node: NodeSlotId, notice: ShellStyleChangeNotice<'_>) {
        self.update_layout_style(node, notice, LayoutStyle::reset_table_properties);
    }

    pub(crate) fn reinherit_anonymous_descendants(&self, node: NodeSlotId, notice: ShellStyleChangeNotice<'_>) {
        self.assert_owner_thread();
        if self.node_style_record(node) == 0 {
            return;
        }
        let parent = self.data(node).parent.get();
        let parent_is_table_wrapper_of_this_table_box = !parent.is_invalid()
            && self.data(parent).kind.get() == NodeKind::TableWrapper
            && self
                .style_payloads(node)
                .is_some_and(|payloads| ComputedValuesView::new(&payloads.groups).display().is_table_inside());
        if parent_is_table_wrapper_of_this_table_box {
            let derived = self.derive_anonymous_style_record(
                self.node_style_record(node),
                AnonymousStyleKind::TableWrapper,
                AnonymousStyleOverrides::default(),
            );
            self.apply_reinherited_style_record(parent, derived, notice);
            self.reset_table_box_style_used_by_wrapper(node, notice);
        }
        self.reinherit_anonymous_children(node, self.node_style_record(node), notice);
    }

    fn reinherit_anonymous_children(
        &self,
        parent: NodeSlotId,
        parent_style_record: u64,
        notice: ShellStyleChangeNotice<'_>,
    ) {
        let mut child = self.data(parent).first_child.get();
        while !child.is_invalid() {
            let next_sibling = self.data(child).next_sibling.get();
            let data = self.data(child);
            let flags = data.flags.get();
            let is_anonymous_styled_child = flags & NodeFlag::Anonymous as u32 != 0
                && flags & NodeFlag::HasStyle as u32 != 0
                && data.kind.get() != NodeKind::TableWrapper;
            if is_anonymous_styled_child && flags & NodeFlag::IsPseudoElementPrincipalBox as u32 == 0 {
                // Generated content with no layout-derived overrides follows its principal
                // pseudo's complete record. Anonymous wrappers inherit only inherited groups. The marker of a
                // list-item pseudo is generated for that pseudo but carries its own ::marker record.
                let follows_principal = data.generated_for.get() != 0
                    && data.kind.get() != NodeKind::ListItemMarkerBox
                    && (!self.node_style_record_is_pinned_by_arena(child)
                        || flags & NodeFlag::FollowsPrincipalStyle as u32 != 0)
                    && self.data(parent).flags.get() & NodeFlag::IsPseudoElementPrincipalBox as u32 != 0
                    && self.data(parent).generated_for.get() == data.generated_for.get();
                if follows_principal {
                    let derived = self.with_style_engine(|engine| {
                        engine.pin_layout_style_record(parent_style_record);
                        DerivedStyleRecord {
                            record: parent_style_record,
                            payloads: StylePayloadsRef::new(
                                engine
                                    .style_record_payloads(parent_style_record)
                                    .unwrap()
                                    .as_ptr()
                                    .cast(),
                            ),
                        }
                    });
                    self.apply_reinherited_style_record(child, derived, notice);
                    self.set_node_flag(child, NodeFlag::FollowsPrincipalStyle, true);
                    self.reinherit_anonymous_descendants(child, notice);
                    self.notify_shell_of_style_change(child, true, notice);
                } else {
                    let derived =
                        self.reinherit_anonymous_style_record(self.node_style_record(child), parent_style_record);
                    self.apply_reinherited_style_record(child, derived, notice);
                    self.reinherit_anonymous_children(child, derived.record, notice);
                }
            }
            child = next_sibling;
        }
    }

    fn apply_reinherited_style_record(
        &self,
        slot: NodeSlotId,
        derived: DerivedStyleRecord,
        notice: ShellStyleChangeNotice<'_>,
    ) {
        let previous_payloads = self.data(slot).style.get();
        let changes_layout_affecting_style =
            !style_payloads_equal_in_layout_affecting_groups(previous_payloads.as_ptr(), derived.payloads.as_ptr());
        self.replace_arena_pinned_style_record(slot, derived);
        if changes_layout_affecting_style {
            self.bump_fragment_cache_epoch_of_self_and_ancestors(slot);
            self.reset_cached_intrinsic_sizes_of_self_and_ancestors(slot);
        }
        self.notify_shell_of_style_change(slot, false, notice);
    }

    fn notify_shell_of_style_change(
        &self,
        slot: NodeSlotId,
        attach_resources: bool,
        notice: ShellStyleChangeNotice<'_>,
    ) {
        let Some(shell) = self.data(slot).shell.get() else {
            return;
        };
        match notice {
            ShellStyleChangeNotice::Now(main_thread) => {
                self.tell_shell_of_style_change(main_thread, slot, shell, attach_resources);
            }
            ShellStyleChangeNotice::Handback => self.hand_back(HostHandback::ShellStyleChanged {
                slot,
                shell,
                attach_resources,
            }),
        }
    }

    fn tell_shell_of_style_change(
        &self,
        main_thread: &crate::stage::MainThread,
        slot: NodeSlotId,
        shell: ShellId,
        attach_resources: bool,
    ) {
        let (context, shell_style_changed) = main_thread
            .host_tables()
            .and_then(|host_tables| host_tables.shell_style_changed_host.get())
            .expect("layout node arena has no style record host");
        super::tree_build_seal::note_host_call("shell_style_changed");
        // SAFETY: The engine and shell remain live. Native style-store mutation has finished before
        // the host can reenter Rust through its resource consumers.
        unsafe {
            shell_style_changed(
                context,
                shell.host_object(main_thread),
                self.node_style_record(slot),
                self.data(slot).style.get().as_ptr(),
                attach_resources,
            );
        };
    }

    pub(crate) fn continue_containing_block_search(
        &self,
        search: &mut super::abspos_inputs::ContainingBlockSearch,
        limit: NodeSlotId,
    ) {
        if !search.containing_block.is_invalid() {
            return;
        }
        let establishes_containing_block =
            super::node_facts::containing_block_establishment_flag(search.is_fixed_position);
        let looks_for_inline_containing_block = !search.is_fixed_position;
        let has_lifted_boxes = !self.inline_boxes_lifted_out_of.borrow().is_empty();
        let mut node = search.frontier;
        while node != limit {
            self.assert_layout_read_is_in_scope(node);
            let ancestor = self.data(node).parent.get();
            if ancestor.is_invalid() {
                break;
            }
            if looks_for_inline_containing_block
                && has_lifted_boxes
                && search.inline_containing_block.is_invalid()
                && let Some(inline_box) = self.inline_box_lifted_out_of(node)
            {
                search.inline_containing_block = self.nearest_inline_containing_block_from(inline_box);
            }
            node = ancestor;
            self.assert_layout_read_is_in_scope(node);
            let data = self.data(node);
            let kind = data.kind.get();
            if super::node_facts::kind_is_box(kind) {
                if super::node_facts::has_flag(data, establishes_containing_block) {
                    search.containing_block = node;
                    break;
                }
            } else if looks_for_inline_containing_block
                && search.inline_containing_block.is_invalid()
                && kind == NodeKind::InlineNode
                && super::node_facts::has_flag(data, NodeFlag::EstablishesAbsolutePositionContainingBlock)
            {
                search.inline_containing_block = node;
            }
        }
        search.frontier = node;
        if search.containing_block.is_invalid() && search.is_fixed_position && self.data(node).parent.get().is_invalid()
        {
            search.containing_block = node;
        }
    }

    fn nearest_inline_containing_block_from(&self, inline_box: NodeSlotId) -> NodeSlotId {
        let mut inline_ancestor = inline_box;
        while !inline_ancestor.is_invalid() {
            self.assert_layout_read_is_in_scope(inline_ancestor);
            let data = self.data(inline_ancestor);
            if data.kind.get() != NodeKind::InlineNode {
                break;
            }
            if super::node_facts::has_flag(data, NodeFlag::EstablishesAbsolutePositionContainingBlock) {
                return inline_ancestor;
            }
            inline_ancestor = data.parent.get();
        }
        NodeSlotId::INVALID
    }

    fn derive_containing_block_establishment_flags(&self, node: NodeSlotId) {
        let data = self.data(node);
        let previous_flags = data.flags.get();
        let (absolute, fixed) = if data.kind.get() == NodeKind::InlineNode {
            let absolute = !super::node_facts::has_flag(data, NodeFlag::Anonymous)
                && self
                    .node_style_if_live(node)
                    .is_some_and(crate::painting::style_queries::inline_establishes_absolute_position_containing_block);
            (absolute, false)
        } else {
            crate::painting::style_queries::establishes_positioning_containing_blocks(self, node)
        };
        let establishment_flags = NodeFlag::EstablishesAbsolutePositionContainingBlock as u32
            | NodeFlag::EstablishesFixedPositionContainingBlock as u32;
        let mut flags = previous_flags & !establishment_flags;
        if absolute {
            flags |= NodeFlag::EstablishesAbsolutePositionContainingBlock as u32;
        }
        if fixed {
            flags |= NodeFlag::EstablishesFixedPositionContainingBlock as u32;
        }
        data.flags.set(flags);

        let previous = previous_flags & establishment_flags;
        let current = flags & establishment_flags;
        let gained = current & !previous;
        let lost = previous & !current;
        let catches_escaping_boxes = gained != 0 && previous_flags & NodeFlag::AbsposDescendantEscapes as u32 != 0;
        let releases_contained_boxes = lost != 0
            && self
                .out_of_flow_positioning_contained
                .borrow()
                .get(&node)
                .is_some_and(|contained| contained & lost != 0);
        if !catches_escaping_boxes && !releases_contained_boxes {
            return;
        }
        self.set_needs_layout_update(node, true);
        self.scrollable_overflow.contained_boxes_dirty.set(true);
        if releases_contained_boxes {
            self.record_partial_relayout_escape();
        }
    }

    pub(crate) fn containing_block_by_walking_ancestors(&self, node: NodeSlotId) -> NodeSlotId {
        use crate::css::css_enums::positioning;
        let position = if super::node_facts::kind_is_text(self.data(node).kind.get()) {
            positioning::STATIC
        } else {
            crate::painting::style_queries::position(self, node)
        };
        if position != positioning::ABSOLUTE && position != positioning::FIXED {
            return self.nearest_ancestor_capable_of_forming_a_containing_block(node);
        }
        let mut search = super::abspos_inputs::ContainingBlockSearch::starting_at(node, position == positioning::FIXED);
        self.continue_containing_block_search(&mut search, NodeSlotId::INVALID);
        search.containing_block
    }

    fn mark_nodes_escaped_by_out_of_flow_box(&self, node: NodeSlotId, containing_block: NodeSlotId) {
        if let Some(inline_box) = self.inline_box_lifted_out_of(node) {
            let mut inline_ancestor = inline_box;
            while !inline_ancestor.is_invalid() && self.data(inline_ancestor).kind.get() == NodeKind::InlineNode {
                self.set_node_flag(inline_ancestor, NodeFlag::AbsposDescendantEscapes, true);
                inline_ancestor = self.data(inline_ancestor).parent.get();
            }
        }
        let mut ancestor = self.data(node).parent.get();
        while !ancestor.is_invalid() && ancestor != containing_block {
            self.set_node_flag(ancestor, NodeFlag::AbsposDescendantEscapes, true);
            ancestor = self.data(ancestor).parent.get();
        }
    }

    fn mark_nodes_escaped_by_attached_out_of_flow_box(&self, node: NodeSlotId) {
        if !super::node_facts::kind_is_box(self.data(node).kind.get())
            || !self
                .node_style_if_live(node)
                .is_some_and(|style| style.is_absolutely_positioned())
        {
            return;
        }
        let containing_block = self.containing_block_by_walking_ancestors(node);
        self.mark_nodes_escaped_by_out_of_flow_box(node, containing_block);
    }

    pub(crate) fn forget_committed_out_of_flow_facts(&self, node: NodeSlotId) {
        let data = self.data(node);
        data.flags
            .set(data.flags.get() & !(NodeFlag::AbsposDescendantEscapes as u32));
        let mut contained = self.out_of_flow_positioning_contained.borrow_mut();
        if !contained.is_empty() {
            contained.remove(&node);
        }
    }

    pub(crate) fn note_committed_out_of_flow_box(&self, node: NodeSlotId, inputs: &AbsposLayoutInputs) {
        let positioning = super::node_facts::containing_block_establishment_flag(
            crate::painting::style_queries::is_fixed_position(self, node),
        ) as u32;
        {
            let mut contained = self.out_of_flow_positioning_contained.borrow_mut();
            *contained.entry(inputs.containing_block).or_default() |= positioning;
            if !inputs.inline_containing_block.is_invalid() {
                *contained.entry(inputs.inline_containing_block).or_default() |=
                    NodeFlag::EstablishesAbsolutePositionContainingBlock as u32;
            }
        }
        self.mark_nodes_escaped_by_out_of_flow_box(node, inputs.containing_block);
    }

    fn derive_containing_block_establishment_flags_of_children(&self, parent: NodeSlotId) {
        self.for_each_node_in_layout_subtree_in_pre_order_with_pruning(parent, |node| {
            if node == parent {
                return true;
            }
            self.derive_containing_block_establishment_flags(node);
            super::node_facts::has_flag(self.data(node), NodeFlag::Anonymous)
        });
    }

    fn refresh_style_flags(&self, slot: NodeSlotId) {
        let style = self.node_style_if_live(slot).expect("styled layout node");
        let had_preserve_3d_transform_style =
            super::node_facts::has_flag(self.data(slot), NodeFlag::HasPreserve3dTransformStyle);
        self.set_node_flag(
            slot,
            NodeFlag::HasAnchorNames,
            !style.anchor().anchor_names.as_slice().is_empty(),
        );
        self.refresh_insets_use_anchor_functions_flag(slot);
        self.set_node_flag(slot, NodeFlag::HasAnimatedOpacityOrTransform, false);
        self.set_node_flag(
            slot,
            NodeFlag::HasPreserve3dTransformStyle,
            style.transform().transform_style == crate::css::css_enums::transform_style::PRESERVE_3D,
        );
        if !self.data(slot).parent.get().is_invalid() || self.data(slot).kind.get() == NodeKind::Viewport {
            self.derive_containing_block_establishment_flags(slot);
        }
        if had_preserve_3d_transform_style
            || super::node_facts::has_flag(self.data(slot), NodeFlag::HasPreserve3dTransformStyle)
        {
            self.derive_containing_block_establishment_flags_of_children(slot);
        }
        self.refresh_ancestor_facts_of_anonymous_children(slot);
    }

    pub(crate) fn enroll_text_node_for_content_sync(&self, node: NodeSlotId) {
        self.text_nodes_enrolled_for_content_sync.borrow_mut().insert(node);
    }

    pub(crate) fn stamp_anonymous_box(&self, slot: NodeSlotId, kind: NodeKind, derived: DerivedStyleRecord) {
        self.assert_owner_thread();
        let data = self.data(slot);
        assert_eq!(
            data.kind.get(),
            NodeKind::Unset,
            "stamped an anonymous box onto a bound slot"
        );
        assert!(derived.record != 0 && !derived.payloads.is_null());
        data.kind.set(kind);
        data.flags.set(super::node_facts::construction_flags(
            &FfiNodeConstructionFacts {
                kind,
                shell: std::ptr::null_mut(),
                is_anonymous: true,
                dom_paint_facts: 0,
                style_node: 0,
            },
            0,
        ));
        self.style_records[slot.slot_index() as usize].set(derived.record);
        self.style_records_pinned_by_arena[slot.slot_index() as usize].set(true);
        data.style.set(derived.payloads);
        self.enroll_node_for_replaced_content_facts_sync_if_eligible(slot);
    }

    /// Stamp a row the build allocated for a DOM node, before any shell exists for it. What
    /// `bind_shell` reads off the caller's construction facts is read here from the style mirror
    /// under the node's identity instead; the shell answers for the paint facts once it is
    /// materialised, since those are not published. The document names no identity of its own,
    /// and its row is recognised by its kind.
    /// The row generated text is rendered from: an anonymous row that names no DOM node and
    /// carries no style, as the shell the retired host path allocated for one did.
    pub(crate) fn stamp_anonymous_text_row(&self, slot: NodeSlotId) {
        self.assert_owner_thread();
        let data = self.data(slot);
        assert_eq!(
            data.kind.get(),
            NodeKind::Unset,
            "stamped a prepared row onto a bound slot"
        );
        data.kind.set(NodeKind::GeneratedTextNode);
        data.flags.set(super::node_facts::construction_flags(
            &FfiNodeConstructionFacts {
                kind: NodeKind::GeneratedTextNode,
                shell: std::ptr::null_mut(),
                is_anonymous: true,
                dom_paint_facts: 0,
                style_node: 0,
            },
            0,
        ));
    }

    pub(crate) fn stamp_dom_row(&self, slot: NodeSlotId, kind: NodeKind, style_node: Option<StyleNodeID>) {
        self.assert_owner_thread();
        let data = self.data(slot);
        assert_eq!(
            data.kind.get(),
            NodeKind::Unset,
            "stamped a prepared row onto a bound slot"
        );
        data.kind.set(kind);
        data.flags.set(super::node_facts::construction_flags(
            &FfiNodeConstructionFacts {
                kind,
                shell: std::ptr::null_mut(),
                is_anonymous: false,
                dom_paint_facts: 0,
                style_node: style_node.map_or(0, StyleNodeID::raw),
            },
            self.element_construction_facts(style_node),
        ));
        self.set_node_style_node(slot, style_node);
        // The name the document knows the row's node by. The mirror publishes one for an element
        // and for the document; a text node's row answers for nothing, as its identity reads zero.
        let unique_node_id = style_node.map_or(0, |style_node| {
            self.with_style_store(|engine| engine.element_unique_node_id(style_node))
        });
        self.unique_node_ids().publish(slot, unique_node_id);
        // What a row built for the node is painted and hit-tested with, published under the node's
        // identity by the DOM steps that derive it. The document's row takes its own, as with the
        // name it answers by.
        let scroll_offset = style_node.map_or_else(FfiCssPixelPoint::default, |style_node| {
            let facts = self.with_style_store(|engine| engine.node_dom_paint_facts(style_node));
            self.data(slot).dom_paint_facts.set(facts);
            // What the element has scrolled to, which its own box carries. The viewport's row
            // takes the navigable's offset instead, and a text node's row never scrolls.
            self.element_scroll_offset(style_node)
        });
        self.set_node_flag(
            slot,
            NodeFlag::HasScrollOffset,
            scroll_offset != FfiCssPixelPoint::default(),
        );
        self.scroll_offsets().publish(slot, scroll_offset.into());
        // Only a row in a user agent shadow tree can be in a text control's, which the
        // construction flags already answered, so this asks for almost no row at all.
        if crate::layout::node_facts::has_flag(self.data(slot), NodeFlag::IsInUserAgentShadowTree) {
            let in_focused_text_control =
                style_node.is_some_and(|style_node| self.is_identity_in_focused_text_control(style_node));
            self.set_node_flag(slot, NodeFlag::IsInFocusedTextControl, in_focused_text_control);
        }
        self.enroll_node_for_replaced_content_facts_sync_if_eligible(slot);
    }

    /// Stamp a row the build allocated for an element. Beyond what `stamp_dom_row` records, the
    /// row takes the style record the mirror published under the element's identity, so the kind
    /// the row is built with and the style it is built from come out of the same published answer.
    /// The scroll offset the pseudo-element `pseudo_kind` names on `generator` holds. Zero while
    /// nothing has scrolled it.
    pub(crate) fn pseudo_element_scroll_offset(&self, generator: StyleNodeID, pseudo_kind: u8) -> FfiCssPixelPoint {
        self.pseudo_element_scroll_offsets
            .get(&(generator, pseudo_kind))
            .copied()
            .unwrap_or_default()
    }

    /// Whether the node sits in the user agent shadow tree of the focused text control.
    pub(crate) fn is_identity_in_focused_text_control(&self, node: StyleNodeID) -> bool {
        self.identities_in_focused_text_control.contains(&node)
    }

    /// Record whether the node sits in the user agent shadow tree of the focused text control.
    pub(crate) fn set_identity_in_focused_text_control(&mut self, node: StyleNodeID, value: bool) {
        let identities = &mut self.identities_in_focused_text_control;
        if value {
            identities.insert(node);
        } else {
            identities.remove(&node);
        }
    }

    /// What the element has scrolled to. Zero while nothing has scrolled it.
    pub(crate) fn element_scroll_offset(&self, element: StyleNodeID) -> FfiCssPixelPoint {
        self.element_scroll_offsets.get(&element).copied().unwrap_or_default()
    }

    /// Record what the element has scrolled to. A zero offset is the absence of one, as for a
    /// pseudo-element.
    pub(crate) fn set_element_scroll_offset(&mut self, element: StyleNodeID, offset: FfiCssPixelPoint) {
        let offsets = &mut self.element_scroll_offsets;
        if offset == FfiCssPixelPoint::default() {
            offsets.remove(&element);
        } else {
            offsets.insert(element, offset);
        }
    }

    /// Record what the pseudo-element has scrolled to. A zero offset is the absence of one, which
    /// is what an identity that has never scrolled reads as.
    pub(crate) fn set_pseudo_element_scroll_offset(
        &mut self,
        generator: StyleNodeID,
        pseudo_kind: u8,
        offset: FfiCssPixelPoint,
    ) {
        let offsets = &mut self.pseudo_element_scroll_offsets;
        if offset == FfiCssPixelPoint::default() {
            offsets.remove(&(generator, pseudo_kind));
        } else {
            offsets.insert((generator, pseudo_kind), offset);
        }
    }

    /// An element keeps what its pseudo-elements have scrolled to across an identity change, as it
    /// keeps their bindings.
    pub(crate) fn move_pseudo_element_scroll_offsets(
        &mut self,
        old_generator: StyleNodeID,
        new_generator: StyleNodeID,
    ) {
        let offsets = &mut self.pseudo_element_scroll_offsets;
        let moved = offsets
            .iter()
            .filter(|((generator, _), _)| *generator == old_generator)
            .map(|((_, pseudo_kind), offset)| (*pseudo_kind, *offset))
            .collect::<Vec<_>>();
        for (pseudo_kind, offset) in moved {
            offsets.remove(&(old_generator, pseudo_kind));
            offsets.insert((new_generator, pseudo_kind), offset);
        }
    }

    pub(crate) fn stamp_dom_element_row(&self, slot: NodeSlotId, kind: NodeKind, style_node: StyleNodeID) {
        self.stamp_dom_row(slot, kind, Some(style_node));
        let (record, payloads) = self
            .with_style_engine(|engine| engine.element_published_style_record(style_node))
            .expect("an element whose box is built has published its style");
        if self.set_node_style(slot, record, payloads) {
            self.refresh_style_flags(slot);
        }
        self.enroll_node_for_svg_paint_resources_sync(slot);
    }

    /// The row a pseudo-element's box is built in. The row names no DOM node of its own: the kind
    /// its published display asks for and the record the mirror published for the pseudo-element
    /// are all it is stamped out of, and `stamp_pseudo_element_box` is what makes it the box the
    /// pseudo-element holds.
    pub(crate) fn stamp_pseudo_element_row(
        &self,
        slot: NodeSlotId,
        kind: NodeKind,
        generator: StyleNodeID,
        pseudo_kind: u8,
    ) {
        self.stamp_dom_row(slot, kind, None);
        // A pseudo-element's box names no DOM node of its own, which is what the walks that look
        // for the row of the nearest element skip it for. What it is generated for is what tells
        // an anonymous row apart from one stamped for a pseudo-element, and the row carries it
        // before its shell exists; binding it to the pseudo-element comes later.
        self.set_node_flag(slot, NodeFlag::Anonymous, true);
        self.data(slot).generated_for.set(pseudo_kind + 1);
        let (record, payloads) = self
            .with_style_engine(|engine| engine.pseudo_published_style_record(generator, pseudo_kind))
            .expect("a pseudo-element whose box is built has published its style");
        if self.set_node_style(slot, record, payloads) {
            self.refresh_style_flags(slot);
        }
        self.enroll_node_for_svg_paint_resources_sync(slot);
        // A pseudo-element's box can outlive replacement of the record it was stamped from, until
        // the layout tree is rebuilt, so the record is rooted across that gap.
        self.pin_node_style_record_for_host(slot, record);
    }

    /// The paint facts a row is built with, answered by the shell a prepared row was materialised
    /// into. A row being built is not a published row, so this is the plain write `bind_shell`
    /// performs rather than the change funnel a live row's facts move through.
    pub(crate) fn set_constructed_row_dom_paint_facts(&self, slot: NodeSlotId, facts: u8) {
        self.assert_owner_thread();
        self.data(slot).dom_paint_facts.set(facts);
    }

    pub(crate) fn refresh_insets_use_anchor_functions_flag(&self, slot: NodeSlotId) {
        let insets_use_anchor_functions = self.style_payloads(slot).is_some_and(|payloads| {
            super::node_facts::style_insets_use_anchor_functions(ComputedValuesView::new(&payloads.groups))
        });
        self.set_node_flag(slot, NodeFlag::InsetsUseAnchorFunctions, insets_use_anchor_functions);
    }

    /// Give `slot` the image provider it owns. A row is given one once, while it is being built.
    pub(crate) fn set_owned_image_provider(&self, host_tables: &HostTables, slot: NodeSlotId, provider: *mut c_void) {
        self.assert_owner_thread();
        assert!(!provider.is_null(), "a row was given a null owned image provider");
        let previous = host_tables.owned_image_providers.borrow_mut().insert(slot, provider);
        assert!(previous.is_none(), "a row was given a second owned image provider");
        self.rows_with_owned_image_provider.borrow_mut().insert(slot);
    }

    /// The image provider `slot` owns, or null for a row whose image comes from its DOM element or
    /// whose provider the arena has handed back.
    pub(crate) fn owned_image_provider(&self, host_tables: &HostTables, slot: NodeSlotId) -> *mut c_void {
        if !self.rows_with_owned_image_provider.borrow().contains(&slot) {
            return std::ptr::null_mut();
        }
        host_tables
            .owned_image_providers
            .borrow()
            .get(&slot)
            .copied()
            .unwrap_or(std::ptr::null_mut())
    }

    /// Give `slot` the image observer set its style asks for, and hand back the set it held. The
    /// caller deletes the old set after this returns, so a resource both sets observe is never
    /// dropped and refetched between them.
    pub(crate) fn replace_image_observers(
        &self,
        host_tables: &HostTables,
        slot: NodeSlotId,
        observers: *mut c_void,
    ) -> *mut c_void {
        self.assert_owner_thread();
        let mut rows = self.rows_with_image_observers.borrow_mut();
        let mut sets = host_tables.image_observer_sets.borrow_mut();
        if !rows.contains(&slot)
            && let Some(owed) = sets.remove(&slot)
        {
            // The arena has handed this set back and the host has not been paid for it yet. It
            // stays owed to the handback rather than to this caller.
            host_tables.image_observer_sets_owed.borrow_mut().push((slot, owed));
        }
        let previous = if observers.is_null() {
            rows.remove(&slot);
            sets.remove(&slot)
        } else {
            rows.insert(slot);
            sets.insert(slot, observers)
        };
        previous.unwrap_or(std::ptr::null_mut())
    }

    /// The image observer set `slot` holds, or null for a row whose style asks for none or whose
    /// set the arena has handed back.
    pub(crate) fn image_observers(&self, host_tables: &HostTables, slot: NodeSlotId) -> *mut c_void {
        if !self.rows_with_image_observers.borrow().contains(&slot) {
            return std::ptr::null_mut();
        }
        host_tables
            .image_observer_sets
            .borrow()
            .get(&slot)
            .copied()
            .unwrap_or(std::ptr::null_mut())
    }

    /// Records whether the host listens for box presence, which it registers in the host tables.
    pub(crate) fn set_host_hears_box_presence(&self, hears: bool) {
        self.host_hears_box_presence.set(hears);
        if !hears {
            self.rows_with_changed_committed_box.borrow_mut().clear();
        }
    }

    /// What boxes the node bound to `row` has. An invalid row means the node has none.
    fn box_presence_bits(&self, row: NodeSlotId) -> u8 {
        if row.is_invalid() {
            return 0;
        }
        let mut bits = BOX_PRESENCE_HAS_LAYOUT_BOX;
        if self.paintable_rows().paintable_row_is_populated(row) {
            bits |= BOX_PRESENCE_HAS_COMMITTED_BOX;
        }
        bits
    }

    /// Tells the host what boxes `node` has now. A pseudo-element's boxes stay unmirrored, since
    /// nothing on the DOM side reads them as a bit.
    fn notify_box_presence(&self, node: BoundNode) {
        if !self.host_hears_box_presence.get() {
            return;
        }
        let style_node = match node {
            BoundNode::Identity(style_node) => style_node.raw(),
            BoundNode::Document => 0,
            BoundNode::PseudoElement(..) => return,
        };
        self.hand_back(HostHandback::BoxPresence(style_node));
    }

    /// Tells the host what boxes the node `style_node` names has now. No row list may be borrowed
    /// here.
    fn tell_host_box_presence(&self, main_thread: &crate::stage::MainThread, style_node: u32) {
        let Some((context, callback)) = main_thread
            .host_tables()
            .and_then(|host_tables| host_tables.box_presence_host.get())
        else {
            return;
        };
        let row = match StyleNodeID::from_raw(style_node) {
            Some(style_node) => self.bound_row(style_node),
            None => self.bound_viewport_row(),
        };
        super::tree_build_seal::note_host_call("notify_box_presence");
        // SAFETY: Registration and unregistration keep the host context live, and the host does
        // not reenter the arena.
        unsafe { callback(context, style_node, self.box_presence_bits(row)) };
    }

    /// Opens a span of work whose handbacks the main thread pays once the span is over. Every
    /// handback is made inside one, so none waits for a payer that is not coming.
    pub(crate) fn begin_paying_host_handbacks(&self, _: &crate::stage::MainThread) {
        self.open_host_handback_span();
    }

    /// Closes the span [`Self::begin_paying_host_handbacks`] opened, and pays what is owed.
    pub(crate) fn finish_paying_host_handbacks(&self, main_thread: &crate::stage::MainThread) {
        self.pay_host_handbacks(main_thread);
        self.close_host_handback_span();
    }

    /// Takes what the arena owes the host so far, for a commit whose host half is paid later.
    pub(crate) fn take_host_handbacks_ahead_of_payment(&self) -> HostHandbacks {
        std::mem::take(&mut *self.host_handbacks.borrow_mut())
    }

    /// Pays what [`Self::take_host_handbacks_ahead_of_payment`] took, and anything paying it hands
    /// back, ahead of what later commits owe, then closes the span the commit opened.
    pub(crate) fn finish_paying_taken_host_handbacks(
        &self,
        main_thread: &crate::stage::MainThread,
        handbacks: HostHandbacks,
    ) {
        let later = std::mem::take(&mut *self.host_handbacks.borrow_mut());
        self.pay_tree_build_handbacks(main_thread, handbacks);
        self.pay_host_handbacks(main_thread);
        *self.host_handbacks.borrow_mut() = later;
        self.close_host_handback_span();
    }

    /// Opens the span of a layout pass's commit, which may run off the document thread. The
    /// commit's host half closes it with [`Self::finish_paying_host_handbacks`].
    pub(crate) fn begin_layout_commit_handbacks(&self) {
        self.open_host_handback_span();
    }

    /// Opens the span of a layout pass's preparation off the document thread. What it owes is paid
    /// with the handbacks of the pass's commit, which is followed by
    /// [`Self::end_layout_pass_preparation_handbacks`].
    pub(crate) fn begin_layout_pass_preparation_handbacks(&self) {
        self.open_host_handback_span();
    }

    /// Closes the span [`Self::begin_layout_pass_preparation_handbacks`] opened, once the pass's
    /// commit has paid what it owed.
    pub(crate) fn end_layout_pass_preparation_handbacks(&self, _: &crate::stage::MainThread) {
        assert!(
            self.host_handbacks.borrow().handbacks.is_empty(),
            "a layout pass's commit left its preparation's handbacks unpaid"
        );
        self.close_host_handback_span();
    }

    /// Opens the tree build's span. The build cannot pay the host, so what it owes becomes part of
    /// its output instead; it starts with nothing owed from before it.
    pub(crate) fn begin_tree_build_handbacks(&self) {
        assert!(
            self.host_handbacks.borrow().handbacks.is_empty(),
            "a tree build started while the host was still owed something"
        );
        self.open_host_handback_span();
    }

    /// Closes the tree build's span, and returns what the build owes the host.
    pub(crate) fn take_tree_build_handbacks(&self) -> HostHandbacks {
        self.close_host_handback_span();
        std::mem::take(&mut *self.host_handbacks.borrow_mut())
    }

    fn open_host_handback_span(&self) {
        let spans = &self.host_handback_spans;
        spans.set(spans.get().checked_add(1).expect("host handback spans overflowed"));
    }

    fn close_host_handback_span(&self) {
        let spans = &self.host_handback_spans;
        spans.set(spans.get().checked_sub(1).expect("unbalanced host handback span"));
    }

    /// Pays what the arena owes the host, in the order it was handed back.
    pub(crate) fn pay_host_handbacks(&self, main_thread: &crate::stage::MainThread) {
        loop {
            let handbacks = std::mem::take(&mut *self.host_handbacks.borrow_mut());
            if handbacks.handbacks.is_empty() {
                return;
            }
            self.pay_tree_build_handbacks(main_thread, handbacks);
        }
    }

    /// Pays what a finished tree build owes the host, in the order the build let go of it.
    pub(crate) fn pay_tree_build_handbacks(&self, main_thread: &crate::stage::MainThread, handbacks: HostHandbacks) {
        // Every host object is looked up before any is paid for. No host code ran between the
        // arena letting go of them and here, so the tables still hold each as it was then, and the
        // host code paying runs cannot change what the rest of the batch pays with.
        let objects = self.take_host_objects_owed(main_thread, &handbacks.handbacks);
        for (handback, object) in handbacks.handbacks.into_iter().zip(objects) {
            self.pay_host_handback(main_thread, handback, object);
        }
    }

    /// The host object each handback names, or null for one that names none, in the order of the
    /// handbacks. A provider or observer set leaves the host tables here; a detach notice leaves
    /// the provider where it is.
    fn take_host_objects_owed(
        &self,
        main_thread: &crate::stage::MainThread,
        handbacks: &[HostHandback],
    ) -> Vec<*mut c_void> {
        let Some(host_tables) = main_thread.host_tables() else {
            return vec![std::ptr::null_mut(); handbacks.len()];
        };
        handbacks
            .iter()
            .map(|handback| {
                let object = match handback {
                    HostHandback::OwnedImageProvider(row) => host_tables.owned_image_providers.borrow_mut().remove(row),
                    HostHandback::OwnedImageProviderDetach(row) => {
                        host_tables.owned_image_providers.borrow().get(row).copied()
                    }
                    HostHandback::ImageObservers(row) => {
                        let mut owed = host_tables.image_observer_sets_owed.borrow_mut();
                        match owed.iter().position(|(owed_row, _)| owed_row == row) {
                            Some(index) => Some(owed.remove(index).1),
                            None => host_tables.image_observer_sets.borrow_mut().remove(row),
                        }
                    }
                    HostHandback::BoxPresence(_)
                    | HostHandback::Shell(_)
                    | HostHandback::PaintableRowReset(_)
                    | HostHandback::ShellStyleChanged { .. } => None,
                };
                object.unwrap_or(std::ptr::null_mut())
            })
            .collect()
    }

    /// Hands back the reset of a row whose paint state is being cleared.
    pub(crate) fn hand_back_paintable_row_reset(&self, reset: crate::painting::paintable_rows::PaintableRowReset) {
        self.hand_back(HostHandback::PaintableRowReset(reset));
    }

    /// Queues what the arena owes the host. Nothing here can reach the host: only a main-thread
    /// payer hands the queue over.
    fn hand_back(&self, handback: HostHandback) {
        assert!(
            self.host_handback_spans.get() != 0,
            "the arena owes the host something outside any span that pays it"
        );
        self.host_handbacks.borrow_mut().push(handback);
    }

    fn pay_host_handback(&self, main_thread: &crate::stage::MainThread, handback: HostHandback, object: *mut c_void) {
        use crate::layout::tree_mutation::{
            destroy_image_observers, destroy_owned_image_provider, destroy_shell, notify_owned_image_provider_of_detach,
        };
        match handback {
            HostHandback::BoxPresence(style_node) => self.tell_host_box_presence(main_thread, style_node),
            HostHandback::Shell(shell) => destroy_shell(main_thread, shell.host_object(main_thread)),
            HostHandback::OwnedImageProvider(_) => destroy_owned_image_provider(main_thread, object),
            HostHandback::ImageObservers(_) => destroy_image_observers(main_thread, object),
            HostHandback::OwnedImageProviderDetach(_) => {
                notify_owned_image_provider_of_detach(main_thread, object);
            }
            HostHandback::PaintableRowReset(reset) => {
                super::tree_build_seal::note_host_call("paintable_row_reset");
                reset.invoke_callback_on_main_thread(main_thread);
            }
            HostHandback::ShellStyleChanged {
                slot,
                shell,
                attach_resources,
            } => {
                if self.slot_is_live(slot) && self.data(slot).shell.get() == Some(shell) {
                    self.tell_shell_of_style_change(main_thread, slot, shell, attach_resources);
                }
            }
        }
    }

    /// Hands back the objects a freed subtree's rows held. The style records the arena pinned for
    /// the rows are released now.
    pub(crate) fn hand_back_freed_subtree(&self, freed: FreedSubtree) {
        let FreedSubtree {
            shells,
            rows_with_owned_image_provider,
            rows_with_image_observers,
            paintable_row_resets,
            arena_pinned_style_records,
            style_engine,
        } = freed;
        for shell in shells {
            self.hand_back(HostHandback::Shell(shell));
        }
        for row in rows_with_owned_image_provider {
            self.hand_back(HostHandback::OwnedImageProvider(row));
        }
        for row in rows_with_image_observers {
            self.hand_back(HostHandback::ImageObservers(row));
        }
        for reset in paintable_row_resets {
            self.hand_back(HostHandback::PaintableRowReset(reset));
        }
        FreedSubtree::unpin_arena_pinned_style_records(style_engine, arena_pinned_style_records);
    }

    /// Records that `row` is gaining or losing its committed box. The paint state is borrowed for
    /// writing while that happens, so the host hears about it from the flush that follows.
    pub(crate) fn note_committed_box_changed(&mut self, row: NodeSlotId) {
        if self.host_hears_box_presence.get() {
            self.rows_with_changed_committed_box.get_mut().push(row);
        }
    }

    /// Tells the host about the committed boxes that appeared or went away since it last heard.
    pub(crate) fn flush_committed_box_changes(&self) {
        loop {
            let rows = std::mem::take(&mut *self.rows_with_changed_committed_box.borrow_mut());
            if rows.is_empty() {
                return;
            }
            for row in rows {
                // A row freed after its committed box went away named its node through the free.
                if !self.slot_is_live(row) {
                    continue;
                }
                let Some(node) = self.bound_node_of(row) else {
                    continue;
                };
                self.notify_box_presence(node);
            }
        }
    }

    fn materialize_shell(&self, main_thread: &crate::stage::MainThread, id: NodeSlotId) -> *mut c_void {
        let Some((context, factory)) = main_thread
            .host_tables()
            .and_then(|host_tables| host_tables.shell_factory.get())
        else {
            return std::ptr::null_mut();
        };
        let data = self.data(id);
        if data.kind.get() == NodeKind::Unset {
            return std::ptr::null_mut();
        }
        super::tree_build_seal::note_host_call("layout_node_shell_factory");
        super::seal::note_host_call(self.layout_pass_is_running(), "layout_node_shell_factory");
        crate::painting::seal::note_host_call("layout_node_shell_factory");
        // SAFETY: Registration and unregistration keep the factory context live; the factory binds a
        // shell to this live slot and writes nothing but the slot's shell cell.
        unsafe { factory(context, id, data.kind.get()) };
        data.shell
            .get()
            .map_or(std::ptr::null_mut(), |shell| shell.host_object(main_thread))
    }

    pub(crate) fn shell_count(&self) -> u32 {
        let mut count = 0;
        for (index, metadata) in self.slot_metadata.iter().enumerate() {
            if metadata.occupied
                && !self
                    .data(NodeSlotId::new(index as u32, metadata.generation))
                    .shell
                    .get()
                    .is_none()
            {
                count += 1;
            }
        }
        count
    }

    pub(crate) fn replace_arena_pinned_style_record(&self, slot: NodeSlotId, derived: DerivedStyleRecord) {
        self.assert_owner_thread();
        let previously_pinned = self.style_records_pinned_by_arena[slot.slot_index() as usize].replace(true);
        assert!(derived.record != 0 && !derived.payloads.is_null());
        let previous_style_record = self.style_records[slot.slot_index() as usize].replace(derived.record);
        self.data(slot).style.set(derived.payloads);
        self.refresh_style_flags(slot);
        self.invalidate_overflow_after_style_change(slot);
        self.enroll_text_children_for_content_sync(slot);
        self.enroll_node_for_replaced_content_facts_sync_if_eligible(slot);
        self.enroll_node_for_svg_paint_resources_sync(slot);
        if previously_pinned {
            self.with_style_engine(|engine| engine.unpin_layout_style_record(previous_style_record));
        }
    }

    pub(crate) fn attach_shell(&self, slot: NodeSlotId, shell: *mut c_void) {
        self.assert_owner_thread();
        let shell = ShellId::of_host_object(shell).expect("a shell is attached");
        let data = self.data(slot);
        assert!(
            data.shell.get().is_none(),
            "layout node arena attached a second shell to a slot"
        );
        data.shell.set(Some(shell));
    }

    /// Record whether attaching the row's style resources loaded any image, which is what decides
    /// whether the row has anything for a later attach to take away.
    pub(crate) fn note_style_image_resources_attached(&self, id: NodeSlotId, attached: bool) {
        let mut rows = self.style_image_resources_attached.borrow_mut();
        if attached {
            rows.insert(id);
        } else {
            rows.remove(&id);
        }
    }

    /// Whether attaching a row's style resources could change anything the row holds. Nearly every
    /// box has a style with no `<image>` anywhere in it, holds nothing a style that had one left
    /// behind, and carries no paint facts of its own; for those the attach republishes exactly
    /// what the row already has, so the build need not ask the document for it.
    pub(crate) fn style_resources_attach_can_change_anything(&self, id: NodeSlotId) -> bool {
        if self.style_image_resources_attached.borrow().contains(&id) {
            return true;
        }
        let data = self.data(id);
        // The kinds whose paint facts are read off the element the box was built for, and the
        // image element, whose row answers for its map whatever box it ended up with.
        if matches!(
            data.kind.get(),
            NodeKind::CheckBox
                | NodeKind::RadioButton
                | NodeKind::CanvasBox
                | NodeKind::ImageBox
                | NodeKind::SVGImageBox
                | NodeKind::VideoBox
                | NodeKind::NavigableContainerViewport
        ) {
            return true;
        }
        let style_node = self.node_style_node(id);
        if self.element_construction_facts(style_node)
            & crate::css::style::bridge::element_construction_fact::IS_HTML_IMAGE_ELEMENT
            != 0
        {
            return true;
        }
        // A row that carries layer image facts loses them when its style stops holding images.
        if self.layer_image_paint_facts.borrow().contains_key(&id) {
            return true;
        }
        let record = self.node_style_record(id);
        self.with_style_store(|engine| engine.style_record_dependency_flags(record))
            .is_some_and(|flags| flags & crate::css::style::HOLDS_IMAGE_VALUES != 0)
    }

    pub(crate) fn replaced_paint_facts(
        &self,
        id: NodeSlotId,
    ) -> Option<crate::painting::replaced_paint_facts::ReplacedPaintFacts> {
        self.replaced_paint_facts.borrow().get(&id).cloned()
    }

    pub(crate) fn for_each_replaced_paint_facts(
        &self,
        mut callback: impl FnMut(NodeSlotId, &crate::painting::replaced_paint_facts::ReplacedPaintFacts),
    ) {
        for (id, facts) in self.replaced_paint_facts.borrow().iter() {
            callback(*id, facts);
        }
    }

    pub(crate) fn for_each_layer_image_paint_facts_owner(&self, mut callback: impl FnMut(NodeSlotId)) {
        for id in self.layer_image_paint_facts.borrow().keys() {
            callback(*id);
        }
    }

    pub(crate) fn layer_image_paint_facts(
        &self,
        id: NodeSlotId,
        list: crate::painting::host::FfiLayerImageList,
        computed_index: u32,
    ) -> Option<crate::painting::layer_image_paint_facts::LayerImagePaintFacts> {
        let table = self.layer_image_paint_facts.borrow();
        let entries = table.get(&id)?;
        entries
            .iter()
            .find(|entry| entry.list == list && entry.computed_index == computed_index)
            .map(|entry| entry.facts.clone())
    }

    pub(crate) fn layer_image_paint_facts_for_verification(&self, id: NodeSlotId) -> String {
        format!("{:?}", self.layer_image_paint_facts.borrow().get(&id))
    }

    pub(crate) fn set_layer_image_paint_facts(
        &self,
        id: NodeSlotId,
        entries: Vec<crate::painting::layer_image_paint_facts::LayerImagePaintFactsEntry>,
    ) -> bool {
        self.assert_owner_thread();
        if !self.slot_is_live(id) {
            return false;
        }
        let mut table = self.layer_image_paint_facts.borrow_mut();
        let changed = if entries.is_empty() {
            table.remove(&id).is_some_and(|previous| !previous.is_empty())
        } else if table.get(&id) == Some(&entries) {
            false
        } else {
            table.insert(id, entries);
            true
        };
        drop(table);
        if changed {
            use crate::painting::record::damage::PaintDamage;
            self.push_paint_damage_for_repaint(
                id,
                PaintDamage::DRAW_BACKGROUND | PaintDamage::DRAW_BORDER | PaintDamage::SCOPE_PREAMBLE,
            );
        }
        changed
    }

    pub(crate) fn set_replaced_paint_facts(
        &self,
        id: NodeSlotId,
        facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts,
    ) -> bool {
        self.assert_owner_thread();
        if !self.slot_is_live(id) {
            return false;
        }
        let mut any_changed = false;
        for row in self.rows_sharing_dom_node_with(id) {
            let mut table = self.replaced_paint_facts.borrow_mut();
            if table.get(&row) == Some(&facts) {
                continue;
            }
            table.insert(row, facts.clone());
            drop(table);
            any_changed = true;
            self.push_paint_damage_for_repaint(row, crate::painting::record::damage::PaintDamage::DRAW_FOREGROUND);
        }
        any_changed
    }

    pub(crate) fn svg_attribute_facts(&self, id: NodeSlotId) -> FfiSvgAttributeFacts {
        match self.node_style_node(id) {
            Some(style_node) => self.style_node_svg_attribute_facts(style_node),
            None => FfiSvgAttributeFacts::default(),
        }
    }

    /// The SVG presentation attributes an element published, named by its style node. An element
    /// the document never published for - anything that is not an SVG element - answers with the
    /// default facts, whose `geometry_kind` says it draws no shape.
    pub(crate) fn style_node_svg_attribute_facts(&self, style_node: StyleNodeID) -> FfiSvgAttributeFacts {
        match self.svg_attribute_facts.get(&style_node) {
            Some(facts) => **facts,
            None => FfiSvgAttributeFacts::default(),
        }
    }

    /// The `points` list a <polyline> or <polygon> parsed.
    pub(crate) fn svg_points(&self, id: NodeSlotId) -> Option<Arc<[super::svg_formatting_context::FfiFloatPoint]>> {
        self.style_node_svg_points(self.node_style_node(id)?)
    }

    pub(crate) fn style_node_svg_points(
        &self,
        style_node: StyleNodeID,
    ) -> Option<Arc<[super::svg_formatting_context::FfiFloatPoint]>> {
        self.svg_points.get(&style_node).cloned()
    }

    /// Replace only the four names a graphics element's style carries. An element that has not
    /// published its attributes yet has no place to put them, and will carry them itself when it
    /// does: the publication is made when the style tree names the element, which is before any
    /// style of its own is installed.
    pub(crate) fn set_style_node_svg_style_references(&mut self, style_node: StyleNodeID, references: [u32; 4]) {
        self.assert_owner_thread();
        let Some(facts) = self.svg_attribute_facts.get_mut(&style_node) else {
            return;
        };
        let replaced = Self::published_reference_atoms(facts);
        [
            facts.mask_reference_atom,
            facts.clip_path_reference_atom,
            facts.fill_reference_atom,
            facts.stroke_reference_atom,
        ] = references;
        let retained = Self::published_reference_atoms(facts);
        self.retain_published_reference_atoms(retained, replaced);
    }

    /// Replace what one tree scope registers. C++ rebuilds a scope's counter styles whole, so the
    /// publication does too.
    pub(crate) fn publish_counter_styles(
        &self,
        tree_scope: u32,
        scope: crate::css::counter_representation::CounterStyleScope,
    ) {
        self.assert_owner_thread();
        self.counter_styles.borrow_mut().publish_scope(tree_scope, scope);
    }

    pub(crate) fn counters_sets(&self) -> &RefCell<super::counters::CountersSets> {
        self.assert_owner_thread();
        &self.counters_sets
    }

    pub(crate) fn generated_content(&self) -> &RefCell<super::generated_content::GeneratedContent> {
        self.assert_owner_thread();
        &self.generated_content
    }

    pub(crate) fn with_counter_style_registry<T>(
        &self,
        callback: impl FnOnce(&crate::css::counter_representation::CounterStyleRegistry) -> T,
    ) -> T {
        self.assert_owner_thread();
        callback(&self.counter_styles.borrow())
    }

    pub(crate) fn set_style_node_svg_attribute_facts(
        &mut self,
        style_node: StyleNodeID,
        facts: FfiSvgAttributeFacts,
        points: &[super::svg_formatting_context::FfiFloatPoint],
    ) {
        self.assert_owner_thread();
        let replaced = match self.svg_attribute_facts.entry(style_node) {
            std::collections::hash_map::Entry::Occupied(mut published) => {
                let replaced = Self::published_reference_atoms(published.get());
                **published.get_mut() = facts;
                replaced
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(Box::new(facts));
                [0; PUBLISHED_REFERENCE_ATOM_COUNT]
            }
        };
        self.retain_published_reference_atoms(Self::published_reference_atoms(&facts), replaced);
        if points.is_empty() {
            self.svg_points.remove(&style_node);
        } else {
            self.svg_points.insert(style_node, points.into());
        }
    }

    /// The element an SVG reference resolves to, named by the atom its URL fragment interned to.
    /// `SVGGraphicsElement::resolve_fragment_identifier_to_element` asks the document first and the
    /// shadow tree the referring element sits in second, so the lookup is made in that order.
    pub(crate) fn element_by_svg_reference(&self, referrer: StyleNodeID, name: u32) -> Option<StyleNodeID> {
        let name = crate::css::style::index::StyleAtomID(name);
        if name.is_none() {
            return None;
        }
        self.with_style_store(|engine| {
            let scope = engine.tree().tree_scope(referrer);
            engine
                .element_by_id(crate::css::style::tree::TreeScopeID::DOCUMENT, name)
                .or_else(|| {
                    (scope != crate::css::style::tree::TreeScopeID::DOCUMENT)
                        .then(|| engine.element_by_id(scope, name))
                        .flatten()
                })
        })
    }

    /// The element the document's id index holds for the atom `name`, which is what
    /// `Document::get_element_by_id` answers with. A reference that resolves in the document scope
    /// alone - an SVG `href` chain - asks for this rather than for `element_by_svg_reference`.
    pub(crate) fn element_by_document_id(&self, name: u32) -> Option<StyleNodeID> {
        let name = crate::css::style::index::StyleAtomID(name);
        if name.is_none() {
            return None;
        }
        self.with_style_store(|engine| engine.element_by_id(crate::css::style::tree::TreeScopeID::DOCUMENT, name))
    }

    /// The published computed style of an element the style tree names, which a box's own style
    /// pointer cannot reach: a `<defs>` builds no box, so nothing under it has a row.
    ///
    /// The group pointers are copied out rather than borrowed, since the style store's borrow ends
    /// with the query while the record they address is retained for the pass.
    pub(crate) fn style_node_style_payloads(&self, style_node: StyleNodeID) -> Option<FfiStylePayloads> {
        self.with_style_store(|engine| {
            let groups = engine.element_published_style_payloads(style_node)?;
            let mut payloads = FfiStylePayloads::default();
            payloads.groups.copy_from_slice(groups);
            Some(payloads)
        })
    }

    pub(crate) fn clear_style_node_svg_attribute_facts(&mut self, style_node: StyleNodeID) {
        self.assert_owner_thread();
        let removed = self.svg_attribute_facts.remove(&style_node);
        if let Some(removed) = removed {
            self.retain_published_reference_atoms(
                [0; PUBLISHED_REFERENCE_ATOM_COUNT],
                Self::published_reference_atoms(&removed),
            );
        }
        self.svg_points.remove(&style_node);
    }

    /// The names a publication holds a sweep retention on: the one an `href` names, and the four
    /// a graphics element's style names.
    fn published_reference_atoms(facts: &FfiSvgAttributeFacts) -> [u32; PUBLISHED_REFERENCE_ATOM_COUNT] {
        [
            facts.reference_fragment_atom,
            facts.mask_reference_atom,
            facts.clip_path_reference_atom,
            facts.fill_reference_atom,
            facts.stroke_reference_atom,
        ]
    }

    /// Hand the retention a publication's SVG references hold from the names they used to carry to
    /// the names they carry now, so the style engine's atom sweep cannot reissue either number
    /// while a publication still reads it. The atom an id names is otherwise rooted only by the
    /// element answering to it, and a reference to an id that is in no document has no such
    /// element.
    fn retain_published_reference_atoms(
        &self,
        retained: [u32; PUBLISHED_REFERENCE_ATOM_COUNT],
        released: [u32; PUBLISHED_REFERENCE_ATOM_COUNT],
    ) {
        if retained == released {
            return;
        }
        // A document being torn down drops its style record host before the last publication is
        // cleared. The engine it named is going with it, so there is nothing left to retain for.
        let style_engine = self.style_engine.get().0;
        if style_engine.is_null() {
            return;
        }
        // SAFETY: As with `with_style_engine`, the engine outlives the arena's live nodes and no
        // host callback runs while the borrow is active.
        let engine = unsafe { &mut *style_engine.cast::<StyleEngine>() };
        for atom in retained {
            engine.retain_published_atom(crate::css::style::index::StyleAtomID(atom));
        }
        for atom in released {
            engine.release_published_atom(crate::css::style::index::StyleAtomID(atom));
        }
    }

    pub(crate) fn node_has_dom_paint_fact(&self, id: NodeSlotId, fact: DomPaintFact) -> bool {
        self.data(id).dom_paint_facts.get() & fact as u8 != 0
    }

    pub(crate) fn node_dom_paint_facts(&self, id: NodeSlotId) -> u8 {
        self.data(id).dom_paint_facts.get()
    }

    pub(crate) fn set_node_dom_paint_facts(&self, id: NodeSlotId, facts: u8) -> bool {
        self.assert_owner_thread();
        crate::painting::published_immutable::note_row_mutation(self, id, "M5 layout_arena_set_node_dom_paint_facts");
        let mut any_changed = false;
        for row in self.rows_sharing_dom_node_with(id) {
            let data = self.data(row);
            if data.dom_paint_facts.get() == facts {
                continue;
            }
            data.dom_paint_facts.set(facts);
            any_changed = true;
            use crate::painting::record::damage::PaintDamage;
            self.push_paint_damage_for_repaint(row, PaintDamage::ALL_HIT | PaintDamage::SCROLL_METADATA);
        }
        any_changed
    }

    pub(crate) fn note_rows_share_dom_node(&self, bound_row: NodeSlotId, added_row: NodeSlotId) {
        self.assert_owner_thread();
        assert!(
            self.node_is_dom_backed(added_row),
            "layout node arena shared rows of an anonymous node"
        );
        assert!(
            self.bound_node_of(added_row) == self.bound_node_of(bound_row),
            "layout node arena shared rows of different DOM nodes"
        );
        if self.rows_sharing_dom_node_with(bound_row).contains(&added_row) {
            return;
        }
        let bound_link = &self.next_rows_built_for_same_node[bound_row.slot_index() as usize];
        let next = bound_link.get();
        self.next_rows_built_for_same_node[added_row.slot_index() as usize].set(if next.is_invalid() {
            bound_row
        } else {
            next
        });
        bound_link.set(added_row);
    }

    /// Visits `id` and every other row built for the same DOM node. A row that shares its node
    /// with none, which is nearly all of them, costs one link read.
    pub(crate) fn for_each_row_built_for_same_node(&self, id: NodeSlotId, mut visit: impl FnMut(NodeSlotId)) {
        visit(id);
        let Some(link) = self.next_rows_built_for_same_node.get(id.slot_index() as usize) else {
            return;
        };
        let mut row = link.get();
        if row.is_invalid() {
            return;
        }
        while row != id {
            visit(row);
            row = self.next_rows_built_for_same_node[row.slot_index() as usize].get();
        }
    }

    pub(crate) fn rows_sharing_dom_node_with(&self, id: NodeSlotId) -> Vec<NodeSlotId> {
        let mut rows = Vec::new();
        self.for_each_row_built_for_same_node(id, |row| rows.push(row));
        rows
    }

    fn forget_row_sharing_dom_node(&mut self, id: NodeSlotId) {
        let successor = self.next_rows_built_for_same_node[id.slot_index() as usize].replace(NodeSlotId::INVALID);
        if successor.is_invalid() {
            return;
        }
        // Close the ring behind the row that is leaving.
        let mut predecessor = successor;
        loop {
            let link = &self.next_rows_built_for_same_node[predecessor.slot_index() as usize];
            if link.get() == id {
                link.set(if predecessor == successor {
                    NodeSlotId::INVALID
                } else {
                    successor
                });
                break;
            }
            predecessor = link.get();
        }
        // The node stays bound to one of the rows that still share it.
        let Some(node) = self.bound_node_of(id) else {
            return;
        };
        if self.bound_node_of(successor) != Some(node) {
            return;
        }
        self.with_bound_row_entry_of(node, |bound_row| {
            if *bound_row == id {
                *bound_row = successor;
            }
        });
    }

    pub(crate) fn set_node_flag(&self, id: NodeSlotId, flag: NodeFlag, value: bool) {
        self.assert_owner_thread();
        let data = self.data(id);
        let previous = data.flags.get();
        let mut updated = previous;
        if value {
            updated |= flag as u32;
        } else {
            updated &= !(flag as u32);
        }
        if value
            && matches!(flag, NodeFlag::NeedsLayoutUpdate | NodeFlag::NeedsOwnGeometryUpdate)
            && data.flags.get() & (NodeFlag::NeedsLayoutUpdate as u32 | NodeFlag::NeedsOwnGeometryUpdate as u32) == 0
        {
            let mut nodes = self.nodes_with_layout_update_flags.borrow_mut();
            let previous = self
                .layout_update_flag_node_indices
                .borrow_mut()
                .insert(id, nodes.len());
            debug_assert!(previous.is_none());
            nodes.push(id);
        } else if !value
            && matches!(flag, NodeFlag::NeedsLayoutUpdate | NodeFlag::NeedsOwnGeometryUpdate)
            && updated & (NodeFlag::NeedsLayoutUpdate as u32 | NodeFlag::NeedsOwnGeometryUpdate as u32) == 0
        {
            self.remove_layout_update_flag_node(id);
        }
        data.flags.set(updated);
        // Retaining compositor-animated content decides whether a non-invertible transform
        // still records its stacking context.
        if flag == NodeFlag::HasAnimatedOpacityOrTransform && updated != previous {
            self.push_paint_damage(id, crate::painting::record::damage::PaintDamage::ELIGIBILITY);
        }
        // A commit leaves an ordinary inline's overflow unmeasured, since nothing reads it. Once
        // the inline stores a scroll offset, its overflow clamps that offset, so it is measured
        // before the rows are next published.
        if flag == NodeFlag::HasScrollOffset
            && updated != previous
            && value
            && self.paintable_row_is_populated(id)
            && !self.paintable_side_data(id).overflow_measured_this_commit.get()
        {
            self.note_row_overflow_unmeasured(id);
        }
    }

    pub(crate) fn node_has_compositor_animation_frame(
        &self,
        id: NodeSlotId,
        kind: super::node_data::CompositorAnimationFrameKind,
    ) -> bool {
        self.data(id).compositor_animation_frame_kinds.get() & kind as u8 != 0
    }

    pub(crate) fn set_node_needs_compositor_animation_frame(
        &self,
        id: NodeSlotId,
        kind: super::node_data::CompositorAnimationFrameKind,
        value: bool,
    ) {
        self.assert_owner_thread();
        let frame_kinds = &self.data(id).compositor_animation_frame_kinds;
        let mut updated = frame_kinds.get();
        if value {
            updated |= kind as u8;
        } else {
            updated &= !(kind as u8);
        }
        frame_kinds.set(updated);
    }

    pub(crate) fn for_each_node_in_layout_subtree_in_pre_order(
        &self,
        root: NodeSlotId,
        mut callback: impl FnMut(NodeSlotId),
    ) {
        self.for_each_node_in_layout_subtree_in_pre_order_with_pruning(root, |node| {
            callback(node);
            true
        });
    }

    pub(crate) fn for_each_node_in_layout_subtree_in_pre_order_with_pruning(
        &self,
        root: NodeSlotId,
        mut visit_node_and_report_whether_to_descend: impl FnMut(NodeSlotId) -> bool,
    ) {
        let mut current = root;
        loop {
            let descend_into_children = visit_node_and_report_whether_to_descend(current);
            let data = self.data(current);
            let (parent, first_child, next_sibling) =
                { (data.parent.get(), data.first_child.get(), data.next_sibling.get()) };

            if descend_into_children && !first_child.is_invalid() {
                current = first_child;
                continue;
            }
            if current == root {
                break;
            }
            if !next_sibling.is_invalid() {
                current = next_sibling;
                continue;
            }

            current = parent;
            while current != root {
                let data = self.data(current);
                let next_sibling = data.next_sibling.get();
                if !next_sibling.is_invalid() {
                    current = next_sibling;
                    break;
                }
                current = data.parent.get();
            }
            if current == root {
                break;
            }
        }
    }

    fn remove_layout_update_flag_node(&self, node: NodeSlotId) {
        let mut indices = self.layout_update_flag_node_indices.borrow_mut();
        let Some(index) = indices.remove(&node) else {
            return;
        };
        let mut nodes = self.nodes_with_layout_update_flags.borrow_mut();
        nodes.swap_remove(index);
        if let Some(&moved_node) = nodes.get(index) {
            *indices.get_mut(&moved_node).unwrap() = index;
        }
    }

    pub(crate) fn reset_layout_update_flags_in_subtree(&self, root: NodeSlotId) {
        self.assert_owner_thread();
        let flags_to_clear = NodeFlag::NeedsLayoutUpdate as u32 | NodeFlag::NeedsOwnGeometryUpdate as u32;
        if self.data(root).kind.get() != NodeKind::Viewport {
            // NB: A partial-relayout batch commits each independent boundary separately.
            // Scanning the document's dirty list per boundary would make cleanup quadratic.
            self.for_each_node_in_layout_subtree_in_pre_order(root, |node| {
                let data = self.data(node);
                data.flags.set(data.flags.get() & !flags_to_clear);
                self.remove_layout_update_flag_node(node);
            });
            return;
        }

        // NB: Dirty nodes share ancestor chains. Cache membership so a deeply nested
        // dirty chain is checked once, while detached dirty subtrees remain pending.
        let mut membership = HashMap::default();
        membership.insert(root, true);
        membership.insert(NodeSlotId::INVALID, false);
        let mut ancestors = Vec::new();
        let mut index = 0;
        loop {
            let Some(node) = self.nodes_with_layout_update_flags.borrow().get(index).copied() else {
                break;
            };
            let mut ancestor = node;
            while !membership.contains_key(&ancestor) {
                ancestors.push(ancestor);
                ancestor = self.data(ancestor).parent.get();
                #[cfg(test)]
                self.layout_update_flag_ancestor_visits
                    .set(self.layout_update_flag_ancestor_visits.get() + 1);
            }
            let is_in_subtree = membership[&ancestor];
            for ancestor in ancestors.drain(..) {
                membership.insert(ancestor, is_in_subtree);
            }
            if !is_in_subtree {
                index += 1;
                continue;
            }
            let data = self.data(node);
            data.flags.set(data.flags.get() & !flags_to_clear);
            self.remove_layout_update_flag_node(node);
        }
    }

    fn node_is_capable_of_forming_a_containing_block(&self, id: NodeSlotId) -> bool {
        let data = self.data(id);
        super::node_facts::node_forms_containing_block_for_children(data, self.node_style_if_live(id))
    }

    fn nearest_ancestor_capable_of_forming_a_containing_block(&self, node: NodeSlotId) -> NodeSlotId {
        let mut ancestor = self.data(node).parent.get();
        while !ancestor.is_invalid() {
            if self.node_is_capable_of_forming_a_containing_block(ancestor) {
                return ancestor;
            }
            ancestor = self.data(ancestor).parent.get();
        }
        NodeSlotId::INVALID
    }

    /// Returns whether the node's ancestor facts changed.
    fn derive_ancestor_facts_for_node(&self, node: NodeSlotId) -> bool {
        let data = self.data(node);
        let parent = data.parent.get();
        let mut facts = 0;
        if !parent.is_invalid() {
            let parent_data = self.data(parent);
            let parent_style = super::node_facts::node_style_view(parent_data);
            if super::node_facts::node_is_flex_or_grid_container(parent_style) {
                facts |= AncestorFact::ParentIsFlexOrGridContainer as u8;
            }
            if parent_style.is_none_or(|style| {
                !style.is_floating() && (style.display().is_flow_inside() || style.display().is_flow_root_inside())
            }) {
                facts |= AncestorFact::ParentIsUnfloatedFlowContainer as u8;
            }
            if super::node_facts::has_flag(data, NodeFlag::Anonymous) {
                if super::node_facts::has_flag(parent_data, NodeFlag::UsesButtonLayout) {
                    facts |= AncestorFact::IsAnonymousButtonContentWrapper as u8;
                }
                if super::node_facts::has_ancestor_fact(parent_data, AncestorFact::IsAnonymousButtonContentWrapper) {
                    facts |= AncestorFact::IsAnonymousButtonContentBox as u8;
                }
                let inherits_text_overflow_ellipsis = if super::node_facts::has_flag(parent_data, NodeFlag::Anonymous) {
                    super::node_facts::has_ancestor_fact(parent_data, AncestorFact::InheritsTextOverflowEllipsis)
                } else {
                    super::node_facts::node_applies_text_overflow_ellipsis(parent_style)
                };
                if inherits_text_overflow_ellipsis {
                    facts |= AncestorFact::InheritsTextOverflowEllipsis as u8;
                }
            }
            if super::node_facts::has_ancestor_fact(parent_data, AncestorFact::HasInlineLevelInclusiveAncestor) {
                facts |= AncestorFact::HasInlineLevelInclusiveAncestor as u8;
            }
        }
        if super::node_facts::node_is_inline_outside(super::node_facts::node_style_view(data)) {
            facts |= AncestorFact::HasInlineLevelInclusiveAncestor as u8;
        }
        data.ancestor_facts.replace(facts) != facts
    }

    /// A style change reaches the anonymous boxes below the node without rebuilding them, and
    /// they take some of their ancestor facts from it.
    fn refresh_ancestor_facts_of_anonymous_children(&self, parent: NodeSlotId) {
        let mut child = self.data(parent).first_child.get();
        while !child.is_invalid() {
            let data = self.data(child);
            if super::node_facts::has_flag(data, NodeFlag::Anonymous) && self.derive_ancestor_facts_for_node(child) {
                self.bump_fragment_cache_epoch_of_self_and_ancestors(child);
                self.refresh_ancestor_facts_of_anonymous_children(child);
            }
            child = data.next_sibling.get();
        }
    }

    /// Returns every attached subtree root the derivation visited.
    pub(crate) fn derive_facts_after_tree_update(&self, rebuilt_roots: &[NodeSlotId]) -> HashSet<NodeSlotId> {
        // NB: Anonymous wrappers, generated content, and table fixup can attach nodes
        // outside the builder's reported rebuild roots. Include every attached subtree.
        let mut pending = self.pending_attached_subtree_roots.borrow_mut();
        let roots: HashSet<_> = pending
            .drain(..)
            .chain(rebuilt_roots.iter().copied())
            .filter(|&root| self.slot_is_live(root))
            .collect();
        drop(pending);
        for &root in &roots {
            let mut ancestor = self.data(root).parent.get();
            if ancestor.is_invalid() && self.data(root).kind.get() != NodeKind::Viewport {
                continue;
            }
            while !ancestor.is_invalid() && !roots.contains(&ancestor) {
                ancestor = self.data(ancestor).parent.get();
            }
            if ancestor.is_invalid() {
                self.derive_facts_in_subtree(root);
            }
        }
        roots
    }

    /// Derives what the nodes in the inclusive subtree of `root` take from their ancestors: whether they
    /// establish containing blocks, and their ancestor facts, which the pre-order walk finds already derived
    /// for the parent. Out-of-flow boxes in the subtree also mark the ancestors they escape.
    pub(crate) fn derive_facts_in_subtree(&self, root: NodeSlotId) {
        self.assert_owner_thread();
        self.for_each_node_in_layout_subtree_in_pre_order(root, |node| {
            self.derive_containing_block_establishment_flags(node);
            self.mark_nodes_escaped_by_attached_out_of_flow_box(node);
            self.derive_ancestor_facts_for_node(node);
        });
    }

    fn slot_for_data(&self, data: &NodeData) -> (u32, SlotMetadata) {
        let data_address = std::ptr::from_ref(data) as usize;
        let slot_size = size_of::<NodeData>();

        let address_index = self
            .chunks_by_address
            .partition_point(|address| address.start <= data_address);
        assert_ne!(
            address_index, 0,
            "layout node data pointer does not belong to this arena"
        );
        let address = self.chunks_by_address[address_index - 1];
        let chunk_end = address.start + size_of::<[NodeData; SLOTS_PER_CHUNK]>();
        assert!(
            data_address < chunk_end,
            "layout node data pointer does not belong to this arena"
        );

        let offset = data_address - address.start;
        assert_eq!(offset % slot_size, 0, "unaligned layout node arena data pointer");
        let index = address.chunk_index * SLOTS_PER_CHUNK + offset / slot_size;
        let index = u32::try_from(index).expect("layout node arena slot index overflowed");
        let metadata = *self.metadata(index);
        assert!(metadata.occupied, "layout node arena access for an unused slot");
        let generation = data.slot_generation.get();
        assert_eq!(
            generation, metadata.generation,
            "layout node arena access used a stale slot"
        );
        (index, metadata)
    }

    pub(crate) fn node_pre_order_label(&self, id: NodeSlotId) -> u64 {
        let _ = self.data(id);
        self.pre_order_labels[id.slot_index() as usize].get()
    }

    fn set_node_pre_order_label(&self, id: NodeSlotId, label: u64) {
        self.pre_order_labels[id.slot_index() as usize].set(label);
    }

    pub(crate) fn pre_order_relabel_count(&self) -> u64 {
        self.pre_order_relabel_count.get()
    }

    pub(crate) fn count_nodes_in_layout_subtree(&self, root: NodeSlotId) -> u64 {
        let mut count = 0u64;
        self.for_each_node_in_layout_subtree_in_pre_order(root, |_| count += 1);
        count
    }

    fn last_descendant_in_pre_order(&self, node: NodeSlotId) -> NodeSlotId {
        let mut current = node;
        loop {
            let last_child = self.data(current).last_child.get();
            if last_child.is_invalid() {
                return current;
            }
            current = last_child;
        }
    }

    pub(super) fn pre_order_label_of_subtree_successor(&self, node: NodeSlotId) -> u64 {
        let mut current = node;
        loop {
            let data = self.data(current);
            let (parent, next_sibling) = { (data.parent.get(), data.next_sibling.get()) };
            if !next_sibling.is_invalid() {
                return self.node_pre_order_label(next_sibling);
            }
            if parent.is_invalid() {
                return u64::MAX;
            }
            current = parent;
        }
    }

    fn assign_pre_order_labels_to_inserted_subtree(&self, parent: NodeSlotId, child: NodeSlotId) {
        let child_data = self.data(child);
        let (previous_sibling, next_sibling) = { (child_data.previous_sibling.get(), child_data.next_sibling.get()) };
        let lower = if previous_sibling.is_invalid() {
            self.node_pre_order_label(parent)
        } else {
            self.node_pre_order_label(self.last_descendant_in_pre_order(previous_sibling))
        };
        let upper = if next_sibling.is_invalid() {
            self.pre_order_label_of_subtree_successor(parent)
        } else {
            self.node_pre_order_label(next_sibling)
        };
        debug_assert!(lower < upper, "pre-order labels lost their strict order");
        let inserted_node_count = self.count_nodes_in_layout_subtree(child);
        let stride = ((upper - lower) / (inserted_node_count + 1)).min(MAXIMUM_PRE_ORDER_LABEL_STRIDE);
        if stride >= 2 {
            // Placement is biased toward the insertion direction, so a one-directional hot
            // spot consumes the gap linearly instead of halving it.
            let mut position_in_subtree = 0u64;
            self.for_each_node_in_layout_subtree_in_pre_order(child, |node| {
                position_in_subtree += 1;
                let label = if next_sibling.is_invalid() {
                    lower + stride * position_in_subtree
                } else {
                    upper - stride * (inserted_node_count + 1 - position_in_subtree)
                };
                self.set_node_pre_order_label(node, label);
            });
            debug_assert!(lower < self.node_pre_order_label(child));
            debug_assert!(self.node_pre_order_label(self.last_descendant_in_pre_order(child)) < upper);
            return;
        }
        let mut ancestor = parent;
        loop {
            let ancestor_parent = self.data(ancestor).parent.get();
            if ancestor_parent.is_invalid() {
                self.set_node_pre_order_label(ancestor, 0);
                let spread_succeeded = self.spread_pre_order_labels_evenly_over_descendants(ancestor, 0, u64::MAX);
                assert!(spread_succeeded, "pre-order label space exhausted");
                return;
            }
            let ancestor_lower = self.node_pre_order_label(ancestor);
            let ancestor_upper = self.pre_order_label_of_subtree_successor(ancestor);
            if self.spread_pre_order_labels_evenly_over_descendants(ancestor, ancestor_lower, ancestor_upper) {
                return;
            }
            ancestor = ancestor_parent;
        }
    }

    fn spread_pre_order_labels_evenly_over_descendants(
        &self,
        subtree_root: NodeSlotId,
        lower: u64,
        upper: u64,
    ) -> bool {
        let descendant_count = self.count_nodes_in_layout_subtree(subtree_root) - 1;
        if descendant_count == 0 {
            return true;
        }
        let step = (upper - lower) / (descendant_count + 1);
        // NB: Merely restoring strict order can leave the subtree dense enough to need another
        //     relabel on the next insertion. Restore the normal insertion spacing, or try a larger
        //     ancestor. The root already uses all available label space, so it cannot expand further.
        if step < MAXIMUM_PRE_ORDER_LABEL_STRIDE && !self.data(subtree_root).parent.get().is_invalid() {
            return false;
        }
        assert!(step >= 2, "pre-order label space exhausted");
        let mut position_in_subtree = 0u64;
        self.for_each_node_in_layout_subtree_in_pre_order(subtree_root, |node| {
            if node == subtree_root {
                return;
            }
            position_in_subtree += 1;
            self.set_node_pre_order_label(node, lower + step * position_in_subtree);
        });
        self.pre_order_relabel_count.set(self.pre_order_relabel_count.get() + 1);
        true
    }

    #[cfg(debug_assertions)]
    fn nodes_share_a_layout_tree_root(&self, node: NodeSlotId, other: NodeSlotId) -> bool {
        let root_of = |mut slot: NodeSlotId| loop {
            let parent = self.data(slot).parent.get();
            if parent.is_invalid() {
                return slot;
            }
            slot = parent;
        };
        root_of(node) == root_of(other)
    }

    pub(crate) fn is_before(&self, node: &NodeData, other: &NodeData) -> bool {
        let (node_index, node_metadata) = self.slot_for_data(node);
        let (other_index, other_metadata) = self.slot_for_data(other);
        let node = NodeSlotId::new(node_index, node_metadata.generation);
        let other = NodeSlotId::new(other_index, other_metadata.generation);
        assert_ne!(node, other, "a layout node cannot precede itself");
        #[cfg(debug_assertions)]
        debug_assert!(
            self.nodes_share_a_layout_tree_root(node, other),
            "layout nodes belong to different trees"
        );
        self.node_pre_order_label(node) < self.node_pre_order_label(other)
    }

    pub(crate) fn note_table_cell_measurement_cache_miss(&self) {
        self.table_cell_measurement_cache_misses
            .set(self.table_cell_measurement_cache_misses.get() + 1);
    }

    pub(crate) fn table_cell_measurement_cache_miss_count(&self) -> u64 {
        self.table_cell_measurement_cache_misses.get()
    }

    pub(crate) fn note_intrinsic_inline_measurement(&self) {
        self.intrinsic_inline_measurements
            .set(self.intrinsic_inline_measurements.get() + 1);
    }

    pub(crate) fn intrinsic_inline_measurement_count(&self) -> u64 {
        self.intrinsic_inline_measurements.get()
    }

    pub(crate) fn note_intrinsic_measurement(&self) {
        self.intrinsic_measurements.set(self.intrinsic_measurements.get() + 1);
    }

    pub(crate) fn intrinsic_measurement_count(&self) -> u64 {
        self.intrinsic_measurements.get()
    }

    pub(crate) fn saved_abspos_layout_inputs(&self, data: &NodeData) -> Option<AbsposLayoutInputs> {
        let (index, metadata) = self.slot_for_data(data);
        self.paintable_rows
            .with_committed_fragment_link(index, metadata.generation, |link| {
                link.and_then(|link| link.abspos_layout_inputs)
            })
            .or_else(|| {
                self.confined_abspos_layout_inputs
                    .borrow()
                    .get(&NodeSlotId::new(index, metadata.generation))
                    .copied()
            })
    }

    pub(crate) fn set_default_scroll_shift(
        &self,
        id: NodeSlotId,
        anchor: NodeSlotId,
        compensates_for_horizontal_scroll: bool,
        compensates_for_vertical_scroll: bool,
    ) {
        let anchor_is_live = self.slot_is_live(anchor);
        let anchor = if anchor_is_live { anchor } else { NodeSlotId::INVALID };
        let compensates_for_horizontal_scroll = anchor_is_live && compensates_for_horizontal_scroll;
        let compensates_for_vertical_scroll = anchor_is_live && compensates_for_vertical_scroll;
        let previous_flags = self.node_flags_if_live(id);
        let scroll_shift_inputs_changed = self.default_scroll_shift_anchor(id) != anchor
            || (previous_flags & NodeFlag::CompensatesForHorizontalScroll as u32 != 0)
                != compensates_for_horizontal_scroll
            || (previous_flags & NodeFlag::CompensatesForVerticalScroll as u32 != 0) != compensates_for_vertical_scroll;
        {
            let mut slots = self.default_scroll_shift_anchors.borrow_mut();
            let index = id.slot_index() as usize;
            if anchor_is_live {
                if slots.len() <= index {
                    slots.resize_with(index + 1, DefaultScrollShiftAnchorSlot::default);
                }
                slots[index] = DefaultScrollShiftAnchorSlot {
                    generation: id.generation(),
                    anchor,
                };
            } else if let Some(slot) = slots.get_mut(index) {
                *slot = DefaultScrollShiftAnchorSlot::default();
            }
        }
        self.set_node_flag(
            id,
            NodeFlag::CompensatesForHorizontalScroll,
            compensates_for_horizontal_scroll,
        );
        self.set_node_flag(
            id,
            NodeFlag::CompensatesForVerticalScroll,
            compensates_for_vertical_scroll,
        );
        if anchor_is_live {
            self.any_default_scroll_shift_anchor_ever_stored.set(true);
        }
        if scroll_shift_inputs_changed {
            self.note_visual_context_box_dirty(
                id,
                crate::painting::visual_context::dirty::VisualContextBoxDirtyKind::DefaultScrollShiftInputsChanged,
            );
        }
    }

    pub(crate) fn default_scroll_shift_anchor(&self, id: NodeSlotId) -> NodeSlotId {
        if !self.any_default_scroll_shift_anchor_ever_stored.get() {
            return NodeSlotId::INVALID;
        }
        let slots = self.default_scroll_shift_anchors.borrow();
        let Some(slot) = slots.get(id.slot_index() as usize) else {
            return NodeSlotId::INVALID;
        };
        if slot.generation != id.generation() || !self.slot_is_live(slot.anchor) {
            return NodeSlotId::INVALID;
        }
        slot.anchor
    }

    pub(crate) fn may_have_default_scroll_shift_anchor(&self) -> bool {
        self.any_default_scroll_shift_anchor_ever_stored.get()
    }

    pub(crate) fn for_each_default_scroll_shift_anchor(&self, mut visit: impl FnMut(NodeSlotId, NodeSlotId)) {
        if !self.any_default_scroll_shift_anchor_ever_stored.get() {
            return;
        }
        let slots = self.default_scroll_shift_anchors.borrow();
        for (index, slot) in slots.iter().enumerate() {
            if slot.anchor.is_invalid() || slot.generation == 0 {
                continue;
            }
            let positioned = NodeSlotId::new(index as u32, slot.generation);
            if self.slot_is_live(positioned) && self.slot_is_live(slot.anchor) {
                visit(positioned, slot.anchor);
            }
        }
    }

    pub(crate) fn committed_fragment_link(&self, data: &NodeData) -> Option<super::fragment_tree::FragmentLink> {
        let (index, metadata) = self.slot_for_data(data);
        let link = self
            .paintable_rows
            .committed_fragment_link_cloned(index, metadata.generation);

        let flags = data.flags.get();
        assert_eq!(
            flags & NodeFlag::HasCommittedFragmentLink as u32 != 0,
            link.is_some(),
            "committed fragment link presence flag disagrees with the arena side table"
        );
        link
    }

    pub(crate) fn set_committed_fragment_link(
        &self,
        data: &NodeData,
        link: super::fragment_tree::FragmentLink,
        geometry_epoch: Option<u32>,
    ) {
        let (index, metadata) = self.slot_for_data(data);
        self.paintable_rows
            .set_committed_fragment_link(index, metadata.generation, geometry_epoch, link);

        data.flags
            .set(data.flags.get() | NodeFlag::HasCommittedFragmentLink as u32);
    }

    pub(crate) fn epoch_of_geometry_laid_out_in_this_pass(&self, data: &NodeData) -> Option<u32> {
        (!self.fragment_cache_epoch_changed_during_layout_pass.get()).then(|| data.fragment_cache_epoch.get())
    }

    pub(crate) fn with_current_committed_fragment<R>(
        &self,
        node: NodeSlotId,
        read: impl FnOnce(&super::fragment_tree::Fragment) -> R,
    ) -> Option<R> {
        self.paintable_rows.with_current_committed_fragment(
            node.slot_index(),
            node.generation(),
            self.data(node).fragment_cache_epoch.get(),
            read,
        )
    }

    pub(crate) fn take_committed_fragment_link(&self, data: &NodeData) -> Option<super::fragment_tree::FragmentLink> {
        let (index, metadata) = self.slot_for_data(data);
        let link = self
            .paintable_rows
            .take_committed_fragment_link(index, metadata.generation);

        assert_eq!(
            data.flags.get() & NodeFlag::HasCommittedFragmentLink as u32 != 0,
            link.is_some(),
            "committed fragment link presence flag disagrees with the arena side table"
        );
        data.flags
            .set(data.flags.get() & !(NodeFlag::HasCommittedFragmentLink as u32));
        link
    }

    pub(crate) fn clear_committed_fragment_link(&self, id: NodeSlotId) {
        // Cached runs may reuse the committed paintable subtree without replaying its fragments.
        // Once that subtree is cleared, a later layout must rebuild it instead.
        self.fc_run_cache_store.remove_entry(id.slot_index());
        drop(self.take_committed_fragment_link(self.data(id)));
    }

    fn text_node_state_mut(&mut self, id: NodeSlotId) -> &mut TextNodeState {
        self.assert_owner_thread();
        self.data(id);
        let index = id.slot_index() as usize;
        if self.text_nodes.len() <= index {
            self.text_nodes.resize_with(index + 1, TextNodeSlot::default);
        }
        let slot = &mut self.text_nodes[index];
        if slot.generation != id.generation() {
            *slot = TextNodeSlot {
                generation: id.generation(),
                ..TextNodeSlot::default()
            };
        }
        slot.state.get_or_insert_with(Default::default)
    }

    fn text_node_state(&self, id: NodeSlotId) -> Option<&TextNodeState> {
        if !self.slot_is_live(id) {
            return None;
        }
        self.text_nodes
            .get(id.slot_index() as usize)
            .filter(|slot| slot.generation == id.generation())
            .and_then(|slot| slot.state.as_deref())
    }

    pub(crate) fn set_text_content(&mut self, id: NodeSlotId, content: TextContent) {
        let state = self.text_node_state_mut(id);
        if let Some(previous) = state.content.as_mut()
            && previous.has_same_content_as(&content)
        {
            previous.rendering_key = content.rendering_key;
            return;
        }
        state.content = Some(content);
        self.searchable_text = None;
        // Publication can happen through a C++ text read before the enrolled
        // sync runs. Invalidate here so every publication invalidates layout,
        // including mapping-only changes with identical rendered code units.
        self.bump_fragment_cache_epoch_of_self_and_ancestors(id);
    }

    pub(super) fn invalidate_text_content(&mut self, id: NodeSlotId) {
        self.data(id);
        if let Some(slot) = self.text_nodes.get_mut(id.slot_index() as usize)
            && slot.generation == id.generation()
            && let Some(state) = slot.state.as_mut()
            && let Some(content) = state.content.as_mut()
        {
            content.rendering_key = None;
        }
        self.enroll_text_node_for_content_sync(id);
    }

    pub(super) fn finish_text_content_sync(&self, id: NodeSlotId) {
        self.text_nodes_enrolled_for_content_sync.borrow_mut().remove(&id);
    }

    pub(super) fn pending_text_nodes_for_content_sync(&self) -> Vec<NodeSlotId> {
        self.text_nodes_enrolled_for_content_sync
            .borrow()
            .iter()
            .copied()
            .collect()
    }

    pub(super) fn text_content_needs_sync(&self, id: NodeSlotId) -> bool {
        self.text_nodes_enrolled_for_content_sync.borrow().contains(&id)
            || !self
                .text_content(id)
                .is_some_and(|content| content.rendering_key.is_some())
    }

    pub(crate) fn set_replaced_content_facts(&mut self, id: NodeSlotId, facts: FfiReplacedContentFacts) -> bool {
        self.assert_owner_thread();
        self.data(id);
        let index = id.slot_index() as usize;
        if self.replaced_content_facts.len() <= index {
            self.replaced_content_facts
                .resize_with(index + 1, ReplacedContentFactsSlot::default);
        }
        let previous = &self.replaced_content_facts[index];
        let changed = previous.generation != id.generation() || previous.facts != Some(facts);
        self.replaced_content_facts[index] = ReplacedContentFactsSlot {
            generation: id.generation(),
            facts: Some(facts),
        };
        changed
    }

    pub(crate) fn replaced_content_facts(&self, id: NodeSlotId) -> Option<FfiReplacedContentFacts> {
        assert!(!id.is_invalid(), "invalid layout node arena slot ID");
        self.replaced_content_facts
            .get(id.slot_index() as usize)
            .filter(|slot| slot.generation == id.generation())
            .and_then(|slot| slot.facts)
    }

    pub(crate) fn set_raw_table_column_span(&mut self, id: NodeSlotId, value: u32) -> u32 {
        self.assert_owner_thread();
        self.data(id);
        if value == 1 {
            self.raw_table_column_spans.remove(&id).unwrap_or(1)
        } else {
            self.raw_table_column_spans.insert(id, value).unwrap_or(1)
        }
    }

    pub(crate) fn raw_table_column_span(&self, id: NodeSlotId) -> u32 {
        // data() validates that id names a live slot with a matching generation.
        self.data(id);
        self.raw_table_column_spans.get(&id).copied().unwrap_or(1)
    }

    pub(crate) fn text_content(&self, id: NodeSlotId) -> Option<&TextContent> {
        self.text_node_state(id)?.content.as_ref()
    }

    /// Record what a generated text row spells, as the build resolved it.
    pub(crate) fn set_generated_text(&mut self, id: NodeSlotId, text: ak::Utf16String) {
        self.text_node_state_mut(id).generated_text = Some(text);
    }

    fn generated_text(&self, id: NodeSlotId) -> Option<&ak::Utf16String> {
        self.text_node_state(id)?.generated_text.as_ref()
    }

    /// Everything a text row renders from. Generated content carries its own characters; a row
    /// bound to a DOM text node reads the characters and facts the style mirror publishes for it.
    pub(crate) fn published_text_source(&self, id: NodeSlotId, uses_locale: bool) -> PublishedTextSource {
        if self.data(id).kind.get() == NodeKind::GeneratedTextNode {
            return PublishedTextSource {
                data: self.generated_text(id).cloned().unwrap_or_default(),
                locale: uses_locale.then(|| self.generated_text_language_tag(id)).flatten(),
                is_password_input: false,
            };
        }
        let Some(style_node) = self.node_style_node(id).filter(|style_node| style_node.is_text()) else {
            return PublishedTextSource::default();
        };
        self.with_style_engine(|engine| engine.published_text_source(style_node, uses_locale))
    }

    /// The language tag a generated text row's transform reads: the one the element the content
    /// was generated for resolves to. The row is either the pseudo-element's own box or a child of
    /// it, and a generated row under an ordinary box reads no tag at all.
    fn generated_text_language_tag(&self, id: NodeSlotId) -> Option<Vec<u16>> {
        let generator = if self.node_is_generated_for_pseudo_element(id) {
            self.node_style_node(id)
        } else {
            let parent = self.data(id).parent.get();
            match !parent.is_invalid() && self.node_is_generated_for_pseudo_element(parent) {
                true => self.node_style_node(parent),
                false => None,
            }
        }?;
        let tag = self.with_style_engine(|engine| engine.element_language_tag(generator).to_vec());
        (!tag.is_empty()).then_some(tag)
    }

    pub(super) fn set_first_letter_slices(
        &mut self,
        first_letter: NodeSlotId,
        remainder: NodeSlotId,
        letter_end: usize,
        source_length: usize,
    ) {
        assert_ne!(first_letter, remainder);
        assert_eq!(self.data(first_letter).kind.get(), NodeKind::TextNode);
        assert_eq!(self.data(remainder).kind.get(), NodeKind::TextNode);
        assert!(letter_end <= source_length);
        self.text_node_state_mut(first_letter).source_range = Some(FfiTextSourceRange {
            start: 0,
            length: letter_end,
        });
        let remainder_state = self.text_node_state_mut(remainder);
        remainder_state.source_range = Some(FfiTextSourceRange {
            start: letter_end,
            length: source_length - letter_end,
        });
        remainder_state.first_letter = first_letter;
        self.invalidate_text_content(first_letter);
        self.invalidate_text_content(remainder);
    }

    pub(crate) fn text_source_range(&self, id: NodeSlotId, source_length: usize) -> FfiTextSourceRange {
        self.data(id);
        self.text_node_state(id)
            .and_then(|state| state.source_range)
            .unwrap_or(FfiTextSourceRange {
                start: 0,
                length: source_length,
            })
    }

    pub(super) fn text_has_source_range(&self, id: NodeSlotId) -> bool {
        self.text_node_state(id)
            .is_some_and(|state| state.source_range.is_some())
    }

    pub(crate) fn text_fragments(&self, primary: NodeSlotId) -> TextFragments {
        let mut fragments = TextFragments {
            nodes: [NodeSlotId::INVALID; 2],
            length: 0,
        };
        if !self.slot_is_live(primary) || !super::node_facts::kind_is_text(self.data(primary).kind.get()) {
            return fragments;
        }
        if let Some(state) = self.text_node_state(primary)
            && self.slot_is_live(state.first_letter)
        {
            fragments.nodes[0] = state.first_letter;
            fragments.length = 1;
        }
        fragments.nodes[fragments.length] = primary;
        fragments.length += 1;
        fragments
    }

    /// The node's group payload pointer array, read in place from the
    /// Rust-owned style container that NodeData.style addresses. The node's
    /// retained immutable ComputedValues owns the container, and the pointer
    /// is only replaced between passes, so the array stays valid for as long
    /// as the node occupies its arena slot.
    pub(crate) fn style_payloads(&self, id: NodeSlotId) -> Option<&FfiStylePayloads> {
        let style = self.data(id).style.get();
        // SAFETY: A non-null style pointer addresses the container's group
        // pointer array, which FfiStylePayloads mirrors exactly.
        (!style.is_null()).then(|| unsafe { &*style.as_ptr().cast::<FfiStylePayloads>() })
    }

    // OPTIMIZATION: The edit invalidates line data at its direct parent and every formatting
    // ancestor. Preserve the structural proof along the same unbounded path as the fragment
    // epoch bumps so each affected inline context can reuse its unchanged line prefix.
    // NB: Bumps can legitimately run while another document's layout pass is on the stack (a
    // parent pass sizing a child navigable's viewport invalidates the child document), so the
    // helpers must not assert against the process-global pass flag. A bump landing between a
    // run's probe and its store is handled by storing the probe-time validity, which turns it
    // into a fail-safe miss.
    fn invalidate_at_and_above(&self, mut node: NodeSlotId, invalidation: AncestorInvalidation) {
        let epochs_enabled =
            super::fc_run_cache::fc_run_cache_mode_from_environment() != super::fc_run_cache::FcRunCacheMode::Disabled;
        let paintable_rows = self.paintable_rows();
        while !node.is_invalid() {
            let data = self.data(node);
            if invalidation == AncestorInvalidation::StructuralChange {
                self.fc_run_cache_store.note_inline_layout_damage(node);
            }
            if epochs_enabled {
                self.bump_fragment_cache_epoch(node);
            }
            let (kind, parent) = (data.kind.get(), data.parent.get());
            if super::node_facts::kind_is_box(kind) {
                paintable_rows.clear_cached_overflow_data(node);
            }
            node = parent;
        }
    }

    fn bump_fragment_cache_epoch(&self, node: NodeSlotId) {
        let data = self.data(node);
        let epoch = data.fragment_cache_epoch.get().wrapping_add(1);
        data.fragment_cache_epoch.set(epoch);
        if epoch == 0 {
            self.paintable_rows.invalidate_committed_geometry(node.slot_index());
        }
        if self.layout_pass_is_running() {
            self.fragment_cache_epoch_changed_during_layout_pass.set(true);
        }
        self.fc_run_cache_store.note_invalidated_entry(node);
    }

    pub(super) fn bump_fragment_cache_epoch_below_bumped_parent(&self, child: NodeSlotId) {
        self.bump_fragment_cache_epoch(child);
    }

    pub(crate) fn note_structural_change_at_and_above(&self, node: NodeSlotId) {
        if !self.scrollable_overflow.non_child_boxes.borrow().is_empty() {
            self.scrollable_overflow.contained_boxes_dirty.set(true);
        }
        self.invalidate_at_and_above(node, AncestorInvalidation::StructuralChange);
    }

    pub(crate) fn bump_fragment_cache_epoch_of_self_and_ancestors(&self, node: NodeSlotId) {
        self.invalidate_at_and_above(node, AncestorInvalidation::ContentChange);
    }

    pub(crate) fn insert_child(&self, parent: NodeSlotId, child: NodeSlotId, before: NodeSlotId) {
        self.assert_owner_thread();
        assert_ne!(parent, child, "a layout node cannot become its own child");
        let parent_data = self.data(parent);
        let child_data = self.data(child);

        let child_parent = child_data.parent.get();
        let child_previous_sibling = child_data.previous_sibling.get();
        let child_next_sibling = child_data.next_sibling.get();
        assert!(
            child_parent.is_invalid(),
            "inserted layout node is still linked to a parent"
        );
        assert!(
            child_previous_sibling.is_invalid(),
            "inserted layout node is still linked to a previous sibling"
        );
        assert!(
            child_next_sibling.is_invalid(),
            "inserted layout node is still linked to a next sibling"
        );
        debug_assert_eq!(
            parent_data.first_child.get().is_invalid(),
            parent_data.last_child.get().is_invalid(),
            "layout node child list endpoints disagree"
        );

        #[cfg(debug_assertions)]
        {
            let mut ancestor = parent;
            while !ancestor.is_invalid() {
                assert_ne!(ancestor, child, "layout node insertion would create a cycle");
                ancestor = self.data(ancestor).parent.get();
            }
        }

        let previous = if before.is_invalid() {
            parent_data.last_child.get()
        } else {
            assert_ne!(before, child, "a layout node cannot be inserted before itself");
            let before_data = self.data(before);
            assert_eq!(
                before_data.parent.get(),
                parent,
                "insertion reference is not a child of the parent"
            );
            before_data.previous_sibling.get()
        };

        child_data.parent.set(parent);
        child_data.previous_sibling.set(previous);
        child_data.next_sibling.set(before);
        if previous.is_invalid() {
            parent_data.first_child.set(child);
        } else {
            self.data(previous).next_sibling.set(child);
        }
        if before.is_invalid() {
            parent_data.last_child.set(child);
        } else {
            self.data(before).previous_sibling.set(child);
        }

        self.assign_pre_order_labels_to_inserted_subtree(parent, child);
        self.note_layout_subtree_attached(child);
        self.pending_attached_subtree_roots.borrow_mut().push(child);
        self.note_structural_change_at_and_above(parent);
    }

    pub(crate) fn remove_child(&self, parent: NodeSlotId, child: NodeSlotId) {
        if self.paintable_row_count() > 0 {
            self.push_enclosing_paint_order_damage(child);
        }
        self.unlink_child(parent, child);
        self.note_structural_change_at_and_above(parent);
    }

    fn unlink_child(&self, parent: NodeSlotId, child: NodeSlotId) {
        self.assert_owner_thread();
        let parent_data = self.data(parent);
        let child_data = self.data(child);

        let child_parent = child_data.parent.get();
        assert_eq!(child_parent, parent, "removed layout node is not a child of the parent");
        let previous = child_data.previous_sibling.get();
        let next = child_data.next_sibling.get();

        if previous.is_invalid() {
            let first_child = parent_data.first_child.get();
            assert_eq!(first_child, child, "layout node child list lost its first child");
            parent_data.first_child.set(next);
        } else {
            let previous_data = self.data(previous);
            let previous_next_sibling = previous_data.next_sibling.get();
            assert_eq!(
                previous_next_sibling, child,
                "layout node sibling chain is inconsistent"
            );
            previous_data.next_sibling.set(next);
        }

        if next.is_invalid() {
            let last_child = parent_data.last_child.get();
            assert_eq!(last_child, child, "layout node child list lost its last child");
            parent_data.last_child.set(previous);
        } else {
            let next_data = self.data(next);
            let next_previous_sibling = next_data.previous_sibling.get();
            assert_eq!(
                next_previous_sibling, child,
                "layout node sibling chain is inconsistent"
            );
            next_data.previous_sibling.set(previous);
        }

        child_data.parent.set(NodeSlotId::INVALID);
        child_data.previous_sibling.set(NodeSlotId::INVALID);
        child_data.next_sibling.set(NodeSlotId::INVALID);
    }

    pub(crate) fn paint_state(&self) -> &RefCell<crate::painting::paint_state::PaintState> {
        &self.paint_state
    }

    pub(crate) fn recording_scratch(&self) -> &crate::painting::record::scratch::RecordingScratchSlot {
        &self.recording_scratch
    }

    pub(crate) fn node_flags_if_live(&self, id: NodeSlotId) -> u32 {
        if !self.slot_is_live(id) {
            return 0;
        }
        self.data(id).flags.get()
    }

    pub(crate) fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool {
        if !self.slot_is_live(id) {
            return false;
        }
        self.data(id).generated_for.get() != 0
    }

    pub(crate) fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind> {
        if !self.slot_is_live(id) {
            return None;
        }
        Some(self.data(id).kind.get())
    }

    pub(crate) fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        if !self.slot_is_live(id) {
            return None;
        }
        let parent = self.data(id).parent.get();
        (!parent.is_invalid()).then_some(parent)
    }

    pub(crate) fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        if !self.slot_is_live(id) {
            return None;
        }
        let child = self.data(id).first_child.get();
        (!child.is_invalid()).then_some(child)
    }

    pub(crate) fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        if !self.slot_is_live(id) {
            return None;
        }
        let sibling = self.data(id).next_sibling.get();
        (!sibling.is_invalid()).then_some(sibling)
    }

    pub(crate) fn node_data_if_live(&self, id: NodeSlotId) -> Option<&NodeData> {
        if !self.slot_is_live(id) {
            return None;
        }
        Some(self.data(id))
    }

    pub(crate) fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool {
        self.node_data_if_live(id)
            .is_some_and(|data| super::node_facts::node_is_out_of_flow(data, self.node_style_if_live(id)))
    }

    pub(crate) fn note_inline_box_lifted_out_of(&self, node: NodeSlotId, inline_box: Option<NodeSlotId>) {
        let mut lifted = self.inline_boxes_lifted_out_of.borrow_mut();
        match inline_box {
            Some(inline_box) => {
                lifted.insert(node, inline_box);
            }
            None => {
                lifted.remove(&node);
            }
        }
    }

    pub(crate) fn inline_box_lifted_out_of(&self, node: NodeSlotId) -> Option<NodeSlotId> {
        self.inline_boxes_lifted_out_of
            .borrow()
            .get(&node)
            .copied()
            .filter(|&inline_box| self.slot_is_live(inline_box))
    }

    pub(crate) fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        if !self.slot_is_live(id) {
            return None;
        }
        let block = self.containing_block_by_walking_ancestors(id);
        (!block.is_invalid()).then_some(block)
    }

    pub(crate) fn node_style_if_live(
        &self,
        id: NodeSlotId,
    ) -> Option<crate::css::computed_value_views::ComputedValuesView<'_>> {
        if !self.slot_is_live(id) {
            return None;
        }
        let payloads = self.style_payloads(id)?;
        Some(crate::css::computed_value_views::ComputedValuesView::new(
            &payloads.groups,
        ))
    }

    pub(crate) fn slot_is_live(&self, id: NodeSlotId) -> bool {
        if id.is_invalid() {
            return false;
        }
        self.slot_metadata
            .get(id.slot_index() as usize)
            .is_some_and(|metadata| metadata.occupied && metadata.generation == id.generation())
    }

    /// Whether the row was built for a DOM node: an element, a text node or the document. Anonymous
    /// boxes and generated content were not, so they have none. The node itself is named by the row's
    /// identity and resolved on the host side.
    pub(crate) fn node_is_dom_backed(&self, id: NodeSlotId) -> bool {
        self.node_data_if_live(id).is_some_and(|data| {
            // A slot that has not been given a shell yet stands for nothing at all, and its flags
            // do not say so.
            data.kind.get() != NodeKind::Unset && !crate::layout::node_facts::has_flag(data, NodeFlag::Anonymous)
        })
    }

    /// Whether the row was built for an element, as opposed to the document, a text node or nothing.
    pub(crate) fn node_is_element_backed(&self, id: NodeSlotId) -> bool {
        if !self.node_is_dom_backed(id) {
            return false;
        }
        let kind = self.data(id).kind.get();
        kind != NodeKind::Viewport && !crate::layout::node_facts::kind_is_text(kind)
    }

    pub(crate) fn previous_dom_backed_or_generated_node(
        &self,
        start: NodeSlotId,
        previous_sibling_only: bool,
    ) -> NodeSlotId {
        let mut current = start;
        loop {
            let data = self.data(current);
            current = if previous_sibling_only {
                data.previous_sibling.get()
            } else if data.previous_sibling.get().is_invalid() {
                data.parent.get()
            } else {
                let mut deepest_last_descendant = data.previous_sibling.get();
                loop {
                    let last_child = self.data(deepest_last_descendant).last_child.get();
                    if last_child.is_invalid() {
                        break;
                    }
                    deepest_last_descendant = last_child;
                }
                deepest_last_descendant
            };
            if current.is_invalid() {
                return NodeSlotId::INVALID;
            }
            if self.node_is_dom_backed(current) || self.data(current).generated_for.get() != 0 {
                return current;
            }
        }
    }

    pub(crate) fn enroll_text_children_for_content_sync(&self, parent: NodeSlotId) {
        let mut child = self.data(parent).first_child.get();
        while !child.is_invalid() {
            let data = self.data(child);
            if crate::layout::node_facts::kind_is_text(data.kind.get()) {
                self.enroll_text_node_for_content_sync(child);
            }
            child = data.next_sibling.get();
        }
    }

    pub(crate) fn live_slot_count(&self) -> u32 {
        self.live_count
    }

    pub(crate) fn shell_if_live(&self, main_thread: &crate::stage::MainThread, id: NodeSlotId) -> *mut c_void {
        if !self.slot_is_live(id) {
            return std::ptr::null_mut();
        }
        self.node_shell(main_thread, id)
    }

    pub(crate) fn node_link_slot(&self, id: NodeSlotId, link: FfiNodeLink) -> NodeSlotId {
        let data = self.data(id);
        match link {
            FfiNodeLink::Parent => data.parent.get(),
            FfiNodeLink::FirstChild => data.first_child.get(),
            FfiNodeLink::LastChild => data.last_child.get(),
            FfiNodeLink::PreviousSibling => data.previous_sibling.get(),
            FfiNodeLink::NextSibling => data.next_sibling.get(),
        }
    }

    pub(crate) fn node_link_shell(
        &self,
        main_thread: &crate::stage::MainThread,
        id: NodeSlotId,
        link: FfiNodeLink,
    ) -> *mut c_void {
        let linked = self.node_link_slot(id, link);
        if linked.is_invalid() {
            return std::ptr::null_mut();
        }
        self.node_shell(main_thread, linked)
    }

    pub(crate) fn node_containing_block_shell_if_live(
        &self,
        main_thread: &crate::stage::MainThread,
        id: NodeSlotId,
    ) -> *mut c_void {
        self.node_containing_block_if_live(id)
            .map_or(std::ptr::null_mut(), |containing_block| {
                self.shell_if_live(main_thread, containing_block)
            })
    }

    pub(crate) fn node_flags(&self, id: NodeSlotId) -> u32 {
        self.data(id).flags.get()
    }

    pub(crate) fn node_generated_for(&self, id: NodeSlotId) -> u8 {
        self.data(id).generated_for.get()
    }

    /// Owes the host `id`'s shell once the running build is over, so that the build itself only
    /// stamps rows. A reader that asks for the shell before then materialises it on demand.
    pub(crate) fn defer_shell(&self, id: NodeSlotId) {
        self.shells_owed_to_host.borrow_mut().push(id);
    }

    /// Owes the host the attachment of `id`'s style resources once the running frame is over.
    pub(crate) fn defer_style_resources(&self, id: NodeSlotId, owns_content_replacement_image: bool) {
        if owns_content_replacement_image {
            self.image_boxes_awaiting_owned_provider.borrow_mut().insert(id);
        }
        self.image_resources_owed_to_host.borrow_mut().push((
            id,
            OwedImageResources::StyleResources {
                owns_content_replacement_image,
            },
        ));
    }

    /// Whether the build about to run may build the viewport, which is what needs the document's
    /// style: there is no viewport row yet, the whole tree is to be rebuilt, or the document is.
    pub(crate) fn tree_build_may_create_viewport(&self, document_style_node: Option<StyleNodeID>) -> bool {
        self.bound_viewport_row().is_invalid()
            || self.needs_full_layout_tree_update()
            || self.needs_layout_tree_update(document_style_node)
    }

    /// Holds the document's style for the build about to run.
    pub(crate) fn publish_document_style(&self, record: u64) {
        let derived = self.with_style_engine(|engine| {
            engine.pin_layout_style_record(record);
            DerivedStyleRecord {
                record,
                payloads: StylePayloadsRef::new(
                    engine
                        .style_record_payloads(record)
                        .expect("the document's style must be live")
                        .as_ptr()
                        .cast(),
                ),
            }
        });
        self.release_published_document_style();
        self.published_document_style.set(Some(derived));
    }

    /// Stamps the viewport's row with the document's style the build was handed.
    pub(crate) fn adopt_published_document_style(&self, viewport: NodeSlotId) {
        let derived = self
            .published_document_style
            .take()
            .expect("a build that builds the viewport is handed the document's style");
        self.apply_reinherited_style_record(viewport, derived, ShellStyleChangeNotice::Handback);
    }

    /// Releases the document's style if the build did not build a viewport to take it.
    pub(crate) fn release_published_document_style(&self) {
        if let Some(derived) = self.published_document_style.take() {
            self.with_style_engine(|engine| engine.unpin_layout_style_record(derived.record));
        }
    }

    /// Owes the host the provider for the image a pseudo-element's generated content names, and
    /// the image box's style resources, once the running frame is over.
    pub(crate) fn defer_generated_image(
        &self,
        id: NodeSlotId,
        generator: StyleNodeID,
        pseudo_element: super::tree_builder::FfiPseudoElement,
        item: super::tree_builder::FfiGeneratedContentItem,
        pseudo_element_box: NodeSlotId,
    ) {
        self.image_boxes_awaiting_owned_provider.borrow_mut().insert(id);
        self.image_resources_owed_to_host.borrow_mut().push((
            id,
            OwedImageResources::GeneratedImage {
                generator,
                pseudo_element,
                item,
                pseudo_element_box,
            },
        ));
    }

    /// The record `record` derives with its display replaced, as the style of a box that takes
    /// another's style but lays out as `display` would.
    pub(crate) fn derive_style_record_with_display(
        &self,
        record: u64,
        display: crate::css::display::FfiDisplay,
    ) -> DerivedStyleRecord {
        self.with_style_engine(|engine| {
            let mut style = LayoutStyle::from_record(engine, record);
            style.set_display(display);
            style.intern(engine)
        })
    }

    /// The rows the finished build owes a shell. A row the build freed again, such as whitespace
    /// table fixup removed, is owed nothing.
    pub(crate) fn take_shells_owed_to_host(&self) -> Vec<NodeSlotId> {
        let mut owed = std::mem::take(&mut *self.shells_owed_to_host.borrow_mut());
        owed.retain(|&row| self.slot_is_live(row));
        owed
    }

    /// The image resources the finished builds owe the host, in the order they came to owe them.
    /// A later build can still free a row, so whoever pays them asks whether it is live first.
    pub(crate) fn take_image_resources_owed_to_host(&self) -> Vec<(NodeSlotId, OwedImageResources)> {
        std::mem::take(&mut *self.image_resources_owed_to_host.borrow_mut())
    }

    /// Whether `id` is an image box that owns its image's provider and has not been handed it yet.
    pub(crate) fn image_box_awaits_owned_provider(&self, id: NodeSlotId) -> bool {
        self.image_boxes_awaiting_owned_provider.borrow().contains(&id)
    }

    /// Notes that the host is about to hand `id` the provider it owns, if it was waiting for one.
    pub(crate) fn note_owned_provider_handed_over(&self, id: NodeSlotId) {
        self.image_boxes_awaiting_owned_provider.borrow_mut().remove(&id);
    }

    /// The shell of `id`, made now if nothing has asked for it before. Making one runs the host's
    /// shell factory, so only the main thread can ask.
    pub(crate) fn node_shell(&self, main_thread: &crate::stage::MainThread, id: NodeSlotId) -> *mut c_void {
        if let Some(shell) = self.data(id).shell.get() {
            return shell.host_object(main_thread);
        }
        self.materialize_shell(main_thread, id)
    }

    pub(crate) fn dom_offset_for_rendered_text_offset(
        &self,
        id: NodeSlotId,
        offset: usize,
        boundary: RenderedTextBoundary,
    ) -> usize {
        if !self.node_kind_if_live(id).is_some_and(super::node_facts::kind_is_text) {
            return offset;
        }
        self.text_content(id)
            .expect("text must be published before mapping rendered offsets")
            .dom_offset_for_rendered_text_offset(offset, boundary)
    }

    pub(crate) fn rendered_text_offset_for_dom_offset(
        &self,
        id: NodeSlotId,
        offset: usize,
        boundary: RenderedTextBoundary,
    ) -> usize {
        if !self.node_kind_if_live(id).is_some_and(super::node_facts::kind_is_text) {
            return offset;
        }
        self.text_content(id)
            .expect("text must be published before mapping DOM offsets")
            .rendered_text_offset_for_dom_offset(offset, boundary)
    }

    #[track_caller]
    pub(crate) unsafe fn from_handle<'a>(arena: *mut c_void) -> &'a Self {
        assert!(!arena.is_null(), "layout node arena handle is null");
        crate::stage_thread::join_overlapping_stage();
        // SAFETY: Layout passes borrow the document's arena synchronously,
        // and the document keeps it alive for the duration of the pass.
        super::main_side_census::note_arena_access(std::panic::Location::caller(), arena);
        unsafe { &*arena.cast::<Self>() }
    }

    #[track_caller]
    pub(crate) unsafe fn from_handle_mut<'a>(arena: *mut c_void) -> &'a mut Self {
        assert!(!arena.is_null(), "layout node arena handle is null");
        crate::stage_thread::join_overlapping_stage();
        super::main_side_census::note_arena_access(std::panic::Location::caller(), arena);
        // SAFETY: The caller guarantees exclusive access to the arena for the
        // duration of the returned borrow.
        unsafe { &mut *arena.cast::<Self>() }
    }

    fn data_mut(&mut self, index: u32) -> &mut NodeData {
        let index = index as usize;
        let chunk = self
            .chunks
            .get_mut(index / SLOTS_PER_CHUNK)
            .expect("invalid layout node arena slot ID");
        &mut chunk.slots[index % SLOTS_PER_CHUNK]
    }

    fn metadata(&self, index: u32) -> &SlotMetadata {
        self.slot_metadata
            .get(index as usize)
            .expect("invalid layout node arena slot ID")
    }

    fn metadata_mut(&mut self, index: u32) -> &mut SlotMetadata {
        self.slot_metadata
            .get_mut(index as usize)
            .expect("invalid layout node arena slot ID")
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct NodeAllocation {
    pub(crate) slot: NodeSlotId,
}

#[unsafe(no_mangle)]
pub extern "C" fn layout_arena_create() -> *mut c_void {
    Box::into_raw(Box::new(super::ArenaHandle::new())).cast()
}

/// Whether the row is an image box that owns its image's provider and has not been handed it yet,
/// which it is from the tree build that stamps it until the frame the build runs in is over.
///
/// # Safety
///
/// The arena must be live on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_image_box_awaits_owned_provider(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: The C++ wrapper keeps the arena alive for this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.image_box_awaits_owned_provider(slot)
}

/// Records whether attaching a row's style resources loaded any image.
///
/// # Safety
///
/// The arena must be live on the document thread and `slot` must name a live row.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_note_style_image_resources_attached(
    arena: *mut c_void,
    slot: NodeSlotId,
    attached: bool,
) {
    // SAFETY: The C++ wrapper keeps the arena alive for this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.note_style_image_resources_attached(slot, attached);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_destroy(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and ownership is
    // transferred back exactly once by the C++ RAII wrapper.
    let handle = unsafe { Box::from_raw(arena.cast::<super::ArenaHandle>()) };
    let arena = handle.arena();
    arena.assert_owner_thread();
    assert_eq!(arena.live_count, 0, "layout node arena destroyed with live slots");
    crate::painting::published_immutable::finish(arena);
    super::tree_build_seal::flush_census();
    super::main_side_census::flush();
}

/// Detaches `node` from its parent and frees its subtree, handing back what the rows held. Answers
/// whether the node was attached.
pub(crate) fn detach_and_free_subtree(arena: *mut LayoutNodeArena, node: NodeSlotId) -> bool {
    // SAFETY: The caller keeps the arena alive; the shared borrow ends before the subtree is freed.
    let was_attached = unsafe { &*arena }.detach_from_parent(node);
    crate::layout::tree_mutation::free_subtree_and_hand_back(arena, node);
    was_attached
}

/// Runs `operation`, an arena change a main-thread entry makes, and pays what it hands back once it
/// returns, which is where the host heard about it when the arena called the host directly.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and no
/// borrow of it may be live across this call other than the ones `operation` takes.
#[track_caller]
pub(crate) unsafe fn paying_host_handbacks<R>(
    main_thread: &crate::stage::MainThread,
    arena: *mut c_void,
    operation: impl FnOnce() -> R,
) -> R {
    // SAFETY: Guaranteed by the caller; each borrow here ends before `operation` runs or after it
    // has returned.
    super::main_side_census::note_arena_access(std::panic::Location::caller(), arena);
    unsafe { &*arena.cast::<LayoutNodeArena>() }.begin_paying_host_handbacks(main_thread);
    // The operation is the passage just counted, however many times it turns the handle around.
    let result = super::main_side_census::within_counted_passage(operation);
    // SAFETY: As above.
    unsafe { &*arena.cast::<LayoutNodeArena>() }.finish_paying_host_handbacks(main_thread);
    result
}

fn counter_owner(style_node: u32, generated_for: u8) -> Option<super::counters::CounterOwner> {
    StyleNodeID::from_raw(style_node).map(|element| super::counters::CounterOwner { element, generated_for })
}

/// Whether the counter styles the record of the pseudo-element `generated_for` of the element
/// `style_node` names now differ from the ones the box built for it renders from. Answers
/// `CONTENT_COUNTER_STYLES_NOT_RECORDED` while no box of that pseudo-element has recorded any,
/// which is every element that generates no content.
///
/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_content_counter_styles_changed(
    arena: *mut c_void,
    style_node: u32,
    generated_for: u8,
) -> u8 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(owner) = counter_owner(style_node, generated_for) else {
        return CONTENT_COUNTER_STYLES_NOT_RECORDED;
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on the
    // document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    match super::generated_content::content_counter_styles_changed(arena, owner) {
        None => CONTENT_COUNTER_STYLES_NOT_RECORDED,
        Some(false) => CONTENT_COUNTER_STYLES_UNCHANGED,
        Some(true) => CONTENT_COUNTER_STYLES_CHANGED,
    }
}

pub const CONTENT_COUNTER_STYLES_NOT_RECORDED: u8 = 0;
pub const CONTENT_COUNTER_STYLES_UNCHANGED: u8 = 1;
pub const CONTENT_COUNTER_STYLES_CHANGED: u8 = 2;

/// The text the content of the pseudo-element `generated_for` of the element `style_node` names last
/// resolved to, the way accessibility reads it: the alt text when there is one, otherwise every
/// string in order. The result is an `AK::Utf16String` raw representation the caller adopts.
///
/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_generated_content_accessible_text(
    arena: *mut c_void,
    style_node: u32,
    generated_for: u8,
) -> usize {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(owner) = counter_owner(style_node, generated_for) else {
        return ak::Utf16String::from_utf16(&[]).into_raw();
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on the
    // document thread.
    let generated_content = unsafe { LayoutNodeArena::from_handle(arena) }
        .generated_content()
        .borrow();
    ak::Utf16String::from_utf16(generated_content.accessible_text(owner)).into_raw()
}

/// Whether the innermost `list-item` counter in the counters set of the element `style_node` names
/// counts forward and was created by that element.
///
/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_innermost_list_item_counter_is_own_forward_counter(
    arena: *mut c_void,
    style_node: u32,
) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(element) = StyleNodeID::from_raw(style_node) else {
        return false;
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on the
    // document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }
        .counters_sets()
        .borrow()
        .innermost_list_item_counter_is_own_forward_counter(element)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_live_slot_count(arena: *mut c_void) -> u32 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.live_slot_count()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_table_cell_measurement_cache_miss_count(arena: *mut c_void) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.table_cell_measurement_cache_miss_count()
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_intrinsic_measurement_count(arena: *mut c_void) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.intrinsic_measurement_count()
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_intrinsic_inline_measurement_count(arena: *mut c_void) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive and serializes access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.intrinsic_inline_measurement_count()
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_pre_order_relabel_count(arena: *mut c_void) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.pre_order_relabel_count()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_link_slot(
    arena: *mut c_void,
    id: NodeSlotId,
    link: FfiNodeLink,
) -> NodeSlotId {
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_link_slot(id, link)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_flags(arena: *mut c_void, id: NodeSlotId) -> u32 {
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_flags(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_has_compositor_animation_frame(
    arena: *mut c_void,
    id: NodeSlotId,
    kind: super::node_data::CompositorAnimationFrameKind,
) -> bool {
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_has_compositor_animation_frame(id, kind)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_generated_for(arena: *mut c_void, id: NodeSlotId) -> u8 {
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_generated_for(id)
}

/// # Safety
///
/// The arena must remain valid for the duration of the call, and `node` must name a live node
/// in this arena.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_bump_fragment_cache_epoch_of_self_and_ancestors(
    arena: *mut c_void,
    node: NodeSlotId,
) {
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.bump_fragment_cache_epoch_of_self_and_ancestors(node);
}

/// Publishes the elements registered under one anchor name in one tree scope, in tree order.
///
/// # Safety
///
/// `elements` must name `count` element identities for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_anchor_name_elements(
    arena: *mut c_void,
    scope_host: u32,
    anchor_name: usize,
    elements: *const u32,
    count: usize,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let elements: Vec<_> = if count == 0 {
        Vec::new()
    } else {
        // SAFETY: The C++ caller keeps the element array alive for this call.
        unsafe { std::slice::from_raw_parts(elements, count) }
            .iter()
            .filter_map(|&raw| StyleNodeID::from_raw(raw))
            .collect()
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_anchor_name_elements(scope_host, anchor_name, &elements);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_node_dom_paint_facts(arena: *mut c_void, id: NodeSlotId, facts: u8) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_node_dom_paint_facts(id, facts)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_note_rows_share_dom_node(
    arena: *mut c_void,
    bound_row: NodeSlotId,
    added_row: NodeSlotId,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.note_rows_share_dom_node(bound_row, added_row);
}

/// The row the element or text node with `style_node` is bound to, or an invalid slot if it has
/// none. Unlike its shell, asking for the row makes nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_bound_row(arena: *mut c_void, style_node: u32) -> NodeSlotId {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return NodeSlotId::INVALID;
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.bound_row(style_node)
}

/// Pins, for the host, the style record of the box the element or text node with `style_node` is
/// bound to, or of the box of its pseudo-element of kind `generated_for`, so that the box keeps
/// its style readable once the node has left the document. A text box has no record of its own.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_pin_bound_box_style_record_for_detachment(
    arena: *mut c_void,
    style_node: u32,
    generated_for: u8,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return;
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let row = if generated_for == 0 {
        arena.bound_row(style_node)
    } else {
        arena.bound_pseudo_element_row(style_node, generated_for)
    };
    if row.is_invalid() || !super::tree_builder::node_kind_is_node_with_style(arena.data(row).kind.get()) {
        return;
    }
    arena.pin_node_style_record_for_host(row, arena.node_style_record(row));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_node_flag(arena: *mut c_void, id: NodeSlotId, flag: NodeFlag, value: bool) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_node_flag(id, flag, value);
}

/// What the door of one main-side writer cost: how often it was passed, and how often and for how
/// long a pass had to wait for the frame in flight. The passes rank the writers for moving from the
/// door to the journal, which queues a write instead of waiting.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FfiDoorCounters {
    pub passes: u64,
    pub waits: u64,
    pub wait_nanoseconds: u64,
}

thread_local! {
    // By writer, in the order each was first passed. There are only a few writers, so a scan is enough.
    static DOOR_COUNTERS: RefCell<Vec<(&'static str, FfiDoorCounters)>> = const { RefCell::new(Vec::new()) };
}

/// Counts a pass through `writer`'s door, and the wait for the frame in flight it took, if any.
fn record_door_pass(writer: &'static str, wait: Option<std::time::Duration>) {
    DOOR_COUNTERS.with_borrow_mut(|counters| {
        let index = match counters.iter().position(|(name, _)| *name == writer) {
            Some(index) => index,
            None => {
                counters.push((writer, FfiDoorCounters::default()));
                counters.len() - 1
            }
        };
        let entry = &mut counters[index].1;
        entry.passes += 1;
        if let Some(wait) = wait {
            entry.waits += 1;
            entry.wait_nanoseconds += u64::try_from(wait.as_nanos()).unwrap_or(u64::MAX);
        }
    });
}

/// Calls `callback` with each door writer's name (UTF-8, not NUL-terminated) and counters on this
/// thread.
///
/// # Safety
///
/// `callback` must be safe to call with `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_for_each_door_counters(
    context: *mut c_void,
    callback: unsafe extern "C" fn(
        context: *mut c_void,
        name: *const u8,
        name_length: usize,
        counters: FfiDoorCounters,
    ),
) {
    // Copied out first, so the callback may pass a door itself.
    let counters = DOOR_COUNTERS.with_borrow(|counters| counters.clone());
    for (name, counters) in counters {
        // SAFETY: The caller guarantees the callback is safe to call with its context.
        unsafe { callback(context, name.as_ptr(), name.len(), counters) };
    }
}

/// Forgets every door pass counted on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn layout_arena_reset_door_counters() {
    DOOR_COUNTERS.with_borrow_mut(Vec::clear);
}

/// Joins the frame in flight ahead of a DOM tree mutation. The mutation splices the style mirror
/// and builds, frees and marks the arena's rows as it goes, in an order the retirement of the
/// mutated nodes' style identities depends on, so none of it is journalled apart from the rest, and
/// the whole mutation waits for the frame instead.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_join_frame_for_dom_tree_mutation(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    drop(arena.join_frame_for_main_side_write(LayoutNodeArena::DOM_TREE_MUTATION_WRITER));
}

/// Whether the box keeps content the compositor animates. Like the frames below, it is chosen by
/// the rendering update between frames, so it goes through the same door rather than a journal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_node_retains_compositor_animated_content(
    arena: *mut c_void,
    id: NodeSlotId,
    value: bool,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::COMPOSITOR_ELIGIBILITY_WRITER);
    arena.set_node_flag(id, NodeFlag::HasAnimatedOpacityOrTransform, value);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_node_needs_compositor_animation_frame(
    arena: *mut c_void,
    id: NodeSlotId,
    kind: super::node_data::CompositorAnimationFrameKind,
    value: bool,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::COMPOSITOR_ELIGIBILITY_WRITER);
    arena.set_node_needs_compositor_animation_frame(id, kind, value);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_node_generated_for(
    arena: *mut c_void,
    id: NodeSlotId,
    generated_for: u8,
    generator_style_node: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_node_generated_for(
        id,
        generated_for,
        StyleNodeID::from_raw(generator_style_node),
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_node(arena: *mut c_void, id: NodeSlotId) -> u32 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }
        .node_style_node(id)
        .map_or(0, StyleNodeID::raw)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_pseudo_element_scroll_offset(
    arena: *mut c_void,
    generator: u32,
    pseudo_kind: u8,
) -> FfiCssPixelPoint {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(generator) = StyleNodeID::from_raw(generator) else {
        return FfiCssPixelPoint::default();
    };
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.pseudo_element_scroll_offset(generator, pseudo_kind)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_pseudo_element_scroll_offset(
    arena: *mut c_void,
    generator: u32,
    pseudo_kind: u8,
    offset: FfiCssPixelPoint,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(generator) = StyleNodeID::from_raw(generator) else {
        return;
    };
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle_mut(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER);
    arena.set_pseudo_element_scroll_offset(generator, pseudo_kind, offset);
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_identity_in_focused_text_control(arena: *mut c_void, node: u32, value: bool) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle_mut(arena) }.set_identity_in_focused_text_control(node, value);
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_element_scroll_offset(
    arena: *mut c_void,
    element: u32,
    offset: FfiCssPixelPoint,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(element) = StyleNodeID::from_raw(element) else {
        return;
    };
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle_mut(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER);
    arena.set_element_scroll_offset(element, offset);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_pin_node_style_record_for_host(
    arena: *mut c_void,
    slot: NodeSlotId,
    record: u64,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.pin_node_style_record_for_host(slot, record);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_release_node_style_record_pin_for_host(arena: *mut c_void, slot: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.release_node_style_record_pin_for_host(slot);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_record_pinned_by_host(arena: *mut c_void, slot: NodeSlotId) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_style_record_pinned_by_host(slot)
}

/// `Painting::PaintCacheInvalidationStage::DetachCleanup`, which is what the retired host-side
/// preparation attributed its invalidation to.
const PAINT_CACHE_INVALIDATION_STAGE_DETACH_CLEANUP: u8 = 2;

/// Prepares `row` for leaving the layout tree. A detached box is read until its row is freed, so
/// its style record is pinned for the host and its paint cache is cleaned here rather than through
/// the journal, which would resolve the identity after a replacement row had been bound. The image
/// resources the row holds go with it.
pub(crate) fn prepare_row_for_detach(arena: *mut c_void, row: NodeSlotId) {
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    let arena_ref = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let kind = arena_ref.data(row).kind.get();
    let is_node_with_style = super::tree_builder::node_kind_is_node_with_style(kind);
    if is_node_with_style {
        let style_record = arena_ref.style_records[row.slot_index() as usize].get();
        if style_record != 0 {
            arena_ref.pin_node_style_record_for_host(row, style_record);
        }
    }
    // SAFETY: The handle is the one this call was given, and the invalidation borrows the arena
    // for itself.
    unsafe {
        crate::painting::ffi::layout_arena_paintable_invalidate_paint_cache(
            arena,
            row,
            false,
            PAINT_CACHE_INVALIDATION_STAGE_DETACH_CLEANUP,
        );
    }
    if is_node_with_style && arena_ref.rows_with_image_observers.borrow_mut().remove(&row) {
        arena_ref.hand_back(HostHandback::ImageObservers(row));
    }
    if kind == NodeKind::ImageBox && arena_ref.rows_with_owned_image_provider.borrow().contains(&row) {
        arena_ref.hand_back(HostHandback::OwnedImageProviderDetach(row));
    }
}

/// Prepares every row in the layout subtree `root` heads for leaving the tree, handing back the
/// image resources the rows hold.
pub(crate) fn prepare_subtree_for_detach(arena: *mut c_void, root: NodeSlotId) {
    // SAFETY: The caller keeps the arena alive for this call.
    let arena_ref = unsafe { &*arena.cast::<LayoutNodeArena>() };
    arena_ref.assert_owner_thread();
    let mut rows = Vec::new();
    arena_ref.for_each_node_in_layout_subtree_in_pre_order(root, |row| rows.push(row));
    for row in rows {
        prepare_row_for_detach(arena, row);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_replace_image_observers(
    arena: *mut c_void,
    slot: NodeSlotId,
    observers: *mut c_void,
) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.replace_image_observers(
        unsafe { super::HostTables::from_handle(arena) },
        slot,
        observers,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_image_observers(arena: *mut c_void, slot: NodeSlotId) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }
        .image_observers(unsafe { super::HostTables::from_handle(arena) }, slot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_owned_image_provider(
    arena: *mut c_void,
    slot: NodeSlotId,
    provider: *mut c_void,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_owned_image_provider(
        unsafe { super::HostTables::from_handle(arena) },
        slot,
        provider,
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_owned_image_provider(arena: *mut c_void, slot: NodeSlotId) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }
        .owned_image_provider(unsafe { super::HostTables::from_handle(arena) }, slot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_move_pseudo_element_scroll_offsets(
    arena: *mut c_void,
    old_generator: u32,
    new_generator: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let (Some(old_generator), Some(new_generator)) = (
        StyleNodeID::from_raw(old_generator),
        StyleNodeID::from_raw(new_generator),
    ) else {
        return;
    };
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle_mut(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER);
    arena.move_pseudo_element_scroll_offsets(old_generator, new_generator);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_node_style(
    arena: *mut c_void,
    id: NodeSlotId,
    style_record: u64,
    payloads: *const c_void,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    if arena.set_node_style(id, style_record, payloads) {
        arena.refresh_style_flags(id);
    }
    arena.enroll_node_for_svg_paint_resources_sync(id);
}

/// # Safety
///
/// The arena must be live on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_tree_build_may_create_viewport(
    arena: *mut c_void,
    document_style_node: u32,
) -> bool {
    unsafe { LayoutNodeArena::from_handle(arena) }
        .tree_build_may_create_viewport(StyleNodeID::from_raw(document_style_node))
}

/// # Safety
///
/// The arena and record must be live on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_document_style_record(arena: *mut c_void, record: u64) {
    unsafe { LayoutNodeArena::from_handle(arena) }.publish_document_style(record);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_has_derived_style(arena: *mut c_void, node: NodeSlotId) -> bool {
    unsafe { LayoutNodeArena::from_handle(arena) }.node_style_record_is_pinned_by_arena(node)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_record(arena: *mut c_void, id: NodeSlotId) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_style_record(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_payloads(arena: *mut c_void, id: NodeSlotId) -> *const c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }
        .data(id)
        .style
        .get()
        .as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_shell_count(arena: *mut c_void) -> u32 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.shell_count()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_shell_factory(
    arena: *mut c_void,
    context: *mut c_void,
    factory: unsafe extern "C" fn(*mut c_void, NodeSlotId, NodeKind),
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { super::HostTables::from_handle(arena) }
        .shell_factory
        .set(Some((context, factory)));
}

/// # Safety
///
/// `arena` must be a live handle on the document thread. The host must stay registered only while
/// its context is live, and must not reenter the arena from the callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_box_presence_host(
    arena: *mut c_void,
    context: *mut c_void,
    callback: unsafe extern "C" fn(*mut c_void, u32, u8),
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { super::HostTables::from_handle(arena) }
        .box_presence_host
        .set(Some((context, callback)));
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_host_hears_box_presence(true);
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clear_box_presence_host(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { super::HostTables::from_handle(arena) }
        .box_presence_host
        .set(None);
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_host_hears_box_presence(false);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clear_shell_factory(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { super::HostTables::from_handle(arena) }.shell_factory.set(None);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_attach_shell(arena: *mut c_void, id: NodeSlotId, shell: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.attach_shell(id, shell);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_style_record_host_callbacks(
    arena: *mut c_void,
    callbacks: FfiStyleRecordHostCallbacks,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { super::HostTables::from_handle(arena) }
        .shell_style_changed_host
        .set(Some((callbacks.context, callbacks.shell_style_changed)));
    assert!(!callbacks.style_engine.is_null());
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_style_engine(callbacks.style_engine);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clear_style_record_host_callbacks(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { super::HostTables::from_handle(arena) }
        .shell_style_changed_host
        .set(None);
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_style_engine(std::ptr::null_mut());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_layout_pass_is_running(arena: *mut c_void) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.layout_pass_is_running()
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_list_owner_has_stale_item_counters(
    arena: *mut c_void,
    list_owner: u32,
    value: bool,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let list_owner = StyleNodeID::from_raw(list_owner).expect("a list owner has an identity");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_list_owner_has_stale_item_counters(list_owner, value);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_needs_full_layout_tree_update(arena: *mut c_void) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.needs_full_layout_tree_update()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_needs_full_layout_tree_update(arena: *mut c_void, value: bool) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.set_needs_full_layout_tree_update(value);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_layout_root(arena: *mut c_void) -> NodeSlotId {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.layout_root()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_layout_is_up_to_date(
    arena: *mut c_void,
    document_needs_layout_tree_build: bool,
) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.layout_is_up_to_date(document_needs_layout_tree_build)
}

/// What the host answers for the content enrolled for sync: the replaced-content facts of each
/// enrolled node that still has a shell. Text content is published to the arena and needs no host.
#[derive(Default)]
pub(crate) struct EnrolledContentSources {
    /// Nothing is synced when this is read inside a running pass.
    pass_was_running: bool,
    /// The enrolled replaced nodes the facts were read for, whether or not they are still live.
    enrolled_replaced_node_count: usize,
    replaced_content_facts: Vec<(NodeSlotId, FfiReplacedContentFacts)>,
}

/// The host half of the enrolled content sync. It reads the replaced-content facts of the nodes
/// enrolled now, so [`apply_enrolled_content_sources`] needs no host and can run on a stage.
///
/// # Safety
///
/// `arena` must be a live handle with a registered layout host, used on the document thread.
pub(crate) unsafe fn read_enrolled_content_sources(
    main_thread: &crate::stage::MainThread,
    arena: *mut c_void,
) -> EnrolledContentSources {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY (for every derive below): the caller keeps the arena alive for this call and
    // serializes all access on the document thread; no shared borrow outlives a callback.
    if unsafe { &*arena.cast::<LayoutNodeArena>() }.layout_pass_is_running() {
        return EnrolledContentSources {
            pass_was_running: true,
            ..EnrolledContentSources::default()
        };
    }
    let host = LayoutHost::of(main_thread);
    let enrolled_replaced_nodes = unsafe { &*arena.cast::<LayoutNodeArena>() }
        .nodes_enrolled_for_replaced_content_facts_sync
        .borrow()
        .clone();
    let mut replaced_content_facts = Vec::with_capacity(enrolled_replaced_nodes.len());
    for &node in &enrolled_replaced_nodes {
        let shell = unsafe { &*arena.cast::<LayoutNodeArena>() }.shell_if_live(main_thread, node);
        if shell.is_null() {
            continue;
        }
        let mut facts = FfiReplacedContentFacts::default();
        super::seal::note_host_call(
            unsafe { &*arena.cast::<LayoutNodeArena>() }.layout_pass_is_running(),
            "build_replaced_content_facts",
        );
        super::tree_build_seal::note_host_call("build_replaced_content_facts");
        // SAFETY: The callback receives a live shell and a valid out-pointer.
        unsafe { host.build_replaced_content_facts(main_thread, shell, &raw mut facts) };
        replaced_content_facts.push((node, facts));
    }
    EnrolledContentSources {
        pass_was_running: false,
        enrolled_replaced_node_count: enrolled_replaced_nodes.len(),
        replaced_content_facts,
    }
}

/// The arena half of the enrolled content sync: refreshes the text content of every enrolled text
/// node and stores the replaced-content facts `sources` read. Nodes enrolled after `sources` was
/// read stay enrolled for the next sync.
///
/// # Safety
///
/// `arena` must be a live handle whose owner waits for this call or makes it itself, and nothing
/// may have freed a row since `sources` was read from it.
pub(crate) unsafe fn apply_enrolled_content_sources(arena: *mut c_void, sources: EnrolledContentSources) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    if sources.pass_was_running {
        return;
    }
    // SAFETY (for every derive below): the caller keeps the arena alive for this call and
    // serializes all access to it; no shared borrow outlives the text sync.
    let enrolled_text_nodes = unsafe { &*arena.cast::<LayoutNodeArena>() }.pending_text_nodes_for_content_sync();
    for node in enrolled_text_nodes {
        if !unsafe { &*arena.cast::<LayoutNodeArena>() }.slot_is_live(node) {
            continue;
        }
        let parent = unsafe { &*arena.cast::<LayoutNodeArena>() }.data(node).parent.get();
        // Detached nodes retain enrollment until a parent supplies their style.
        if parent.is_invalid() {
            continue;
        }
        // SAFETY: The slot is live, and no arena borrow survives the sync.
        unsafe { super::rendered_text::ensure_text_content(arena.cast(), node) };
    }

    let mut live_replaced_nodes = Vec::with_capacity(sources.replaced_content_facts.len());
    for (node, facts) in sources.replaced_content_facts {
        live_replaced_nodes.push(node);
        // Changed facts invalidate cached formatting-context runs regardless of which
        // channel produced the change, including sources with no invalidation of their own.
        // SAFETY: As above; the shared borrows ended with their statements.
        if unsafe { &mut *arena.cast::<LayoutNodeArena>() }.set_replaced_content_facts(node, facts) {
            unsafe { &*arena.cast::<LayoutNodeArena>() }.bump_fragment_cache_epoch_of_self_and_ancestors(node);
        }
    }
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let mut enrolled_replaced_nodes = arena.nodes_enrolled_for_replaced_content_facts_sync.borrow_mut();
    let enrolled_since_read = enrolled_replaced_nodes.split_off(sources.enrolled_replaced_node_count);
    *enrolled_replaced_nodes = live_replaced_nodes;
    enrolled_replaced_nodes.extend(enrolled_since_read);
}

/// # Safety
///
/// The arena must remain valid for the duration of the call, and `id` must name a live node
/// in this arena.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_table_spans(
    arena: *mut c_void,
    id: NodeSlotId,
    column_span: u16,
    row_span: u16,
    raw_column_span: u32,
) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle_mut(arena) };
    let data = arena.data(id);
    let effective_spans_changed = data.table_column_span.get() != column_span || data.table_row_span.get() != row_span;
    data.table_column_span.set(column_span);
    data.table_row_span.set(row_span);
    let previous_raw_column_span = arena.set_raw_table_column_span(id, raw_column_span);
    effective_spans_changed || previous_raw_column_span != raw_column_span
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::layout::abspos_inputs::{
        AbsposAxisMode, AbsposContainingBlockInfo, AbsposLayoutInputs, StaticPositionAlignment, StaticPositionRect,
    };
    use crate::layout::layout_node_arena::{
        Chunk, DerivedStyleRecord, IntrinsicBlockSizeMeasurement, IntrinsicInlineSizeMeasurement,
        IntrinsicSizeCacheKey, IntrinsicSizeCacheKind, LayoutNodeArena, SLOTS_PER_CHUNK, TableCellMeasurement,
        TableCellMeasurementKey,
    };
    use crate::layout::node_data::{FfiNodeConstructionFacts, NodeFlag, NodeKind, NodeSlotId};
    use crate::layout::{CssPixels, fragment_tree, used_values};
    use std::ffi::c_void;

    fn test_construction_facts() -> FfiNodeConstructionFacts {
        test_construction_facts_with_kind(NodeKind::Box)
    }

    fn test_anonymous_construction_facts() -> FfiNodeConstructionFacts {
        FfiNodeConstructionFacts {
            is_anonymous: true,
            ..test_construction_facts()
        }
    }

    fn test_construction_facts_with_kind(kind: NodeKind) -> FfiNodeConstructionFacts {
        FfiNodeConstructionFacts {
            kind,
            shell: std::ptr::null_mut(),
            is_anonymous: false,
            dom_paint_facts: 0,
            style_node: 0,
        }
    }

    #[test]
    fn a_stage_on_the_stage_thread_acts_for_the_arenas_owner() {
        let mut arena = LayoutNodeArena::new();
        let parent = arena.allocate(test_construction_facts());
        let child = arena.allocate(test_construction_facts());
        let stage_arena = &mut arena;
        crate::stage_thread::run_stage_for_test(move || {
            stage_arena.attach_child(
                parent,
                crate::layout::tree_mutation::UnplacedLayoutNode::new(child),
                NodeSlotId::INVALID,
            );
        });
        assert_eq!(arena.data(child).parent.get(), parent);
    }

    #[test]
    fn a_retired_style_node_leaves_every_row_carrying_it() {
        use crate::css::style::tree::StyleNodeID;
        let mut arena = LayoutNodeArena::new();
        let style_node = StyleNodeID::element(3);
        let rows: Vec<_> = (0..3)
            .map(|_| {
                arena.allocate(FfiNodeConstructionFacts {
                    style_node: style_node.raw(),
                    ..test_construction_facts()
                })
            })
            .collect();
        let generated = arena.allocate(test_anonymous_construction_facts());
        arena.set_node_generated_for(generated, 1, Some(style_node));
        assert!(rows.iter().all(|row| arena.node_style_node(*row) == Some(style_node)));

        arena.free_subtree(rows[1]).destroy_shells_and_invoke_callbacks();
        arena.forget_style_node(style_node);
        assert_eq!(arena.node_style_node(rows[0]), None);
        assert_eq!(arena.node_style_node(rows[2]), None);
        assert_eq!(arena.node_style_node(generated), None);

        let reconnected = StyleNodeID::element(1);
        arena.set_style_node_of_generated_subtree(generated, Some(reconnected));
        assert_eq!(arena.node_style_node(generated), Some(reconnected));
        for row in [rows[0], rows[2], generated] {
            arena.free_subtree(row).destroy_shells_and_invoke_callbacks();
        }
        arena.forget_style_node(reconnected);
    }

    #[test]
    fn a_node_is_bound_only_to_the_row_that_took_the_binding() {
        use crate::css::style::tree::StyleNodeID;
        let mut arena = LayoutNodeArena::new();
        let element = StyleNodeID::element(3);
        let facts = FfiNodeConstructionFacts {
            style_node: element.raw(),
            ..test_construction_facts()
        };
        let old_row = arena.allocate(facts);
        assert!(arena.bound_row(element).is_invalid());
        arena.bind_row(old_row);
        assert_eq!(arena.bound_row(element), old_row);

        // A rebuilt row takes the binding; a row for another referencer only shares the node.
        let new_row = arena.allocate(facts);
        arena.note_rows_share_dom_node(old_row, new_row);
        arena.bind_row(new_row);
        let referencer_row = arena.allocate(facts);
        arena.note_rows_share_dom_node(new_row, referencer_row);
        assert_eq!(arena.bound_row(element), new_row);

        // A changed identity carries the binding along.
        let changed = StyleNodeID::element(4);
        arena.set_style_node_of_rows_sharing_dom_node_with(new_row, Some(changed));
        assert!(arena.bound_row(element).is_invalid());
        assert_eq!(arena.bound_row(changed), new_row);

        // Freeing the bound row binds the node to one of the rows that still share it.
        arena.free_subtree(new_row).destroy_shells_and_invoke_callbacks();
        assert_eq!(arena.bound_row(changed), referencer_row);
        arena.unbind_row(referencer_row);
        assert!(arena.bound_row(changed).is_invalid());
        arena.bind_row(old_row);
        arena.forget_style_node(changed);
        assert!(arena.bound_row(changed).is_invalid());

        let viewport = arena.allocate(test_construction_facts_with_kind(NodeKind::Viewport));
        arena.bind_row(viewport);
        assert_eq!(arena.bound_viewport_row(), viewport);
        arena.free_subtree(viewport).destroy_shells_and_invoke_callbacks();
        assert!(arena.bound_viewport_row().is_invalid());

        for row in [old_row, referencer_row] {
            arena.free_subtree(row).destroy_shells_and_invoke_callbacks();
        }
    }

    thread_local! {
        static TOLD_BOX_PRESENCE: std::cell::RefCell<Vec<(u32, u8)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    unsafe extern "C" fn record_box_presence(_: *mut c_void, style_node: u32, bits: u8) {
        TOLD_BOX_PRESENCE.with(|told| told.borrow_mut().push((style_node, bits)));
    }

    #[test]
    fn an_observer_set_handed_back_stays_owed_when_a_newer_set_displaces_it() {
        let mut arena = LayoutNodeArena::new();
        let host_tables = crate::layout::HostTables::default();
        let main_thread = crate::stage::MainThread::for_test_with_host(&host_tables);
        let row = arena.allocate(test_construction_facts());
        let first = std::ptr::dangling_mut::<u8>().wrapping_add(1).cast::<c_void>();
        let second = std::ptr::dangling_mut::<u8>().wrapping_add(2).cast::<c_void>();
        assert!(arena.replace_image_observers(&host_tables, row, first).is_null());

        // The row lets go of its set, and the host gives it a new one before the handback is paid.
        arena.begin_paying_host_handbacks(&main_thread);
        assert!(arena.rows_with_image_observers.borrow_mut().remove(&row));
        arena.hand_back(super::HostHandback::ImageObservers(row));
        assert!(arena.image_observers(&host_tables, row).is_null());
        assert!(arena.replace_image_observers(&host_tables, row, second).is_null());
        assert_eq!(arena.image_observers(&host_tables, row), second);

        // The handback still pays with the set the row let go of.
        let handbacks = std::mem::take(&mut *arena.host_handbacks.borrow_mut());
        assert_eq!(
            arena.take_host_objects_owed(&main_thread, &handbacks.handbacks),
            vec![first]
        );
        assert_eq!(arena.image_observers(&host_tables, row), second);
        arena.close_host_handback_span();

        assert_eq!(
            arena.replace_image_observers(&host_tables, row, std::ptr::null_mut()),
            second
        );
        arena.free_subtree(row).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_tree_build_tells_the_host_each_node_s_boxes_once_it_is_over() {
        use super::BOX_PRESENCE_HAS_LAYOUT_BOX;
        use crate::css::style::tree::StyleNodeID;
        let mut arena = LayoutNodeArena::new();
        let host_tables = crate::layout::HostTables::default();
        host_tables
            .box_presence_host
            .set(Some((std::ptr::null_mut(), record_box_presence)));
        arena.set_host_hears_box_presence(true);
        let first = StyleNodeID::element(3);
        let second = StyleNodeID::element(4);
        let facts = |style_node: StyleNodeID| FfiNodeConstructionFacts {
            style_node: style_node.raw(),
            ..test_construction_facts()
        };
        let main_thread = crate::stage::MainThread::for_test_with_host(&host_tables);
        arena.begin_paying_host_handbacks(&main_thread);
        let old_row = arena.allocate(facts(first));
        arena.bind_row(old_row);
        arena.finish_paying_host_handbacks(&main_thread);
        TOLD_BOX_PRESENCE.with(|told| told.borrow_mut().clear());

        arena.begin_tree_build_handbacks();
        arena.free_subtree(old_row).destroy_shells_and_invoke_callbacks();
        let new_row = arena.allocate(facts(first));
        arena.bind_row(new_row);
        let other_row = arena.allocate(facts(second));
        arena.bind_row(other_row);
        arena.unbind_row(other_row);
        let handbacks = arena.take_tree_build_handbacks();
        assert!(TOLD_BOX_PRESENCE.with(|told| told.borrow().is_empty()));

        // Each node is told once, with what it has once the build is over.
        arena.pay_tree_build_handbacks(&main_thread, handbacks);
        assert_eq!(
            TOLD_BOX_PRESENCE.with(|told| std::mem::take(&mut *told.borrow_mut())),
            vec![(first.raw(), BOX_PRESENCE_HAS_LAYOUT_BOX), (second.raw(), 0)]
        );

        // Outside a build the host hears as the change's payer returns.
        arena.begin_paying_host_handbacks(&main_thread);
        arena.bind_row(other_row);
        arena.finish_paying_host_handbacks(&main_thread);
        assert_eq!(
            TOLD_BOX_PRESENCE.with(|told| std::mem::take(&mut *told.borrow_mut())),
            vec![(second.raw(), BOX_PRESENCE_HAS_LAYOUT_BOX)]
        );

        host_tables.box_presence_host.set(None);
        arena.set_host_hears_box_presence(false);
        for row in [new_row, other_row] {
            arena.free_subtree(row).destroy_shells_and_invoke_callbacks();
        }
        assert!(TOLD_BOX_PRESENCE.with(|told| told.borrow().is_empty()));
    }

    #[test]
    fn element_and_text_style_nodes_with_the_same_index_chain_separately() {
        use crate::css::style::tree::StyleNodeID;
        let mut arena = LayoutNodeArena::new();
        let element = StyleNodeID::element(2);
        let text = StyleNodeID::text(2);
        let element_row = arena.allocate(FfiNodeConstructionFacts {
            style_node: element.raw(),
            ..test_construction_facts()
        });
        let text_row = arena.allocate(FfiNodeConstructionFacts {
            style_node: text.raw(),
            ..test_construction_facts()
        });
        assert_eq!(arena.node_style_node(element_row), Some(element));
        assert_eq!(arena.node_style_node(text_row), Some(text));

        arena.forget_style_node(text);
        assert_eq!(arena.node_style_node(text_row), None);
        assert_eq!(arena.node_style_node(element_row), Some(element));

        arena.set_style_node_of_rows_sharing_dom_node_with(text_row, Some(StyleNodeID::text(5)));
        assert_eq!(arena.node_style_node(text_row), Some(StyleNodeID::text(5)));
        for row in [element_row, text_row] {
            arena.free_subtree(row).destroy_shells_and_invoke_callbacks();
        }
    }

    #[test]
    fn an_unbound_slot_has_no_shell_until_a_shell_is_bound() {
        let mut arena = LayoutNodeArena::new();
        let slot = arena.allocate_unbound();
        assert!(arena.slot_is_live(slot));
        let main_thread = crate::stage::MainThread::for_test();
        assert!(arena.node_shell(&main_thread, slot).is_null());
        assert_eq!(arena.data(slot).kind.get(), NodeKind::Unset);

        let unbound_freed = arena.free_subtree(slot);
        assert_eq!(unbound_freed.shell_count(), 0);
        unbound_freed.destroy_shells_and_invoke_callbacks();
        assert!(!arena.slot_is_live(slot));

        let slot = arena.allocate_unbound();
        arena.bind_shell(slot, test_construction_facts());
        assert_eq!(arena.data(slot).kind.get(), NodeKind::Box);
        assert!(arena.data(slot).flags.get() & NodeFlag::HasStyle as u32 != 0);
        arena.free_subtree(slot).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn freeing_the_layout_root_forgets_it_and_the_pending_rebuilt_roots() {
        let mut arena = LayoutNodeArena::new();
        let viewport = arena.allocate_unbound();
        let rebuilt = arena.allocate_unbound();
        arena.set_layout_root(viewport);
        arena.set_pending_rebuilt_subtree_roots(vec![rebuilt], true);
        assert_eq!(arena.layout_root(), viewport);

        arena.free_subtree(rebuilt).destroy_shells_and_invoke_callbacks();
        assert_eq!(arena.layout_root(), viewport);
        assert_eq!(arena.take_pending_rebuilt_subtree_roots(), (vec![rebuilt], true));

        arena.set_pending_rebuilt_subtree_roots(vec![viewport], false);
        arena.free_subtree(viewport).destroy_shells_and_invoke_callbacks();
        assert!(arena.layout_root().is_invalid());
        assert_eq!(arena.take_pending_rebuilt_subtree_roots(), (Vec::new(), false));
    }

    #[test]
    fn an_anonymous_box_stamped_by_the_arena_keeps_its_style_record_until_freed() {
        let mut arena = LayoutNodeArena::new();
        let payloads = [std::ptr::null::<c_void>(); 1];
        let slot = arena.allocate_unbound();
        arena.stamp_anonymous_box(
            slot,
            NodeKind::InlineNode,
            DerivedStyleRecord {
                record: 7,
                payloads: crate::layout::node_data::StylePayloadsRef::new(payloads.as_ptr().cast()),
            },
        );
        assert_eq!(arena.data(slot).kind.get(), NodeKind::InlineNode);
        assert!(arena.data(slot).flags.get() & NodeFlag::Anonymous as u32 != 0);
        assert!(arena.data(slot).flags.get() & NodeFlag::HasStyle as u32 != 0);
        assert_eq!(arena.node_style_record(slot), 7);
        assert!(arena.node_style_record_is_pinned_by_arena(slot));

        let element = arena.allocate(test_construction_facts_with_kind(NodeKind::InlineNode));
        arena.set_node_style(element, 9, payloads.as_ptr().cast());
        assert_eq!(arena.node_style_record(element), 9);
        assert!(!arena.node_style_record_is_pinned_by_arena(element));

        let freed = arena.free_subtree(slot);
        assert_eq!(freed.arena_pinned_style_record_count(), 1);
        freed.destroy_shells_and_invoke_callbacks();
        let freed = arena.free_subtree(element);
        assert_eq!(freed.arena_pinned_style_record_count(), 0);
        freed.destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn previous_dom_backed_or_generated_node_skips_anonymous_slots() {
        let mut arena = LayoutNodeArena::new();
        let root = arena.allocate(test_construction_facts());
        let anonymous_wrapper = arena.allocate(test_anonymous_construction_facts());
        let nested_anonymous = arena.allocate(test_anonymous_construction_facts());
        let element = arena.allocate(test_construction_facts());
        arena.insert_child(root, anonymous_wrapper, NodeSlotId::INVALID);
        arena.insert_child(anonymous_wrapper, nested_anonymous, NodeSlotId::INVALID);
        arena.insert_child(root, element, NodeSlotId::INVALID);

        assert_eq!(arena.previous_dom_backed_or_generated_node(element, false), root);
        assert!(arena.previous_dom_backed_or_generated_node(element, true).is_invalid());
        assert!(arena.previous_dom_backed_or_generated_node(root, false).is_invalid());

        arena.data(nested_anonymous).generated_for.set(1);
        assert_eq!(
            arena.previous_dom_backed_or_generated_node(element, false),
            nested_anonymous
        );

        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn rows_are_dom_backed_only_while_their_slot_is_live() {
        let mut arena = LayoutNodeArena::new();
        let anonymous = arena.allocate(test_anonymous_construction_facts());
        let element = arena.allocate(test_construction_facts());
        assert!(!arena.node_is_dom_backed(anonymous));
        assert!(arena.node_is_dom_backed(element));
        assert_eq!(arena.live_slot_count(), 2);

        arena.free_subtree(element).destroy_shells_and_invoke_callbacks();
        assert!(!arena.node_is_dom_backed(element));
        let reoccupant = arena.allocate_for_test();
        assert_eq!(reoccupant.slot.slot_index(), element.slot_index());
        assert!(!arena.node_is_dom_backed(element));
        assert!(!arena.node_is_dom_backed(reoccupant.slot));

        arena.free_subtree(anonymous).destroy_shells_and_invoke_callbacks();
        arena
            .free_subtree(reoccupant.slot)
            .destroy_shells_and_invoke_callbacks();
        assert_eq!(arena.live_slot_count(), 0);
    }

    #[test]
    fn committed_geometry_requires_a_current_layout_commit() {
        if super::super::fc_run_cache::fc_run_cache_mode_from_environment()
            == super::super::fc_run_cache::FcRunCacheMode::Disabled
        {
            return;
        }
        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        let current = |arena: &LayoutNodeArena| arena.with_current_committed_fragment(node, |fragment| fragment.node);
        let commit_from_layout = |arena: &LayoutNodeArena| {
            let data = arena.data(node);
            arena.set_committed_fragment_link(
                data,
                test_fragment_link(node),
                arena.epoch_of_geometry_laid_out_in_this_pass(data),
            );
        };
        commit_from_layout(&arena);
        assert_eq!(current(&arena), Some(node));

        arena.bump_fragment_cache_epoch_of_self_and_ancestors(node);
        assert_eq!(current(&arena), None);
        commit_from_layout(&arena);
        assert_eq!(current(&arena), Some(node));

        let moved = arena.take_committed_fragment_link(arena.data(node)).unwrap();
        arena.set_committed_fragment_link(arena.data(node), moved, None);
        assert_eq!(current(&arena), None);

        arena.begin_active_layout_pass();
        arena.bump_fragment_cache_epoch_of_self_and_ancestors(node);
        commit_from_layout(&arena);
        arena.end_active_layout_pass();
        assert_eq!(current(&arena), None);
        arena.begin_active_layout_pass();
        commit_from_layout(&arena);
        arena.end_active_layout_pass();
        assert_eq!(current(&arena), Some(node));

        arena.data(node).fragment_cache_epoch.set(0);
        commit_from_layout(&arena);
        arena.data(node).fragment_cache_epoch.set(u32::MAX);
        arena.bump_fragment_cache_epoch_of_self_and_ancestors(node);
        assert_eq!(arena.data(node).fragment_cache_epoch.get(), 0);
        assert_eq!(current(&arena), None);
    }

    fn test_fragment_link(node: NodeSlotId) -> fragment_tree::FragmentLink {
        fragment_tree::FragmentLink {
            fragment: std::sync::Arc::new(fragment_tree::Fragment {
                identity: 1,
                node,
                content_inline_size: CssPixels::default(),
                content_block_size: CssPixels::default(),
                margin_left: CssPixels::default(),
                margin_right: CssPixels::default(),
                margin_top: CssPixels::default(),
                margin_bottom: CssPixels::default(),
                border_left: CssPixels::default(),
                border_right: CssPixels::default(),
                border_top: CssPixels::default(),
                border_bottom: CssPixels::default(),
                padding_left: CssPixels::default(),
                padding_right: CssPixels::default(),
                padding_top: CssPixels::default(),
                padding_bottom: CssPixels::default(),
                uses_collapsing_borders_model: false,
                is_collapsed_borders_table_box: false,
                table_column_index: 0,
                table_column_span: 0,
                hidden_by_collapsed_columns: false,
                collapsed_table_borders: None,
                line_data: None,
                grid_layout_data: None,
                flex_layout_data: None,
                used_grid_tracks: None,
                svg: Default::default(),
                computed_svg_path: None,
                has_line_clamp_point: false,
                is_invisible_for_line_clamp: false,
                children: Vec::new(),
            }),
            committed_offset: Default::default(),
            inset_left: CssPixels::default(),
            inset_right: CssPixels::default(),
            inset_top: CssPixels::default(),
            inset_bottom: CssPixels::default(),
            containing_line_box_index: None,
            abspos_layout_inputs: None,
            containing_block: NodeSlotId::INVALID,
        }
    }

    #[test]
    fn node_data_addresses_remain_stable_when_chunks_are_added() {
        let mut arena = LayoutNodeArena::new();
        let first = arena.allocate_for_test();
        let first_data_address = std::ptr::from_ref(arena.data(first.slot)) as usize;

        let mut allocations = Vec::new();
        for _ in 0..SLOTS_PER_CHUNK * 2 {
            allocations.push(arena.allocate_for_test());
        }

        assert_eq!(first_data_address, std::ptr::from_ref(arena.data(first.slot)) as usize);
        arena.data(first.slot).table_column_span.set(42);
        assert_eq!(arena.data(first.slot).table_column_span.get(), 42);
        arena.free_subtree(first.slot).destroy_shells_and_invoke_callbacks();
        for allocation in allocations {
            arena
                .free_subtree(allocation.slot)
                .destroy_shells_and_invoke_callbacks();
        }
    }

    #[test]
    fn node_data_slots_are_cache_line_aligned() {
        assert_eq!(align_of::<Chunk>() % 64, 0);
        let mut arena = LayoutNodeArena::new();
        let allocation = arena.allocate_for_test();
        assert_eq!(std::ptr::from_ref(arena.data(allocation.slot)) as usize % 64, 0);
        arena
            .free_subtree(allocation.slot)
            .destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn freed_slots_are_reused_with_a_new_generation() {
        let mut arena = LayoutNodeArena::new();
        let first = arena.allocate_for_test();
        arena.free_subtree(first.slot).destroy_shells_and_invoke_callbacks();

        let second = arena.allocate_for_test();
        assert_eq!(second.slot.slot_index(), first.slot.slot_index());
        assert_ne!(second.slot, first.slot);
        assert_ne!(second.slot.generation(), first.slot.generation());
        arena.free_subtree(second.slot).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn default_scroll_shift_anchors_behave_like_weak_references() {
        let mut arena = LayoutNodeArena::new();
        let positioned = arena.allocate_for_test();
        let anchor = arena.allocate_for_test();

        arena.set_default_scroll_shift(positioned.slot, anchor.slot, true, false);
        assert!(arena.may_have_default_scroll_shift_anchor());
        assert_eq!(arena.default_scroll_shift_anchor(positioned.slot), anchor.slot);
        let flags = arena.data(positioned.slot).flags.get();
        assert_ne!(flags & NodeFlag::CompensatesForHorizontalScroll as u32, 0);
        assert_eq!(flags & NodeFlag::CompensatesForVerticalScroll as u32, 0);

        arena.free_subtree(anchor.slot).destroy_shells_and_invoke_callbacks();
        assert!(arena.default_scroll_shift_anchor(positioned.slot).is_invalid());

        let anchor_slot_reoccupant = arena.allocate_for_test();
        assert_eq!(anchor_slot_reoccupant.slot.slot_index(), anchor.slot.slot_index());
        assert!(arena.default_scroll_shift_anchor(positioned.slot).is_invalid());

        arena.set_default_scroll_shift(positioned.slot, anchor_slot_reoccupant.slot, true, true);
        assert_eq!(
            arena.default_scroll_shift_anchor(positioned.slot),
            anchor_slot_reoccupant.slot
        );
        arena.set_default_scroll_shift(positioned.slot, NodeSlotId::INVALID, false, false);
        assert!(arena.default_scroll_shift_anchor(positioned.slot).is_invalid());
        let cleared_flags = arena.data(positioned.slot).flags.get();
        assert_eq!(cleared_flags & NodeFlag::CompensatesForHorizontalScroll as u32, 0);
        assert_eq!(cleared_flags & NodeFlag::CompensatesForVerticalScroll as u32, 0);

        arena
            .free_subtree(positioned.slot)
            .destroy_shells_and_invoke_callbacks();
        let positioned_slot_reoccupant = arena.allocate_for_test();
        assert_eq!(
            positioned_slot_reoccupant.slot.slot_index(),
            positioned.slot.slot_index()
        );
        arena.set_default_scroll_shift(positioned_slot_reoccupant.slot, anchor_slot_reoccupant.slot, true, true);
        arena
            .free_subtree(positioned_slot_reoccupant.slot)
            .destroy_shells_and_invoke_callbacks();
        let next_reoccupant = arena.allocate_for_test();
        assert!(arena.default_scroll_shift_anchor(next_reoccupant.slot).is_invalid());

        arena
            .free_subtree(next_reoccupant.slot)
            .destroy_shells_and_invoke_callbacks();
        arena
            .free_subtree(anchor_slot_reoccupant.slot)
            .destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn default_scroll_shift_input_changes_note_the_positioned_box_dirty() {
        use crate::painting::visual_context::dirty::{BoxDirtyBits, VisualContextBoxDirtyKind};
        let mut arena = LayoutNodeArena::new();
        let positioned = arena.allocate_for_test();
        let anchor = arena.allocate_for_test();
        let other_anchor = arena.allocate_for_test();
        let take_dirty_bits = |arena: &LayoutNodeArena| -> Option<BoxDirtyBits> {
            let mut paint_state = arena.paint_state().borrow_mut();
            let bits = paint_state
                .visual_context
                .dirty_boxes
                .boxes
                .get(&positioned.slot)
                .copied();
            paint_state.visual_context.dirty_boxes.clear();
            bits
        };
        let notes_scroll_shift_change = |arena: &LayoutNodeArena| {
            take_dirty_bits(arena)
                .is_some_and(|bits| bits.contains(VisualContextBoxDirtyKind::DefaultScrollShiftInputsChanged))
        };

        arena.set_default_scroll_shift(positioned.slot, NodeSlotId::INVALID, false, false);
        assert!(!notes_scroll_shift_change(&arena));

        arena.set_default_scroll_shift(positioned.slot, anchor.slot, true, false);
        assert!(notes_scroll_shift_change(&arena));
        arena.set_default_scroll_shift(positioned.slot, anchor.slot, true, false);
        assert!(!notes_scroll_shift_change(&arena));

        arena.set_default_scroll_shift(positioned.slot, anchor.slot, true, true);
        assert!(notes_scroll_shift_change(&arena));
        arena.set_default_scroll_shift(positioned.slot, other_anchor.slot, true, true);
        assert!(notes_scroll_shift_change(&arena));
        arena.set_default_scroll_shift(positioned.slot, NodeSlotId::INVALID, false, false);
        assert!(notes_scroll_shift_change(&arena));

        let mut anchored_pairs = Vec::new();
        arena.set_default_scroll_shift(positioned.slot, anchor.slot, false, true);
        arena.for_each_default_scroll_shift_anchor(|positioned, anchor| anchored_pairs.push((positioned, anchor)));
        assert_eq!(anchored_pairs, vec![(positioned.slot, anchor.slot)]);
        arena.free_subtree(anchor.slot).destroy_shells_and_invoke_callbacks();
        anchored_pairs.clear();
        arena.for_each_default_scroll_shift_anchor(|positioned, anchor| anchored_pairs.push((positioned, anchor)));
        assert!(anchored_pairs.is_empty());

        arena
            .free_subtree(positioned.slot)
            .destroy_shells_and_invoke_callbacks();
        arena
            .free_subtree(other_anchor.slot)
            .destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn layout_update_flags_are_reset_only_in_the_requested_subtree() {
        let mut arena = LayoutNodeArena::new();
        let root = arena.allocate_for_test();
        let child = arena.allocate_for_test();
        let detached = arena.allocate_for_test();
        arena.data(root.slot).kind.set(NodeKind::Viewport);
        arena.insert_child(root.slot, child.slot, NodeSlotId::INVALID);
        let update_flags = NodeFlag::NeedsLayoutUpdate as u32 | NodeFlag::NeedsOwnGeometryUpdate as u32;
        for node in [root.slot, child.slot, detached.slot] {
            arena.set_node_flag(node, NodeFlag::NeedsLayoutUpdate, true);
            arena.set_node_flag(node, NodeFlag::NeedsOwnGeometryUpdate, true);
        }

        arena.reset_layout_update_flags_in_subtree(child.slot);
        assert_eq!(arena.data(root.slot).flags.get() & update_flags, update_flags);
        assert_eq!(arena.data(child.slot).flags.get() & update_flags, 0);
        // An own-geometry update must be found even when its ancestors are clean.
        arena.reset_layout_update_flags_in_subtree(root.slot);
        arena.set_node_flag(child.slot, NodeFlag::NeedsOwnGeometryUpdate, true);
        arena.set_node_flag(child.slot, NodeFlag::NeedsOwnGeometryUpdate, true);
        arena.reset_layout_update_flags_in_subtree(root.slot);

        assert_eq!(arena.data(root.slot).flags.get() & update_flags, 0);
        assert_eq!(arena.data(child.slot).flags.get() & update_flags, 0);
        assert_eq!(arena.data(detached.slot).flags.get() & update_flags, update_flags);
        arena.remove_child(root.slot, child.slot);
        arena.free_subtree(root.slot).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(child.slot).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(detached.slot).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn independent_partial_commits_remove_only_their_own_dirty_nodes() {
        let mut arena = LayoutNodeArena::new();
        let viewport = arena.allocate_for_test();
        arena.data(viewport.slot).kind.set(NodeKind::Viewport);
        let mut boundaries = Vec::new();
        for _ in 0..32 {
            let boundary = arena.allocate_for_test();
            let child = arena.allocate_for_test();
            arena.insert_child(viewport.slot, boundary.slot, NodeSlotId::INVALID);
            arena.insert_child(boundary.slot, child.slot, NodeSlotId::INVALID);
            arena.set_node_flag(boundary.slot, NodeFlag::NeedsLayoutUpdate, true);
            arena.set_node_flag(child.slot, NodeFlag::NeedsOwnGeometryUpdate, true);
            boundaries.push((boundary, child));
        }
        for (index, (boundary, child)) in boundaries.iter().enumerate() {
            arena.reset_layout_update_flags_in_subtree(boundary.slot);
            assert_eq!(arena.data(boundary.slot).flags.get(), 0);
            assert_eq!(arena.data(child.slot).flags.get(), 0);
            assert_eq!(arena.nodes_with_layout_update_flags.borrow().len(), (31 - index) * 2);
            assert_eq!(arena.layout_update_flag_node_indices.borrow().len(), (31 - index) * 2);
            // Re-enrollment after removal must not leave duplicates or stale indices.
            arena.set_node_flag(child.slot, NodeFlag::NeedsLayoutUpdate, true);
            arena.set_node_flag(child.slot, NodeFlag::NeedsLayoutUpdate, false);
        }
        assert!(arena.nodes_with_layout_update_flags.borrow().is_empty());
        arena.set_node_flag(boundaries[0].0.slot, NodeFlag::NeedsLayoutUpdate, true);
        arena.free_subtree(viewport.slot).destroy_shells_and_invoke_callbacks();
        assert!(arena.nodes_with_layout_update_flags.borrow().is_empty());
        assert!(arena.layout_update_flag_node_indices.borrow().is_empty());
    }

    #[test]
    fn full_commit_checks_shared_dirty_ancestor_chains_once() {
        let mut arena = LayoutNodeArena::new();
        let viewport = arena.allocate_for_test();
        arena.data(viewport.slot).kind.set(NodeKind::Viewport);
        let detached = arena.allocate_for_test();
        let mut dirty_nodes = Vec::new();
        for root in [viewport.slot, detached.slot] {
            let mut parent = root;
            for _ in 0..64 {
                let child = arena.allocate_for_test();
                arena.insert_child(parent, child.slot, NodeSlotId::INVALID);
                dirty_nodes.push(child.slot);
                parent = child.slot;
            }
        }
        for &node in dirty_nodes.iter().rev() {
            arena.set_node_flag(node, NodeFlag::NeedsLayoutUpdate, true);
        }
        arena.reset_layout_update_flags_in_subtree(viewport.slot);
        // Each chain node and the detached root is inspected once, independent of depth.
        assert_eq!(arena.layout_update_flag_ancestor_visits.get(), 129);
        for (index, &node) in dirty_nodes.iter().enumerate() {
            assert_eq!(
                arena.data(node).flags.get() & NodeFlag::NeedsLayoutUpdate as u32 != 0,
                index >= 64
            );
        }
        assert_eq!(arena.nodes_with_layout_update_flags.borrow().len(), 64);
        arena.free_subtree(viewport.slot).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(detached.slot).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn stale_slot_ids_do_not_resolve_to_a_new_occupant() {
        let mut arena = LayoutNodeArena::new();
        let first = arena.allocate_for_test();
        arena.free_subtree(first.slot).destroy_shells_and_invoke_callbacks();
        let second = arena.allocate_for_test();

        let stale_read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arena.data(first.slot)));
        assert!(stale_read.is_err());
        arena.free_subtree(second.slot).destroy_shells_and_invoke_callbacks();
    }

    fn test_abspos_layout_inputs() -> AbsposLayoutInputs {
        AbsposLayoutInputs {
            containing_block: NodeSlotId::INVALID,
            inline_containing_block: NodeSlotId::INVALID,
            static_position_rect: StaticPositionRect {
                rect: Default::default(),
                inline_alignment: StaticPositionAlignment::Center,
                block_alignment: StaticPositionAlignment::End,
                alignment_derives_from_own_computed_values: true,
                is_known: true,
            },
            containing_block_info: AbsposContainingBlockInfo {
                rect: Default::default(),
                inline_axis_mode: AbsposAxisMode::StaticPosition,
                block_axis_mode: AbsposAxisMode::InsetFromRect,
                inline_alignment: None,
                block_alignment: None,
                derives_from_own_computed_values: true,
            },
            resolved_anchor_insets: None,
        }
    }

    #[test]
    fn clearing_a_committed_box_evicts_its_fragment_link_and_abspos_inputs() {
        let mut arena = LayoutNodeArena::new();
        let allocation = arena.allocate_for_test();
        let inputs = test_abspos_layout_inputs();
        let mut link = test_fragment_link(allocation.slot);
        link.abspos_layout_inputs = Some(inputs);
        arena.set_committed_fragment_link(arena.data(allocation.slot), link, None);
        assert!(arena.committed_fragment_link(arena.data(allocation.slot)).is_some());
        assert_eq!(
            arena.saved_abspos_layout_inputs(arena.data(allocation.slot)),
            Some(inputs)
        );

        // SAFETY: arena is a live handle on this thread, and allocation names
        // a live slot in it.
        unsafe {
            let handle = std::ptr::from_mut(&mut arena).cast();
            crate::layout::paying_host_handbacks(&crate::stage::MainThread::for_test(), handle, || {
                crate::painting::ffi::clear_paintable_row_of_node(handle, allocation.slot);
            });
        }

        assert!(arena.committed_fragment_link(arena.data(allocation.slot)).is_none());
        assert_eq!(arena.saved_abspos_layout_inputs(arena.data(allocation.slot)), None);
        arena
            .free_subtree(allocation.slot)
            .destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn committed_fragment_links_move_abspos_inputs_between_slots() {
        let mut arena = LayoutNodeArena::new();
        let old = arena.allocate_for_test();
        let new = arena.allocate_for_test();
        let inputs = test_abspos_layout_inputs();
        let mut link = test_fragment_link(old.slot);
        link.abspos_layout_inputs = Some(inputs);
        let retained_fragment = link.fragment.clone();
        arena.set_committed_fragment_link(arena.data(old.slot), link, None);

        let moved = arena
            .take_committed_fragment_link(arena.data(old.slot))
            .expect("old slot must retain its committed fragment");
        assert!(std::sync::Arc::ptr_eq(&moved.fragment, &retained_fragment));
        arena.set_committed_fragment_link(arena.data(new.slot), moved, None);

        assert!(arena.committed_fragment_link(arena.data(old.slot)).is_none());
        assert_eq!(arena.saved_abspos_layout_inputs(arena.data(old.slot)), None);
        assert_eq!(arena.saved_abspos_layout_inputs(arena.data(new.slot)), Some(inputs));
        let moved = arena
            .committed_fragment_link(arena.data(new.slot))
            .expect("new slot must receive the committed fragment");
        assert!(std::sync::Arc::ptr_eq(&moved.fragment, &retained_fragment));
        arena.set_committed_fragment_link(arena.data(new.slot), test_fragment_link(new.slot), None);
        assert_eq!(arena.saved_abspos_layout_inputs(arena.data(new.slot)), None);
        arena.free_subtree(old.slot).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(new.slot).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn committed_fragment_links_are_read_at_the_generation_published_with_the_rows() {
        use crate::painting::paintable_rows::PaintableRowsRead;

        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.populate_paintable_row(node);
        let mut link = test_fragment_link(node);
        link.inset_left = CssPixels::from_integer(10);
        arena.set_committed_fragment_link(arena.data(node), link.clone(), None);
        arena.publish_paintable_rows();
        link.inset_left = CssPixels::from_integer(20);
        arena.set_committed_fragment_link(arena.data(node), link, None);

        let published_inset_left =
            |arena: &LayoutNodeArena| arena.published_fragment_link_for_test(node).map(|link| link.inset_left);
        assert_eq!(published_inset_left(&arena), Some(CssPixels::from_integer(10)));
        assert_eq!(
            arena
                .paintable_rows()
                .with_committed_fragment_link(node, |link| link.map(|link| link.inset_left)),
            Some(CssPixels::from_integer(20))
        );
        assert_eq!(
            arena
                .committed_paintable_rows()
                .with_committed_fragment_link(node, |link| link.map(|link| link.inset_left)),
            Some(CssPixels::from_integer(20))
        );
        arena.free_subtree(node).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn main_side_scroll_offset_write_joins_the_frame() {
        let arena = LayoutNodeArena::new();
        drop(arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER));
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::Recording);
        let write = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER));
        }));
        assert!(
            write.is_err(),
            "a write made while a render stage runs must not pass the join"
        );
    }

    #[test]
    fn intrinsic_size_cache_validates_epoch_and_generation() {
        let mut arena = LayoutNodeArena::new();
        let caches = super::IntrinsicSizeCaches::default();
        let first = arena.allocate_for_test();
        let key = IntrinsicSizeCacheKey {
            measured_at_inline_size: Some(CssPixels::from_raw(64)),
            ..Default::default()
        };
        let value = IntrinsicBlockSizeMeasurement {
            size: CssPixels::from_raw(128),
            depends_on_percentage_block_size: false,
            depends_on_percentage_inline_basis: false,
        };
        let inline_measurement = IntrinsicInlineSizeMeasurement {
            automatic_content_inline_size: CssPixels::from_raw(192),
            min_content_inline_size_from_max_content_layout: Some(CssPixels::from_raw(96)),
            layout: Some(super::IntrinsicInlineMeasurementLayout {
                available_block_size: crate::layout::layout_node_arena::AvailableSize::MaxContent,
                content_inline_size: CssPixels::from_raw(192),
                content_block_size: CssPixels::from_raw(256),
                automatic_content_block_size: CssPixels::from_raw(320),
                uses_collapsing_borders_model: true,
                is_collapsed_borders_table_box: true,
                has_first_baseline: true,
                first_baseline: CssPixels::from_raw(64),
                has_last_baseline: true,
                last_baseline: CssPixels::from_raw(128),
            }),
            depends_on_percentage_block_size: false,
            depends_on_percentage_inline_basis: false,
        };
        let dependency_computations = Cell::new(0);

        let first_data = arena.data(first.slot);
        caches.intrinsic_block_size_cache_put(&arena, first_data, IntrinsicSizeCacheKind::MinContentBlock, key, value);
        caches.intrinsic_inline_size_measurement_cache_put(
            &arena,
            first_data,
            IntrinsicSizeCacheKind::MaxContentInline,
            key,
            inline_measurement,
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, first_data, IntrinsicSizeCacheKind::MinContentBlock, key),
            Some(value)
        );
        assert_eq!(
            caches.intrinsic_inline_size_measurement_cache_get(
                &arena,
                first_data,
                IntrinsicSizeCacheKind::MaxContentInline,
                key
            ),
            Some(inline_measurement)
        );
        assert!(
            caches.intrinsic_inline_size_depends_on_block_size(&arena, first_data, || {
                dependency_computations.set(dependency_computations.get() + 1);
                true
            })
        );
        assert!(caches.intrinsic_inline_size_depends_on_block_size(&arena, first_data, || false));
        assert_eq!(dependency_computations.get(), 1);

        first_data
            .intrinsic_cache_epoch
            .set(first_data.intrinsic_cache_epoch.get() + 1);
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, first_data, IntrinsicSizeCacheKind::MinContentBlock, key),
            None
        );
        assert_eq!(
            caches.intrinsic_inline_size_measurement_cache_get(
                &arena,
                first_data,
                IntrinsicSizeCacheKind::MaxContentInline,
                key
            ),
            None
        );
        assert!(
            !caches.intrinsic_inline_size_depends_on_block_size(&arena, first_data, || {
                dependency_computations.set(dependency_computations.get() + 1);
                false
            })
        );
        assert_eq!(dependency_computations.get(), 2);
        arena.free_subtree(first.slot).destroy_shells_and_invoke_callbacks();
        caches.drop_slots(arena.take_intrinsic_size_caches_to_drop());

        let second = arena.allocate_for_test();
        assert_eq!(second.slot.slot_index(), first.slot.slot_index());
        assert_ne!(second.slot, first.slot);
        let second_data = &*arena.data(second.slot);
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, second_data, IntrinsicSizeCacheKind::MinContentBlock, key),
            None
        );
        assert_eq!(
            caches.intrinsic_inline_size_measurement_cache_get(
                &arena,
                second_data,
                IntrinsicSizeCacheKind::MaxContentInline,
                key
            ),
            None
        );
        arena.free_subtree(second.slot).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn intrinsic_size_cache_answers_masked_probes_only_from_independent_measurements() {
        let mut arena = LayoutNodeArena::new();
        let caches = super::IntrinsicSizeCaches::default();
        let allocation = arena.allocate_for_test();
        let data = arena.data(allocation.slot);
        let key_with_basis = |basis: i32| IntrinsicSizeCacheKey {
            measured_at_inline_size: Some(CssPixels::from_raw(64)),
            percentage_basis_block_size: Some(CssPixels::from_raw(basis)),
            quirks_mode_percentage_basis_block_size: Some(CssPixels::from_raw(basis)),
            ..Default::default()
        };
        let key_without_basis = IntrinsicSizeCacheKey {
            measured_at_inline_size: Some(CssPixels::from_raw(64)),
            ..Default::default()
        };
        let max_content = IntrinsicSizeCacheKind::MaxContentBlock;
        let min_content = IntrinsicSizeCacheKind::MinContentBlock;

        let independent = IntrinsicBlockSizeMeasurement {
            size: CssPixels::from_raw(128),
            depends_on_percentage_block_size: false,
            depends_on_percentage_inline_basis: false,
        };
        caches.intrinsic_block_size_cache_put(&arena, data, max_content, key_with_basis(100), independent);
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, max_content, key_with_basis(100)),
            Some(independent)
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, max_content, key_with_basis(200)),
            Some(independent)
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, max_content, key_without_basis),
            Some(independent)
        );

        let dependent = IntrinsicBlockSizeMeasurement {
            size: CssPixels::from_raw(256),
            depends_on_percentage_block_size: true,
            depends_on_percentage_inline_basis: false,
        };
        caches.intrinsic_block_size_cache_put(&arena, data, min_content, key_without_basis, dependent);
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, min_content, key_without_basis),
            Some(dependent)
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, min_content, key_with_basis(100)),
            None
        );
        caches.intrinsic_block_size_cache_put(&arena, data, min_content, key_with_basis(100), dependent);
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, min_content, key_with_basis(100)),
            Some(dependent)
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(&arena, data, min_content, key_with_basis(200)),
            None
        );

        let key_at_another_inline_size_with_inline_basis = |basis: i32| IntrinsicSizeCacheKey {
            measured_at_inline_size: Some(CssPixels::from_raw(96)),
            percentage_basis_inline_size: Some(CssPixels::from_raw(basis)),
            ..key_with_basis(100)
        };
        let observes_inline_basis = IntrinsicBlockSizeMeasurement {
            size: CssPixels::from_raw(512),
            depends_on_percentage_block_size: false,
            depends_on_percentage_inline_basis: true,
        };
        caches.intrinsic_block_size_cache_put(
            &arena,
            data,
            max_content,
            key_at_another_inline_size_with_inline_basis(300),
            observes_inline_basis,
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(
                &arena,
                data,
                max_content,
                key_at_another_inline_size_with_inline_basis(300)
            ),
            Some(observes_inline_basis)
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(
                &arena,
                data,
                max_content,
                IntrinsicSizeCacheKey {
                    percentage_basis_block_size: Some(CssPixels::from_raw(200)),
                    ..key_at_another_inline_size_with_inline_basis(300)
                }
            ),
            Some(observes_inline_basis)
        );
        assert_eq!(
            caches.intrinsic_block_size_cache_get(
                &arena,
                data,
                max_content,
                key_at_another_inline_size_with_inline_basis(400)
            ),
            None
        );
        arena
            .free_subtree(allocation.slot)
            .destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn table_cell_measurements_follow_the_intrinsic_cache_epoch() {
        let mut arena = LayoutNodeArena::new();
        let caches = super::IntrinsicSizeCaches::default();
        let first = arena.allocate_for_test();
        let key = TableCellMeasurementKey {
            layout_mode: crate::layout::layout_node_arena::LayoutMode::Normal,
            available_space: crate::layout::layout_node_arena::AvailableSpace {
                inline_size: crate::layout::layout_node_arena::AvailableSize::definite(CssPixels::from_raw(640)),
                block_size: crate::layout::layout_node_arena::AvailableSize::Indefinite,
            },
            content_inline_size: CssPixels::from_raw(640),
            content_block_size: CssPixels::default(),
            has_definite_inline_size: true,
            has_definite_block_size: false,
            inline_size_constraint: used_values::SizeConstraint::None,
            block_size_constraint: used_values::SizeConstraint::None,
            uses_collapsing_borders_model: true,
            adopt_automatic_content_block_size: true,
        };
        let value = TableCellMeasurement {
            automatic_content_block_size: CssPixels::from_raw(320),
            baselines: crate::layout::layout_node_arena::DerivedBaselines {
                first: Some(CssPixels::from_raw(64)),
                last: None,
            },
            depends_on_percentage_block_size: true,
        };

        let first_data = arena.data(first.slot);
        assert_eq!(caches.table_cell_measurement_cache_get(&arena, first_data, key), None);
        caches.table_cell_measurement_cache_put(&arena, first_data, key, value);
        assert_eq!(
            caches.table_cell_measurement_cache_get(&arena, first_data, key),
            Some(value)
        );
        let percentage_resolved_key = TableCellMeasurementKey {
            content_block_size: CssPixels::from_raw(512),
            has_definite_block_size: true,
            adopt_automatic_content_block_size: false,
            ..key
        };
        assert_eq!(
            caches.table_cell_measurement_cache_get(&arena, first_data, percentage_resolved_key),
            None
        );

        first_data
            .intrinsic_cache_epoch
            .set(first_data.intrinsic_cache_epoch.get() + 1);
        assert_eq!(caches.table_cell_measurement_cache_get(&arena, first_data, key), None);
        arena.free_subtree(first.slot).destroy_shells_and_invoke_callbacks();
        caches.drop_slots(arena.take_intrinsic_size_caches_to_drop());

        let second = arena.allocate_for_test();
        assert_eq!(second.slot.slot_index(), first.slot.slot_index());
        let second_data = &*arena.data(second.slot);
        assert_eq!(caches.table_cell_measurement_cache_get(&arena, second_data, key), None);
        arena.free_subtree(second.slot).destroy_shells_and_invoke_callbacks();
    }
}

// Every column is sendable, the style engine link because the engine is, so a stage that holds
// the arena exclusively may run on another thread.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<LayoutNodeArena>();
};
