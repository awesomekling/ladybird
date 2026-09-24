/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::style::fast_hash::FastMap;
use crate::layout::node_data::NodeSlotId;
use libgfx_rust::WindingRule;
use libgfx_rust::path::PathBuilder;
use std::cell::RefCell;
use std::sync::Arc;

/// The state an `<area>`'s `shape` attribute represents, as the HTML image map processing model
/// enumerates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaShape {
    Circle,
    Default,
    Polygon,
    Rectangle,
}

impl AreaShape {
    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Circle,
            1 => Self::Default,
            2 => Self::Polygon,
            _ => Self::Rectangle,
        }
    }
}

/// One `<area>` of an image map, as the image's row holds it: the style-tree identity to name as
/// the hit target, the parsed shape the point is tested against, and whether the area is editable
/// or an editing host. An area is never rendered, so it has no row to carry that last fact the way
/// every other hit target does, and it rides here instead.
pub struct PublishedImageMapArea {
    pub style_node: u32,
    pub shape: AreaShape,
    pub editable: bool,
    pub coords: Box<[f64]>,
}

impl PublishedImageMapArea {
    /// The shape to layer onto the image, built with the same operations the DOM side built it
    /// with so that the containment test is the one `Gfx::Path` already answered.
    fn shape_path(&self, image_width: f32, image_height: f32) -> Option<libgfx_rust::path::OwnedPath> {
        let coords = &self.coords;
        let vertex = |builder: &mut PathBuilder, index: usize, line: bool| {
            let x = coords[2 * index] as f32;
            let y = coords[2 * index + 1] as f32;
            if line {
                builder.line_to(x, y);
            } else {
                builder.move_to(x, y);
            }
        };
        let rectangle_path = |left: f32, top: f32, right: f32, bottom: f32| {
            let mut builder = PathBuilder::new();
            builder.move_to(left, top);
            builder.line_to(right, top);
            builder.line_to(right, bottom);
            builder.line_to(left, bottom);
            builder.close();
            builder.build()
        };

        match self.shape {
            AreaShape::Circle => {
                if coords.len() < 3 {
                    return None;
                }
                // A circle with a radius of zero or less is an empty shape.
                if coords[2] <= 0.0 {
                    return None;
                }
                let x = coords[0] as f32;
                let y = coords[1] as f32;
                let radius = coords[2] as f32;
                let mut builder = PathBuilder::new();
                builder.move_to(x + radius, y);
                builder.arc_to(x, y + radius, radius, false, true);
                builder.arc_to(x - radius, y, radius, false, true);
                builder.arc_to(x, y - radius, radius, false, true);
                builder.arc_to(x + radius, y, radius, false, true);
                builder.close();
                Some(builder.build())
            }
            AreaShape::Default => Some(rectangle_path(0.0, 0.0, image_width, image_height)),
            AreaShape::Polygon => {
                if coords.len() < 6 {
                    return None;
                }
                let mut builder = PathBuilder::new();
                vertex(&mut builder, 0, false);
                for index in 1..coords.len() / 2 {
                    vertex(&mut builder, index, true);
                }
                builder.close();
                Some(builder.build())
            }
            AreaShape::Rectangle => {
                if coords.len() < 4 {
                    return None;
                }
                // The two corners are ordered before they are read, so a rectangle given
                // right-to-left or bottom-to-top still covers the same area.
                let mut corners = [coords[0], coords[1], coords[2], coords[3]];
                if corners[0] > corners[2] {
                    corners.swap(0, 2);
                }
                if corners[1] > corners[3] {
                    corners.swap(1, 3);
                }
                Some(rectangle_path(
                    corners[0] as f32,
                    corners[1] as f32,
                    corners[2] as f32,
                    corners[3] as f32,
                ))
            }
        }
    }

    fn contains_point(&self, x: f32, y: f32, image_width: f32, image_height: f32) -> bool {
        // The default state covers everything that hits the image, borders and padding included.
        if self.shape == AreaShape::Default {
            return true;
        }
        let Some(path) = self.shape_path(image_width, image_height) else {
            return false;
        };
        path.contains(x, y, WindingRule::EvenOdd as i32)
    }
}

// The `<area>` elements of the image map an image is associated with, as the render side sees
// them. The association and the areas themselves still live on the DOM, and this column is the
// copy a hit test can read without asking for either. It is kept in step at the moments the list
// can change: the image taking a box, its `usemap`, and any change to the maps and areas of the
// document.
//
// Keyed by the image's paintable row, because that is the key the hit test has, and an area is
// named by its style-tree identity, because that is what the hit hands back. A row's id carries
// the generation of the slot it came from, so an entry left behind by a freed row names nothing a
// live row can ask for. An image with no image map has no entry, which is nearly every image.
//
// The entries are shared with the snapshots the paintable rows publish, and copied the next time
// one changes while a snapshot holds them. They are few, and a copy shares each image's areas.
#[derive(Default)]
pub struct ImageMapAreaColumn {
    areas: RefCell<Arc<ImageMapAreas>>,
}

/// The image map areas of an [`ImageMapAreaColumn`] as they were when taken.
#[derive(Clone, Default)]
pub struct ImageMapAreas(FastMap<NodeSlotId, Arc<[PublishedImageMapArea]>>);

impl ImageMapAreaColumn {
    pub fn publish(&self, slot: NodeSlotId, areas: Box<[PublishedImageMapArea]>) {
        if slot.is_invalid() || (areas.is_empty() && !self.areas.borrow().0.contains_key(&slot)) {
            return;
        }
        let mut published = self.areas.borrow_mut();
        let published = &mut Arc::make_mut(&mut published).0;
        if areas.is_empty() {
            published.remove(&slot);
        } else {
            published.insert(slot, areas.into());
        }
    }

    pub fn forget(&self, slot: NodeSlotId) {
        if !self.areas.borrow().0.contains_key(&slot) {
            return;
        }
        Arc::make_mut(&mut self.areas.borrow_mut()).0.remove(&slot);
    }

    /// The areas as they are now. They do not see later changes to the column.
    pub fn snapshot(&self) -> Arc<ImageMapAreas> {
        self.areas.borrow().clone()
    }

    pub fn with_areas<R>(&self, read: impl FnOnce(&ImageMapAreas) -> R) -> R {
        read(&self.areas.borrow())
    }
}

impl ImageMapAreas {
    /// The first area of the image's map, in tree order, whose shape covers the point, named by
    /// its style-tree identity. Zero when the image has no map, or no shape covers the point.
    pub fn area_for_point(&self, slot: NodeSlotId, x: f32, y: f32, image_width: f32, image_height: f32) -> u32 {
        let Some(areas) = self.0.get(&slot) else {
            return 0;
        };
        // The shapes are layered in reverse tree order, so the top-most shape covering the point
        // belongs to the first area in tree order whose shape contains it.
        for area in areas.iter() {
            if area.contains_point(x, y, image_width, image_height) {
                return area.style_node;
            }
        }
        0
    }

    /// Whether the area of this image named by `style_node` is editable or an editing host: 1 or
    /// 0, and -1 when the identity names no area of this image.
    pub fn area_editability(&self, slot: NodeSlotId, style_node: u32) -> i8 {
        let Some(areas) = self.0.get(&slot) else {
            return -1;
        };
        for area in areas.iter() {
            if area.style_node == style_node {
                return i8::from(area.editable);
            }
        }
        -1
    }
}
