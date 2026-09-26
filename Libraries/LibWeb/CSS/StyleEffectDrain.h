/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Array.h>
#include <AK/Function.h>
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
//
// They come in two halves by who reads them. The render half is what the engine's later waves, the
// tree build and layout read: engine commits, layout node styles, tree invalidation, inheritance
// marks, anchor names and container feedback. The main half is what only the main thread reads:
// the CSS animation objects and their bookkeeping. The render half applies before the main half,
// each in row order, and nothing in the render half reads the main half.
class StyleEffectDrain {
public:
    // -- The render half ----------------------------------------------------------------------

    static constexpr size_t synthetic_pseudo_element_count = to_underlying(last_synthetic_pseudo_element) - to_underlying(first_synthetic_pseudo_element) + 1;
    // The records of an element's synthetic pseudo-elements, by kind from the first.
    using PseudoElementStyleRecords = Array<StyleRecordID, synthetic_pseudo_element_count>;

    // What applying the records a row installs to its layout nodes adds (style resources, anonymous
    // reinheritance, repaint) is render state. Once the batch is installed, the row names the records
    // its element holds, and the layout nodes are the rows the arena binds to its style node:
    // applying it reads nothing of the element.
    struct LayoutNodeStyle {
        StyleNodeID style_node;
        RequiredInvalidationAfterStyleChange invalidation;
        // None where the element is gone by the time the batch is installed.
        Optional<StyleRecordID> style_record {};
        // Where the drain keeps the records of the element's synthetic pseudo-elements, if it has any.
        u32 pseudo_element_style_records { NumericLimits<u32>::max() };
    };

    // What a row's change invalidates of layout, the layout tree, visual contexts and scroll snapping.
    struct ElementInvalidation {
        StyleNodeID style_node;
        RequiredInvalidationAfterStyleChange invalidation;
    };
    // A row whose record read a non-inherited property straight from the parent, through an
    // explicit `inherit`, owes the parent the mark C++ writes beside such a computation. The engine
    // took its own mark as it published the row; this is the host's, which the facts the host
    // hands the engine for the next wave read.
    struct ExplicitInheritance {
        StyleNodeID style_node;
        u32 style_groups { 0 };
    };
    // The anchor names a row's record registers in its tree scope, in place of the ones the engine
    // holds for the element.
    struct AnchorNames {
        StyleNodeID style_node;
    };
    // What a row's container conditions read of its containers. The engine records what it reads of
    // it as the drain takes it; the host mirrors the rest on the elements through the commit
    // messages, as it does for a row it computes.
    struct ContainerQueryEffects {
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
    using RenderEffect = Variant<LayoutNodeStyle, ElementInvalidation, ExplicitInheritance, AnchorNames, ContainerQueryEffects, RestoreRowDebts, AcknowledgeRecord, DiscardContainerQueryEffects>;

    // -- The main half ------------------------------------------------------------------------

    // Which animations a row's record references, as the index a `@keyframes` rule finds its
    // elements by holds them. A row with an animation plan exists because the declarations naming
    // them moved.
    struct AnimationNames {
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
    using MainEffect = Variant<AnimationNames, AnimationPlan, DisplayNoneAnimations>;

    void append(RenderEffect effect) { m_render_effects.append(move(effect)); }
    void append(MainEffect effect) { m_main_effects.append(move(effect)); }
    void apply(DOM::Document&);
    void apply(StyleDrainScope const&, DOM::Document&);

    // Install a published style batch inside the drain: `install` receives the scope that proves it.
    static void install(DOM::Document&, Function<void(StyleDrainScope const&)> const& install);

    static PseudoElementStyleRecords pseudo_element_style_records_of(DOM::Element const&);
    // Applies the records a row installs to the layout nodes the arena binds to its style node.
    static void apply_layout_node_style(DOM::Document&, StyleNodeID, RequiredInvalidationAfterStyleChange const&, StyleRecordID, PseudoElementStyleRecords const&);

private:
    void take_layout_node_style_records(DOM::Document&);
    void apply_render_half(StyleDrainScope const&, DOM::Document&);
    void apply_main_half(StyleDrainScope const&, DOM::Document&);

    Vector<RenderEffect> m_render_effects;
    Vector<PseudoElementStyleRecords> m_pseudo_element_style_records;
    Vector<MainEffect> m_main_effects;
};

}
