/*
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2024, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include "Animatable.h"
#include <LibWeb/Animations/DocumentTimeline.h>
#include <LibWeb/Animations/PseudoElementParsing.h>
#include <LibWeb/CSS/CSSAnimation.h>
#include <LibWeb/CSS/CSSAnimationProperties.h>
#include <LibWeb/CSS/CSSTransition.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/StyleValueRustFFI.h>

namespace Web::Animations {

struct Animatable::Transition {
    AK_ALLOC_WITH_KMALLOC;

    HashMap<CSS::PropertyID, GC::Ref<CSS::CSSTransition>> associated_transitions;
};

Animatable::Impl::~Impl() = default;

static Animatable::AnimationTimingRowCounters s_animation_timing_row_counters;

static WebIDL::ExceptionOr<Animatable::GetAnimationsOptions> get_animations_options_from_bindings(Bindings::GetAnimationsOptions const& options)
{
    Animatable::GetAnimationsOptions converted_options;
    converted_options.subtree = options.subtree;
    if (options.pseudo_element.has_value())
        converted_options.pseudo_element = TRY(pseudo_element_parsing(options.pseudo_element));
    return converted_options;
}

static WebIDL::ExceptionOr<Animatable::KeyframeAnimationOptions> keyframe_animation_options_from_bindings(Bindings::KeyframeAnimationOptions const& options)
{
    auto effect_options = TRY(keyframe_effect_options_from_bindings(options));
    Animatable::KeyframeAnimationOptions converted_options;
    static_cast<KeyframeEffect::Options&>(converted_options) = move(effect_options);
    converted_options.id = options.id;
    converted_options.timeline = options.timeline;
    return converted_options;
}

static WebIDL::ExceptionOr<Variant<double, Animatable::KeyframeAnimationOptions>> keyframe_animation_options_from_bindings(Variant<double, Bindings::KeyframeAnimationOptions> const& options)
{
    if (options.has<double>())
        return options.get<double>();
    return TRY(keyframe_animation_options_from_bindings(options.get<Bindings::KeyframeAnimationOptions>()));
}

// https://www.w3.org/TR/web-animations-1/#dom-animatable-animate
WebIDL::ExceptionOr<GC::Ref<Animation>> Animatable::animate(Vector<BaseKeyframe> keyframes, Variant<double, KeyframeAnimationOptions> const& options)
{
    // 1. Let target be the object on which this method was called.
    GC::Ref target { *static_cast<DOM::Element*>(this) };

    // 2. Construct a new KeyframeEffect object, effect, in the relevant Realm of target by using the same procedure as
    //    the KeyframeEffect(target, keyframes, options) constructor, passing target as the target argument, and the
    //    keyframes and options arguments as supplied.
    //
    //    If the above procedure causes an exception to be thrown, propagate the exception and abort this procedure.
    auto effect = TRY(options.visit(
        [&](auto const& value) { return KeyframeEffect::create_from_processed_keyframes(target, move(keyframes), value); }));

    // 3. If options is a KeyframeAnimationOptions object, let timeline be the timeline member of options or, if
    //    timeline member of options is missing, be the default document timeline of the node document of the element
    //    on which this method was called.
    Optional<GC::Ptr<AnimationTimeline>> timeline;
    if (options.has<KeyframeAnimationOptions>() && options.get<KeyframeAnimationOptions>().timeline.has_value())
        timeline = options.get<KeyframeAnimationOptions>().timeline.value();
    if (!timeline.has_value())
        timeline = target->document().timeline();

    // 4. Construct a new Animation object, animation, in the relevant Realm of target by using the same procedure as
    //    the Animation() constructor, passing effect and timeline as arguments of the same name.
    auto animation = Animation::create(target->document().relevant_settings_object(), effect, timeline.release_value());

    // 5. If options is a KeyframeAnimationOptions object, assign the value of the id member of options to animation’s
    //    id attribute.
    if (options.has<KeyframeAnimationOptions>())
        animation->set_id(options.get<KeyframeAnimationOptions>().id);

    //  6. Run the procedure to play an animation for animation with the auto-rewind flag set to true.
    TRY(animation->play_an_animation(Animation::AutoRewind::Yes));

    // 7. Return animation.
    return animation;
}

WebIDL::ExceptionOr<GC::Ref<Animation>> Animatable::animate(JS::Realm& realm, GC::Ptr<JS::Object> keyframes, Variant<double, Bindings::KeyframeAnimationOptions> const& options)
{
    auto animation = TRY(animate(TRY(process_keyframes(realm, keyframes)), TRY(keyframe_animation_options_from_bindings(options))));

    return animation;
}

// https://drafts.csswg.org/web-animations-1/#dom-animatable-getanimations
WebIDL::ExceptionOr<Vector<GC::Ref<Animation>>> Animatable::get_animations(GetAnimationsOptions const& options)
{
    as<DOM::Element>(*this).document().update_style();

    return get_animations_internal(GetAnimationsSorted::Yes, options);
}

WebIDL::ExceptionOr<Vector<GC::Ref<Animation>>> Animatable::get_animations(Bindings::GetAnimationsOptions const& options)
{
    return get_animations(TRY(get_animations_options_from_bindings(options)));
}

WebIDL::ExceptionOr<Vector<GC::Ref<Animation>>> Animatable::get_animations_internal(GetAnimationsSorted sorted, GetAnimationsOptions const& options)
{
    // 1. Let object be the object on which this method was called.

    // 3. If pseudoElement is not null, then let target be the pseudo-element identified by pseudoElement with object as the originating element.
    //    Otherwise, let target be object.
    // FIXME: We can't refer to pseudo-elements directly, and they also can't be animated yet.
    (void)options.pseudo_element;
    GC::Ref target { *static_cast<DOM::Element*>(this) };

    // 4. If options is passed with subtree set to true, then return the set of relevant animations for a subtree of target.
    //    Otherwise, return the set of relevant animations for target.
    Vector<GC::Ref<Animation>> relevant_animations;
    if (m_impl) {
        auto& associated_animations = m_impl->associated_animations;
        for (auto const& animation : associated_animations) {
            if (animation->is_relevant())
                relevant_animations.append(*animation);
        }
    }

    if (options.subtree) {
        TRY(target->for_each_child_of_type_fallible<DOM::Element>([&](auto& child) -> WebIDL::ExceptionOr<IterationDecision> {
            relevant_animations.extend(TRY(child.get_animations_internal(GetAnimationsSorted::No, options)));
            return IterationDecision::Continue;
        }));
    }

    // The returned list is sorted using the composite order described for the associated animations of effects in
    // §5.4.2 The effect stack.
    if (sorted == GetAnimationsSorted::Yes) {
        quick_sort(relevant_animations, [](GC::Ref<Animation>& a, GC::Ref<Animation>& b) {
            auto& a_effect = as<KeyframeEffect>(*a->effect());
            auto& b_effect = as<KeyframeEffect>(*b->effect());
            return KeyframeEffect::composite_order(a_effect, b_effect) < 0;
        });
    }

    return relevant_animations;
}

ReadonlySpan<GC::Ref<Animation>> Animatable::associated_animations_in_composite_order()
{
    if (!m_impl)
        return {};

    if (!m_impl->is_sorted_by_composite_order) {
        quick_sort(m_impl->associated_animations, [](auto const& a, auto const& b) {
            auto a_effect = a->effect();
            auto b_effect = b->effect();
            bool a_is_keyframe_effect = a_effect && is<KeyframeEffect>(*a_effect);
            bool b_is_keyframe_effect = b_effect && is<KeyframeEffect>(*b_effect);
            if (a_is_keyframe_effect && b_is_keyframe_effect)
                return KeyframeEffect::composite_order(static_cast<KeyframeEffect&>(*a_effect), static_cast<KeyframeEffect&>(*b_effect)) < 0;
            if (a_is_keyframe_effect != b_is_keyframe_effect)
                return !a_is_keyframe_effect;
            return a->global_animation_list_order() < b->global_animation_list_order();
        });
        m_impl->is_sorted_by_composite_order = true;
    }

    return m_impl->associated_animations.span();
}

void Animatable::invalidate_associated_animation_composite_order()
{
    if (m_impl)
        m_impl->is_sorted_by_composite_order = false;
}

bool Animatable::has_associated_animations() const
{
    return m_impl && !m_impl->associated_animations.is_empty();
}

bool Animatable::has_relevant_animations() const
{
    if (!m_impl)
        return false;

    for (auto const& animation : m_impl->associated_animations) {
        if (animation->is_relevant())
            return true;
    }

    return false;
}

void Animatable::associate_with_animation(GC::Ref<Animation> animation)
{
    auto& impl = ensure_impl();
    if (impl.associated_animations.contains_slow(animation))
        return;
    // The association publishes the element's facts and animation rows to the style mirror the
    // layout frame reads, so an association made beside a frame in flight that reaches the style
    // engine waits for it first. A recording reaches none.
    as<DOM::Element>(*this).document().join_frame_reaching_style_engine();
    impl.associated_animations.append(animation);
    impl.is_sorted_by_composite_order = false;
    // The style engine computes no record for an element whose animations compose its style.
    CSS::record_element_adjustment_facts(as<DOM::Element>(*this));

    as<DOM::Element>(*this).change_associated_animation_count_in_subtree(1);

    as<DOM::Element>(*this).document().associate_with_animation(animation);
    animation->did_associate_with_target();

    publish_animation_timing_rows();
}

void Animatable::disassociate_with_animation(GC::Ref<Animation> animation)
{
    auto& impl = *m_impl;
    auto was_associated = impl.associated_animations.remove_first_matching([&](auto element) { return animation == element; });
    impl.is_sorted_by_composite_order = false;
    CSS::record_element_adjustment_facts(as<DOM::Element>(*this));

    if (was_associated)
        as<DOM::Element>(*this).change_associated_animation_count_in_subtree(-1);

    as<DOM::Element>(*this).document().disassociate_with_animation(animation);

    publish_animation_timing_rows();
}

void Animatable::on_document_changed(DOM::Document& old_document, DOM::Document& new_document)
{
    if (!m_impl)
        return;

    for (auto const& animation : m_impl->associated_animations) {
        old_document.disassociate_with_animation(animation);
        new_document.associate_with_animation(animation);
    }
}

void Animatable::cancel_css_animations_and_transitions()
{
    if (!m_impl)
        return;

    GC::RootVector<GC::Ref<Animation>> animations_to_cancel;
    for (size_t index = 0; index < m_impl->css_defined_animations.size(); ++index) {
        auto& animations = m_impl->css_defined_animations[index];
        if (!animations)
            continue;
        if (animations->is_empty())
            continue;
        for (auto& animation : *animations)
            animations_to_cancel.append(animation);
        animations->clear();
        publish_css_defined_animations(index);
    }
    for (size_t index = 0; index < m_impl->transitions.size(); ++index) {
        auto& transition = m_impl->transitions[index];
        if (!transition || transition->associated_transitions.is_empty())
            continue;
        for (auto& animation : transition->associated_transitions)
            animations_to_cancel.append(animation.value);
        transition->associated_transitions.clear();
        CSS::CSSTransition::publish_transitions(as<DOM::Element>(*this), index == 0 ? Optional<CSS::PseudoElement> {} : static_cast<CSS::PseudoElement>(index - 1));
    }
    m_impl->has_css_defined_animations = false;

    for (auto& animation : animations_to_cancel)
        animation->cancel(Animation::ShouldInvalidate::No);

    publish_animation_timing_rows();
}

// The longhands the element's installed style gives a matching transition-property entry. The
// engine reads them from the style's transition longhands. A declaration whose delay and duration
// are each the single value 0s starts nothing, so it has none unless the element already holds a
// transition, which such an entry could still cancel.
Vector<CSS::PropertyID> Animatable::property_ids_with_matching_transition_property_entry(Optional<CSS::PseudoElement> pseudo_element) const
{
    auto& element = const_cast<DOM::Element&>(static_cast<DOM::Element const&>(*this));
    auto style_record = DOM::AbstractElement { element, pseudo_element }.style_record_identity();
    if (!style_record)
        return {};
    auto style = element.document().style_computer().style_engine().style_record_view(style_record);
    if (!style.present || !style.longhand_table)
        return {};
    if (CSS::StyleValueFFI::rust_transition_delay_and_duration_are_single_zero(style.longhand_table)
        && property_ids_with_existing_transitions(pseudo_element).is_empty())
        return {};
    auto entries = CSS::StyleValueFFI::rust_transition_entries(style.longhand_table);
    Vector<CSS::PropertyID> property_ids;
    property_ids.ensure_capacity(entries.count);
    for (auto const& entry : ReadonlySpan<CSS::StyleValueFFI::FfiTransitionEntry> { entries.entries, entries.count })
        property_ids.unchecked_append(static_cast<CSS::PropertyID>(entry.property_id));
    CSS::StyleValueFFI::rust_transition_entries_release(entries.storage);
    return property_ids;
}

Vector<CSS::PropertyID> Animatable::property_ids_with_existing_transitions(Optional<CSS::PseudoElement> pseudo_element) const
{
    auto const* maybe_transition = transition_if_exists(pseudo_element);

    if (!maybe_transition)
        return {};

    return maybe_transition->associated_transitions.keys();
}

GC::Ptr<CSS::CSSTransition> Animatable::property_transition(Optional<CSS::PseudoElement> pseudo_element, CSS::PropertyID property) const
{
    auto const* maybe_transition = transition_if_exists(pseudo_element);
    if (!maybe_transition)
        return {};
    auto& transition = *maybe_transition;
    if (auto maybe_animation = transition.associated_transitions.get(property); maybe_animation.has_value())
        return maybe_animation.value();
    return {};
}

void Animatable::set_transition(Optional<CSS::PseudoElement> pseudo_element, CSS::PropertyID property, GC::Ref<CSS::CSSTransition> animation)
{
    auto maybe_transition = ensure_transition(pseudo_element);
    if (!maybe_transition)
        return;
    auto& transition = *maybe_transition;
    VERIFY(!transition.associated_transitions.contains(property));
    transition.associated_transitions.set(property, animation);
    CSS::CSSTransition::publish_transitions(as<DOM::Element>(*this), pseudo_element);
}

void Animatable::remove_transition(Optional<CSS::PseudoElement> pseudo_element, CSS::PropertyID property_id)
{
    auto maybe_transition = ensure_transition(pseudo_element);
    if (!maybe_transition)
        return;
    auto& transition = *maybe_transition;
    auto removed_transition = transition.associated_transitions.get(property_id);
    VERIFY(removed_transition.has_value());
    transition.associated_transitions.remove(property_id);
    removed_transition.value()->schedule_disassociation_from_target();
    CSS::CSSTransition::publish_transitions(as<DOM::Element>(*this), pseudo_element);
}

void Animatable::visit_edges(JS::Cell::Visitor& visitor)
{
    if (m_impl)
        m_impl->visit_edges(visitor);
}

void Animatable::Impl::visit_edges(JS::Cell::Visitor& visitor)
{
    visitor.visit(associated_animations);
    for (auto const& css_animation : css_defined_animations) {
        if (css_animation)
            visitor.visit(*css_animation);
    }

    for (auto const& transition : transitions) {
        if (transition)
            visitor.visit(transition->associated_transitions);
    }
}

bool Animatable::has_css_defined_animations() const
{
    if (!m_impl)
        return false;

    return m_impl->has_css_defined_animations;
}

bool Animatable::has_css_animations_or_transitions() const
{
    if (!m_impl)
        return false;
    if (m_impl->has_css_defined_animations)
        return true;
    for (auto const& transition : m_impl->transitions) {
        if (transition && !transition->associated_transitions.is_empty())
            return true;
    }
    return false;
}

Vector<GC::Ref<CSS::CSSAnimation>> const* Animatable::css_defined_animations(Optional<CSS::PseudoElement> pseudo_element)
{
    auto& impl = ensure_impl();

    if (pseudo_element.has_value() && !CSS::Selector::PseudoElementSelector::is_known_pseudo_element_type(pseudo_element.value()))
        return nullptr;

    auto index = pseudo_element
                     .map([](CSS::PseudoElement pseudo_element_value) { return to_underlying(pseudo_element_value) + 1; })
                     .value_or(0);

    if (!impl.css_defined_animations[index])
        impl.css_defined_animations[index] = make<Vector<GC::Ref<CSS::CSSAnimation>>>();

    return impl.css_defined_animations[index];
}

void Animatable::set_css_defined_animations(Optional<CSS::PseudoElement> pseudo_element, Vector<GC::Ref<CSS::CSSAnimation>>&& animations)
{
    auto& impl = ensure_impl();

    if (pseudo_element.has_value() && !CSS::Selector::PseudoElementSelector::is_known_pseudo_element_type(pseudo_element.value()))
        return;

    auto index = pseudo_element
                     .map([](CSS::PseudoElement pseudo_element_value) { return to_underlying(pseudo_element_value) + 1; })
                     .value_or(0);

    // NB: The flag says the element has animations to play or cancel, not that it has been through
    //     the step that would have registered some. Every element goes through that step, so setting
    //     it here unconditionally made it true of every element that had computed a style once.
    //     It stays set once any list is non-empty, since the lists are per pseudo-element and this
    //     is one flag for all of them.
    if (!animations.is_empty())
        impl.has_css_defined_animations = true;
    impl.css_defined_animations[index] = make<Vector<GC::Ref<CSS::CSSAnimation>>>(move(animations));
    publish_css_defined_animations(index);
}

// The style stage decides which animation each of an element's animation definitions claims, so the
// names of the animations it already has are an input to it rather than something it asks for.
void Animatable::publish_css_defined_animations(size_t index)
{
    auto* element = as_if<DOM::Element>(*this);
    if (!element)
        return;

    Vector<Utf16FlyString> names;
    // Beside each name, the definition the plan last applied to that animation. A plan whose
    // definitions all equal these changes nothing when it is applied, and the computation that
    // decides that needs no help from the host to see it.
    Vector<u64> definition_words;
    if (auto const& animations = m_impl->css_defined_animations[index]) {
        names.ensure_capacity(animations->size());
        definition_words.ensure_capacity(animations->size() * CSS::AppliedAnimationDefinitionRow::word_count);
        for (auto const& animation : *animations) {
            names.unchecked_append(animation->animation_name());
            auto row = animation->applied_definition_row();
            definition_words.append(row.words, CSS::AppliedAnimationDefinitionRow::word_count);
        }
    }
    CSS::record_element_css_defined_animations(*element, static_cast<u8>(index), names, definition_words);
}

// Which of the animations an element holds are relevant is a question about the WAAPI timing
// model, not about the GC heap: it is answered from the animation's own timing and the current time
// of its timeline. Publish the timing, once per list, so the style stage can answer it itself.
void Animatable::publish_animation_timing_rows()
{
    // NB: Every Animatable is an Element, and every style update asks this of every animated one, so this does not
    //     pay for a cross-cast to learn it.
    auto* element = static_cast<DOM::Element*>(this);

    auto slot_of = [](KeyframeEffect const& effect) {
        auto pseudo_element = effect.pseudo_element_type();
        return pseudo_element.has_value() ? static_cast<u8>(to_underlying(*pseudo_element) + 1) : static_cast<u8>(0);
    };

    // A transition the style computation has provisionally started is sampled by the pass that
    // started it, and by every animated style update until the stabilization epoch commits, but it
    // is not associated with the element yet. Publish its timing too, or the one computation that
    // samples it has nothing to sample it from.
    // OPTIMIZATION: There are none outside a style update that starts transitions, so no vector is rooted for them.
    Optional<GC::ConservativeVector<GC::Ref<KeyframeEffect>>> provisional_effect_storage;
    element->document().style_computer().for_each_provisional_transition_effect_on_element(*element, [&](KeyframeEffect& effect) {
        if (!provisional_effect_storage.has_value())
            provisional_effect_storage.emplace();
        provisional_effect_storage->append(effect);
    });
    ReadonlySpan<GC::Ref<KeyframeEffect>> provisional_effects;
    if (provisional_effect_storage.has_value())
        provisional_effects = provisional_effect_storage->span();
    // An element whose only animation is a provisionally started transition has no animation state
    // of its own yet, and one with neither has nothing to publish and nothing published.
    if (!m_impl && provisional_effects.is_empty())
        return;
    auto& impl = ensure_impl();

    // OPTIMIZATION: An element holds a few animations, and every style update rebuilds its rows to
    //               learn that they have not moved, so the buffers they are built in start inline.
    Vector<u8, 4> slots_with_rows;
    auto note_slot_of = [&](KeyframeEffect const& effect) {
        auto slot = slot_of(effect);
        if (!slots_with_rows.contains_slow(slot))
            slots_with_rows.append(slot);
    };
    for (auto const& effect : provisional_effects)
        note_slot_of(*effect);
    for (auto const& animation : impl.associated_animations) {
        auto effect = animation->effect();
        if (!effect || !is<KeyframeEffect>(*effect))
            continue;
        note_slot_of(static_cast<KeyframeEffect const&>(*effect));
    }

    Vector<u32, 4 * Animation::StyleTimingRow::word_count> words;
    Vector<u64, 4 * Animation::StyleTimingRow::TimeCount> times;
    // The `linear()` stops the rows name by range, input and output interleaved as raw `f64` bits.
    // The rows are reordered below and this buffer is not, so a range stays the one it was appended
    // at.
    Vector<u64> linear_points;
    Vector<GC::Ref<KeyframeEffect>, 4> effects_in_order;
    // A CSS animation keeps the place in its owning element's `animation-name` list it was given
    // when a plan last applied a definition to it, and script can revive one the element has since
    // stopped listing. Its place is then one another animation holds, so the key alone says nothing
    // about which of the two the element's next plan works on. Say on the row whether the element
    // really lists this animation there.
    auto listed_by_owning_element = [](Animation& animation) -> u32 {
        auto owning_element = animation.owning_element();
        if (!owning_element.has_value())
            return 0;
        auto const* css_defined_animations = owning_element->element().css_defined_animations(owning_element->pseudo_element());
        if (!css_defined_animations)
            return 0;
        auto index = animation.class_specific_composite_order_key();
        if (index >= css_defined_animations->size() || &*css_defined_animations->at(index) != &animation)
            return 0;
        return Animation::StyleTimingRow::listed_by_owning_element;
    };
    auto append_row = [&](KeyframeEffect& keyframe_effect, Animation& animation, u32 extra_flags) {
        auto row = animation.style_timing_row(linear_points);
        row.effect_identity = keyframe_effect.animation_preparation_identity();
        auto const* css_animation = as_if<CSS::CSSAnimation>(animation);
        auto const play_state_overridden = css_animation && css_animation->script_overrode_play_state()
            ? Animation::StyleTimingRow::css_play_state_overridden_by_script
            : 0;
        words.append(row.flags | extra_flags | listed_by_owning_element(animation) | play_state_overridden);
        words.append(row.timeline_identity);
        words.append(bit_cast<u32>(row.easing_interval_count));
        words.append(static_cast<u32>(row.effect_identity));
        words.append(static_cast<u32>(row.effect_identity >> 32));
        words.append(static_cast<u32>(row.composite_class)
            | (static_cast<u32>(row.composite_owning_slot) << 8)
            | (static_cast<u32>(row.composite_transition_property) << 16));
        words.append(row.composite_owning_node);
        words.append(row.composite_class_key);
        words.append(row.global_list_order);
        words.append(row.first_linear_point);
        words.append(row.linear_point_count);
        for (auto time : row.times)
            times.append(bit_cast<u64>(time));
        effects_in_order.append(keyframe_effect);
    };
    // The order the animations happen to sit in the element's list is the order they were
    // associated in. The composite order is the one a consumer of the published list needs, and
    // every number it is decided by travels on the row, so the mirror sorts the list rather than
    // the element sorting its own animations for the occasion.
    Vector<u32> ordered_words;
    Vector<u64> ordered_times;
    Vector<GC::Ref<KeyframeEffect>> ordered_effects;
    Vector<u32> order;
    auto put_rows_in_composite_order = [&] {
        auto row_count = effects_in_order.size();
        order.resize(row_count);
        CSS::StyleValueFFI::rust_animation_timing_rows_composite_order(words.data(), row_count, order.data());
        ordered_words.clear_with_capacity();
        ordered_times.clear_with_capacity();
        ordered_effects.clear_with_capacity();
        for (auto index : order) {
            ordered_words.append(words.data() + index * Animation::StyleTimingRow::word_count, Animation::StyleTimingRow::word_count);
            ordered_times.append(times.data() + index * Animation::StyleTimingRow::TimeCount, Animation::StyleTimingRow::TimeCount);
            ordered_effects.append(effects_in_order[index]);
        }
    };
    // The rows and descriptions a list publishes are a function of what it is built from here, so
    // a list built the same as it was last published has nothing new to tell the engine. Every
    // style update republishes every animated element, and most of them have not moved.
    auto published_list = [&](u8 slot) -> Impl::PublishedTimingRows* {
        for (auto& published : impl.published_timing_rows) {
            if (published.slot == slot)
                return &published;
        }
        return nullptr;
    };
    Vector<u64, 4> effect_generations;
    for (auto slot : slots_with_rows) {
        words.clear_with_capacity();
        times.clear_with_capacity();
        linear_points.clear_with_capacity();
        effects_in_order.clear_with_capacity();
        for (auto& effect : provisional_effects) {
            if (slot_of(*effect) != slot)
                continue;
            if (auto animation = effect->associated_animation())
                append_row(*effect, *animation, Animation::StyleTimingRow::not_associated);
        }
        for (auto const& animation : impl.associated_animations) {
            auto effect = animation->effect();
            if (!effect || !is<KeyframeEffect>(*effect))
                continue;
            auto& keyframe_effect = static_cast<KeyframeEffect&>(*effect);
            if (slot_of(keyframe_effect) != slot)
                continue;
            append_row(keyframe_effect, *animation, 0);
        }
        effect_generations.clear_with_capacity();
        for (auto const& effect : effects_in_order)
            effect_generations.append(effect->animation_preparation_generation());

        auto* published = published_list(slot);
        if (published && !impl.published_timing_rows_are_stale
            && published->words == words && published->times == times
            && published->linear_points == linear_points && published->effect_generations == effect_generations) {
            ++s_animation_timing_row_counters.lists_unchanged;
            continue;
        }

        put_rows_in_composite_order();
        CSS::record_element_animation_timing_rows(*element, slot, ordered_words, ordered_times, linear_points);
        CSS::record_element_animation_effect_descriptions(*element, slot, ordered_effects);
        ++s_animation_timing_row_counters.lists_published;
        if (!published) {
            impl.published_timing_rows.append({});
            published = &impl.published_timing_rows.last();
            published->slot = slot;
        }
        published->words.clear_with_capacity();
        published->words.append(words.data(), words.size());
        published->times.clear_with_capacity();
        published->times.append(times.data(), times.size());
        published->linear_points.clear_with_capacity();
        published->linear_points.append(linear_points.data(), linear_points.size());
        published->effect_generations.clear_with_capacity();
        published->effect_generations.append(effect_generations.data(), effect_generations.size());
    }

    impl.published_timing_rows.remove_all_matching([&](auto const& published) {
        if (slots_with_rows.contains_slow(published.slot))
            return false;
        CSS::record_element_animation_timing_rows(*element, published.slot, {}, {}, {});
        CSS::record_element_animation_effect_descriptions(*element, published.slot, {});
        return true;
    });
    impl.published_timing_rows_are_stale = false;
}

void Animatable::note_animation_timing_rows_identity_changed()
{
    if (m_impl)
        m_impl->published_timing_rows_are_stale = true;
}

Animatable::AnimationTimingRowCounters Animatable::animation_timing_row_counters()
{
    return s_animation_timing_row_counters;
}

Animatable::Impl& Animatable::ensure_impl() const
{
    if (!m_impl)
        m_impl = make<Impl>();
    return *m_impl;
}

static Optional<size_t> transition_index_for_pseudo_element(Optional<CSS::PseudoElement> pseudo_element)
{
    if (!pseudo_element.has_value())
        return 0;
    if (!CSS::Selector::PseudoElementSelector::is_known_pseudo_element_type(pseudo_element.value()))
        return {};
    return to_underlying(pseudo_element.value()) + 1;
}

Animatable::Transition* Animatable::ensure_transition(Optional<CSS::PseudoElement> pseudo_element) const
{
    auto pseudo_element_index = transition_index_for_pseudo_element(pseudo_element);
    if (!pseudo_element_index.has_value())
        return nullptr;

    auto& impl = ensure_impl();
    if (!impl.transitions[*pseudo_element_index])
        impl.transitions[*pseudo_element_index] = make<Transition>();
    return impl.transitions[*pseudo_element_index];
}

Animatable::Transition const* Animatable::transition_if_exists(Optional<CSS::PseudoElement> pseudo_element) const
{
    if (!m_impl)
        return nullptr;
    auto pseudo_element_index = transition_index_for_pseudo_element(pseudo_element);
    if (!pseudo_element_index.has_value())
        return nullptr;
    return m_impl->transitions[*pseudo_element_index];
}

}
