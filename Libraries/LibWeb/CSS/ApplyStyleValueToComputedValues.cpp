/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ApplyStyleValueToComputedValues.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/LengthBox.h>
#include <LibWeb/CSS/PercentageOr.h>
#include <LibWeb/CSS/Size.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusStyleValue.h>
#include <LibWeb/CSS/StyleValues/DisplayStyleValue.h>
#include <LibWeb/CSS/StyleValues/FitContentStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
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

    // Properties that are complex or rarely animated -- fall back to full round-trip
    default:
        return false;
    }
}

}
