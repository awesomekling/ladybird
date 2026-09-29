/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! CSS transition decisions.

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum FfiTransitionActionKind {
    None,
    Remove,
    Cancel,
    Start,
    RemoveAndStart,
    CancelRemoveAndStartReversing,
    CancelRemoveAndStartInterrupted,
}

#[repr(C)]
pub struct FfiTransitionPropertyInput {
    pub property_id: u16,
    pub before_change_value: *const crate::css::style_value::StyleValueData,
    pub after_change_value: *const crate::css::style_value::StyleValueData,
    pub current_value: *const crate::css::style_value::StyleValueData,
    pub existing_end_value: *const crate::css::style_value::StyleValueData,
    pub reversing_adjusted_start_value: *const crate::css::style_value::StyleValueData,
    pub has_matching_transition: bool,
    pub allow_discrete: bool,
    pub has_running_transition: bool,
    pub has_completed_transition: bool,
    pub delay: f64,
    pub duration: f64,
    pub old_timing_function_output: f64,
    pub old_reversing_shortening_factor: f64,
}

#[repr(C)]
pub struct FfiTransitionInput {
    pub context: crate::css::animation::FfiAnimationContext,
    pub properties: *mut FfiTransitionPropertyInput,
    pub property_count: usize,
    /// The target's style node shifted left by 8, or'd with its pseudo-element kind; 0 when it
    /// has no style node.
    pub target_key: u64,
}

#[repr(C)]
pub struct FfiTransitionAction {
    pub property_id: u16,
    pub kind: FfiTransitionActionKind,
    pub delay: f64,
    pub active_duration: f64,
    pub reversing_shortening_factor: f64,
}

fn property_values_are_transitionable(
    context: &crate::css::animation::FfiAnimationContext,
    property_id: u16,
    old_value: *const crate::css::style_value::StyleValueData,
    new_value: *const crate::css::style_value::StyleValueData,
    allow_discrete: bool,
) -> bool {
    let animation_type = crate::css::property_metadata::property_animation_type(property_id);

    // https://drafts.csswg.org/css-transitions/#transitionable
    // When comparing the before-change style and after-change style for a given property, the property values are transitionable if they have an animation type that is neither not animatable nor discrete.
    if animation_type == crate::css::animation::ANIMATION_TYPE_NONE
        || !allow_discrete && animation_type == crate::css::animation::ANIMATION_TYPE_DISCRETE
    {
        return false;
    }
    if allow_discrete {
        return true;
    }

    let result = crate::css::animation::interpolate_value(
        Some(context),
        property_id,
        unsafe { &*old_value },
        unsafe { &*new_value },
        0.5,
    );
    assert!(result.handled);
    if !result.value.is_null() {
        unsafe { crate::css::style_value::release_style_value(result.value) };
        return true;
    }
    false
}

fn decide_transition(
    context: &crate::css::animation::FfiAnimationContext,
    input: &FfiTransitionPropertyInput,
    values_originate_from_current_color: bool,
) -> FfiTransitionAction {
    let values_equal = |first: *const crate::css::style_value::StyleValueData,
                        second: *const crate::css::style_value::StyleValueData| {
        assert!(!first.is_null());
        assert!(!second.is_null());
        let (first, second) = unsafe { (&*first, &*second) };
        std::ptr::eq(first, second) || first == second
    };
    let before_change_value_differs = input.has_matching_transition
        && !values_originate_from_current_color
        && !values_equal(input.before_change_value, input.after_change_value);
    let existing_end_value_differs = input.has_matching_transition
        && (input.has_running_transition || input.has_completed_transition)
        && !values_equal(input.existing_end_value, input.after_change_value);
    let current_value_equals_after = input.has_matching_transition
        && input.has_running_transition
        && values_equal(input.current_value, input.after_change_value);
    let reversing_start_value_equals_after = input.has_matching_transition
        && input.has_running_transition
        && values_equal(input.reversing_adjusted_start_value, input.after_change_value);

    // https://drafts.csswg.org/css-transitions/#transition-combined-duration
    // Define the combined duration of the transition as the sum of max(matching transition duration, 0s) and the matching transition delay.
    let combined_duration = input.duration.max(0.0) + input.delay;
    let before_after_transitionable = input.has_matching_transition
        && before_change_value_differs
        && property_values_are_transitionable(
            context,
            input.property_id,
            input.before_change_value,
            input.after_change_value,
            input.allow_discrete,
        );
    let current_after_transitionable = current_value_equals_after
        || input.has_running_transition
            && existing_end_value_differs
            && property_values_are_transitionable(
                context,
                input.property_id,
                input.current_value,
                input.after_change_value,
                input.allow_discrete,
            );
    let mut action = FfiTransitionAction {
        property_id: input.property_id,
        kind: FfiTransitionActionKind::None,
        delay: input.delay,
        active_duration: input.duration,
        reversing_shortening_factor: 1.0,
    };

    // https://drafts.csswg.org/css-transitions/#starting
    // For each element and property, the implementation must act as follows:

    // 1. If all of the following are true:
    // - the element does not have a running transition for the property,
    // - there is a matching transition-property value, and
    // - the before-change style is different from the after-change style for that property, and the values for the property are transitionable,
    // - the element does not have a completed transition for the property or the end value of the completed transition is different from the
    //   after-change style for the property,
    // - the combined duration is greater than 0s,
    if !input.has_running_transition
        && input.has_matching_transition
        && before_change_value_differs
        && before_after_transitionable
        && (!input.has_completed_transition || existing_end_value_differs)
        && combined_duration > 0.0
    {
        // then implementations must remove the completed transition (if present) from the set of completed transitions
        // and start a transition whose:
        // - start time is the time of the style change event plus the matching transition delay,
        // - end time is the start time plus the matching transition duration,
        // - start value is the value of the transitioning property in the before-change style,
        // - end value is the value of the transitioning property in the after-change style,
        // - reversing-adjusted start value is the same as the start value, and
        // - reversing shortening factor is 1.
        action.kind = if input.has_completed_transition {
            FfiTransitionActionKind::RemoveAndStart
        } else {
            FfiTransitionActionKind::Start
        };
        return action;
    }

    // 2. Otherwise, if the element has a completed transition for the property and the end value of the completed transition is different from the
    //    after-change style for the property, then implementations must remove the completed transition from the set of completed transitions.
    if input.has_completed_transition && existing_end_value_differs {
        action.kind = FfiTransitionActionKind::Remove;
        return action;
    }

    // 3. If the element has a running transition or completed transition for the property, and there is not a matching transition-property value,
    //    then implementations must cancel the running transition or remove the completed transition from the set of completed transitions.
    if !input.has_matching_transition {
        action.kind = if input.has_running_transition {
            FfiTransitionActionKind::Cancel
        } else if input.has_completed_transition {
            FfiTransitionActionKind::Remove
        } else {
            FfiTransitionActionKind::None
        };
        return action;
    }

    // 4. If the element has a running transition for the property, there is a matching transition-property value, and the end value of the running
    //    transition is not equal to the value of the property in the after-change style, then:
    if input.has_running_transition && existing_end_value_differs {
        // 1. If the current value of the property in the running transition is equal to the value of the property in the after-change style, or if
        //    these two values are not transitionable, then implementations must cancel the running transition.
        if current_value_equals_after || !current_after_transitionable {
            action.kind = FfiTransitionActionKind::Cancel;
            return action;
        }

        // 2. Otherwise, if the combined duration is less than or equal to 0s, or if the current value of the property in the running transition is
        //    not transitionable with the value of the property in the after-change style, then implementations must cancel the running transition.
        if combined_duration <= 0.0 || !current_after_transitionable {
            action.kind = FfiTransitionActionKind::Cancel;
            return action;
        }

        // 3. Otherwise, if the reversing-adjusted start value of the running transition is the same as the value of the property in the after-change style
        //    (see the section on reversing of transitions for why these case exists),
        if reversing_start_value_equals_after {
            // implementations must cancel the running transition and start a new transition whose:
            // - reversing-adjusted start value is the end value of the running transition,
            // - reversing shortening factor is the absolute value, clamped to the range [0, 1], of the sum of:
            //   1. the output of the timing function of the old transition at the time of the style change event,
            //      times the reversing shortening factor of the old transition
            //   2. 1 minus the reversing shortening factor of the old transition.
            let term_1 = input.old_timing_function_output * input.old_reversing_shortening_factor;
            let term_2 = 1.0 - input.old_reversing_shortening_factor;
            let reversing_shortening_factor = (term_1 + term_2).abs().clamp(0.0, 1.0);
            action.kind = FfiTransitionActionKind::CancelRemoveAndStartReversing;
            action.reversing_shortening_factor = reversing_shortening_factor;
            action.delay = if input.delay >= 0.0 {
                input.delay
            } else {
                reversing_shortening_factor * input.delay
            };
            // - start time is the time of the style change event plus:
            //   1. if the matching transition delay is nonnegative, the matching transition delay, or
            //   2. if the matching transition delay is negative, the product of the new transition’s reversing shortening factor and the matching transition delay,
            // - end time is the start time plus the product of the matching transition duration and the new transition’s reversing shortening factor,
            // - start value is the current value of the property in the running transition,
            // - end value is the value of the property in the after-change style,
            action.active_duration = input.duration * reversing_shortening_factor;
            return action;
        }

        // 4. Otherwise,
        // implementations must cancel the running transition and start a new transition whose:
        // - start time is the time of the style change event plus the matching transition delay,
        // - end time is the start time plus the matching transition duration,
        // - start value is the current value of the property in the running transition,
        // - end value is the value of the property in the after-change style,
        // - reversing-adjusted start value is the same as the start value, and
        // - reversing shortening factor is 1.
        action.kind = FfiTransitionActionKind::CancelRemoveAndStartInterrupted;
    }

    action
}

fn value_is_current_color(value: *const crate::css::style_value::StyleValueData) -> bool {
    matches!(
        unsafe { value.as_ref() },
        Some(crate::css::style_value::StyleValueData::Keyword { keyword })
            if *keyword == crate::css::style_compute::keyword::CURRENTCOLOR
    )
}

fn computed_value(
    table: &crate::css::computed_longhand_table::ComputedLonghandTable,
    overlay: Option<&crate::css::animated_overlay::AnimatedOverlay>,
    property_id: u16,
) -> *const crate::css::style_value::StyleValueData {
    if let Some(entry) = overlay.and_then(|overlay| overlay.get(property_id))
        && crate::css::animated_overlay::overlay_wins(entry, table.is_important(property_id))
    {
        return entry.value_pointer();
    }
    table
        .get(property_id)
        .expect("a transition property must have a computed value")
        .pointer()
}

fn originates_from_current_color(
    table: &crate::css::computed_longhand_table::ComputedLonghandTable,
    property_id: u16,
) -> bool {
    let value = table
        .retained_inheritance_dependent_values()
        .find_map(|(property, value)| (property == property_id).then_some(value))
        .filter(|value| crate::css::style_value::retained_value_depends_on_current_color(value))
        .or_else(|| table.get(property_id))
        .expect("a transition property must have a computed value");
    crate::css::style_value::retained_value_depends_on_current_color(value)
}

fn prepare_transition_values(
    before_style: (
        &crate::css::computed_longhand_table::ComputedLonghandTable,
        Option<&crate::css::animated_overlay::AnimatedOverlay>,
    ),
    after_table: &crate::css::computed_longhand_table::ComputedLonghandTable,
    after_overlay: Option<&crate::css::animated_overlay::AnimatedOverlay>,
    inherited_animation: Option<crate::css::style::InheritedAnimatedValue<'_>>,
    property: &mut FfiTransitionPropertyInput,
) -> bool {
    let (before_table, before_overlay) = before_style;
    property.before_change_value = computed_value(before_table, before_overlay, property.property_id);
    if !property.has_matching_transition {
        return false;
    }
    // NB: A record the engine derived holds an inherited animated value in its table, where a
    //     record the host computed holds the ancestor's base value and an inherited overlay entry.
    //     The ancestor's entry and base value stand in for that entry and table value.
    if let Some(entry) = after_overlay
        .and_then(|overlay| overlay.get(property.property_id))
        .filter(|entry| !entry.result_of_transition)
        .map(|entry| entry.value_pointer())
        .or_else(|| {
            inherited_animation
                .filter(|inherited| !inherited.entry.result_of_transition)
                .map(|inherited| inherited.entry.value_pointer())
        })
    {
        property.before_change_value = entry;
        property.after_change_value = entry;
        let originates_from_current_color = value_is_current_color(entry);
        if property.has_running_transition {
            property.current_value = computed_value(after_table, after_overlay, property.property_id);
        }
        return originates_from_current_color;
    } else if let Some(inherited) = inherited_animation {
        property.after_change_value = inherited.base_value;
    } else {
        property.after_change_value = computed_value(after_table, None, property.property_id);
    }
    if property.has_running_transition {
        property.current_value = computed_value(after_table, after_overlay, property.property_id);
    }
    originates_from_current_color(before_table, property.property_id)
        && originates_from_current_color(after_table, property.property_id)
}

/// Run the CSS Transitions decision algorithm for every supplied property.
///
/// C++ retains ownership of animation objects and executes the returned actions in order.
///
/// # Safety
/// `input` must point to a live value for the duration of the call. When the input contains
/// properties, `actions` must point at writable storage for `property_count` actions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_decide_transitions(
    style_engine: crate::css::style::StyleEngineHandle,
    before_style_record: u64,
    after_longhand_table: *const std::ffi::c_void,
    after_animated_overlay: *const std::ffi::c_void,
    input: *mut FfiTransitionInput,
    actions: *mut FfiTransitionAction,
) {
    crate::css::ffi_stats::rust_style_ffi_note_transition_decision();
    // SAFETY: Guaranteed by the caller.
    if unsafe { (*input).property_count } == 0 {
        return;
    }
    assert!(!style_engine.is_null(), "transition decisions require a style engine");
    crate::css::style::owner_calls::ask(
        style_engine,
        "rust_decide_transitions",
        crate::css::style::owner_calls::StyleQuery::DecideTransitions {
            before_style_record,
            after_longhand_table,
            after_animated_overlay,
            input,
            actions,
        },
    );
}

/// Answers [`rust_decide_transitions`] from `style_engine`, on the render owner.
///
/// # Safety
///
/// As for [`rust_decide_transitions`], with properties to decide.
pub(crate) unsafe fn owner_decide_transitions(
    style_engine: &crate::css::style::StyleEngine,
    before_style_record: u64,
    after_longhand_table: *const std::ffi::c_void,
    after_animated_overlay: *const std::ffi::c_void,
    input: *mut FfiTransitionInput,
    actions: *mut FfiTransitionAction,
) {
    let input = unsafe { &mut *input };
    let properties = unsafe { std::slice::from_raw_parts_mut(input.properties, input.property_count) };
    let after_table = unsafe {
        after_longhand_table
            .cast::<crate::css::computed_longhand_table::ComputedLonghandTable>()
            .as_ref()
    }
    .expect("transition decisions require an after-change table");
    let after_overlay = unsafe {
        after_animated_overlay
            .cast::<crate::css::animated_overlay::AnimatedOverlay>()
            .as_ref()
    };
    let actions = unsafe { std::slice::from_raw_parts_mut(actions, properties.len()) };
    decide_transitions(
        style_engine,
        before_style_record,
        after_table,
        after_overlay,
        &input.context,
        input.target_key,
        properties,
        actions,
    );
}

/// The decision for every property, over a before-change record and an after-change style, the
/// way `rust_decide_transitions` makes it for the host and the style pass makes it for a row it
/// settled.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_transitions(
    engine: &crate::css::style::RetainedState,
    before_style_record: u64,
    after_table: &crate::css::computed_longhand_table::ComputedLonghandTable,
    after_overlay: Option<&crate::css::animated_overlay::AnimatedOverlay>,
    context: &crate::css::animation::FfiAnimationContext,
    target_key: u64,
    properties: &mut [FfiTransitionPropertyInput],
    actions: &mut [FfiTransitionAction],
) {
    let before_style_view = engine
        .style_record_view(before_style_record)
        .expect("the transition baseline style record must remain live");
    let before_style = (
        unsafe {
            before_style_view
                .longhand_table
                .as_ref()
                .expect("a transition baseline style record must carry a longhand table")
        },
        unsafe { before_style_view.animated_overlay.as_ref() },
    );
    // Only an element's own record inherits from its inheritance parent. Whether an ancestor
    // animates is asked once, by the first property the record inherits.
    let target = (target_key & 0xff == u64::from(u8::MAX))
        .then(|| crate::css::style::tree::StyleNodeID::from_raw((target_key >> 8) as u32))
        .flatten();
    let mut animated_chain = None;
    for (property, action) in properties.iter_mut().zip(actions.iter_mut()) {
        let inherited_animation = target
            .filter(|_| {
                after_table.is_inherited(property.property_id)
                    && after_overlay
                        .and_then(|overlay| overlay.get(property.property_id))
                        .is_none_or(|entry| !entry.inherited)
            })
            .and_then(|target| *animated_chain.get_or_insert_with(|| engine.animated_inheritance_chain(target)))
            .and_then(|chain| chain.inherited_animated_value(after_table, property.property_id));
        let values_originate_from_current_color =
            prepare_transition_values(before_style, after_table, after_overlay, inherited_animation, property);
        *action = decide_transition(context, property, values_originate_from_current_color);
    }
}

/// What a transition on an element resolves its lengths against, answered from the record the
/// element has installed. Returns false before the document published any computation inputs.
///
/// # Safety
/// `style_engine` must be a live style engine, `style_record` a live record of it, and
/// `context` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_transition_length_resolution_context(
    style_engine: crate::css::style::StyleEngineHandle,
    style_record: u64,
    context: *mut crate::css::animation::FfiAnimationLengthResolutionContext,
) -> bool {
    crate::css::style::owner_calls::ask(
        style_engine,
        "rust_transition_length_resolution_context",
        crate::css::style::owner_calls::StyleQuery::TransitionLengthResolutionContext { style_record, context },
    )
    .is()
}

/// Answers [`rust_transition_length_resolution_context`] from `style_engine`, on the render owner.
///
/// # Safety
///
/// As for [`rust_transition_length_resolution_context`].
pub(crate) unsafe fn owner_transition_length_resolution_context(
    style_engine: &mut crate::css::style::StyleEngine,
    style_record: u64,
    context: *mut crate::css::animation::FfiAnimationLengthResolutionContext,
) -> bool {
    let Some(length) = style_engine.transition_length_resolution_context(style_record) else {
        return false;
    };
    unsafe { context.write(length) };
    true
}

/// One `transition-property` entry's attributes, as they apply to one physical longhand.
#[repr(C)]
pub struct FfiTransitionEntry {
    pub property_id: u16,
    pub delay: f64,
    pub duration: f64,
    pub timing_function: *const crate::css::style_value::StyleValueData,
    pub behavior: u8,
}

#[repr(C)]
pub struct FfiTransitionEntries {
    pub entries: *const FfiTransitionEntry,
    pub count: usize,
    pub delay_and_duration_are_single_zero: bool,
    pub storage: *mut std::ffi::c_void,
}

/// The transitions a computed longhand table declares, per physical longhand they name. What the
/// timing functions point at is borrowed from the table.
///
/// # Safety
/// `longhand_table` must point to a live computed longhand table. The result must be released with
/// `rust_transition_entries_release`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_transition_entries(longhand_table: *const std::ffi::c_void) -> FfiTransitionEntries {
    let table = unsafe { &*longhand_table.cast::<crate::css::computed_longhand_table::ComputedLonghandTable>() };
    let (entries, delay_and_duration_are_single_zero) = crate::css::style_compute::transition_entries(table);
    let entries = Box::new(entries.into_boxed_slice());
    FfiTransitionEntries {
        entries: entries.as_ptr(),
        count: entries.len(),
        delay_and_duration_are_single_zero,
        storage: Box::into_raw(entries).cast(),
    }
}

/// # Safety
/// `storage` must come from `rust_transition_entries` and not have been released before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_transition_entries_release(storage: *mut std::ffi::c_void) {
    drop(unsafe { Box::from_raw(storage.cast::<Box<[FfiTransitionEntry]>>()) });
}

/// Whether the table's `transition-delay` and `transition-duration` are each the single value `0s`,
/// which is how nearly every element declares no transition at all.
///
/// # Safety
/// `longhand_table` must point to a live computed longhand table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_transition_delay_and_duration_are_single_zero(
    longhand_table: *const std::ffi::c_void,
) -> bool {
    crate::css::style_compute::transition_delay_and_duration_are_single_zero(unsafe {
        &*longhand_table.cast::<crate::css::computed_longhand_table::ComputedLonghandTable>()
    })
}

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
mod tests {
    use super::*;

    fn animation_context() -> crate::css::animation::FfiAnimationContext {
        let font_metrics = || crate::css::animation::FfiAnimationFontMetrics {
            font_size: 0.0,
            x_height: 0.0,
            cap_height: 0.0,
            zero_advance: 0.0,
            line_height: 0.0,
        };
        crate::css::animation::FfiAnimationContext {
            allow_discrete: false,
            current_color: std::ptr::null(),
            has_length_resolution_context: false,
            length_resolution_context: crate::css::animation::FfiAnimationLengthResolutionContext {
                viewport_width: 0.0,
                viewport_height: 0.0,
                font_metrics: font_metrics(),
                root_font_metrics: font_metrics(),
                font_metrics_depend_on_viewport_metrics: false,
                root_font_metrics_depend_on_viewport_metrics: false,
            },
            has_transform_reference_box: false,
            transform_reference_box_width: 0.0,
            transform_reference_box_height: 0.0,
        }
    }

    fn by_computed_value_property() -> u16 {
        (crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
            ..=crate::css::property_metadata::LAST_LONGHAND_PROPERTY_ID)
            .find(|property_id| {
                crate::css::property_metadata::property_animation_type(*property_id)
                    == crate::css::animation::ANIMATION_TYPE_BY_COMPUTED_VALUE
            })
            .unwrap()
    }

    fn input(
        before_change_value: &crate::css::style_value::StyleValueData,
        after_change_value: &crate::css::style_value::StyleValueData,
        current_value: &crate::css::style_value::StyleValueData,
    ) -> FfiTransitionPropertyInput {
        FfiTransitionPropertyInput {
            property_id: by_computed_value_property(),
            before_change_value,
            after_change_value,
            current_value,
            existing_end_value: std::ptr::null(),
            reversing_adjusted_start_value: std::ptr::null(),
            has_matching_transition: true,
            allow_discrete: false,
            has_running_transition: false,
            has_completed_transition: false,
            delay: 0.0,
            duration: 100.0,
            old_timing_function_output: 0.0,
            old_reversing_shortening_factor: 1.0,
        }
    }

    #[test]
    fn starts_an_initial_transition() {
        let before = crate::css::style_value::StyleValueData::Number { value: 0.0 };
        let after = crate::css::style_value::StyleValueData::Number { value: 1.0 };
        assert_eq!(
            decide_transition(&animation_context(), &input(&before, &after, &before), false).kind,
            FfiTransitionActionKind::Start
        );
    }

    #[test]
    fn accepts_an_empty_transition_batch() {
        let mut input = FfiTransitionInput {
            context: animation_context(),
            properties: std::ptr::null_mut(),
            property_count: 0,
            target_key: 0,
        };
        unsafe {
            rust_decide_transitions(
                crate::css::style::StyleEngineHandle::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
                &raw mut input,
                std::ptr::null_mut(),
            )
        };
    }

    #[test]
    fn equal_nested_values_do_not_start_a_transition() {
        let nested_value = || {
            let number =
                std::sync::Arc::into_raw(std::sync::Arc::new(crate::css::style_value::StyleValueData::Number {
                    value: 0.5,
                }));
            crate::css::style_value::StyleValueData::OpacityValue {
                value: unsafe { crate::css::style_value::RetainedStyleValueData::from_retained_pointer(number) },
            }
        };
        let before = nested_value();
        let after = nested_value();
        assert_eq!(
            decide_transition(&animation_context(), &input(&before, &after, &before), false).kind,
            FfiTransitionActionKind::None
        );
    }

    #[test]
    fn current_color_origins_are_equivalent() {
        let before = crate::css::style_value::StyleValueData::Number { value: 0.0 };
        let after = crate::css::style_value::StyleValueData::Number { value: 1.0 };
        let input = input(&before, &after, &before);
        assert_eq!(
            decide_transition(&animation_context(), &input, true).kind,
            FfiTransitionActionKind::None
        );
    }

    #[test]
    fn nested_current_color_origin_is_recognized() {
        let current_color = crate::css::style_value::RetainedStyleValueData::from_owned(
            crate::css::style_value::StyleValueData::Keyword {
                keyword: crate::css::style_compute::keyword::CURRENTCOLOR,
            },
        );
        let nested = crate::css::style_value::StyleValueData::ValueList {
            values: crate::css::style_value::RetainedStyleValueDataList::from_retained_values(vec![current_color]),
            separator: 0,
            collapsible: false,
        };
        let mut table = crate::css::computed_longhand_table::ComputedLonghandTable::new();
        table.set(
            by_computed_value_property(),
            crate::css::style_value::RetainedStyleValueData::from_owned(nested),
            -1,
        );

        assert!(originates_from_current_color(&table, by_computed_value_property()));
    }

    #[test]
    fn removes_a_completed_transition_before_replacement() {
        let before = crate::css::style_value::StyleValueData::Number { value: 0.0 };
        let after = crate::css::style_value::StyleValueData::Number { value: 1.0 };
        let mut input = input(&before, &after, &before);
        input.has_completed_transition = true;
        input.existing_end_value = &raw const before;
        assert_eq!(
            decide_transition(&animation_context(), &input, false).kind,
            FfiTransitionActionKind::RemoveAndStart
        );
    }

    #[test]
    fn adjusts_a_reversing_transition() {
        let before = crate::css::style_value::StyleValueData::Number { value: 0.0 };
        let after = crate::css::style_value::StyleValueData::Number { value: 1.0 };
        let current = crate::css::style_value::StyleValueData::Number { value: 0.5 };
        let mut input = input(&before, &after, &current);
        input.has_running_transition = true;
        input.existing_end_value = &raw const before;
        input.reversing_adjusted_start_value = &raw const after;
        input.delay = -20.0;
        input.old_timing_function_output = 0.25;
        input.old_reversing_shortening_factor = 0.5;
        let action = decide_transition(&animation_context(), &input, false);
        assert_eq!(action.kind, FfiTransitionActionKind::CancelRemoveAndStartReversing);
        assert_eq!(action.reversing_shortening_factor, 0.625);
        assert_eq!(action.delay, -12.5);
        assert_eq!(action.active_duration, 62.5);
    }
}
