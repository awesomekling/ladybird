/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Utf16FlyString.h>
#include <AK/Utf16String.h>
#include <AK/Variant.h>
#include <LibGC/Ptr.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/EasingFunction.h>
#include <LibWeb/CSS/StyleValues/RustStyleValueHandle.h>
#include <LibWeb/Forward.h>

namespace Web::CSS {

struct TransitionProperties {
    Vector<PropertyID> properties;
    double duration;
    EasingFunction timing_function;
    double delay;
    TransitionBehavior transition_behavior;
};

// The timeline an animation definition asks for. A scroll timeline is a GC object, and a definition
// is built for every animation on every style recomputation while the timeline it names almost
// never changes, so the definition carries the description and the object is materialized only
// where one is actually needed.
struct AnimationTimelineSource {
    enum class Kind : u8 {
        Document,
        None,
        Scroll,
    };

    Kind kind { Kind::Document };
    Scroller scroller {};
    Axis axis {};

    bool operator==(AnimationTimelineSource const&) const = default;
};

struct AnimationProperties {
    Variant<double, Utf16String> duration;
    EasingFunction timing_function;
    double iteration_count;
    AnimationDirection direction;
    AnimationPlayState play_state;
    double delay;
    AnimationFillMode fill_mode;
    AnimationComposition composition;
    Utf16FlyString name;
    AnimationTimelineSource timeline;
    // The computed `animation-timing-function` this definition's easing was parsed out of. The
    // style computation decides whether a plan would change anything by comparing the definitions
    // it just computed against the ones the element's animations last had applied, and it compares
    // computed values rather than easings, so the animation keeps the value the easing came from.
    RustStyleValueHandle timing_function_value;
};

// One animation's applied definition, as the style computation compares it: the fields it computed,
// packed the way a published row travels. The name is not in here - it is published beside the row,
// since the plan matches definitions to animations by name before it compares anything else - and
// neither is `animation-name`'s index in the list, which is always the animation's own position
// because the plan is the only thing that writes either.
struct AppliedAnimationDefinitionRow {
    static constexpr size_t word_count = 6;

    enum Word : size_t {
        Duration,
        IterationCount,
        Delay,
        Flags,
        KeyframeSet,
        TimingFunction,
    };

    static constexpr u64 duration_is_auto = 1;
    static constexpr size_t direction_shift = 8;
    static constexpr size_t play_state_shift = 16;
    static constexpr size_t fill_mode_shift = 24;
    static constexpr size_t composition_shift = 32;
    static constexpr size_t timeline_kind_shift = 40;
    static constexpr size_t scroller_shift = 48;
    static constexpr size_t axis_shift = 56;

    u64 words[word_count] {};
};

}
