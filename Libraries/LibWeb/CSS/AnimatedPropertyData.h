/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2023-2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/NonnullRefPtr.h>
#include <LibWeb/CSS/EasingFunction.h>
#include <LibWeb/CSS/PropertyID.h>

namespace Web::CSS {

struct TransitionProperties {
    Vector<PropertyID> properties;
    double duration;
    EasingFunction timing_function;
    double delay;
    TransitionBehavior transition_behavior;
};

enum class AnimatedPropertyResultOfTransition : u8 {
    No,
    Yes
};

struct AnimatedPropertyData {
    HashMap<PropertyID, NonnullRefPtr<StyleValue const>> values;
    Array<u8, ceil_div(number_of_longhand_properties, 8uz)> inherited {};
    Array<u8, ceil_div(number_of_longhand_properties, 8uz)> result_of_transition {};

    bool is_inherited(PropertyID) const;
    bool is_result_of_transition(PropertyID) const;
    void set_inherited(PropertyID, bool);
    void set_result_of_transition(PropertyID, bool);

    void set(PropertyID, NonnullRefPtr<StyleValue const>, AnimatedPropertyResultOfTransition, bool inherited = false);
    void remove(PropertyID);
    void reset_non_inherited_properties();
};

}
