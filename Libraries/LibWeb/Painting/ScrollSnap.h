/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Optional.h>
#include <AK/Vector.h>
#include <LibCompositing/Scrolling/ScrollSnapSelection.h>
#include <LibGC/Ptr.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWebCommon/PixelUnits.h>

namespace Web::Painting {

class BoxSlot;

using Compositing::MomentumFlingEstimator;
using Compositing::SnapAreaGeometry;
using Compositing::SnapAreaIdentity;
using Compositing::SnapAxes;
using Compositing::SnapContainerGeometry;
using Compositing::SnapDestination;
using Compositing::SnappedAreas;
using Compositing::SnapSelectionStrategy;

WEB_API Compositing::SnapAxes snap_axes_of_scroll_container(BoxSlot const& snap_container);

WEB_API bool is_scroll_snap_container(BoxSlot const&);

// Takes a scroll container a layout tree build gave a style: registers it where it snaps, and otherwise forgets what it
// snapped to.
WEB_API void take_built_scroll_container(DOM::Document&, Compositing::RustFFI::NodeSlotId, bool is_scroll_snap_container);

// The geometry snap position selection runs over, collected from the layout of a snap container and of the snap areas
// it captures.
WEB_API Optional<Compositing::SnapContainerGeometry> snap_container_geometry(BoxSlot const& snap_container);
WEB_API Vector<Compositing::SnapAreaGeometry> collect_snap_areas(BoxSlot const& snap_container);

WEB_API Compositing::SnapDestination adjust_scroll_destination_for_snapping(BoxSlot const& snap_container, CSSPixelPoint destination, Compositing::SnapSelectionStrategy const& strategy = {});

struct ResnapSelection {
    Compositing::SnappedAreas const& snapped_areas;
    GC::Ptr<DOM::Node const> focused_node;
    GC::Ptr<DOM::Element const> targeted_element;
};

WEB_API Compositing::SnapDestination select_resnap_destination(BoxSlot const& snap_container, CSSPixelPoint current_offset, ResnapSelection const&);

}
