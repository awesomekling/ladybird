/*
 * Copyright (c) 2018-2020, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/Forward.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>

namespace Web::Layout {

// A layout tree build's walk runs on the render side; the document readies it and then pays what the
// walk owes it, which `walk` holds.
u32 prepare_layout_tree_build(DOM::Document&);
RustFFI::FfiLayoutTreeBuildOutcome pay_layout_tree_build(DOM::Document&, void* walk);
void detach_top_layer_element_layout_subtree(DOM::Element&);

// What a layout tree build owes the rows it stamped for their images, which the document attaches
// once the frame the build ran in is over: the images a box's style asks for, and the provider of
// the image a box shows in place of its element's contents or of a pseudo-element's generated
// content. Each answers whether the box was handed a provider whose image is already there, which
// the frame laid the box out without.
bool attach_owed_style_resources(DOM::Document&, RustFFI::NodeSlotId, bool owns_content_replacement_image);
bool attach_owed_generated_image(DOM::Document&, RustFFI::NodeSlotId, u32 style_node, RustFFI::FfiPseudoElement, RustFFI::FfiGeneratedContentItem, RustFFI::NodeSlotId pseudo_element_box);

}
