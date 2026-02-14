/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2023-2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/NonnullRefPtr.h>
#include <AK/RefCounted.h>
#include <LibGfx/Font/Font.h>
#include <LibGfx/FontCascadeList.h>
#include <LibGfx/Forward.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/EasingFunction.h>
#include <LibWeb/CSS/FontFeatureData.h>
#include <LibWeb/CSS/LengthBox.h>
#include <LibWeb/CSS/PropertyID.h>
#include <LibWeb/CSS/StyleProperty.h>
#include <LibWeb/Export.h>

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

class WEB_API ComputedProperties final : public RefCounted<ComputedProperties> {
public:
    ComputedProperties();
    ~ComputedProperties();

    template<typename Callback>
    inline void for_each_property(Callback callback) const
    {
        for (size_t i = 0; i < m_property_values.size(); ++i) {
            if (m_property_values[i])
                callback(static_cast<PropertyID>(i + to_underlying(first_longhand_property_id)), *m_property_values[i]);
        }
    }

    enum class Inherited {
        No,
        Yes
    };

    void set_external_animated_data(AnimatedPropertyData* data) { m_external_animated_data = data; }

    AnimatedPropertyData const& animated_property_data() const { return m_external_animated_data ? *m_external_animated_data : m_animated_properties; }
    AnimatedPropertyData& mutable_animated_property_data() { return m_external_animated_data ? *m_external_animated_data : m_animated_properties; }
    HashMap<PropertyID, NonnullRefPtr<StyleValue const>> const& animated_property_values() const { return animated_property_data().values; }

    void clear_computed_font_list_cache()
    {
        m_cached_computed_font_list = nullptr;
        m_cached_first_available_computed_font = nullptr;
    }
    bool is_property_important(PropertyID property_id) const;
    bool is_property_inherited(PropertyID property_id) const;
    void set_property_important(PropertyID, Important);
    void set_property_inherited(PropertyID, Inherited);

    void set_property(PropertyID, NonnullRefPtr<StyleValue const> value, Inherited = Inherited::No, Important = Important::No);
    void set_property_without_modifying_flags(PropertyID, NonnullRefPtr<StyleValue const> value);
    enum class WithAnimationsApplied {
        No,
        Yes,
    };
    StyleValue const& property(PropertyID, WithAnimationsApplied = WithAnimationsApplied::Yes) const;
    void revert_property(PropertyID, ComputedProperties const& style_for_revert);

    Size size_value(PropertyID) const;
    [[nodiscard]] Variant<LengthPercentage, NormalGap> gap_value(PropertyID) const;
    Length length(PropertyID) const;
    LengthBox length_box(PropertyID left_id, PropertyID top_id, PropertyID right_id, PropertyID bottom_id, LengthPercentageOrAuto const& default_value) const;
    Color color_or_fallback(PropertyID, ColorResolutionContext, Color fallback) const;
    HashMap<PropertyID, StyleValueVector> assemble_coordinated_value_list(PropertyID base_property_id, Vector<PropertyID> const& property_ids) const;
    ColorInterpolation color_interpolation() const;
    PreferredColorScheme color_scheme(PreferredColorScheme, Optional<Vector<String> const&> document_supported_schemes) const;
    TextAnchor text_anchor() const;
    TextAlign text_align() const;
    TextJustify text_justify() const;
    TextOverflow text_overflow() const;
    TextRendering text_rendering() const;
    CSSPixels text_underline_offset() const;
    TextUnderlinePosition text_underline_position() const;
    Vector<BackgroundLayerData> background_layers() const;
    BackgroundBox background_color_clip() const;
    Length border_spacing_horizontal(CalculationResolutionContext) const;
    Length border_spacing_vertical(CalculationResolutionContext) const;
    CaptionSide caption_side() const;
    Clip clip() const;
    Display display() const;
    Float float_() const;
    Color caret_color(ColorResolutionContext, Color current_color) const;
    Clear clear() const;
    ColumnSpan column_span() const;
    struct ContentDataAndQuoteNestingLevel {
        ContentData content_data;
        u32 final_quote_nesting_level { 0 };
    };
    ContentDataAndQuoteNestingLevel content(DOM::AbstractElement&, u32 initial_quote_nesting_level) const;
    ContentVisibility content_visibility() const;
    Vector<CursorData> cursor() const;
    Variant<Length, double> tab_size() const;
    WhiteSpaceCollapse white_space_collapse() const;
    WhiteSpaceTrimData white_space_trim() const;
    WordBreak word_break() const;
    CSSPixels word_spacing() const;
    CSSPixels letter_spacing() const;
    LineStyle line_style(PropertyID) const;
    OutlineStyle outline_style() const;
    Vector<TextDecorationLine> text_decoration_line() const;
    TextDecorationStyle text_decoration_style() const;
    TextDecorationThickness text_decoration_thickness() const;
    TextTransform text_transform() const;
    Vector<ShadowData> text_shadow(ColorResolutionContext, CalculationResolutionContext) const;
    TextIndentData text_indent() const;
    TextWrapMode text_wrap_mode() const;
    ListStyleType list_style_type() const;
    ListStylePosition list_style_position() const;
    FlexDirection flex_direction() const;
    FlexWrap flex_wrap() const;
    FlexBasis flex_basis() const;
    float flex_grow() const;
    float flex_shrink() const;
    int order() const;
    Optional<Color> accent_color(ColorResolutionContext) const;
    AlignContent align_content() const;
    AlignItems align_items() const;
    AlignSelf align_self() const;
    Appearance appearance() const;
    Filter backdrop_filter() const;
    Filter filter() const;
    float opacity() const;
    Visibility visibility() const;
    ImageRendering image_rendering() const;
    JustifyContent justify_content() const;
    JustifyItems justify_items() const;
    JustifySelf justify_self() const;
    Overflow overflow_x() const;
    Overflow overflow_y() const;
    Vector<ShadowData> box_shadow(ColorResolutionContext, CalculationResolutionContext) const;
    BoxSizing box_sizing() const;
    PointerEvents pointer_events() const;
    Variant<VerticalAlign, LengthPercentage> vertical_align() const;
    FontFeatureData font_feature_data() const;
    Optional<Gfx::FontVariantAlternates> font_variant_alternates() const;
    FontVariantCaps font_variant_caps() const;
    Optional<Gfx::FontVariantEastAsian> font_variant_east_asian() const;
    FontVariantEmoji font_variant_emoji() const;
    Optional<Gfx::FontVariantLigatures> font_variant_ligatures() const;
    Optional<Gfx::FontVariantNumeric> font_variant_numeric() const;
    FontVariantPosition font_variant_position() const;
    FontKerning font_kerning() const;
    Optional<FlyString> font_language_override() const;
    HashMap<FlyString, u8> font_feature_settings() const;
    HashMap<FlyString, double> font_variation_settings() const;
    GridTrackSizeList grid_auto_columns() const;
    GridTrackSizeList grid_auto_rows() const;
    GridTrackSizeList grid_template_columns() const;
    GridTrackSizeList grid_template_rows() const;
    [[nodiscard]] GridAutoFlow grid_auto_flow() const;
    GridTrackPlacement grid_column_end() const;
    GridTrackPlacement grid_column_start() const;
    GridTrackPlacement grid_row_end() const;
    GridTrackPlacement grid_row_start() const;
    BorderCollapse border_collapse() const;
    CSS::EmptyCells empty_cells() const;
    Vector<Vector<String>> grid_template_areas() const;
    ObjectFit object_fit() const;
    Position object_position() const;
    TableLayout table_layout() const;
    Direction direction() const;
    UnicodeBidi unicode_bidi() const;
    WritingMode writing_mode() const;
    UserSelect user_select() const;
    Isolation isolation() const;
    TouchActionData touch_action() const;
    Containment contain() const;
    ContainerType container_type() const;
    MixBlendMode mix_blend_mode() const;
    Optional<FlyString> view_transition_name() const;
    struct AnimationProperties {
        Variant<double, String> duration;
        EasingFunction timing_function;
        double iteration_count;
        AnimationDirection direction;
        AnimationPlayState play_state;
        double delay;
        AnimationFillMode fill_mode;
        AnimationComposition composition;
        FlyString name;
        GC::Ptr<Animations::AnimationTimeline> timeline;
    };
    Vector<AnimationProperties> animations(DOM::AbstractElement const&) const;
    Vector<TransitionProperties> transitions() const;

    Display display_before_box_type_transformation() const;
    void set_display_before_box_type_transformation(Display value);

    static Vector<NonnullRefPtr<TransformationStyleValue const>> transformations_for_style_value(StyleValue const& value);
    Vector<NonnullRefPtr<TransformationStyleValue const>> transformations() const;
    TransformBox transform_box() const;
    TransformOrigin transform_origin() const;
    TransformStyle transform_style() const;
    RefPtr<TransformationStyleValue const> rotate() const;
    RefPtr<TransformationStyleValue const> translate() const;
    RefPtr<TransformationStyleValue const> scale() const;
    Optional<CSSPixels> perspective() const;
    Position perspective_origin() const;

    MaskType mask_type() const;
    float stop_opacity() const;
    float fill_opacity() const;
    Vector<Variant<LengthPercentage, float>> stroke_dasharray() const;
    StrokeLinecap stroke_linecap() const;
    StrokeLinejoin stroke_linejoin() const;
    double stroke_miterlimit() const;
    float stroke_opacity() const;
    FillRule fill_rule() const;
    ClipRule clip_rule() const;
    float flood_opacity() const;
    CSS::ShapeRendering shape_rendering() const;
    PaintOrderList paint_order() const;

    WillChange will_change() const;

    ValueComparingRefPtr<Gfx::FontCascadeList const> cached_computed_font_list() const { return m_cached_computed_font_list; }
    ValueComparingNonnullRefPtr<Gfx::FontCascadeList const> computed_font_list(FontComputer const&) const;
    ValueComparingNonnullRefPtr<Gfx::Font const> first_available_computed_font(FontComputer const&) const;

    MathStyle math_style() const;
    int math_depth() const;
    [[nodiscard]] CSSPixels line_height() const;
    [[nodiscard]] CSSPixels font_size() const;
    double font_weight() const;
    Percentage font_width() const;
    int font_slope() const;
    FontOpticalSizing font_optical_sizing() const;

    bool operator==(ComputedProperties const&) const;

    Positioning position() const;
    Optional<int> z_index() const;

    QuotesData quotes() const;
    Vector<CounterData> counter_data(PropertyID) const;

    ScrollbarColorData scrollbar_color(ColorResolutionContext) const;
    ScrollbarWidth scrollbar_width() const;
    Resize resize() const;

    static NonnullRefPtr<Gfx::Font const> font_fallback(bool monospace, bool bold, float point_size);

private:
    Overflow overflow(PropertyID) const;
    Vector<ShadowData> shadow(PropertyID, ColorResolutionContext, CalculationResolutionContext) const;
    Position position_value(PropertyID) const;

    Array<RefPtr<StyleValue const>, number_of_longhand_properties> m_property_values;
    Array<u8, ceil_div(number_of_longhand_properties, 8uz)> m_property_important {};
    Array<u8, ceil_div(number_of_longhand_properties, 8uz)> m_property_inherited {};
    AnimatedPropertyData m_animated_properties;
    AnimatedPropertyData* m_external_animated_data { nullptr };

    Display m_display_before_box_type_transformation { InitialValues::display() };

    RefPtr<Gfx::FontCascadeList const> m_cached_computed_font_list;
    RefPtr<Gfx::Font const> m_cached_first_available_computed_font;
};

}
