/*
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/Animations/Animation.h>
#include <LibWeb/CSS/CSSAnimationProperties.h>
#include <LibWeb/CSS/EasingFunction.h>
#include <LibWeb/CSS/StyleValues/StyleValue.h>

namespace Web::CSS {

// https://www.w3.org/TR/css-animations-2/#cssanimation
class CSSAnimation : public Animations::Animation {
    WEB_WRAPPABLE(CSSAnimation, Animations::Animation);
    GC_DECLARE_ALLOCATOR(CSSAnimation);

public:
    static GC::Ref<CSSAnimation> create(HTML::EnvironmentSettingsObject&);

    Utf16FlyString const& animation_name() const { return m_animation_name; }
    void set_animation_name(Utf16FlyString const& animation_name) { m_animation_name = animation_name; }

    virtual Animations::AnimationClass animation_class() const override;
    virtual int class_specific_composite_order(GC::Ref<Animations::Animation> other) const override;
    virtual u32 class_specific_composite_order_key() const override { return static_cast<u32>(m_animation_name_index); }

    void apply_css_properties(AnimationProperties const&, DOM::AbstractElement timeline_target);

    void set_animation_name_index(size_t index);

    EasingFunction const& default_easing() const { return m_default_easing; }

    virtual void set_timeline_for_bindings(GC::Ptr<Animations::AnimationTimeline> timeline) override;

    virtual WebIDL::ExceptionOr<void> set_start_time_for_bindings(Animations::NullableCSSNumberish const&) override;
    virtual WebIDL::ExceptionOr<void> set_current_time_for_bindings(Animations::NullableCSSNumberish const&) override;
    virtual WebIDL::ExceptionOr<void> set_playback_rate(double) override;
    virtual void cancel(Animations::Animation::ShouldInvalidate = Animations::Animation::ShouldInvalidate::Yes) override;
    virtual WebIDL::ExceptionOr<void> play(Animations::Animation::ShouldInvalidate = Animations::Animation::ShouldInvalidate::Yes) override;
    virtual WebIDL::ExceptionOr<void> pause() override;
    virtual WebIDL::ExceptionOr<void> update_playback_rate(double) override;
    virtual WebIDL::ExceptionOr<void> reverse() override;

    void play_from_css();
    void pause_from_css();

    Optional<CSS::AnimationPlayState> last_css_animation_play_state() const { return m_last_css_animation_play_state; }
    void set_last_css_animation_play_state(CSS::AnimationPlayState state) { m_last_css_animation_play_state = state; }
    bool script_overrode_play_state() const { return m_script_overrode_play_state; }

    // The definition the last plan applied, which is what the next plan's definition is compared
    // against to decide whether applying it would change anything at all.
    AppliedAnimationDefinitionRow applied_definition_row() const;

private:
    struct AppliedCSSProperties {
        Variant<double, Utf16String> duration;
        EasingFunction timing_function;
        double iteration_count;
        AnimationDirection direction;
        AnimationPlayState play_state;
        double delay;
        AnimationFillMode fill_mode;
        AnimationComposition composition;

        bool operator==(AppliedCSSProperties const&) const = default;
    };

    explicit CSSAnimation(HTML::EnvironmentSettingsObject&);

    virtual bool is_css_animation() const override { return true; }

    // https://drafts.csswg.org/css-animations-2/#dom-cssanimation-animationname
    Utf16FlyString m_animation_name;

    // https://drafts.csswg.org/css-animations-1/#animation-timing-function
    // The default per-keyframe easing, from the animation-timing-function property on the element.
    EasingFunction m_default_easing { EasingFunction::ease() };

    HashTable<CSS::PropertyID> m_ignored_css_properties;

    Optional<CSS::AnimationPlayState> m_last_css_animation_play_state;
    Optional<AppliedCSSProperties> m_applied_css_properties;

    // What the style computation last computed for this animation, kept as computed values rather
    // than as the easings and enums `m_applied_css_properties` holds, so that the computation can
    // compare a freshly computed definition against it without the host deciding anything.
    // NB: Recorded on every apply, including the one that finds nothing to do, so it always
    //     describes what CSS last said rather than what last changed.
    AppliedAnimationDefinitionRow m_applied_definition_row;
    RustStyleValueHandle m_applied_timing_function_value;
    bool m_has_applied_definition_row { false };

    bool m_script_overrode_play_state { false };
    bool m_applying_css_play_state { false };

    size_t m_animation_name_index { 0 };

    void mark_script_play_state_override();
};

}
