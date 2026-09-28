/*
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2024, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Badge.h>
#include <AK/FlyString.h>
#include <AK/HashMap.h>
#include <AK/Utf16FlyString.h>
#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/Bindings/Animatable.h>
#include <LibWeb/Export.h>

namespace Web::CSS {

class CSSTransition;

}

namespace Web::Animations {

// https://drafts.csswg.org/web-animations-1/#animatable
class WEB_API Animatable {
public:
    virtual ~Animatable() = default;

    enum class GetAnimationsSorted {
        No,
        Yes
    };

    struct GetAnimationsOptions {
        bool subtree { false };
        Optional<CSS::Selector::PseudoElementSelector> pseudo_element;
    };

    struct KeyframeAnimationOptions : public KeyframeEffect::Options {
        Utf16FlyString id;
        Optional<GC::Ptr<AnimationTimeline>> timeline;
    };

    WebIDL::ExceptionOr<GC::Ref<Animation>> animate(Vector<BaseKeyframe> keyframes, Variant<double, KeyframeAnimationOptions> const& options);
    WebIDL::ExceptionOr<GC::Ref<Animation>> animate(JS::Realm&, GC::Ptr<JS::Object> keyframes, Variant<double, Bindings::KeyframeAnimationOptions> const& options);
    WebIDL::ExceptionOr<Vector<GC::Ref<Animation>>> get_animations(GetAnimationsOptions const& options);
    WebIDL::ExceptionOr<Vector<GC::Ref<Animation>>> get_animations(Bindings::GetAnimationsOptions const& options);
    WebIDL::ExceptionOr<Vector<GC::Ref<Animation>>> get_animations_internal(GetAnimationsSorted sorted, GetAnimationsOptions const& options);
    ReadonlySpan<GC::Ref<Animation>> associated_animations_in_composite_order();
    ReadonlySpan<GC::Ref<Animation>> associated_animations_unordered() const { return m_impl ? m_impl->associated_animations.span() : ReadonlySpan<GC::Ref<Animation>> {}; }
    void invalidate_associated_animation_composite_order();
    bool has_relevant_animations() const;
    bool has_associated_animations() const;

    void associate_with_animation(GC::Ref<Animation>);
    void disassociate_with_animation(GC::Ref<Animation>);
    void on_document_changed(DOM::Document& old_document, DOM::Document& new_document);
    void cancel_css_animations_and_transitions();

    // The timing of every animation this element holds a keyframe effect for, which is what the
    // style stage decides relevance from. Everything a row is built from moves through
    // invalidate_animation_timing_rows(), which queues the element on its document, and the
    // document publishes the elements it took off that queue (Document::publish_dirty_animation_timing_rows()).
    void invalidate_animation_timing_rows();
    void publish_animation_timing_rows(Badge<DOM::Document>);
    // The style engine holds nothing this element published under an identity it no longer has.
    void note_animation_timing_rows_identity_changed();

    struct AnimationTimingRowCounters {
        u64 lists_published { 0 };
        u64 lists_unchanged { 0 };
        u64 rows_published { 0 };
    };
    static AnimationTimingRowCounters animation_timing_row_counters();

    bool has_css_defined_animations() const;
    bool has_css_animations_or_transitions() const;
    Vector<GC::Ref<CSS::CSSAnimation>> const* css_defined_animations(Optional<CSS::PseudoElement>);
    void set_css_defined_animations(Optional<CSS::PseudoElement>, Vector<GC::Ref<CSS::CSSAnimation>>&&);

    Vector<CSS::PropertyID> property_ids_with_matching_transition_property_entry(Optional<CSS::PseudoElement>) const;
    void set_transition(Optional<CSS::PseudoElement>, CSS::PropertyID, GC::Ref<CSS::CSSTransition>);
    void remove_transition(Optional<CSS::PseudoElement>, CSS::PropertyID);
    Vector<CSS::PropertyID> property_ids_with_existing_transitions(Optional<CSS::PseudoElement>) const;
    GC::Ptr<CSS::CSSTransition> property_transition(Optional<CSS::PseudoElement>, CSS::PropertyID) const;

protected:
    void visit_edges(JS::Cell::Visitor&);

private:
    void publish_css_defined_animations(size_t index);

    struct Transition;

    struct Impl {
        AK_ALLOC_WITH_KMALLOC;

        Vector<GC::Ref<Animation>> associated_animations;
        // What one animation list last published, as it was built before its rows were put in
        // composite order. The effects' descriptions are named by identity and generation, which
        // moves on everything a description is built from.
        struct PublishedTimingRows {
            u8 slot { 0 };
            Vector<u32> words;
            Vector<u64> times;
            Vector<u64> linear_points;
            Vector<u64> effect_generations;
        };
        // The animation lists the element last published timing rows for, so a list that empties
        // can be cleared without walking every pseudo-element's slot, and one built the same as
        // it was last published is not published again.
        Vector<PublishedTimingRows> published_timing_rows;
        // The element's style node changed since the lists above were published.
        bool published_timing_rows_are_stale { false };
        // The element is queued on its document to publish its timing rows.
        bool timing_rows_are_dirty { false };
        bool is_sorted_by_composite_order { true };
        bool has_css_defined_animations { false };

        mutable Array<OwnPtr<Vector<GC::Ref<CSS::CSSAnimation>>>, to_underlying(CSS::PseudoElement::KnownPseudoElementCount) + 1> css_defined_animations;
        mutable Array<OwnPtr<Transition>, to_underlying(CSS::PseudoElement::KnownPseudoElementCount) + 1> transitions;

        ~Impl();

        void visit_edges(JS::Cell::Visitor&);
    };
    Impl& ensure_impl() const;
    Transition* ensure_transition(Optional<CSS::PseudoElement>) const;
    Transition const* transition_if_exists(Optional<CSS::PseudoElement>) const;

    mutable OwnPtr<Impl> m_impl;
};

}
