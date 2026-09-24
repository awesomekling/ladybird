/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Utf16FlyString.h>
#include <AK/Variant.h>
#include <AK/Vector.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/CSS/StyleInvalidation.h>
#include <LibWeb/Forward.h>

namespace Web::CSS {

// The host effects a style batch's rows leave, as messages naming each row's element by style node.
// The host applies them once the whole batch is installed, in the order the batch applied the rows,
// which is flat-tree order. Nothing a later row of the batch computes may depend on one of them.
class StyleEffectDrain {
public:
    // The element and its layout node already hold the record; what applying it to the layout node
    // adds (style resources, anonymous reinheritance, repaint) is render state.
    struct LayoutNodeStyle {
        StyleNodeID style_node;
        RequiredInvalidationAfterStyleChange invalidation;
    };
    // What a row's change invalidates of layout, the layout tree, visual contexts and scroll snapping.
    struct ElementInvalidation {
        StyleNodeID style_node;
        RequiredInvalidationAfterStyleChange invalidation;
    };
    // A row whose record read a non-inherited property straight from the parent, through an
    // explicit `inherit`, owes the parent the mark C++ writes beside such a computation.
    struct ExplicitInheritance {
        StyleNodeID style_node;
        u32 style_groups { 0 };
    };
    // The anchor names a row's record registers in its tree scope, in place of the ones the record
    // it replaced registered.
    struct AnchorNames {
        StyleNodeID style_node;
        Vector<Utf16FlyString> old_names;
    };
    // Which animations a row's record references, as the index a `@keyframes` rule finds its
    // elements by holds them. A row with an animation plan exists because the declarations naming
    // them moved.
    struct AnimationNames {
        StyleNodeID style_node;
    };
    using Effect = Variant<LayoutNodeStyle, ElementInvalidation, ExplicitInheritance, AnchorNames, AnimationNames>;

    void append(Effect effect) { m_effects.append(move(effect)); }
    void apply(DOM::Document&);

private:
    Vector<Effect> m_effects;
};

}
