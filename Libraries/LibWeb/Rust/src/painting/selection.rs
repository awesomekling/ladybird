/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::css_pixels::CssPixels;
use crate::css::style::tree::StyleNodeID;
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::painting::display_list::commands::OptionalColor;
use crate::painting::fragment_ownership;
use crate::painting::host::FfiSelectionStyleFacts;
use crate::painting::paintable_data::{
    FfiSelectionEntry, FfiSelectionSnapshot, FfiSelectionSnapshotNode, FfiSelectionSnapshotRole, SELECTION_STATE_END,
    SELECTION_STATE_FULL, SELECTION_STATE_NONE, SELECTION_STATE_START, SELECTION_STATE_START_AND_END,
};
use crate::painting::paintable_rows::PaintableRowsMut;
use crate::painting::record::damage::PaintDamage;
use crate::painting::record::paint::text::{SelectionStyleAnswer, ShadowLayer};
use crate::painting::text_fragment;
use libgfx_rust::Color;

#[derive(Debug)]
pub(crate) struct SelectionRange {
    pub start_offset: usize,
    pub end_offset: usize,
    pub text_states: std::collections::HashMap<NodeSlotId, u8>,
}

fn invalidate_text_node(layout_arena: &PaintableRowsMut<'_>, node: NodeSlotId) {
    if let Some(containing_block) = text_fragment::containing_block_paintable_of_node(layout_arena, node) {
        layout_arena.push_paint_damage(containing_block, PaintDamage::ALL_DRAW);
    }
    if let Some(inline_box) = fragment_ownership::nearest_self_painting_inline_box(layout_arena, node) {
        layout_arena.push_paint_damage(inline_box, PaintDamage::ALL_DRAW);
    }
}

pub(crate) fn clear(layout_arena: &mut PaintableRowsMut<'_>, viewport: NodeSlotId) {
    let _writer = crate::painting::published_immutable::enter_writer_if_unattributed("selection update");
    let previous = layout_arena.paint_state().borrow_mut().selection.take();
    if let Some(previous) = previous {
        for node in previous.text_states.keys() {
            invalidate_text_node(layout_arena, *node);
        }
    }
    let mut slots = Vec::new();
    crate::painting::paint_order::for_each_in_paint_subtree(layout_arena, viewport, |slot| {
        slots.push(slot);
    });
    for current in slots {
        if layout_arena.paintable_data(current).selection_state != SELECTION_STATE_NONE {
            layout_arena.paintable_data_mut(current).selection_state = SELECTION_STATE_NONE;
            layout_arena.push_paint_damage(current, PaintDamage::ALL_DRAW);
        }
    }
}

pub(crate) fn apply(
    layout_arena: &mut PaintableRowsMut<'_>,
    viewport: NodeSlotId,
    entries: &[FfiSelectionEntry],
) -> std::collections::HashMap<NodeSlotId, u8> {
    let _writer = crate::painting::published_immutable::enter_writer_if_unattributed("selection update");
    clear(layout_arena, viewport);
    let mut text_states = std::collections::HashMap::new();
    for entry in entries {
        if entry.is_text_node_entry {
            for &node in layout_arena.text_fragments(entry.layout_node).as_slice() {
                text_states.insert(node, entry.state);
                invalidate_text_node(layout_arena, node);
            }
        } else {
            if !layout_arena.paintable_row_is_populated(entry.layout_node) {
                continue;
            }
            if layout_arena.paintable_data(entry.layout_node).selection_state != entry.state {
                layout_arena.paintable_data_mut(entry.layout_node).selection_state = entry.state;
                layout_arena.push_paint_damage(entry.layout_node, PaintDamage::ALL_DRAW);
            }
        }
    }
    text_states
}

/// A selection range as the document read it: which nodes it reaches and what excludes them from
/// selection are settled on the document's side, and which of them have a box to stamp is read from
/// the rows they are bound to when the snapshot is applied. A layout commit can therefore stamp the
/// boxes it built from the snapshot the frame read before it, without the range.
#[derive(Clone)]
pub(crate) struct SelectionSnapshot {
    nodes: Vec<FfiSelectionSnapshotNode>,
    start_offset: usize,
    end_offset: usize,
    starts_and_ends_in_one_container: bool,
}

impl SelectionSnapshot {
    /// # Safety
    ///
    /// `snapshot.nodes` must address `snapshot.node_count` nodes for the call.
    pub(crate) unsafe fn from_ffi(snapshot: &FfiSelectionSnapshot) -> Self {
        let nodes = if snapshot.node_count == 0 {
            Vec::new()
        } else {
            // SAFETY: Guaranteed by the caller.
            unsafe { std::slice::from_raw_parts(snapshot.nodes, snapshot.node_count) }.to_vec()
        };
        Self {
            nodes,
            start_offset: snapshot.start_offset,
            end_offset: snapshot.end_offset,
            starts_and_ends_in_one_container: snapshot.starts_and_ends_in_one_container,
        }
    }

    fn container(&self, role: FfiSelectionSnapshotRole) -> Option<&FfiSelectionSnapshotNode> {
        self.nodes.iter().find(|node| node.role == role)
    }

    /// The selection states of the rows the snapshot's nodes are bound to.
    fn entries(&self, arena: &LayoutNodeArena) -> Vec<FfiSelectionEntry> {
        let row = |node: &FfiSelectionSnapshotNode| {
            if node.is_document {
                return arena.bound_viewport_row();
            }
            StyleNodeID::from_raw(node.style_node).map_or(NodeSlotId::INVALID, |style_node| arena.bound_row(style_node))
        };
        let has_row = |node: &FfiSelectionSnapshotNode| !row(node).is_invalid();

        // https://drafts.csswg.org/css-ui/#valdef-user-select-none
        // "The content of the element must be excluded from selection by [...] the selection methods of the
        // Selection API and the like." We honor this by leaving such nodes at SelectionState::None, even when they
        // fall inside the range. So, the selection highlight skips them.
        let is_excluded_from_selection =
            |node: &FfiSelectionSnapshotNode| node.is_inert || (has_row(node) && node.user_select_is_none);

        let mut entries = Vec::new();
        let mut set_selection_state = |node: &FfiSelectionSnapshotNode, state: u8| {
            let layout_node = row(node);
            if layout_node.is_invalid() || (!node.is_text && !arena.paintable_row_is_populated(layout_node)) {
                return;
            }
            entries.push(FfiSelectionEntry {
                is_text_node_entry: node.is_text,
                layout_node,
                state,
            });
        };

        let start_container = self.container(FfiSelectionSnapshotRole::StartContainer);
        let end_container = self.container(FfiSelectionSnapshotRole::EndContainer);

        // 2. If the selection starts and ends in the same node:
        if self.starts_and_ends_in_one_container {
            // 1. If the selection starts and ends at the same offset, return.
            if self.start_offset == self.end_offset {
                // NOTE: A zero-length selection should not be visible.
                return entries;
            }

            // 2. If it's a text node, mark it as StartAndEnd and return.
            if let Some(start_container) =
                start_container.filter(|node| node.is_text && !is_excluded_from_selection(node))
            {
                set_selection_state(start_container, SELECTION_STATE_START_AND_END);
                return entries;
            }
        }

        // 3. Mark the selection start node as Start (if text) or Full (if anything else).
        if let Some(start_container) = start_container.filter(|node| !is_excluded_from_selection(node) && has_row(node))
        {
            let state = if start_container.is_text {
                SELECTION_STATE_START
            } else {
                SELECTION_STATE_FULL
            };
            set_selection_state(start_container, state);
        }

        // 4. Mark the nodes between the start and end of the selection as Full.
        for node in self
            .nodes
            .iter()
            .filter(|node| node.role == FfiSelectionSnapshotRole::Covered)
        {
            if !is_excluded_from_selection(node) {
                set_selection_state(node, SELECTION_STATE_FULL);
            }
        }

        // 5. Mark the selection end node as End if it is a text node.
        if let Some(end_container) =
            end_container.filter(|node| !is_excluded_from_selection(node) && node.is_text && has_row(node))
        {
            set_selection_state(end_container, SELECTION_STATE_END);
        }

        entries
    }

    /// Stamps the selection states of the rows the snapshot's nodes are bound to, in place of the
    /// ones stamped before.
    pub(crate) fn apply(&self, arena: &mut LayoutNodeArena) {
        let viewport = arena.bound_viewport_row();
        if viewport.is_invalid() || !arena.paintable_row_is_populated(viewport) {
            return;
        }
        let entries = self.entries(arena);
        let text_states = apply(&mut arena.paintable_rows_mut(), viewport, &entries);
        arena.paint_state().borrow_mut().selection = Some(std::sync::Arc::new(SelectionRange {
            start_offset: self.start_offset,
            end_offset: self.end_offset,
            text_states,
        }));
    }
}

const SELECTION_PSEUDO_KIND: u8 = 6;

/// https://drafts.csswg.org/css-pseudo-4/#highlight-styling
/// What selected text under `element` paints with, read from the element's published
/// `::selection` record, or `None` when that record styles nothing a selection paints.
fn selection_pseudo_style_of_element(
    engine: &crate::css::style::StyleEngine,
    element: StyleNodeID,
) -> Option<SelectionStyleAnswer> {
    use crate::css::computed_longhand_table::{HIGHLIGHT_COLOR_IS_CURRENT_COLOR, HIGHLIGHT_COLORS_AUTHORED};
    let style = engine.published_style_view(element, Some(SELECTION_PSEUDO_KIND))?;
    let dependency_flags = engine
        .published_style_dependency_flags(element, Some(SELECTION_PSEUDO_KIND))
        .unwrap_or(0);
    // https://drafts.csswg.org/css-pseudo-4/#paired-defaults
    // Paired default highlight colors must only be used when neither 'color' nor 'background-color' yield a
    // cascaded value from the author origin (or inherit their value from the author origin).
    let mut facts = FfiSelectionStyleFacts {
        colors_authored: dependency_flags & HIGHLIGHT_COLORS_AUTHORED != 0,
        ..Default::default()
    };
    if facts.colors_authored {
        facts.background_color = Color(style.background().background_color);
        // https://drafts.csswg.org/css-pseudo-4/#highlight-text
        // currentColor on a highlight pseudo-element's 'color' property represents the color of the next active
        // highlight pseudo-element layer below, falling back finally to the colors that would otherwise have been
        // used.
        if dependency_flags & HIGHLIGHT_COLOR_IS_CURRENT_COLOR == 0 {
            facts.text_color = OptionalColor::some(Color(style.inherited_text().color));
        }

        // https://drafts.csswg.org/css-pseudo-4/#highlight-replaced
        // This wash should be of the specified 'background-color' if that is not 'transparent', else of the
        // specified 'color'; however the UA may adjust the alpha channel.
        let mut wash_color = facts.background_color;
        if wash_color.alpha() == 0 {
            wash_color = facts.text_color.get().unwrap_or_else(|| {
                Color(
                    engine
                        .published_style_view(element, None)
                        .map_or(0, |element_style| element_style.inherited_text().color),
                )
            });
        }
        facts.wash_color = transform_selection_background_color(wash_color);
    }

    let shadows: Vec<ShadowLayer> = style
        .inherited_text()
        .text_shadow
        .as_slice()
        .iter()
        .map(|shadow| ShadowLayer {
            color: shadow.color,
            offset_x: CssPixels::from_raw(shadow.offset_x),
            offset_y: CssPixels::from_raw(shadow.offset_y),
            blur_radius: CssPixels::from_raw(shadow.blur_radius),
        })
        .collect();
    facts.has_text_shadow = !shadows.is_empty();

    let text_reset = style.text_reset();
    let lines = text_reset.text_decoration_lines.as_slice();
    if !lines.is_empty() {
        facts.has_text_decoration = true;
        let count = lines.len().min(facts.text_decoration_lines.len());
        facts.text_decoration_lines[..count].copy_from_slice(&lines[..count]);
        facts.text_decoration_line_count = count as u32;
        facts.text_decoration_style = text_reset.text_decoration_style;
        facts.text_decoration_color = Color(text_reset.text_decoration_color);
    }

    (facts.colors_authored || facts.has_text_shadow || facts.has_text_decoration)
        .then_some(SelectionStyleAnswer { facts, shadows })
}

/// The UA's adjustment of a selection wash's alpha channel: an opaque color becomes the most
/// transparent color that, over white, still looks the same.
fn transform_selection_background_color(color: Color) -> Color {
    if color.alpha() < 255 {
        return color;
    }

    const START_ALPHA: i32 = 153; // 60%
    const END_ALPHA: i32 = 204; // 80%
    const ALPHA_INCREMENT: i32 = 17;

    let blend_component = |component: u8, alpha: i32| (i32::from(component) - (255 - alpha)) * 255 / alpha;

    let mut result = Color::default();
    let mut alpha = START_ALPHA;
    while alpha <= END_ALPHA {
        let red = blend_component(color.red(), alpha);
        let green = blend_component(color.green(), alpha);
        let blue = blend_component(color.blue(), alpha);
        result = Color::from_rgba(
            red.clamp(0, 255) as u8,
            green.clamp(0, 255) as u8,
            blue.clamp(0, 255) as u8,
            alpha as u8,
        );
        if red >= 0 && green >= 0 && blue >= 0 {
            break;
        }
        alpha += ALPHA_INCREMENT;
    }
    result
}

/// Gives the rows that paint text under `element` what its published `::selection` record says
/// selected text paints with: the element's own rows, or, while it has no box, the rows of its
/// text children, which then have no element row above them to find it on.
pub(crate) fn sync_selection_pseudo_style(arena: &LayoutNodeArena, element: StyleNodeID) {
    let (answer, text_children) = arena.with_style_store(|engine| {
        let answer = selection_pseudo_style_of_element(engine, element).map(std::sync::Arc::new);
        let text_children: Vec<StyleNodeID> = engine
            .tree()
            .dom_children(element)
            .filter(|child| child.is_text())
            .collect();
        (answer, text_children)
    });
    let element_row = arena.bound_row(element);
    let text_rows: Vec<NodeSlotId> = text_children
        .into_iter()
        .map(|text| arena.bound_row(text))
        .filter(|row| !row.is_invalid())
        .flat_map(|row| arena.rows_sharing_dom_node_with(row))
        .collect();
    if element_row.is_invalid() {
        set_selection_pseudo_style_of_rows(arena, text_rows, answer.as_ref());
    } else {
        set_selection_pseudo_style_of_rows(arena, text_rows, None);
        set_selection_pseudo_style_of_rows(arena, arena.rows_sharing_dom_node_with(element_row), answer.as_ref());
    }
}

/// Gives a row the tree build made what the published `::selection` record of `element` says
/// selected text paints with: `element`'s own row, or a text row whose parent element has no box.
pub(crate) fn note_built_row_selection_pseudo_style(arena: &LayoutNodeArena, row: NodeSlotId, element: StyleNodeID) {
    let answer = arena
        .with_style_store(|engine| selection_pseudo_style_of_element(engine, element))
        .map(std::sync::Arc::new);
    set_selection_pseudo_style_of_rows(arena, arena.rows_sharing_dom_node_with(row), answer.as_ref());
}

fn set_selection_pseudo_style_of_rows(
    arena: &LayoutNodeArena,
    rows: Vec<NodeSlotId>,
    answer: Option<&std::sync::Arc<SelectionStyleAnswer>>,
) {
    let mut paint_state = arena.paint_state().borrow_mut();
    let styles = &mut paint_state.selection_pseudo_styles;
    for row in rows {
        match answer {
            Some(answer) => std::sync::Arc::make_mut(styles).insert(row, answer.clone()),
            None if styles.contains_key(&row) => std::sync::Arc::make_mut(styles).remove(&row),
            None => None,
        };
    }
}
