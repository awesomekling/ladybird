/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/PropertyID.h>
#include <LibWeb/CSS/StyleValueFromComputedValues.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/DisplayStyleValue.h>
#include <LibWeb/CSS/StyleValues/EdgeStyleValue.h>
#include <LibWeb/CSS/StyleValues/FilterValueListStyleValue.h>
#include <LibWeb/CSS/StyleValues/FitContentStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridAutoFlowStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackPlacementStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackSizeListStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/PositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/RatioStyleValue.h>
#include <LibWeb/CSS/StyleValues/RectStyleValue.h>
#include <LibWeb/CSS/StyleValues/ShadowStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/TimeStyleValue.h>
#include <LibWeb/CSS/StyleValues/TransformationStyleValue.h>
#include <LibWeb/CSS/StyleValues/URLStyleValue.h>

namespace Web::CSS {

static NonnullRefPtr<StyleValue const> style_value_for_length_percentage(LengthPercentage const& length_percentage)
{
    if (length_percentage.is_percentage())
        return PercentageStyleValue::create(length_percentage.percentage());
    if (length_percentage.is_length())
        return LengthStyleValue::create(length_percentage.length());
    return length_percentage.calculated();
}

static NonnullRefPtr<StyleValue const> style_value_for_length_percentage_or_auto(LengthPercentageOrAuto const& value)
{
    if (value.is_auto())
        return KeywordStyleValue::create(Keyword::Auto);
    if (value.is_percentage())
        return PercentageStyleValue::create(value.percentage());
    if (value.is_length())
        return LengthStyleValue::create(value.length());
    return value.calculated();
}

static NonnullRefPtr<StyleValue const> style_value_for_size(Size const& size)
{
    if (size.is_none())
        return KeywordStyleValue::create(Keyword::None);
    if (size.is_percentage())
        return PercentageStyleValue::create(size.percentage());
    if (size.is_length())
        return LengthStyleValue::create(size.length());
    if (size.is_auto())
        return KeywordStyleValue::create(Keyword::Auto);
    if (size.is_calculated())
        return size.calculated();
    if (size.is_min_content())
        return KeywordStyleValue::create(Keyword::MinContent);
    if (size.is_max_content())
        return KeywordStyleValue::create(Keyword::MaxContent);
    if (size.is_fit_content()) {
        if (auto available_space = size.fit_content_available_space(); available_space.has_value())
            return FitContentStyleValue::create(available_space.release_value());
        return FitContentStyleValue::create();
    }
    VERIFY_NOT_REACHED();
}

static NonnullRefPtr<StyleValue const> style_value_for_color(Color color)
{
    return ColorStyleValue::create_from_color(color, ColorSyntax::Modern);
}

static RefPtr<StyleValue const> style_value_for_shadow(ShadowStyleValue::ShadowType shadow_type, Vector<ShadowData> const& shadow_data)
{
    if (shadow_data.is_empty())
        return KeywordStyleValue::create(Keyword::None);

    auto make_shadow_style_value = [shadow_type](ShadowData const& shadow) {
        return ShadowStyleValue::create(
            shadow_type,
            style_value_for_color(shadow.color),
            style_value_for_length_percentage(shadow.offset_x),
            style_value_for_length_percentage(shadow.offset_y),
            style_value_for_length_percentage(shadow.blur_radius),
            style_value_for_length_percentage(shadow.spread_distance),
            shadow.placement);
    };

    if (shadow_data.size() == 1)
        return make_shadow_style_value(shadow_data.first());

    StyleValueVector style_values;
    style_values.ensure_capacity(shadow_data.size());
    for (auto const& shadow : shadow_data)
        style_values.unchecked_append(make_shadow_style_value(shadow));
    return StyleValueList::create(move(style_values), StyleValueList::Separator::Comma);
}

static NonnullRefPtr<StyleValue const> style_value_for_gap(Variant<LengthPercentage, NormalGap> const& gap)
{
    return gap.visit(
        [](LengthPercentage const& length_percentage) -> NonnullRefPtr<StyleValue const> {
            return style_value_for_length_percentage(length_percentage);
        },
        [](NormalGap) -> NonnullRefPtr<StyleValue const> {
            return KeywordStyleValue::create(Keyword::Normal);
        });
}

static NonnullRefPtr<StyleValue const> style_value_for_border_radius(BorderRadiusData const& data)
{
    return BorderRadiusStyleValue::create(
        style_value_for_length_percentage(data.horizontal_radius),
        style_value_for_length_percentage(data.vertical_radius));
}

RefPtr<StyleValue const> style_value_for_property(PropertyID property_id, ComputedValues const& computed_values)
{
    switch (property_id) {

    // ========== Simple keyword/enum properties ==========
    case PropertyID::AlignContent:
        return KeywordStyleValue::create(to_keyword(computed_values.align_content()));
    case PropertyID::AlignItems:
        return KeywordStyleValue::create(to_keyword(computed_values.align_items()));
    case PropertyID::AlignSelf:
        return KeywordStyleValue::create(to_keyword(computed_values.align_self()));
    case PropertyID::Appearance:
        return KeywordStyleValue::create(to_keyword(computed_values.appearance()));
    case PropertyID::BackgroundClip:
        return KeywordStyleValue::create(to_keyword(computed_values.background_color_clip()));
    case PropertyID::BorderCollapse:
        return KeywordStyleValue::create(to_keyword(computed_values.border_collapse()));
    case PropertyID::BoxSizing:
        return KeywordStyleValue::create(to_keyword(computed_values.box_sizing()));
    case PropertyID::CaptionSide:
        return KeywordStyleValue::create(to_keyword(computed_values.caption_side()));
    case PropertyID::Clear:
        return KeywordStyleValue::create(to_keyword(computed_values.clear()));
    case PropertyID::ClipRule:
        return KeywordStyleValue::create(to_keyword(computed_values.clip_rule()));
    case PropertyID::ColorInterpolation:
        return KeywordStyleValue::create(to_keyword(computed_values.color_interpolation()));
    case PropertyID::ColumnSpan:
        return KeywordStyleValue::create(to_keyword(computed_values.column_span()));
    case PropertyID::ContentVisibility:
        return KeywordStyleValue::create(to_keyword(computed_values.content_visibility()));
    case PropertyID::Direction:
        return KeywordStyleValue::create(to_keyword(computed_values.direction()));
    case PropertyID::EmptyCells:
        return KeywordStyleValue::create(to_keyword(computed_values.empty_cells()));
    case PropertyID::FillRule:
        return KeywordStyleValue::create(to_keyword(computed_values.fill_rule()));
    case PropertyID::FlexDirection:
        return KeywordStyleValue::create(to_keyword(computed_values.flex_direction()));
    case PropertyID::FlexWrap:
        return KeywordStyleValue::create(to_keyword(computed_values.flex_wrap()));
    case PropertyID::Float:
        return KeywordStyleValue::create(to_keyword(computed_values.float_()));
    // NB: ImageRendering is not handled here because the legacy keyword aliases
    //     (optimizequality -> smooth, optimizespeed -> pixelated) cause a different
    //     serialization when round-tripping through the enum.
    case PropertyID::Isolation:
        return KeywordStyleValue::create(to_keyword(computed_values.isolation()));
    case PropertyID::JustifyContent:
        return KeywordStyleValue::create(to_keyword(computed_values.justify_content()));
    case PropertyID::JustifyItems:
        return KeywordStyleValue::create(to_keyword(computed_values.justify_items()));
    case PropertyID::JustifySelf:
        return KeywordStyleValue::create(to_keyword(computed_values.justify_self()));
    case PropertyID::ListStylePosition:
        return KeywordStyleValue::create(to_keyword(computed_values.list_style_position()));
    case PropertyID::MaskType:
        return KeywordStyleValue::create(to_keyword(computed_values.mask_type()));
    case PropertyID::MathShift:
        return KeywordStyleValue::create(to_keyword(computed_values.math_shift()));
    case PropertyID::MathStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.math_style()));
    case PropertyID::MixBlendMode:
        return KeywordStyleValue::create(to_keyword(computed_values.mix_blend_mode()));
    case PropertyID::ObjectFit:
        return KeywordStyleValue::create(to_keyword(computed_values.object_fit()));
    case PropertyID::OutlineStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.outline_style()));
    case PropertyID::OverflowX:
        return KeywordStyleValue::create(to_keyword(computed_values.overflow_x()));
    case PropertyID::OverflowY:
        return KeywordStyleValue::create(to_keyword(computed_values.overflow_y()));
    case PropertyID::PointerEvents:
        return KeywordStyleValue::create(to_keyword(computed_values.pointer_events()));
    case PropertyID::Position:
        return KeywordStyleValue::create(to_keyword(computed_values.position()));
    case PropertyID::Resize:
        return KeywordStyleValue::create(to_keyword(computed_values.resize()));
    case PropertyID::ScrollbarWidth:
        return KeywordStyleValue::create(to_keyword(computed_values.scrollbar_width()));
    case PropertyID::ShapeRendering:
        return KeywordStyleValue::create(to_keyword(computed_values.shape_rendering()));
    case PropertyID::StrokeLinecap:
        return KeywordStyleValue::create(to_keyword(computed_values.stroke_linecap()));
    case PropertyID::StrokeLinejoin:
        return KeywordStyleValue::create(to_keyword(computed_values.stroke_linejoin()));
    case PropertyID::TableLayout:
        return KeywordStyleValue::create(to_keyword(computed_values.table_layout()));
    case PropertyID::TextAlign:
        return KeywordStyleValue::create(to_keyword(computed_values.text_align()));
    case PropertyID::TextAnchor:
        return KeywordStyleValue::create(to_keyword(computed_values.text_anchor()));
    case PropertyID::TextDecorationStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.text_decoration_style()));
    case PropertyID::TextJustify:
        return KeywordStyleValue::create(to_keyword(computed_values.text_justify()));
    case PropertyID::TextOverflow:
        return KeywordStyleValue::create(to_keyword(computed_values.text_overflow()));
    case PropertyID::TextTransform:
        return KeywordStyleValue::create(to_keyword(computed_values.text_transform()));
    case PropertyID::TextWrapMode:
        return KeywordStyleValue::create(to_keyword(computed_values.text_wrap_mode()));
    case PropertyID::TransformBox:
        return KeywordStyleValue::create(to_keyword(computed_values.transform_box()));
    case PropertyID::TransformStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.transform_style()));
    case PropertyID::UnicodeBidi:
        return KeywordStyleValue::create(to_keyword(computed_values.unicode_bidi()));
    case PropertyID::UserSelect:
        return KeywordStyleValue::create(to_keyword(computed_values.user_select()));
    case PropertyID::Visibility:
        return KeywordStyleValue::create(to_keyword(computed_values.visibility()));
    case PropertyID::WhiteSpaceCollapse:
        return KeywordStyleValue::create(to_keyword(computed_values.white_space_collapse()));
    case PropertyID::WordBreak:
        return KeywordStyleValue::create(to_keyword(computed_values.word_break()));
    case PropertyID::WritingMode:
        return KeywordStyleValue::create(to_keyword(computed_values.writing_mode()));

    // ========== Color properties ==========
    case PropertyID::AccentColor:
        if (auto accent = computed_values.accent_color(); accent.has_value())
            return style_value_for_color(accent.value());
        return KeywordStyleValue::create(Keyword::Auto);
    case PropertyID::BackgroundColor:
        return style_value_for_color(computed_values.background_color());
    case PropertyID::BorderBottomColor:
        return style_value_for_color(computed_values.border_bottom().color);
    case PropertyID::BorderLeftColor:
        return style_value_for_color(computed_values.border_left().color);
    case PropertyID::BorderRightColor:
        return style_value_for_color(computed_values.border_right().color);
    case PropertyID::BorderTopColor:
        return style_value_for_color(computed_values.border_top().color);
    case PropertyID::CaretColor:
        return style_value_for_color(computed_values.caret_color());
    case PropertyID::Color:
        return style_value_for_color(computed_values.color());
    case PropertyID::FloodColor:
        return style_value_for_color(computed_values.flood_color());
    case PropertyID::OutlineColor:
        return style_value_for_color(computed_values.outline_color());
    case PropertyID::StopColor:
        return style_value_for_color(computed_values.stop_color());
    case PropertyID::TextDecorationColor:
        return style_value_for_color(computed_values.text_decoration_color());
    case PropertyID::WebkitTextFillColor:
        return style_value_for_color(computed_values.webkit_text_fill_color());

    // ========== Float/number properties ==========
    case PropertyID::FillOpacity:
        return NumberStyleValue::create(computed_values.fill_opacity());
    case PropertyID::FlexGrow:
        return NumberStyleValue::create(computed_values.flex_grow());
    case PropertyID::FlexShrink:
        return NumberStyleValue::create(computed_values.flex_shrink());
    case PropertyID::FloodOpacity:
        return NumberStyleValue::create(computed_values.flood_opacity());
    case PropertyID::Opacity:
        return NumberStyleValue::create(computed_values.opacity());
    case PropertyID::StopOpacity:
        return NumberStyleValue::create(computed_values.stop_opacity());
    case PropertyID::StrokeOpacity:
        return NumberStyleValue::create(computed_values.stroke_opacity());
    case PropertyID::StrokeMiterlimit:
        return NumberStyleValue::create(computed_values.stroke_miterlimit());
    case PropertyID::FontWeight:
        return NumberStyleValue::create(computed_values.font_weight());

    // ========== Integer properties ==========
    case PropertyID::MathDepth:
        return IntegerStyleValue::create(computed_values.math_depth());
    case PropertyID::Order:
        return IntegerStyleValue::create(computed_values.order());

    // ========== CSSPixels → Length properties ==========
    case PropertyID::FontSize:
        return LengthStyleValue::create(Length::make_px(computed_values.font_size()));
    // NB: LetterSpacing is not handled here because percentage values are lost
    //     when resolved to CSSPixels. Fall through to ComputedProperties.
    case PropertyID::LineHeight:
        if (computed_values.line_height() == 0)
            return KeywordStyleValue::create(Keyword::Normal);
        return LengthStyleValue::create(Length::make_px(computed_values.line_height()));
    case PropertyID::OutlineWidth:
        return LengthStyleValue::create(Length::make_px(computed_values.outline_width()));
    case PropertyID::TextUnderlineOffset:
        if (auto offset = computed_values.text_underline_offset(); offset.has_value())
            return LengthStyleValue::create(Length::make_px(offset.value()));
        return KeywordStyleValue::create(Keyword::Auto);
    case PropertyID::WordSpacing:
        return LengthStyleValue::create(Length::make_px(computed_values.word_spacing()));

    // ========== Length properties ==========
    case PropertyID::BorderSpacing: {
        auto horizontal = LengthStyleValue::create(computed_values.border_spacing_horizontal());
        auto vertical = LengthStyleValue::create(computed_values.border_spacing_vertical());
        return StyleValueList::create(StyleValueVector { move(horizontal), move(vertical) }, StyleValueList::Separator::Space);
    }
    case PropertyID::OutlineOffset:
        return LengthStyleValue::create(computed_values.outline_offset());

    // ========== Size properties ==========
    case PropertyID::ColumnHeight:
        return style_value_for_size(computed_values.column_height());
    case PropertyID::ColumnWidth:
        return style_value_for_size(computed_values.column_width());
    case PropertyID::Height:
        return style_value_for_size(computed_values.height());
    case PropertyID::MaxHeight:
        return style_value_for_size(computed_values.max_height());
    case PropertyID::MaxWidth:
        return style_value_for_size(computed_values.max_width());
    case PropertyID::MinHeight:
        return style_value_for_size(computed_values.min_height());
    case PropertyID::MinWidth:
        return style_value_for_size(computed_values.min_width());
    case PropertyID::Width:
        return style_value_for_size(computed_values.width());

    // ========== LengthPercentage properties ==========
    case PropertyID::Cx:
        return style_value_for_length_percentage(computed_values.cx());
    case PropertyID::Cy:
        return style_value_for_length_percentage(computed_values.cy());
    case PropertyID::R:
        return style_value_for_length_percentage(computed_values.r());
    case PropertyID::StrokeDashoffset:
        return style_value_for_length_percentage(computed_values.stroke_dashoffset());
    case PropertyID::StrokeWidth:
        return style_value_for_length_percentage(computed_values.stroke_width());
    case PropertyID::X:
        return style_value_for_length_percentage(computed_values.x());
    case PropertyID::Y:
        return style_value_for_length_percentage(computed_values.y());

    // ========== LengthPercentageOrAuto properties ==========
    // FIXME: Rx causes test regression in all-prop-revert-layer; needs investigation.
    //        Fall through to ComputedProperties for now.
    case PropertyID::Ry:
        return style_value_for_length_percentage_or_auto(computed_values.ry());

    // ========== Inset (LengthBox) ==========
    case PropertyID::Bottom:
        return style_value_for_length_percentage_or_auto(computed_values.inset().bottom());
    case PropertyID::Left:
        return style_value_for_length_percentage_or_auto(computed_values.inset().left());
    case PropertyID::Right:
        return style_value_for_length_percentage_or_auto(computed_values.inset().right());
    case PropertyID::Top:
        return style_value_for_length_percentage_or_auto(computed_values.inset().top());

    // ========== Margin (LengthBox) ==========
    case PropertyID::MarginBottom:
        return style_value_for_length_percentage_or_auto(computed_values.margin().bottom());
    case PropertyID::MarginLeft:
        return style_value_for_length_percentage_or_auto(computed_values.margin().left());
    case PropertyID::MarginRight:
        return style_value_for_length_percentage_or_auto(computed_values.margin().right());
    case PropertyID::MarginTop:
        return style_value_for_length_percentage_or_auto(computed_values.margin().top());

    // ========== Padding (LengthBox) ==========
    case PropertyID::PaddingBottom:
        return style_value_for_length_percentage_or_auto(computed_values.padding().bottom());
    case PropertyID::PaddingLeft:
        return style_value_for_length_percentage_or_auto(computed_values.padding().left());
    case PropertyID::PaddingRight:
        return style_value_for_length_percentage_or_auto(computed_values.padding().right());
    case PropertyID::PaddingTop:
        return style_value_for_length_percentage_or_auto(computed_values.padding().top());

    // ========== Border styles ==========
    case PropertyID::BorderBottomStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.border_bottom().line_style));
    case PropertyID::BorderLeftStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.border_left().line_style));
    case PropertyID::BorderRightStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.border_right().line_style));
    case PropertyID::BorderTopStyle:
        return KeywordStyleValue::create(to_keyword(computed_values.border_top().line_style));

    // ========== Border widths ==========
    case PropertyID::BorderBottomWidth:
        return LengthStyleValue::create(Length::make_px(computed_values.border_bottom().width));
    case PropertyID::BorderLeftWidth:
        return LengthStyleValue::create(Length::make_px(computed_values.border_left().width));
    case PropertyID::BorderRightWidth:
        return LengthStyleValue::create(Length::make_px(computed_values.border_right().width));
    case PropertyID::BorderTopWidth:
        return LengthStyleValue::create(Length::make_px(computed_values.border_top().width));

    // ========== Border radii ==========
    case PropertyID::BorderBottomLeftRadius:
        return style_value_for_border_radius(computed_values.border_bottom_left_radius());
    case PropertyID::BorderBottomRightRadius:
        return style_value_for_border_radius(computed_values.border_bottom_right_radius());
    case PropertyID::BorderTopLeftRadius:
        return style_value_for_border_radius(computed_values.border_top_left_radius());
    case PropertyID::BorderTopRightRadius:
        return style_value_for_border_radius(computed_values.border_top_right_radius());

    // ========== Shadow properties ==========
    case PropertyID::BoxShadow:
        return style_value_for_shadow(ShadowStyleValue::ShadowType::Normal, computed_values.box_shadow());
    case PropertyID::TextShadow:
        return style_value_for_shadow(ShadowStyleValue::ShadowType::Text, computed_values.text_shadow());

    // ========== Gap properties ==========
    case PropertyID::ColumnGap:
        return style_value_for_gap(computed_values.column_gap());
    case PropertyID::RowGap:
        return style_value_for_gap(computed_values.row_gap());

    // ========== Display ==========
    case PropertyID::Display:
        return DisplayStyleValue::create(computed_values.display());

    // ========== Filter properties ==========
    case PropertyID::BackdropFilter:
        if (computed_values.backdrop_filter().is_none())
            return KeywordStyleValue::create(Keyword::None);
        return FilterValueListStyleValue::create(Vector<FilterValue>(computed_values.backdrop_filter().filters()));
    case PropertyID::Filter:
        if (computed_values.filter().is_none())
            return KeywordStyleValue::create(Keyword::None);
        return FilterValueListStyleValue::create(Vector<FilterValue>(computed_values.filter().filters()));

    // ========== Grid properties ==========
    // NB: GridAutoColumns and GridAutoRows are not handled here because the
    //     created values don't match their initial values via equals(),
    //     breaking grid shorthand serialization. Fall through to ComputedProperties.
    case PropertyID::GridColumnEnd:
        return GridTrackPlacementStyleValue::create(computed_values.grid_column_end());
    case PropertyID::GridColumnStart:
        return GridTrackPlacementStyleValue::create(computed_values.grid_column_start());
    case PropertyID::GridRowEnd:
        return GridTrackPlacementStyleValue::create(computed_values.grid_row_end());
    case PropertyID::GridRowStart:
        return GridTrackPlacementStyleValue::create(computed_values.grid_row_start());
    // NB: GridTemplateColumns, GridTemplateRows, GridTemplateAreas,
    //     GridAutoFlow, GridAutoColumns, and GridAutoRows are not handled here
    //     because the created values don't match their initial values via
    //     equals(), breaking grid shorthand serialization.
    //     Fall through to ComputedProperties.

    // ========== Transform properties ==========
    case PropertyID::Transform: {
        auto const& transformations = computed_values.transformations();
        if (transformations.is_empty())
            return KeywordStyleValue::create(Keyword::None);
        StyleValueVector values;
        values.ensure_capacity(transformations.size());
        for (auto const& transformation : transformations)
            values.append(transformation);
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }
    case PropertyID::Rotate:
        if (auto const& rotate = computed_values.rotate())
            return *rotate;
        return KeywordStyleValue::create(Keyword::None);
    case PropertyID::Scale:
        if (auto const& scale = computed_values.scale())
            return *scale;
        return KeywordStyleValue::create(Keyword::None);
    case PropertyID::Translate:
        if (auto const& translate = computed_values.translate())
            return *translate;
        return KeywordStyleValue::create(Keyword::None);
    case PropertyID::TransformOrigin: {
        auto const& origin = computed_values.transform_origin();
        StyleValueVector values;
        values.append(style_value_for_length_percentage(origin.x));
        values.append(style_value_for_length_percentage(origin.y));
        values.append(style_value_for_length_percentage(origin.z));
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }
    case PropertyID::Perspective:
        if (auto const& perspective = computed_values.perspective(); perspective.has_value())
            return LengthStyleValue::create(Length::make_px(perspective.value()));
        return KeywordStyleValue::create(Keyword::None);
    case PropertyID::PerspectiveOrigin: {
        auto const& origin = computed_values.perspective_origin();
        return PositionStyleValue::create(
            EdgeStyleValue::create({}, style_value_for_length_percentage(origin.offset_x)),
            EdgeStyleValue::create({}, style_value_for_length_percentage(origin.offset_y)));
    }

    // ========== Object position ==========
    case PropertyID::ObjectPosition: {
        auto const& position = computed_values.object_position();
        return PositionStyleValue::create(
            EdgeStyleValue::create({}, style_value_for_length_percentage(position.offset_x)),
            EdgeStyleValue::create({}, style_value_for_length_percentage(position.offset_y)));
    }

    // ========== Vertical align ==========
    case PropertyID::VerticalAlign:
        return computed_values.vertical_align().visit(
            [](VerticalAlign align) -> NonnullRefPtr<StyleValue const> {
                return KeywordStyleValue::create(to_keyword(align));
            },
            [](LengthPercentage const& length_percentage) -> NonnullRefPtr<StyleValue const> {
                return style_value_for_length_percentage(length_percentage);
            });

    // ========== Flex basis ==========
    case PropertyID::FlexBasis:
        return computed_values.flex_basis().visit(
            [](FlexBasisContent) -> NonnullRefPtr<StyleValue const> {
                return KeywordStyleValue::create(Keyword::Content);
            },
            [](Size const& size) -> NonnullRefPtr<StyleValue const> {
                return style_value_for_size(size);
            });

    // ========== Z-index ==========
    case PropertyID::ZIndex:
        if (auto z_index = computed_values.z_index(); z_index.has_value())
            return IntegerStyleValue::create(z_index.value());
        return KeywordStyleValue::create(Keyword::Auto);

    case PropertyID::ColumnCount:
        if (computed_values.column_count().is_auto())
            return KeywordStyleValue::create(Keyword::Auto);
        return IntegerStyleValue::create(computed_values.column_count().value());

    // ========== Tab size ==========
    case PropertyID::TabSize:
        return computed_values.tab_size().visit(
            [](Length const& length) -> NonnullRefPtr<StyleValue const> {
                return LengthStyleValue::create(length);
            },
            [](double number) -> NonnullRefPtr<StyleValue const> {
                return NumberStyleValue::create(number);
            });

    // NB: TransitionDelay is not handled here because populate_computed_values()
    //     doesn't correctly handle the StyleValueList case for coordinating-list
    //     properties. Fall through to ComputedProperties.

    // ========== Aspect ratio ==========
    case PropertyID::AspectRatio: {
        auto const& aspect_ratio = computed_values.aspect_ratio();
        if (aspect_ratio.use_natural_aspect_ratio_if_available && !aspect_ratio.preferred_ratio.has_value())
            return KeywordStyleValue::create(Keyword::Auto);
        if (!aspect_ratio.use_natural_aspect_ratio_if_available && aspect_ratio.preferred_ratio.has_value()) {
            return RatioStyleValue::create(aspect_ratio.preferred_ratio.value());
        }
        if (aspect_ratio.use_natural_aspect_ratio_if_available && aspect_ratio.preferred_ratio.has_value()) {
            StyleValueVector values;
            values.append(KeywordStyleValue::create(Keyword::Auto));
            values.append(RatioStyleValue::create(aspect_ratio.preferred_ratio.value()));
            return StyleValueList::create(move(values), StyleValueList::Separator::Space);
        }
        return KeywordStyleValue::create(Keyword::Auto);
    }

    // ========== SVG paint properties ==========
    case PropertyID::Fill:
        if (auto const& fill = computed_values.fill(); fill.has_value()) {
            if (fill->is_color())
                return style_value_for_color(fill->as_color());
            if (fill->is_url())
                return URLStyleValue::create(fill->as_url());
        }
        return KeywordStyleValue::create(Keyword::None);
    case PropertyID::Stroke:
        if (auto const& stroke = computed_values.stroke(); stroke.has_value()) {
            if (stroke->is_color())
                return style_value_for_color(stroke->as_color());
            if (stroke->is_url())
                return URLStyleValue::create(stroke->as_url());
        }
        return KeywordStyleValue::create(Keyword::None);

    // ========== Stroke dasharray ==========
    case PropertyID::StrokeDasharray: {
        auto const& dasharray = computed_values.stroke_dasharray();
        if (dasharray.is_empty())
            return KeywordStyleValue::create(Keyword::None);
        StyleValueVector values;
        values.ensure_capacity(dasharray.size());
        for (auto const& entry : dasharray) {
            entry.visit(
                [&values](LengthPercentage const& length_percentage) {
                    values.append(style_value_for_length_percentage(length_percentage));
                },
                [&values](float number) {
                    values.append(NumberStyleValue::create(number));
                });
        }
        return StyleValueList::create(move(values), StyleValueList::Separator::Comma);
    }

    // NB: PaintOrder is not handled here because the conversion from
    //     [Fill, Stroke, Markers] to "normal" is lossy (paint-order: fill
    //     also resolves to the same array). Fall through to ComputedProperties.

    // ========== Text decoration line ==========
    case PropertyID::TextDecorationLine: {
        auto const& lines = computed_values.text_decoration_line();
        if (lines.is_empty() || (lines.size() == 1 && lines[0] == TextDecorationLine::None))
            return KeywordStyleValue::create(Keyword::None);
        StyleValueVector values;
        values.ensure_capacity(lines.size());
        for (auto line : lines)
            values.append(KeywordStyleValue::create(to_keyword(line)));
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }

    // ========== Text decoration thickness ==========
    case PropertyID::TextDecorationThickness:
        return computed_values.text_decoration_thickness().value.visit(
            [](TextDecorationThickness::Auto) -> NonnullRefPtr<StyleValue const> {
                return KeywordStyleValue::create(Keyword::Auto);
            },
            [](TextDecorationThickness::FromFont) -> NonnullRefPtr<StyleValue const> {
                return KeywordStyleValue::create(Keyword::FromFont);
            },
            [](LengthPercentage const& length_percentage) -> NonnullRefPtr<StyleValue const> {
                return style_value_for_length_percentage(length_percentage);
            });

    // ========== Text indent ==========
    case PropertyID::TextIndent: {
        auto const& indent = computed_values.text_indent();
        StyleValueVector values;
        values.append(style_value_for_length_percentage(indent.length_percentage));
        if (indent.each_line)
            values.append(KeywordStyleValue::create(Keyword::EachLine));
        if (indent.hanging)
            values.append(KeywordStyleValue::create(Keyword::Hanging));
        if (values.size() == 1)
            return values[0];
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }

    // ========== Text underline position ==========
    case PropertyID::TextUnderlinePosition: {
        auto const& position = computed_values.text_underline_position();
        StyleValueVector values;
        values.append(KeywordStyleValue::create(to_keyword(position.horizontal)));
        values.append(KeywordStyleValue::create(to_keyword(position.vertical)));
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }

    // ========== View transition name ==========
    case PropertyID::ViewTransitionName:
        if (auto name = computed_values.view_transition_name(); name.has_value())
            return CustomIdentStyleValue::create(name.release_value());
        return KeywordStyleValue::create(Keyword::None);

    // ========== Clip ==========
    case PropertyID::Clip: {
        auto const& clip = computed_values.clip();
        if (clip.is_auto())
            return KeywordStyleValue::create(Keyword::Auto);
        return RectStyleValue::create(clip.to_rect());
    }

    // ========== Contain ==========
    case PropertyID::Contain: {
        auto const& contain = computed_values.contain();
        if (contain.is_empty())
            return KeywordStyleValue::create(Keyword::None);
        if (contain.layout_containment && contain.style_containment && contain.paint_containment) {
            if (contain.size_containment)
                return KeywordStyleValue::create(Keyword::Strict);
            if (!contain.inline_size_containment)
                return KeywordStyleValue::create(Keyword::Content);
        }
        StyleValueVector values;
        if (contain.size_containment)
            values.append(KeywordStyleValue::create(Keyword::Size));
        else if (contain.inline_size_containment)
            values.append(KeywordStyleValue::create(Keyword::InlineSize));
        if (contain.layout_containment)
            values.append(KeywordStyleValue::create(Keyword::Layout));
        if (contain.style_containment)
            values.append(KeywordStyleValue::create(Keyword::Style));
        if (contain.paint_containment)
            values.append(KeywordStyleValue::create(Keyword::Paint));
        if (values.size() == 1)
            return values[0];
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }

    // ========== Container type ==========
    case PropertyID::ContainerType: {
        auto const& type = computed_values.container_type();
        if (type.is_empty())
            return KeywordStyleValue::create(Keyword::Normal);
        if (type.is_size_container)
            return KeywordStyleValue::create(Keyword::Size);
        if (type.is_inline_size_container)
            return KeywordStyleValue::create(Keyword::InlineSize);
        if (type.is_scroll_state_container)
            return KeywordStyleValue::create(Keyword::ScrollState);
        return KeywordStyleValue::create(Keyword::Normal);
    }

    // ========== List style type ==========
    case PropertyID::ListStyleType:
        return computed_values.list_style_type().visit(
            [&](Empty) -> NonnullRefPtr<StyleValue const> {
                return KeywordStyleValue::create(Keyword::None);
            },
            [](CounterStyleNameKeyword keyword) -> NonnullRefPtr<StyleValue const> {
                return KeywordStyleValue::create(to_keyword(keyword));
            },
            [](String const&) -> NonnullRefPtr<StyleValue const> {
                // FIXME: Serialize custom counter style string properly
                return KeywordStyleValue::create(Keyword::None);
            });

    // ========== Mask/clip path ==========
    // FIXME: MaskImage causes test regression in all-prop-revert-layer; needs investigation.
    //        Fall through to ComputedProperties for now.
    case PropertyID::ClipPath:
        if (auto const& clip_path = computed_values.clip_path(); clip_path.has_value()) {
            if (clip_path->is_url())
                return URLStyleValue::create(clip_path->url());
            if (clip_path->is_basic_shape())
                return clip_path->basic_shape();
        }
        return KeywordStyleValue::create(Keyword::None);

    // ========== Touch action ==========
    case PropertyID::TouchAction: {
        auto const& touch = computed_values.touch_action();
        if (!touch.allow_left && !touch.allow_right && !touch.allow_up && !touch.allow_down && !touch.allow_pinch_zoom && !touch.allow_other)
            return KeywordStyleValue::create(Keyword::None);
        if (touch.allow_left && touch.allow_right && touch.allow_up && touch.allow_down && touch.allow_pinch_zoom && touch.allow_other)
            return KeywordStyleValue::create(Keyword::Auto);
        if (touch.allow_left && touch.allow_right && touch.allow_up && touch.allow_down && !touch.allow_other)
            return KeywordStyleValue::create(Keyword::Manipulation);
        StyleValueVector values;
        if (touch.allow_left && touch.allow_right)
            values.append(KeywordStyleValue::create(Keyword::PanX));
        else if (touch.allow_left)
            values.append(KeywordStyleValue::create(Keyword::PanLeft));
        else if (touch.allow_right)
            values.append(KeywordStyleValue::create(Keyword::PanRight));
        if (touch.allow_up && touch.allow_down)
            values.append(KeywordStyleValue::create(Keyword::PanY));
        else if (touch.allow_up)
            values.append(KeywordStyleValue::create(Keyword::PanUp));
        else if (touch.allow_down)
            values.append(KeywordStyleValue::create(Keyword::PanDown));
        if (values.size() == 1)
            return values[0];
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }

    case PropertyID::ScrollbarColor:
        if (auto scrollbar = computed_values.scrollbar_color(); scrollbar.has_value()) {
            StyleValueVector values;
            values.append(style_value_for_color(scrollbar->thumb_color));
            values.append(style_value_for_color(scrollbar->track_color));
            return StyleValueList::create(move(values), StyleValueList::Separator::Space);
        }
        return KeywordStyleValue::create(Keyword::Auto);

    // ========== Cursor ==========
    case PropertyID::Cursor: {
        auto const& cursors = computed_values.cursor();
        if (cursors.size() == 1) {
            return cursors[0].visit(
                [](NonnullRefPtr<CursorStyleValue const> const& cursor_style_value) -> NonnullRefPtr<StyleValue const> {
                    return cursor_style_value;
                },
                [](CursorPredefined predefined) -> NonnullRefPtr<StyleValue const> {
                    return KeywordStyleValue::create(to_keyword(predefined));
                });
        }
        StyleValueVector values;
        values.ensure_capacity(cursors.size());
        for (auto const& cursor : cursors) {
            cursor.visit(
                [&values](NonnullRefPtr<CursorStyleValue const> const& cursor_style_value) {
                    values.append(cursor_style_value);
                },
                [&values](CursorPredefined predefined) {
                    values.append(KeywordStyleValue::create(to_keyword(predefined)));
                });
        }
        return StyleValueList::create(move(values), StyleValueList::Separator::Comma);
    }

    // ========== Quotes ==========
    case PropertyID::Quotes: {
        auto const& quotes = computed_values.quotes();
        if (quotes.type == QuotesData::Type::None)
            return KeywordStyleValue::create(Keyword::None);
        if (quotes.type == QuotesData::Type::Auto)
            return KeywordStyleValue::create(Keyword::Auto);
        // FIXME: Serialize specified quote strings properly
        return nullptr;
    }

    // NB: CounterIncrement, CounterReset, CounterSet, WhiteSpaceTrim,
    //     and ColorScheme fall through to the default case which reads
    //     from the supplementary property value map.

    // ========== Font properties (partially in ComputedValues) ==========
    case PropertyID::FontLanguageOverride:
        if (auto override = computed_values.font_language_override(); override.has_value())
            return CustomIdentStyleValue::create(override.release_value());
        return KeywordStyleValue::create(Keyword::Normal);

    default:
        return nullptr;
    }
}

}
