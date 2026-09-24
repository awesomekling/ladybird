/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/Utf16FlyString.h>
#include <AK/Variant.h>
#include <AK/Vector.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleDrainScope.h>
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
    // What a row's container conditions read of its containers, which the host records for the
    // commit as it records them for a row it computes.
    struct ContainerQueryEffects {
        StyleNodeID style_node;
    };
    // The animation plan a row leaves for the element's CSS animations. The pass sampled the element
    // over the effect stack the plan leaves and published the composition the rows after it read,
    // so the animations the plan starts, retimes and cancels are nothing the batch reads.
    struct AnimationPlan {
        StyleNodeID style_node;
        StyleComputer::SettledAnimationPlan plan;
    };
    // A row whose display left or entered none, ignoring animations, terminates or resumes the
    // animations of its subtree. The rows of the batch read the animations' published timing, which
    // moves only once the batch is installed.
    struct DisplayNoneAnimations {
        StyleNodeID style_node;
    };
    // The debts the engine took as it published a row the batch did not install, which the node
    // owes again for a later row. The node's element may be gone.
    struct RestoreRowDebts {
        StyleNodeID style_node;
        u32 explicit_inheritance_debt { 0 };
        u8 row_effect_debt { 0 };
    };
    // The host installed the record the engine computed for a row: the engine commits the state it
    // computed the record from. The node's element may be gone.
    struct AcknowledgeRecord {
        StyleNodeID style_node;
    };
    // What a declined row's computation read of its containers, which nothing records: the next
    // transaction computes the element again. The node's element may be gone.
    struct DiscardContainerQueryEffects {
        StyleNodeID style_node;
    };
    using Effect = Variant<LayoutNodeStyle, ElementInvalidation, ExplicitInheritance, AnchorNames, AnimationNames, ContainerQueryEffects, AnimationPlan, DisplayNoneAnimations, RestoreRowDebts, AcknowledgeRecord, DiscardContainerQueryEffects>;

    void append(Effect effect) { m_effects.append(move(effect)); }
    void apply(DOM::Document&);
    void apply(StyleDrainScope const&, DOM::Document&);

    // Install a published style batch inside the drain: `install` receives the scope that proves it.
    static void install(DOM::Document&, Function<void(StyleDrainScope const&)> const& install);

private:
    Vector<Effect> m_effects;
};

}
