/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::css_pixels::{CssPixelRect, CssPixels};
use crate::css::retained_fly_string::{RetainedUtf16FlyString, RetainedUtf16FlyStringList};
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::painting::display_list::commands::DisplayListResourceId;
use crate::painting::host::FfiVectorImageRenderRequest;
use crate::painting::published_frame::PaintRead;
use libgfx_rust::{FloatRect, FloatSize, IntSize};
use std::collections::HashMap;

/// The published half of a `color-scheme` declaration an SVG-as-image can answer with.
pub(crate) fn declares_light_or_dark_color_scheme(schemes: &RetainedUtf16FlyStringList) -> bool {
    fn keyword_raws() -> (usize, usize) {
        thread_local! {
            // Fly strings are interned, so equal raw words mean equal strings. These two are
            // CSS keywords and live as long as the process does; leak one reference each rather
            // than interning them per layer.
            static RAWS: (usize, usize) = {
                let light = RetainedUtf16FlyString::from_utf16(&[b'l'.into(), b'i'.into(), b'g'.into(), b'h'.into(), b't'.into()]);
                let dark = RetainedUtf16FlyString::from_utf16(&[b'd'.into(), b'a'.into(), b'r'.into(), b'k'.into()]);
                let raws = (light.raw(), dark.raw());
                std::mem::forget(light);
                std::mem::forget(dark);
                raws
            };
        }
        RAWS.with(|raws| *raws)
    }
    let (light, dark) = keyword_raws();
    schemes
        .as_slice()
        .iter()
        .any(|scheme| scheme.raw() == light || scheme.raw() == dark)
}

/// The scheme an SVG-as-image referenced by `owner` answers `prefers-color-scheme` with.
/// Its used `color-scheme` counts only when the element or the document declared a scheme the
/// image can answer with; otherwise, like Firefox, the preferred scheme wins.
pub(crate) fn image_color_scheme(
    layout_arena: &impl PaintRead,
    owner: NodeSlotId,
    document_declares_light_or_dark_color_scheme: bool,
    image_color_scheme_fallback: u8,
) -> u8 {
    layout_arena
        .node_style_if_live(owner)
        .map_or(image_color_scheme_fallback, |style| {
            let ui = style.inherited_ui();
            if declares_light_or_dark_color_scheme(&ui.color_schemes) || document_declares_light_or_dark_color_scheme {
                ui.color_scheme
            } else {
                image_color_scheme_fallback
            }
        })
}

/// What predicting the renders of a recording's first paints reads from its inputs.
pub(crate) struct FirstPaintPredictionInputs {
    pub device_pixels_per_css_pixel: f64,
    pub root_background_source: Option<crate::painting::host::FfiRootBackgroundSource>,
    pub css_viewport_rect: CssPixelRect,
    pub document_declares_light_or_dark_color_scheme: bool,
    pub image_color_scheme_fallback: u8,
}

/// The renders the next recording is predicted to need for the SVG-as-images it paints afresh:
/// image elements, and background and mask layers. Each is predicted at the size its committed
/// box gives and the scale of an untransformed box. The renders the last recording painted are
/// resolved already, so this covers a first paint, and a render it predicts wrong is only a miss.
/// Images that share a source share one document, which keeps the layout of its last render, so the
/// requests are in tree order, as the recording would meet them.
pub(crate) fn predict_first_paint_renders(
    layout_arena: &LayoutNodeArena,
    inputs: &FirstPaintPredictionInputs,
) -> Vec<VectorImageRenderRequest> {
    use crate::painting::image_content::ImageContent;
    use crate::painting::record::paint::background_resolution::{
        LayerResolutionContext, ResolvedBackground, committed_layer_image_paint_facts, resolve_background_for_paint,
        resolve_mask_layers,
    };
    use crate::painting::replaced_paint_facts::{ImagePaintFacts, ReplacedPaintFacts};
    let rows = layout_arena.paintable_rows();
    let converter =
        crate::painting::display_list::device_pixels::DevicePixelConverter::new(inputs.device_pixels_per_css_pixel);
    let every_row_records = layout_arena.paint_damage_covers_everything();
    let records_afresh = |row: NodeSlotId| {
        rows.paintable_row_is_populated(row)
            && (every_row_records || !layout_arena.row_paint_state(row).damage().is_empty())
    };
    let color_scheme = |owner: NodeSlotId| {
        image_color_scheme(
            layout_arena,
            owner,
            inputs.document_declares_light_or_dark_color_scheme,
            inputs.image_color_scheme_fallback,
        )
    };
    let mut requests = Vec::new();
    let mut predict = |owner: NodeSlotId,
                       image_identity: u64,
                       color_scheme: u8,
                       dest_rect: libgfx_rust::IntRect,
                       has_active_view_box| {
        if dest_rect.is_empty() {
            return;
        }
        let geometry = vector_image_render_geometry(
            dest_rect.to_float(),
            FloatSize {
                width: 1.0,
                height: 1.0,
            },
            has_active_view_box,
        );
        let request = VectorImageRenderRequest::new(
            image_identity,
            color_scheme,
            geometry.css_width,
            geometry.css_height,
            geometry.raster_scale,
        );
        requests.push((layout_arena.node_pre_order_label(owner), request));
    };

    layout_arena.for_each_replaced_paint_facts(|row, facts| {
        let ReplacedPaintFacts::Image(ImagePaintFacts {
            natural,
            content:
                ImageContent::Vector {
                    image_identity,
                    has_active_view_box,
                    ..
                },
        }) = facts
        else {
            return;
        };
        if !records_afresh(row) {
            return;
        }
        let draw_rect =
            crate::painting::record::paint::replaced::image_content_draw_rect(&rows, converter, row, natural);
        predict(row, *image_identity, color_scheme(row), draw_rect, *has_active_view_box);
    });

    // A layer paints its image at its image rect, whose device size is what a first paint renders at,
    // tiled or not.
    let Some(root_background_source) = inputs.root_background_source else {
        return in_tree_order(requests);
    };
    let context = LayerResolutionContext {
        layout_arena: &rows,
        root_background_source,
        css_viewport_rect: inputs.css_viewport_rect,
    };
    let mut predict_layers = |row: NodeSlotId, resolved: &ResolvedBackground<'_>| {
        for layer in &resolved.layers {
            let Some(image) = layer.image else {
                continue;
            };
            let ImageContent::Vector {
                image_identity,
                has_active_view_box,
                ..
            } = committed_layer_image_paint_facts(layout_arena, &image).content
            else {
                continue;
            };
            let mut image_rect = layer.image_rect;
            image_rect.x = layer.background_positioning_area.left() + layer.position_x;
            image_rect.y = layer.background_positioning_area.top() + layer.position_y;
            let mut dest_rect = converter.rounded_device_rect(image_rect);
            dest_rect.width = dest_rect.width.max(1);
            dest_rect.height = dest_rect.height.max(1);
            predict(
                row,
                image_identity,
                color_scheme(image.facts_owner),
                dest_rect,
                has_active_view_box,
            );
        }
    };
    let mut painting_rows = Vec::new();
    layout_arena.for_each_layer_image_paint_facts_owner(|owner| {
        painting_rows.push(owner);
        // The body's background paints on the root element when it propagates there.
        if root_background_source.use_body_background_properties && owner == root_background_source.body_layout_node {
            painting_rows.push(root_background_source.root_layout_node);
        }
    });
    for row in painting_rows {
        if !records_afresh(row) {
            continue;
        }
        if let Some(background) = resolve_background_for_paint(context, row) {
            predict_layers(row, &background.resolved);
        }
        if let Some(style) = rows.node_style_if_live(row) {
            let border_box = crate::painting::paintable_geometry::absolute_border_box_rect(&rows, row);
            predict_layers(row, &resolve_mask_layers(context, row, style, border_box));
        }
    }
    in_tree_order(requests)
}

/// The requests ordered by their painting rows in tree order, each at its first occurrence.
fn in_tree_order(mut requests: Vec<(u64, VectorImageRenderRequest)>) -> Vec<VectorImageRenderRequest> {
    requests.sort_by_key(|(pre_order_label, _)| *pre_order_label);
    let mut seen = std::collections::HashSet::new();
    requests
        .into_iter()
        .map(|(_, request)| request)
        .filter(|request| seen.insert(*request))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct VectorImageRenderRequest {
    image_identity: u64,
    color_scheme: u8,
    css_width_raw: i32,
    css_height_raw: i32,
    raster_scale_bits: u32,
}

impl VectorImageRenderRequest {
    pub(crate) fn new(
        image_identity: u64,
        color_scheme: u8,
        css_width: CssPixels,
        css_height: CssPixels,
        raster_scale: f32,
    ) -> Self {
        Self {
            image_identity,
            color_scheme,
            css_width_raw: css_width.raw_value(),
            css_height_raw: css_height.raw_value(),
            raster_scale_bits: raster_scale.to_bits(),
        }
    }

    pub(crate) fn raster_scale(&self) -> f32 {
        f32::from_bits(self.raster_scale_bits)
    }

    pub(crate) fn to_ffi(self) -> FfiVectorImageRenderRequest {
        FfiVectorImageRenderRequest {
            image_identity: self.image_identity,
            color_scheme: self.color_scheme,
            css_width: CssPixels::from_raw(self.css_width_raw),
            css_height: CssPixels::from_raw(self.css_height_raw),
            raster_scale: self.raster_scale(),
        }
    }
}

/// The display lists of the SVG-as-image renders a recording paints. Rendering one lays out and
/// records another document, so the main thread resolves them and hands the recording this map;
/// the recording only looks renders up in it.
#[derive(Clone, Default)]
pub(crate) struct VectorImageDisplayLists {
    lists: HashMap<VectorImageRenderRequest, DisplayListResourceId>,
}

impl VectorImageDisplayLists {
    pub(crate) fn get(&self, request: &VectorImageRenderRequest) -> Option<DisplayListResourceId> {
        self.lists.get(request).copied()
    }

    pub(crate) fn insert(&mut self, request: VectorImageRenderRequest, display_list: DisplayListResourceId) {
        self.lists.insert(request, display_list);
    }
}

pub(crate) struct VectorImageRenderGeometry {
    pub css_width: CssPixels,
    pub css_height: CssPixels,
    pub raster_scale: f32,
    pub list_size: IntSize,
}

const MAXIMUM_RASTER_DIMENSION: i32 = 16384;

fn positive_scale_or_one(scale: f32) -> f32 {
    if scale.is_nan() || scale <= 0.0 { 1.0 } else { scale }
}

fn raster_dimension(local_size: f32, scale: f32) -> i32 {
    ((local_size * positive_scale_or_one(scale)).round() as i32).clamp(1, MAXIMUM_RASTER_DIMENSION)
}

pub(crate) fn vector_image_render_geometry(
    dest_rect: FloatRect,
    accumulated_scale: FloatSize,
    has_active_view_box: bool,
) -> VectorImageRenderGeometry {
    if has_active_view_box {
        let raster_size = IntSize {
            width: raster_dimension(dest_rect.width, accumulated_scale.width),
            height: raster_dimension(dest_rect.height, accumulated_scale.height),
        };
        return VectorImageRenderGeometry {
            css_width: CssPixels::from_integer(i64::from(raster_size.width)),
            css_height: CssPixels::from_integer(i64::from(raster_size.height)),
            raster_scale: 1.0,
            list_size: raster_size,
        };
    }
    let smallest_positive = CssPixels::from_raw(1);
    let maximum = CssPixels::from_integer(i64::from(MAXIMUM_RASTER_DIMENSION));
    let css_width = CssPixels::nearest_value_for_f32(dest_rect.width).clamp(smallest_positive, maximum);
    let css_height = CssPixels::nearest_value_for_f32(dest_rect.height).clamp(smallest_positive, maximum);
    let raster_scale = positive_scale_or_one(accumulated_scale.width.max(accumulated_scale.height));
    let maximum_raster_scale = MAXIMUM_RASTER_DIMENSION as f32 / css_width.max(css_height).to_float();
    let raster_scale = raster_scale.min(maximum_raster_scale);
    VectorImageRenderGeometry {
        css_width,
        css_height,
        raster_scale,
        list_size: IntSize {
            width: ((css_width.to_float() * raster_scale).round() as i32).max(1),
            height: ((css_height.to_float() * raster_scale).round() as i32).max(1),
        },
    }
}
