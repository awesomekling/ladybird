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

}
