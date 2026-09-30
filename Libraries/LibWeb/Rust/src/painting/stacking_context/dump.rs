/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::css_pixels::CssPixelRect;
use crate::layout::LayoutNodeArena;
use crate::layout::debug_text::DebugText;
use crate::layout::node_data::NodeSlotId;
use crate::painting::dump::push_css_pixel_rect;
use crate::painting::paintable_geometry;
use crate::painting::style_queries;
use std::fmt::Write;

/// The stacking context tree of the document whose arena is `arena`, for tests: a line per stacking context, nested
/// in paint order, each naming its box. A document with no stacking contexts dumps nothing.
pub(crate) fn stacking_context_tree(arena: &LayoutNodeArena) -> DebugText {
    let mut output = DebugText::default();
    let viewport = arena.bound_viewport_row();
    if arena.stacking_context_entries(viewport).is_some() {
        visit(&mut output, arena, viewport, 0);
    }
    output
}

fn visit(output: &mut DebugText, arena: &LayoutNodeArena, root: NodeSlotId, depth: usize) {
    output.text().extend(std::iter::repeat_n(' ', depth));
    if !arena.slot_is_live(root) {
        output.text().push_str("SC for (gone)\n");
    } else {
        output.text().push_str("SC for ");
        output.push_box(arena, root);
        push_line_after_box(
            output.text(),
            paintable_geometry::absolute_rect_or_default(&arena.paintable_rows(), root),
            effective_z_index(arena, root),
            has_css_transform(arena, root),
        );
    }

    let Some(entries) = arena.stacking_context_entries(root) else {
        return;
    };
    for entry in entries.negative_z_index_child_contexts() {
        visit(output, arena, entry.slot, depth + 1);
    }
    for &descendant in &entries.stack_level_zero_boxes {
        if arena.paintable_row_is_populated(descendant)
            && arena
                .paintable_rows()
                .paintable_data(descendant)
                .establishes_stacking_context
        {
            visit(output, arena, descendant, depth + 1);
        }
    }
    for entry in entries.positive_z_index_child_contexts() {
        visit(output, arena, entry.slot, depth + 1);
    }
}

fn push_line_after_box(output: &mut String, rect: CssPixelRect, effective_z_index: Option<i32>, has_transform: bool) {
    output.push(' ');
    push_css_pixel_rect(output, rect);
    output.push_str(" (z-index: ");
    if let Some(z_index) = effective_z_index {
        let _ = write!(output, "{z_index}");
    } else {
        output.push_str("auto");
    }
    output.push(')');
    if has_transform {
        output.push_str(", has_transform");
    }
    output.push('\n');
}

fn effective_z_index(arena: &LayoutNodeArena, slot: NodeSlotId) -> Option<i32> {
    arena
        .paintable_visual_context_record(slot)
        .and_then(|record| record.stacking_context.effective_z_index)
}

fn has_css_transform(arena: &LayoutNodeArena, slot: NodeSlotId) -> bool {
    arena.paintable_row_is_populated(slot)
        && arena
            .node_style_if_live(slot)
            .is_some_and(|style| style_queries::has_css_transform(arena, slot, style))
}

#[cfg(test)]
mod tests {
    use super::push_line_after_box;
    use crate::css::css_pixels::{CssPixelRect, CssPixels};

    #[test]
    fn stacking_context_lines_match_the_canonical_format() {
        let rect = CssPixelRect::new(
            CssPixels::from_integer(1),
            CssPixels::from_integer(2),
            CssPixels::from_integer(30),
            CssPixels::from_integer(40),
        );
        let mut output = String::new();
        output.push_str("SC for Viewport<#document>");
        push_line_after_box(&mut output, rect, None, false);
        output.push_str("SC for BlockContainer<DIV>#target.a.b");
        push_line_after_box(&mut output, rect, Some(-1), true);
        assert_eq!(
            output,
            concat!(
                "SC for Viewport<#document> [1,2 30x40] (z-index: auto)\n",
                "SC for BlockContainer<DIV>#target.a.b [1,2 30x40] (z-index: -1), has_transform\n",
            )
        );
    }
}
