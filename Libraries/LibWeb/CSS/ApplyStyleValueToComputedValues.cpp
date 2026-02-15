/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ApplyStyleValueToComputedValues.h>
#include <LibWeb/CSS/Clip.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/Filter.h>
#include <LibWeb/CSS/LengthBox.h>
#include <LibWeb/CSS/PercentageOr.h>
#include <LibWeb/CSS/Size.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterDefinitionsStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/CursorStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/DisplayStyleValue.h>
#include <LibWeb/CSS/StyleValues/EdgeStyleValue.h>
#include <LibWeb/CSS/StyleValues/FilterValueListStyleValue.h>
#include <LibWeb/CSS/StyleValues/FitContentStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridAutoFlowStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTemplateAreaStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackPlacementStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackSizeListStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpenTypeTaggedStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/PositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/RatioStyleValue.h>
#include <LibWeb/CSS/StyleValues/RectStyleValue.h>
#include <LibWeb/CSS/StyleValues/ScrollbarColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/ShadowStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/TextIndentStyleValue.h>
#include <LibWeb/CSS/StyleValues/TextUnderlinePositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/TimeStyleValue.h>
#include <LibWeb/CSS/StyleValues/TransformationStyleValue.h>
#include <LibWeb/CSS/StyleValues/URLStyleValue.h>

namespace Web::CSS {

static Size size_value_from_style_value(StyleValue const& value)
{
    if (value.is_keyword()) {
        switch (value.to_keyword()) {
        case Keyword::Auto:
            return Size::make_auto();
        case Keyword::MinContent:
            return Size::make_min_content();
        case Keyword::MaxContent:
            return Size::make_max_content();
        case Keyword::None:
            return Size::make_none();
        default:
            VERIFY_NOT_REACHED();
        }
    }
    if (value.is_fit_content()) {
        auto& fit_content = value.as_fit_content();
        if (auto length_percentage = fit_content.length_percentage(); length_percentage.has_value())
            return Size::make_fit_content(length_percentage.release_value());
        return Size::make_fit_content();
    }
    if (value.is_calculated())
        return Size::make_calculated(value.as_calculated());
    if (value.is_percentage())
        return Size::make_percentage(value.as_percentage().percentage());
    if (value.is_length())
        return Size::make_length(value.as_length().length());
    return Size::make_auto();
}

static Variant<LengthPercentage, NormalGap> gap_value_from_style_value(StyleValue const& value)
{
    if (value.is_keyword()) {
        VERIFY(value.as_keyword().keyword() == Keyword::Normal);
        return NormalGap {};
    }
    return LengthPercentage::from_style_value(value);
}

static Vector<NonnullRefPtr<TransformationStyleValue const>> transformations_from_style_value(StyleValue const& value)
{
    if (value.is_keyword() && value.to_keyword() == Keyword::None)
        return {};
    if (!value.is_value_list())
        return {};
    auto& list = value.as_value_list();
    Vector<NonnullRefPtr<TransformationStyleValue const>> transformations;
    for (auto const& transform_value : list.values()) {
        VERIFY(transform_value->is_transformation());
        transformations.append(transform_value->as_transformation());
    }
    return transformations;
}

bool apply_style_value_to_computed_values(MutableComputedValues& computed_values, PropertyID property_id, StyleValue const& value)
{
    // NB: Animated values are fully resolved, concrete StyleValues. Colors are concrete
    // RGBColorStyleValues (from interpolate_color), numbers are NumberStyleValues, etc.
    // We can extract typed values without needing resolution contexts.

    switch (property_id) {

    // Float/number properties
    case PropertyID::Opacity:
        if (value.is_number()) {
            computed_values.set_opacity(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::FlexGrow:
        if (value.is_number()) {
            computed_values.set_flex_grow(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::FlexShrink:
        if (value.is_number()) {
            computed_values.set_flex_shrink(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::FillOpacity:
        if (value.is_number()) {
            computed_values.set_fill_opacity(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::StrokeOpacity:
        if (value.is_number()) {
            computed_values.set_stroke_opacity(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::FloodOpacity:
        if (value.is_number()) {
            computed_values.set_flood_opacity(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::StopOpacity:
        if (value.is_number()) {
            computed_values.set_stop_opacity(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::StrokeMiterlimit:
        if (value.is_number()) {
            computed_values.set_stroke_miterlimit(value.as_number().number());
            return true;
        }
        return false;

    // Color properties (animated colors are concrete RGBColorStyleValues)
    case PropertyID::Color:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::BackgroundColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_background_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::BorderLeftColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.border_left().color = color.value();
                return true;
            }
        }
        return false;
    case PropertyID::BorderRightColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.border_right().color = color.value();
                return true;
            }
        }
        return false;
    case PropertyID::BorderTopColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.border_top().color = color.value();
                return true;
            }
        }
        return false;
    case PropertyID::BorderBottomColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.border_bottom().color = color.value();
                return true;
            }
        }
        return false;
    case PropertyID::TextDecorationColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_text_decoration_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::OutlineColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_outline_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::WebkitTextFillColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_webkit_text_fill_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::FloodColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_flood_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::StopColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_stop_color(color.value());
                return true;
            }
        }
        return false;

    // Integer properties
    case PropertyID::ZIndex:
        if (value.has_auto()) {
            computed_values.set_z_index({});
            return true;
        }
        if (value.is_integer()) {
            computed_values.set_z_index(value.as_integer().integer());
            return true;
        }
        return false;
    case PropertyID::Order:
        if (value.is_integer()) {
            computed_values.set_order(value.as_integer().integer());
            return true;
        }
        return false;
    case PropertyID::ColumnCount:
        if (value.is_integer()) {
            computed_values.set_column_count(ColumnCount::make_integer(value.as_integer().integer()));
            return true;
        }
        if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_column_count(ColumnCount::make_auto());
            return true;
        }
        return false;

    // Keyword/enum properties
    case PropertyID::Visibility:
        if (auto visibility = keyword_to_visibility(value.to_keyword()); visibility.has_value()) {
            computed_values.set_visibility(visibility.value());
            return true;
        }
        return false;
    case PropertyID::Display:
        if (value.is_display()) {
            computed_values.set_display(value.as_display().display());
            return true;
        }
        return false;
    case PropertyID::Position:
        if (auto position = keyword_to_positioning(value.to_keyword()); position.has_value()) {
            computed_values.set_position(position.value());
            return true;
        }
        return false;
    case PropertyID::Float:
        if (auto float_val = keyword_to_float(value.to_keyword()); float_val.has_value()) {
            computed_values.set_float(float_val.value());
            return true;
        }
        return false;
    case PropertyID::Clear:
        if (auto clear = keyword_to_clear(value.to_keyword()); clear.has_value()) {
            computed_values.set_clear(clear.value());
            return true;
        }
        return false;
    case PropertyID::OverflowX:
        if (auto overflow = keyword_to_overflow(value.to_keyword()); overflow.has_value()) {
            computed_values.set_overflow_x(overflow.value());
            return true;
        }
        return false;
    case PropertyID::OverflowY:
        if (auto overflow = keyword_to_overflow(value.to_keyword()); overflow.has_value()) {
            computed_values.set_overflow_y(overflow.value());
            return true;
        }
        return false;

    // Size properties
    case PropertyID::Width:
        computed_values.set_width(size_value_from_style_value(value));
        return true;
    case PropertyID::Height:
        computed_values.set_height(size_value_from_style_value(value));
        return true;
    case PropertyID::MinWidth:
        computed_values.set_min_width(size_value_from_style_value(value));
        return true;
    case PropertyID::MinHeight:
        computed_values.set_min_height(size_value_from_style_value(value));
        return true;
    case PropertyID::MaxWidth:
        computed_values.set_max_width(size_value_from_style_value(value));
        return true;
    case PropertyID::MaxHeight:
        computed_values.set_max_height(size_value_from_style_value(value));
        return true;
    case PropertyID::ColumnWidth:
        computed_values.set_column_width(size_value_from_style_value(value));
        return true;
    case PropertyID::ColumnHeight:
        computed_values.set_column_height(size_value_from_style_value(value));
        return true;

    // Gap properties
    case PropertyID::ColumnGap:
        computed_values.set_column_gap(gap_value_from_style_value(value));
        return true;
    case PropertyID::RowGap:
        computed_values.set_row_gap(gap_value_from_style_value(value));
        return true;

    // LengthPercentage properties
    case PropertyID::Cx:
        computed_values.set_cx(LengthPercentage::from_style_value(value));
        return true;
    case PropertyID::Cy:
        computed_values.set_cy(LengthPercentage::from_style_value(value));
        return true;
    case PropertyID::R:
        computed_values.set_r(LengthPercentage::from_style_value(value));
        return true;
    case PropertyID::X:
        computed_values.set_x(LengthPercentage::from_style_value(value));
        return true;
    case PropertyID::Y:
        computed_values.set_y(LengthPercentage::from_style_value(value));
        return true;
    case PropertyID::Rx:
        computed_values.set_rx(LengthPercentageOrAuto::from_style_value(value));
        return true;
    case PropertyID::Ry:
        computed_values.set_ry(LengthPercentageOrAuto::from_style_value(value));
        return true;

    // Stroke width/dashoffset (number, length, or percentage)
    case PropertyID::StrokeWidth:
        if (value.is_number())
            computed_values.set_stroke_width(Length::make_px(CSSPixels::nearest_value_for(value.as_number().number())));
        else if (value.is_length())
            computed_values.set_stroke_width(value.as_length().length());
        else if (value.is_percentage())
            computed_values.set_stroke_width(LengthPercentage { value.as_percentage().percentage() });
        else if (value.is_calculated())
            computed_values.set_stroke_width(LengthPercentage { value.as_calculated() });
        else
            return false;
        return true;
    case PropertyID::StrokeDashoffset:
        if (value.is_number())
            computed_values.set_stroke_dashoffset(Length::make_px(CSSPixels::nearest_value_for(value.as_number().number())));
        else if (value.is_length())
            computed_values.set_stroke_dashoffset(value.as_length().length());
        else if (value.is_percentage())
            computed_values.set_stroke_dashoffset(LengthPercentage { value.as_percentage().percentage() });
        else
            return false;
        return true;

    // Margin sides
    case PropertyID::MarginLeft: {
        auto box = computed_values.margin();
        box.left() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_margin(box);
        return true;
    }
    case PropertyID::MarginRight: {
        auto box = computed_values.margin();
        box.right() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_margin(box);
        return true;
    }
    case PropertyID::MarginTop: {
        auto box = computed_values.margin();
        box.top() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_margin(box);
        return true;
    }
    case PropertyID::MarginBottom: {
        auto box = computed_values.margin();
        box.bottom() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_margin(box);
        return true;
    }

    // Padding sides
    case PropertyID::PaddingLeft: {
        auto box = computed_values.padding();
        box.left() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_padding(box);
        return true;
    }
    case PropertyID::PaddingRight: {
        auto box = computed_values.padding();
        box.right() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_padding(box);
        return true;
    }
    case PropertyID::PaddingTop: {
        auto box = computed_values.padding();
        box.top() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_padding(box);
        return true;
    }
    case PropertyID::PaddingBottom: {
        auto box = computed_values.padding();
        box.bottom() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_padding(box);
        return true;
    }

    // Inset sides
    case PropertyID::Left: {
        auto box = computed_values.inset();
        box.left() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_inset(box);
        return true;
    }
    case PropertyID::Right: {
        auto box = computed_values.inset();
        box.right() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_inset(box);
        return true;
    }
    case PropertyID::Top: {
        auto box = computed_values.inset();
        box.top() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_inset(box);
        return true;
    }
    case PropertyID::Bottom: {
        auto box = computed_values.inset();
        box.bottom() = LengthPercentageOrAuto::from_style_value(value);
        computed_values.set_inset(box);
        return true;
    }

    // Border widths
    case PropertyID::BorderLeftWidth: {
        auto line_style = computed_values.border_left().line_style;
        if (line_style == LineStyle::None || line_style == LineStyle::Hidden)
            computed_values.border_left().width = 0;
        else if (value.is_length())
            computed_values.border_left().width = max(CSSPixels { 0 }, value.as_length().length().absolute_length_to_px());
        else
            return false;
        return true;
    }
    case PropertyID::BorderRightWidth: {
        auto line_style = computed_values.border_right().line_style;
        if (line_style == LineStyle::None || line_style == LineStyle::Hidden)
            computed_values.border_right().width = 0;
        else if (value.is_length())
            computed_values.border_right().width = max(CSSPixels { 0 }, value.as_length().length().absolute_length_to_px());
        else
            return false;
        return true;
    }
    case PropertyID::BorderTopWidth: {
        auto line_style = computed_values.border_top().line_style;
        if (line_style == LineStyle::None || line_style == LineStyle::Hidden)
            computed_values.border_top().width = 0;
        else if (value.is_length())
            computed_values.border_top().width = max(CSSPixels { 0 }, value.as_length().length().absolute_length_to_px());
        else
            return false;
        return true;
    }
    case PropertyID::BorderBottomWidth: {
        auto line_style = computed_values.border_bottom().line_style;
        if (line_style == LineStyle::None || line_style == LineStyle::Hidden)
            computed_values.border_bottom().width = 0;
        else if (value.is_length())
            computed_values.border_bottom().width = max(CSSPixels { 0 }, value.as_length().length().absolute_length_to_px());
        else
            return false;
        return true;
    }

    // Border styles
    case PropertyID::BorderLeftStyle:
        if (auto style = keyword_to_line_style(value.to_keyword()); style.has_value()) {
            computed_values.border_left().line_style = style.value();
            return true;
        }
        return false;
    case PropertyID::BorderRightStyle:
        if (auto style = keyword_to_line_style(value.to_keyword()); style.has_value()) {
            computed_values.border_right().line_style = style.value();
            return true;
        }
        return false;
    case PropertyID::BorderTopStyle:
        if (auto style = keyword_to_line_style(value.to_keyword()); style.has_value()) {
            computed_values.border_top().line_style = style.value();
            return true;
        }
        return false;
    case PropertyID::BorderBottomStyle:
        if (auto style = keyword_to_line_style(value.to_keyword()); style.has_value()) {
            computed_values.border_bottom().line_style = style.value();
            return true;
        }
        return false;

    // Border radii
    case PropertyID::BorderBottomLeftRadius:
        if (value.is_border_radius()) {
            computed_values.set_border_bottom_left_radius(BorderRadiusData {
                LengthPercentage::from_style_value(value.as_border_radius().horizontal_radius()),
                LengthPercentage::from_style_value(value.as_border_radius().vertical_radius()) });
            return true;
        }
        return false;
    case PropertyID::BorderBottomRightRadius:
        if (value.is_border_radius()) {
            computed_values.set_border_bottom_right_radius(BorderRadiusData {
                LengthPercentage::from_style_value(value.as_border_radius().horizontal_radius()),
                LengthPercentage::from_style_value(value.as_border_radius().vertical_radius()) });
            return true;
        }
        return false;
    case PropertyID::BorderTopLeftRadius:
        if (value.is_border_radius()) {
            computed_values.set_border_top_left_radius(BorderRadiusData {
                LengthPercentage::from_style_value(value.as_border_radius().horizontal_radius()),
                LengthPercentage::from_style_value(value.as_border_radius().vertical_radius()) });
            return true;
        }
        return false;
    case PropertyID::BorderTopRightRadius:
        if (value.is_border_radius()) {
            computed_values.set_border_top_right_radius(BorderRadiusData {
                LengthPercentage::from_style_value(value.as_border_radius().horizontal_radius()),
                LengthPercentage::from_style_value(value.as_border_radius().vertical_radius()) });
            return true;
        }
        return false;

    // Transform properties
    case PropertyID::Transform:
        computed_values.set_transformations(transformations_from_style_value(value));
        return true;
    case PropertyID::Rotate:
        if (!value.is_transformation()) {
            computed_values.set_rotate({});
            return true;
        }
        computed_values.set_rotate(value.as_transformation());
        return true;
    case PropertyID::Scale:
        if (!value.is_transformation()) {
            computed_values.set_scale({});
            return true;
        }
        computed_values.set_scale(value.as_transformation());
        return true;
    case PropertyID::Translate:
        if (!value.is_transformation()) {
            computed_values.set_translate({});
            return true;
        }
        computed_values.set_translate(value.as_transformation());
        return true;

    // Font properties
    case PropertyID::FontSize:
        if (value.is_length()) {
            computed_values.set_font_size(value.as_length().length().absolute_length_to_px());
            return true;
        }
        return false;
    case PropertyID::FontWeight:
        if (value.is_number()) {
            computed_values.set_font_weight(value.as_number().number());
            return true;
        }
        return false;
    case PropertyID::LineHeight:
        if (value.is_length()) {
            computed_values.set_line_height(value.as_length().length().absolute_length_to_px());
            return true;
        }
        if (value.is_number()) {
            // NB: line-height as a number is a multiplier on font-size, but the animated
            // value should already be resolved to a concrete length. Fall back if not.
            return false;
        }
        return false;

    // Outline
    case PropertyID::OutlineOffset:
        if (value.is_length()) {
            computed_values.set_outline_offset(value.as_length().length());
            return true;
        }
        return false;
    case PropertyID::OutlineWidth:
        if (value.is_length()) {
            computed_values.set_outline_width(max(CSSPixels { 0 }, value.as_length().length().absolute_length_to_px()));
            return true;
        }
        return false;

    // Clip path
    case PropertyID::ClipPath:
        if (value.is_url()) {
            computed_values.set_clip_path(value.as_url().url());
            return true;
        }
        if (value.is_basic_shape()) {
            computed_values.set_clip_path(value.as_basic_shape());
            return true;
        }
        return false;

    // More keyword/enum properties
    case PropertyID::BoxSizing:
        if (auto val = keyword_to_box_sizing(value.to_keyword()); val.has_value()) {
            computed_values.set_box_sizing(val.value());
            return true;
        }
        return false;
    case PropertyID::FlexDirection:
        if (auto val = keyword_to_flex_direction(value.to_keyword()); val.has_value()) {
            computed_values.set_flex_direction(val.value());
            return true;
        }
        return false;
    case PropertyID::FlexWrap:
        if (auto val = keyword_to_flex_wrap(value.to_keyword()); val.has_value()) {
            computed_values.set_flex_wrap(val.value());
            return true;
        }
        return false;
    case PropertyID::AlignContent:
        if (auto val = keyword_to_align_content(value.to_keyword()); val.has_value()) {
            computed_values.set_align_content(val.value());
            return true;
        }
        return false;
    case PropertyID::AlignItems:
        if (auto val = keyword_to_align_items(value.to_keyword()); val.has_value()) {
            computed_values.set_align_items(val.value());
            return true;
        }
        return false;
    case PropertyID::AlignSelf:
        if (auto val = keyword_to_align_self(value.to_keyword()); val.has_value()) {
            computed_values.set_align_self(val.value());
            return true;
        }
        return false;
    case PropertyID::JustifyContent:
        if (auto val = keyword_to_justify_content(value.to_keyword()); val.has_value()) {
            computed_values.set_justify_content(val.value());
            return true;
        }
        return false;
    case PropertyID::JustifyItems:
        if (auto val = keyword_to_justify_items(value.to_keyword()); val.has_value()) {
            computed_values.set_justify_items(val.value());
            return true;
        }
        return false;
    case PropertyID::JustifySelf:
        if (auto val = keyword_to_justify_self(value.to_keyword()); val.has_value()) {
            computed_values.set_justify_self(val.value());
            return true;
        }
        return false;
    case PropertyID::Appearance:
        if (auto val = keyword_to_appearance(value.to_keyword()); val.has_value()) {
            computed_values.set_appearance(val.value());
            return true;
        }
        return false;
    case PropertyID::TextDecorationStyle:
        if (auto val = keyword_to_text_decoration_style(value.to_keyword()); val.has_value()) {
            computed_values.set_text_decoration_style(val.value());
            return true;
        }
        return false;
    case PropertyID::TextOverflow:
        if (auto val = keyword_to_text_overflow(value.to_keyword()); val.has_value()) {
            computed_values.set_text_overflow(val.value());
            return true;
        }
        return false;
    case PropertyID::ObjectFit:
        if (auto val = keyword_to_object_fit(value.to_keyword()); val.has_value()) {
            computed_values.set_object_fit(val.value());
            return true;
        }
        return false;
    case PropertyID::Isolation:
        if (auto val = keyword_to_isolation(value.to_keyword()); val.has_value()) {
            computed_values.set_isolation(val.value());
            return true;
        }
        return false;
    case PropertyID::MixBlendMode:
        if (auto val = keyword_to_mix_blend_mode(value.to_keyword()); val.has_value()) {
            computed_values.set_mix_blend_mode(val.value());
            return true;
        }
        return false;
    case PropertyID::UserSelect:
        if (auto val = keyword_to_user_select(value.to_keyword()); val.has_value()) {
            computed_values.set_user_select(val.value());
            return true;
        }
        return false;
    case PropertyID::UnicodeBidi:
        if (auto val = keyword_to_unicode_bidi(value.to_keyword()); val.has_value()) {
            computed_values.set_unicode_bidi(val.value());
            return true;
        }
        return false;
    case PropertyID::TableLayout:
        if (auto val = keyword_to_table_layout(value.to_keyword()); val.has_value()) {
            computed_values.set_table_layout(val.value());
            return true;
        }
        return false;
    case PropertyID::ContentVisibility:
        if (auto val = keyword_to_content_visibility(value.to_keyword()); val.has_value()) {
            computed_values.set_content_visibility(val.value());
            return true;
        }
        return false;
    case PropertyID::OutlineStyle:
        if (auto val = keyword_to_outline_style(value.to_keyword()); val.has_value()) {
            computed_values.set_outline_style(val.value());
            return true;
        }
        return false;
    case PropertyID::MaskType:
        if (auto val = keyword_to_mask_type(value.to_keyword()); val.has_value()) {
            computed_values.set_mask_type(val.value());
            return true;
        }
        return false;
    case PropertyID::ColumnSpan:
        if (auto val = keyword_to_column_span(value.to_keyword()); val.has_value()) {
            computed_values.set_column_span(val.value());
            return true;
        }
        return false;
    case PropertyID::ContainerType: {
        ContainerType container_type {};
        if (value.is_keyword() && value.to_keyword() == Keyword::Normal) {
            computed_values.set_container_type(container_type);
            return true;
        }
        if (value.is_keyword()) {
            switch (value.to_keyword()) {
            case Keyword::Size:
                container_type.is_size_container = true;
                break;
            case Keyword::InlineSize:
                container_type.is_inline_size_container = true;
                break;
            case Keyword::ScrollState:
                container_type.is_scroll_state_container = true;
                break;
            default:
                return false;
            }
            computed_values.set_container_type(container_type);
            return true;
        }
        if (value.is_value_list()) {
            for (auto const& item : value.as_value_list().values()) {
                switch (item->to_keyword()) {
                case Keyword::Size:
                    container_type.is_size_container = true;
                    break;
                case Keyword::InlineSize:
                    container_type.is_inline_size_container = true;
                    break;
                case Keyword::ScrollState:
                    container_type.is_scroll_state_container = true;
                    break;
                default:
                    break;
                }
            }
            computed_values.set_container_type(container_type);
            return true;
        }
        return false;
    }
    case PropertyID::Resize:
        if (auto val = keyword_to_resize(value.to_keyword()); val.has_value()) {
            computed_values.set_resize(val.value());
            return true;
        }
        return false;
    case PropertyID::ScrollbarWidth:
        if (auto val = keyword_to_scrollbar_width(value.to_keyword()); val.has_value()) {
            computed_values.set_scrollbar_width(val.value());
            return true;
        }
        return false;
    case PropertyID::TextAlign:
        if (auto val = keyword_to_text_align(value.to_keyword()); val.has_value()) {
            computed_values.set_text_align(val.value());
            return true;
        }
        return false;
    case PropertyID::TextJustify:
        if (auto val = keyword_to_text_justify(value.to_keyword()); val.has_value()) {
            computed_values.set_text_justify(val.value());
            return true;
        }
        return false;
    case PropertyID::TextWrapMode:
        if (auto val = keyword_to_text_wrap_mode(value.to_keyword()); val.has_value()) {
            computed_values.set_text_wrap_mode(val.value());
            return true;
        }
        return false;
    case PropertyID::WhiteSpaceCollapse:
        if (auto val = keyword_to_white_space_collapse(value.to_keyword()); val.has_value()) {
            computed_values.set_white_space_collapse(val.value());
            return true;
        }
        return false;
    case PropertyID::WordBreak:
        if (auto val = keyword_to_word_break(value.to_keyword()); val.has_value()) {
            computed_values.set_word_break(val.value());
            return true;
        }
        return false;
    case PropertyID::CaptionSide:
        if (auto val = keyword_to_caption_side(value.to_keyword()); val.has_value()) {
            computed_values.set_caption_side(val.value());
            return true;
        }
        return false;
    case PropertyID::ImageRendering:
        if (auto val = keyword_to_image_rendering(value.to_keyword()); val.has_value()) {
            computed_values.set_image_rendering(val.value());
            return true;
        }
        return false;
    case PropertyID::PointerEvents:
        if (auto val = keyword_to_pointer_events(value.to_keyword()); val.has_value()) {
            computed_values.set_pointer_events(val.value());
            return true;
        }
        return false;
    case PropertyID::TextTransform:
        if (auto val = keyword_to_text_transform(value.to_keyword()); val.has_value()) {
            computed_values.set_text_transform(val.value());
            return true;
        }
        return false;
    case PropertyID::ListStylePosition:
        if (auto val = keyword_to_list_style_position(value.to_keyword()); val.has_value()) {
            computed_values.set_list_style_position(val.value());
            return true;
        }
        return false;
    case PropertyID::ClipRule:
        if (auto val = keyword_to_fill_rule(value.to_keyword()); val.has_value()) {
            computed_values.set_clip_rule(val.value());
            return true;
        }
        return false;
    case PropertyID::FillRule:
        if (auto val = keyword_to_fill_rule(value.to_keyword()); val.has_value()) {
            computed_values.set_fill_rule(val.value());
            return true;
        }
        return false;
    case PropertyID::StrokeLinecap:
        if (auto val = keyword_to_stroke_linecap(value.to_keyword()); val.has_value()) {
            computed_values.set_stroke_linecap(val.value());
            return true;
        }
        return false;
    case PropertyID::StrokeLinejoin:
        if (auto val = keyword_to_stroke_linejoin(value.to_keyword()); val.has_value()) {
            computed_values.set_stroke_linejoin(val.value());
            return true;
        }
        return false;
    case PropertyID::TextAnchor:
        if (auto val = keyword_to_text_anchor(value.to_keyword()); val.has_value()) {
            computed_values.set_text_anchor(val.value());
            return true;
        }
        return false;
    case PropertyID::BorderCollapse:
        if (auto val = keyword_to_border_collapse(value.to_keyword()); val.has_value()) {
            computed_values.set_border_collapse(val.value());
            return true;
        }
        return false;
    case PropertyID::EmptyCells:
        if (auto val = keyword_to_empty_cells(value.to_keyword()); val.has_value()) {
            computed_values.set_empty_cells(val.value());
            return true;
        }
        return false;
    case PropertyID::MathShift:
        if (auto val = keyword_to_math_shift(value.to_keyword()); val.has_value()) {
            computed_values.set_math_shift(val.value());
            return true;
        }
        return false;
    case PropertyID::MathStyle:
        if (auto val = keyword_to_math_style(value.to_keyword()); val.has_value()) {
            computed_values.set_math_style(val.value());
            return true;
        }
        return false;
    case PropertyID::WritingMode:
        if (auto val = keyword_to_writing_mode(value.to_keyword()); val.has_value()) {
            computed_values.set_writing_mode(val.value());
            return true;
        }
        return false;
    case PropertyID::ColorInterpolation:
        if (auto val = keyword_to_color_interpolation(value.to_keyword()); val.has_value()) {
            computed_values.set_color_interpolation(val.value());
            return true;
        }
        return false;
    case PropertyID::ShapeRendering:
        if (auto val = keyword_to_shape_rendering(value.to_keyword()); val.has_value()) {
            computed_values.set_shape_rendering(val.value());
            return true;
        }
        return false;
    case PropertyID::TransformBox:
        if (auto val = keyword_to_transform_box(value.to_keyword()); val.has_value()) {
            computed_values.set_transform_box(val.value());
            return true;
        }
        return false;
    case PropertyID::TransformStyle:
        if (auto val = keyword_to_transform_style(value.to_keyword()); val.has_value()) {
            computed_values.set_transform_style(val.value());
            return true;
        }
        return false;
    case PropertyID::Direction:
        if (auto val = keyword_to_direction(value.to_keyword()); val.has_value()) {
            computed_values.set_direction(val.value());
            return true;
        }
        return false;

    // Color properties (fill, stroke, caret-color, scrollbar-color, accent-color)
    case PropertyID::Fill:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_fill(color.value());
                return true;
            }
        }
        if (value.is_url()) {
            computed_values.set_fill(value.as_url().url());
            return true;
        }
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.clear_fill();
            return true;
        }
        return false;
    case PropertyID::Stroke:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_stroke(color.value());
                return true;
            }
        }
        if (value.is_url()) {
            computed_values.set_stroke(value.as_url().url());
            return true;
        }
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.clear_stroke();
            return true;
        }
        return false;
    case PropertyID::CaretColor:
        if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_caret_color(computed_values.color());
            return true;
        }
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_caret_color(color.value());
                return true;
            }
        }
        return false;
    case PropertyID::AccentColor:
        if (value.has_color()) {
            if (auto color = value.to_color({}); color.has_value()) {
                computed_values.set_accent_color(color.value());
                return true;
            }
        }
        if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_accent_color({});
            return true;
        }
        return false;
    case PropertyID::ScrollbarColor:
        if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_scrollbar_color({});
            return true;
        }
        if (value.is_scrollbar_color()) {
            auto& scrollbar_color = value.as_scrollbar_color();
            auto thumb = scrollbar_color.thumb_color()->to_color({});
            auto track = scrollbar_color.track_color()->to_color({});
            if (thumb.has_value() && track.has_value()) {
                computed_values.set_scrollbar_color(ScrollbarColorData { thumb.value(), track.value() });
                return true;
            }
        }
        return false;

    // Variant properties
    case PropertyID::VerticalAlign:
        if (value.is_keyword()) {
            if (auto val = keyword_to_vertical_align(value.to_keyword()); val.has_value()) {
                computed_values.set_vertical_align(val.value());
                return true;
            }
            return false;
        }
        computed_values.set_vertical_align(LengthPercentage::from_style_value(value));
        return true;
    case PropertyID::FlexBasis:
        if (value.is_keyword() && value.to_keyword() == Keyword::Content) {
            computed_values.set_flex_basis(FlexBasisContent {});
            return true;
        }
        computed_values.set_flex_basis(size_value_from_style_value(value));
        return true;

    // Length/pixel properties
    case PropertyID::WordSpacing:
        if (value.is_keyword() && value.to_keyword() == Keyword::Normal) {
            computed_values.set_word_spacing(0);
            return true;
        }
        if (value.is_length()) {
            computed_values.set_word_spacing(value.as_length().length().absolute_length_to_px());
            return true;
        }
        return false;
    case PropertyID::LetterSpacing:
        if (value.is_keyword() && value.to_keyword() == Keyword::Normal) {
            computed_values.set_letter_spacing(0);
            return true;
        }
        if (value.is_length()) {
            computed_values.set_letter_spacing(value.as_length().length().absolute_length_to_px());
            return true;
        }
        return false;
    case PropertyID::TextUnderlineOffset:
        if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_text_underline_offset({});
            return true;
        }
        if (value.is_length()) {
            computed_values.set_text_underline_offset(value.as_length().length().absolute_length_to_px());
            return true;
        }
        return false;
    case PropertyID::Perspective:
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.set_perspective({});
            return true;
        }
        if (value.is_length()) {
            computed_values.set_perspective(value.as_length().length().absolute_length_to_px());
            return true;
        }
        return false;
    case PropertyID::BorderSpacing: {
        if (value.is_length()) {
            auto length = value.as_length().length();
            computed_values.set_border_spacing_horizontal(length);
            computed_values.set_border_spacing_vertical(length);
            return true;
        }
        if (value.is_value_list()) {
            auto const& list = value.as_value_list();
            if (list.size() >= 2 && list.values()[0]->is_length() && list.values()[1]->is_length()) {
                computed_values.set_border_spacing_horizontal(list.values()[0]->as_length().length());
                computed_values.set_border_spacing_vertical(list.values()[1]->as_length().length());
                return true;
            }
        }
        return false;
    }

    // Text decoration thickness
    case PropertyID::TextDecorationThickness:
        if (value.is_keyword()) {
            if (value.to_keyword() == Keyword::Auto) {
                computed_values.set_text_decoration_thickness(TextDecorationThickness { TextDecorationThickness::Auto {} });
                return true;
            }
            if (value.to_keyword() == Keyword::FromFont) {
                computed_values.set_text_decoration_thickness(TextDecorationThickness { TextDecorationThickness::FromFont {} });
                return true;
            }
        }
        computed_values.set_text_decoration_thickness(TextDecorationThickness { LengthPercentage::from_style_value(value) });
        return true;

    // Text indent
    case PropertyID::TextIndent:
        if (value.is_text_indent()) {
            auto const& text_indent = value.as_text_indent();
            computed_values.set_text_indent({
                .length_percentage = LengthPercentage::from_style_value(text_indent.length_percentage()),
                .each_line = text_indent.each_line(),
                .hanging = text_indent.hanging(),
            });
            return true;
        }
        return false;

    // Tab size
    case PropertyID::TabSize:
        if (value.is_length()) {
            computed_values.set_tab_size(value.as_length().length());
            return true;
        }
        if (value.is_number()) {
            computed_values.set_tab_size(static_cast<double>(value.as_number().number()));
            return true;
        }
        return false;

    // Transition delay
    case PropertyID::TransitionDelay:
        if (value.is_time()) {
            computed_values.set_transition_delay(value.as_time().time());
            return true;
        }
        return false;

    // Text underline position
    case PropertyID::TextUnderlinePosition:
        if (value.is_text_underline_position()) {
            auto const& text_underline_position = value.as_text_underline_position();
            computed_values.set_text_underline_position({
                .horizontal = text_underline_position.horizontal(),
                .vertical = text_underline_position.vertical(),
            });
            return true;
        }
        return false;

    // Text decoration line (value list of keywords)
    case PropertyID::TextDecorationLine:
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.set_text_decoration_line({});
            return true;
        }
        if (value.is_value_list()) {
            Vector<TextDecorationLine> lines;
            for (auto const& item : value.as_value_list().values()) {
                if (auto line = keyword_to_text_decoration_line(item->to_keyword()); line.has_value())
                    lines.append(line.value());
                else
                    return false;
            }
            computed_values.set_text_decoration_line(move(lines));
            return true;
        }
        return false;

    // List style type
    case PropertyID::ListStyleType:
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.set_list_style_type(Empty {});
            return true;
        }
        if (value.is_string()) {
            computed_values.set_list_style_type(value.as_string().string_value().to_string());
            return true;
        }
        if (value.is_counter_style()) {
            if (auto keyword = value.as_counter_style().to_counter_style_name_keyword(); keyword.has_value()) {
                computed_values.set_list_style_type(keyword.release_value());
                return true;
            }
        }
        computed_values.set_list_style_type(Empty {});
        return true;

    // Cursor
    case PropertyID::Cursor: {
        Vector<CursorData> cursors;
        if (value.is_value_list()) {
            for (auto const& item : value.as_value_list().values()) {
                if (item->is_cursor()) {
                    cursors.append({ item->as_cursor() });
                    continue;
                }
                if (auto keyword = keyword_to_cursor_predefined(item->to_keyword()); keyword.has_value())
                    cursors.append(keyword.release_value());
            }
        } else if (value.is_keyword()) {
            if (auto keyword = keyword_to_cursor_predefined(value.to_keyword()); keyword.has_value())
                cursors.append(keyword.release_value());
        }
        if (cursors.is_empty())
            cursors.append(CursorPredefined::Auto);
        computed_values.set_cursor(move(cursors));
        return true;
    }

    // Clip
    case PropertyID::Clip:
        if (value.is_rect()) {
            computed_values.set_clip(Clip(value.as_rect().rect()));
            return true;
        }
        computed_values.set_clip(Clip::make_auto());
        return true;

    // Filters
    case PropertyID::BackdropFilter:
        if (value.is_filter_value_list()) {
            computed_values.set_backdrop_filter(Filter(value.as_filter_value_list()));
            return true;
        }
        computed_values.set_backdrop_filter(Filter::make_none());
        return true;
    case PropertyID::Filter:
        if (value.is_filter_value_list()) {
            computed_values.set_filter(Filter(value.as_filter_value_list()));
            return true;
        }
        computed_values.set_filter(Filter::make_none());
        return true;

    // Shadows
    case PropertyID::TextShadow:
    case PropertyID::BoxShadow: {
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            if (property_id == PropertyID::TextShadow)
                computed_values.set_text_shadow({});
            else
                computed_values.set_box_shadow({});
            return true;
        }
        if (!value.is_value_list())
            return false;
        Vector<ShadowData> shadow_data;
        for (auto const& item : value.as_value_list().values()) {
            if (!item->is_shadow())
                return false;
            auto const& shadow = item->as_shadow();
            if (!shadow.offset_x()->is_length() || !shadow.offset_y()->is_length()
                || !shadow.blur_radius()->is_length() || !shadow.spread_distance()->is_length())
                return false;
            auto color = shadow.color()->to_color({});
            if (!color.has_value())
                return false;
            shadow_data.append({
                shadow.offset_x()->as_length().length(),
                shadow.offset_y()->as_length().length(),
                shadow.blur_radius()->as_length().length(),
                shadow.spread_distance()->as_length().length(),
                color.value(),
                shadow.placement(),
            });
        }
        if (property_id == PropertyID::TextShadow)
            computed_values.set_text_shadow(move(shadow_data));
        else
            computed_values.set_box_shadow(move(shadow_data));
        return true;
    }

    // Aspect ratio
    case PropertyID::AspectRatio:
        if (value.is_value_list()) {
            auto const& list = value.as_value_list().values();
            if (list.size() == 2 && list[0]->is_keyword() && list[0]->to_keyword() == Keyword::Auto && list[1]->is_ratio()) {
                computed_values.set_aspect_ratio({ true, list[1]->as_ratio().ratio() });
                return true;
            }
        } else if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_aspect_ratio({ true, {} });
            return true;
        } else if (value.is_ratio()) {
            if (value.as_ratio().ratio().is_degenerate())
                computed_values.set_aspect_ratio({ true, {} });
            else
                computed_values.set_aspect_ratio({ false, value.as_ratio().ratio() });
            return true;
        }
        return false;

    // Grid properties
    case PropertyID::GridAutoColumns:
        if (value.is_grid_track_size_list()) {
            computed_values.set_grid_auto_columns(value.as_grid_track_size_list().grid_track_size_list());
            return true;
        }
        return false;
    case PropertyID::GridAutoRows:
        if (value.is_grid_track_size_list()) {
            computed_values.set_grid_auto_rows(value.as_grid_track_size_list().grid_track_size_list());
            return true;
        }
        return false;
    case PropertyID::GridTemplateColumns:
        if (value.is_grid_track_size_list()) {
            computed_values.set_grid_template_columns(value.as_grid_track_size_list().grid_track_size_list());
            return true;
        }
        return false;
    case PropertyID::GridTemplateRows:
        if (value.is_grid_track_size_list()) {
            computed_values.set_grid_template_rows(value.as_grid_track_size_list().grid_track_size_list());
            return true;
        }
        return false;
    case PropertyID::GridColumnEnd:
        if (value.is_grid_track_placement()) {
            computed_values.set_grid_column_end(value.as_grid_track_placement().grid_track_placement());
            return true;
        }
        return false;
    case PropertyID::GridColumnStart:
        if (value.is_grid_track_placement()) {
            computed_values.set_grid_column_start(value.as_grid_track_placement().grid_track_placement());
            return true;
        }
        return false;
    case PropertyID::GridRowEnd:
        if (value.is_grid_track_placement()) {
            computed_values.set_grid_row_end(value.as_grid_track_placement().grid_track_placement());
            return true;
        }
        return false;
    case PropertyID::GridRowStart:
        if (value.is_grid_track_placement()) {
            computed_values.set_grid_row_start(value.as_grid_track_placement().grid_track_placement());
            return true;
        }
        return false;
    case PropertyID::GridTemplateAreas:
        if (value.is_grid_template_area()) {
            computed_values.set_grid_template_areas(value.as_grid_template_area().grid_template_area());
            return true;
        }
        return false;
    case PropertyID::GridAutoFlow:
        if (value.is_grid_auto_flow()) {
            auto const& grid_auto_flow = value.as_grid_auto_flow();
            computed_values.set_grid_auto_flow({ .row = grid_auto_flow.is_row(), .dense = grid_auto_flow.is_dense() });
            return true;
        }
        return false;

    // Position properties (object-position, perspective-origin)
    case PropertyID::ObjectPosition:
    case PropertyID::PerspectiveOrigin: {
        if (!value.is_position())
            return false;
        auto const& position = value.as_position();
        auto const& edge_x = position.edge_x()->as_edge();
        auto const& edge_y = position.edge_y()->as_edge();
        CSS::Position pos {
            .offset_x = LengthPercentage::from_style_value(edge_x.offset()),
            .offset_y = LengthPercentage::from_style_value(edge_y.offset()),
        };
        if (property_id == PropertyID::ObjectPosition)
            computed_values.set_object_position(pos);
        else
            computed_values.set_perspective_origin(pos);
        return true;
    }

    // Transform origin
    case PropertyID::TransformOrigin:
        if (value.is_value_list() && value.as_value_list().size() == 3) {
            auto length_percentage_with_keywords_resolved = [](StyleValue const& v) -> LengthPercentage {
                if (v.is_keyword()) {
                    auto keyword = v.to_keyword();
                    if (keyword == Keyword::Left || keyword == Keyword::Top)
                        return Percentage(0);
                    if (keyword == Keyword::Center)
                        return Percentage(50);
                    if (keyword == Keyword::Right || keyword == Keyword::Bottom)
                        return Percentage(100);
                    VERIFY_NOT_REACHED();
                }
                return LengthPercentage::from_style_value(v);
            };
            auto const& list = value.as_value_list();
            computed_values.set_transform_origin({
                length_percentage_with_keywords_resolved(*list.values()[0]),
                length_percentage_with_keywords_resolved(*list.values()[1]),
                LengthPercentage::from_style_value(*list.values()[2]),
            });
            return true;
        }
        return false;

    // Contain
    case PropertyID::Contain: {
        Containment containment = {};
        if (value.is_keyword()) {
            switch (value.to_keyword()) {
            case Keyword::None:
                break;
            case Keyword::Strict:
                containment.size_containment = true;
                containment.layout_containment = true;
                containment.paint_containment = true;
                containment.style_containment = true;
                break;
            case Keyword::Content:
                containment.layout_containment = true;
                containment.paint_containment = true;
                containment.style_containment = true;
                break;
            case Keyword::Size:
                containment.size_containment = true;
                break;
            case Keyword::InlineSize:
                containment.inline_size_containment = true;
                break;
            case Keyword::Layout:
                containment.layout_containment = true;
                break;
            case Keyword::Style:
                containment.style_containment = true;
                break;
            case Keyword::Paint:
                containment.paint_containment = true;
                break;
            default:
                return false;
            }
        } else if (value.is_value_list()) {
            for (auto const& item : value.as_value_list().values()) {
                switch (item->to_keyword()) {
                case Keyword::Size:
                    containment.size_containment = true;
                    break;
                case Keyword::InlineSize:
                    containment.inline_size_containment = true;
                    break;
                case Keyword::Layout:
                    containment.layout_containment = true;
                    break;
                case Keyword::Style:
                    containment.style_containment = true;
                    break;
                case Keyword::Paint:
                    containment.paint_containment = true;
                    break;
                default:
                    break;
                }
            }
        }
        computed_values.set_contain(containment);
        return true;
    }

    // Will change
    case PropertyID::WillChange:
        if (value.is_keyword() && value.to_keyword() == Keyword::Auto) {
            computed_values.set_will_change(WillChange::make_auto());
            return true;
        }
        if (value.is_value_list()) {
            Vector<WillChange::WillChangeEntry> entries;
            for (auto const& item : value.as_value_list().values()) {
                if (item->is_keyword()) {
                    switch (item->to_keyword()) {
                    case Keyword::Contents:
                        entries.append(WillChange::Type::Contents);
                        break;
                    case Keyword::ScrollPosition:
                        entries.append(WillChange::Type::ScrollPosition);
                        break;
                    default:
                        break;
                    }
                } else if (item->is_custom_ident()) {
                    if (auto prop_id = property_id_from_string(item->as_custom_ident().custom_ident()); prop_id.has_value())
                        entries.append(prop_id.release_value());
                }
            }
            computed_values.set_will_change(WillChange(move(entries)));
            return true;
        }
        return false;

    // View transition name
    case PropertyID::ViewTransitionName:
        if (value.is_custom_ident()) {
            computed_values.set_view_transition_name(value.as_custom_ident().custom_ident());
            return true;
        }
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.set_view_transition_name({});
            return true;
        }
        return false;

    // Quotes
    case PropertyID::Quotes:
        if (value.is_keyword()) {
            switch (value.to_keyword()) {
            case Keyword::Auto:
                computed_values.set_quotes({ .type = QuotesData::Type::Auto });
                return true;
            case Keyword::None:
                computed_values.set_quotes({ .type = QuotesData::Type::None });
                return true;
            default:
                return false;
            }
        }
        if (value.is_value_list()) {
            QuotesData data { .type = QuotesData::Type::Specified };
            auto const& list = value.as_value_list();
            for (auto i = 0u; i < list.size(); i += 2) {
                data.strings.empend(
                    list.value_at(i, false)->as_string().string_value(),
                    list.value_at(i + 1, false)->as_string().string_value());
            }
            computed_values.set_quotes(move(data));
            return true;
        }
        return false;

    // Counter properties
    case PropertyID::CounterIncrement:
    case PropertyID::CounterReset:
    case PropertyID::CounterSet: {
        Vector<CounterData> counters;
        if (value.is_counter_definitions()) {
            for (auto const& counter : value.as_counter_definitions().counter_definitions()) {
                CounterData data {
                    .name = counter.name,
                    .is_reversed = counter.is_reversed,
                    .value = {},
                };
                if (counter.value) {
                    if (counter.value->is_integer())
                        data.value = AK::clamp_to<i32>(counter.value->as_integer().integer());
                    else if (counter.value->is_calculated()) {
                        auto maybe_int = counter.value->as_calculated().resolve_integer({});
                        if (maybe_int.has_value())
                            data.value = AK::clamp_to<i32>(*maybe_int);
                    }
                }
                counters.append(move(data));
            }
        }
        if (property_id == PropertyID::CounterIncrement)
            computed_values.set_counter_increment(move(counters));
        else if (property_id == PropertyID::CounterReset)
            computed_values.set_counter_reset(move(counters));
        else
            computed_values.set_counter_set(move(counters));
        return true;
    }

    // Math depth
    case PropertyID::MathDepth:
        if (value.is_integer()) {
            computed_values.set_math_depth(value.as_integer().integer());
            return true;
        }
        return false;

    // Stroke dasharray
    case PropertyID::StrokeDasharray:
        if (value.is_keyword() && value.to_keyword() == Keyword::None) {
            computed_values.set_stroke_dasharray({});
            return true;
        }
        if (value.is_value_list()) {
            Vector<Variant<LengthPercentage, float>> dashes;
            for (auto const& item : value.as_value_list().values()) {
                if (item->is_length())
                    dashes.append(LengthPercentage { item->as_length().length() });
                else if (item->is_percentage())
                    dashes.append(LengthPercentage { item->as_percentage().percentage() });
                else if (item->is_calculated())
                    dashes.append(LengthPercentage { item->as_calculated() });
                else if (item->is_number())
                    dashes.append(item->as_number().number());
                else
                    return false;
            }
            computed_values.set_stroke_dasharray(move(dashes));
            return true;
        }
        return false;

    // Paint order
    case PropertyID::PaintOrder:
        if (value.is_keyword()) {
            auto keyword = value.to_keyword();
            if (keyword == Keyword::Normal) {
                computed_values.set_paint_order(InitialValues::paint_order());
                return true;
            }
            if (auto paint_order = keyword_to_paint_order(keyword); paint_order.has_value()) {
                switch (*paint_order) {
                case PaintOrder::Fill:
                    computed_values.set_paint_order(InitialValues::paint_order());
                    return true;
                case PaintOrder::Stroke:
                    computed_values.set_paint_order(PaintOrderList { PaintOrder::Stroke, PaintOrder::Fill, PaintOrder::Markers });
                    return true;
                case PaintOrder::Markers:
                    computed_values.set_paint_order(PaintOrderList { PaintOrder::Markers, PaintOrder::Fill, PaintOrder::Stroke });
                    return true;
                }
            }
            return false;
        }
        if (value.is_value_list()) {
            auto const& list = value.as_value_list();
            if (list.size() != 2)
                return false;
            PaintOrderList paint_order_list {};
            auto sum = 0;
            for (auto i = 0; i < 2; i++) {
                auto paint_order = keyword_to_paint_order(list.value_at(i, false)->to_keyword());
                if (!paint_order.has_value())
                    return false;
                sum += to_underlying(*paint_order);
                paint_order_list[i] = *paint_order;
            }
            paint_order_list[2] = static_cast<PaintOrder>(3 - sum);
            computed_values.set_paint_order(paint_order_list);
            return true;
        }
        return false;

    // Font language override
    case PropertyID::FontLanguageOverride:
        if (value.is_string()) {
            computed_values.set_font_language_override(value.as_string().string_value());
            return true;
        }
        if (value.is_keyword()) {
            computed_values.set_font_language_override({});
            return true;
        }
        return false;

    // Font variation settings
    case PropertyID::FontVariationSettings:
        if (value.is_keyword()) {
            computed_values.set_font_variation_settings({});
            return true;
        }
        if (value.is_value_list()) {
            HashMap<FlyString, double> result;
            for (auto const& tag_value : value.as_value_list().values()) {
                auto const& axis_tag = tag_value->as_open_type_tagged();
                if (axis_tag.value()->is_number())
                    result.set(axis_tag.tag(), axis_tag.value()->as_number().number());
                else if (axis_tag.value()->is_calculated())
                    result.set(axis_tag.tag(), axis_tag.value()->as_calculated().resolve_number({}).value());
            }
            computed_values.set_font_variation_settings(move(result));
            return true;
        }
        return false;

    // Touch action
    case PropertyID::TouchAction: {
        if (value.is_keyword()) {
            switch (value.to_keyword()) {
            case Keyword::Auto:
                computed_values.set_touch_action(TouchActionData {});
                return true;
            case Keyword::None:
                computed_values.set_touch_action(TouchActionData::none());
                return true;
            case Keyword::Manipulation:
                computed_values.set_touch_action(TouchActionData { .allow_other = false });
                return true;
            default:
                return false;
            }
        }
        if (value.is_value_list()) {
            TouchActionData touch_action_data = TouchActionData::none();
            for (auto const& item : value.as_value_list().values()) {
                switch (item->to_keyword()) {
                case Keyword::PanX:
                    touch_action_data.allow_right = true;
                    touch_action_data.allow_left = true;
                    break;
                case Keyword::PanLeft:
                    touch_action_data.allow_left = true;
                    break;
                case Keyword::PanRight:
                    touch_action_data.allow_right = true;
                    break;
                case Keyword::PanY:
                    touch_action_data.allow_up = true;
                    touch_action_data.allow_down = true;
                    break;
                case Keyword::PanUp:
                    touch_action_data.allow_up = true;
                    break;
                case Keyword::PanDown:
                    touch_action_data.allow_down = true;
                    break;
                default:
                    break;
                }
            }
            computed_values.set_touch_action(touch_action_data);
            return true;
        }
        return false;
    }

    // Properties that need Document for resource resolution -- return false
    case PropertyID::BackgroundImage:
    case PropertyID::BackgroundRepeat:
    case PropertyID::BackgroundSize:
    case PropertyID::BackgroundPosition:
    case PropertyID::BackgroundAttachment:
    case PropertyID::BackgroundOrigin:
    case PropertyID::BackgroundClip:
    case PropertyID::MaskImage:
    case PropertyID::ColorScheme:
        return false;

    default:
        return false;
    }
}

}
