/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 * Copyright (c) 2024, Shannon Booth <shannon@serenityos.org>
 * Copyright (c) 2024, Tommy van der Vorst <tommy@pixelspark.nl>
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2024, Glenn Skrzypczak <glenn.skrzypczak@gmail.com>
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Debug.h>
#include <AK/QuickSort.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/PropertyID.h>
#include <LibWeb/CSS/StyleValues/AngleStyleValue.h>
#include <LibWeb/CSS/StyleValues/BackgroundSizeStyleValue.h>
#include <LibWeb/CSS/StyleValues/BasicShapeStyleValue.h>
#include <LibWeb/CSS/StyleValues/BorderImageSliceStyleValue.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusRectStyleValue.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorSchemeStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/ContentStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterDefinitionsStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleValue.h>
#include <LibWeb/CSS/StyleValues/CursorStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/DisplayStyleValue.h>
#include <LibWeb/CSS/StyleValues/EasingStyleValue.h>
#include <LibWeb/CSS/StyleValues/EdgeStyleValue.h>
#include <LibWeb/CSS/StyleValues/FilterValueListStyleValue.h>
#include <LibWeb/CSS/StyleValues/FlexStyleValue.h>
#include <LibWeb/CSS/StyleValues/FontStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/FrequencyStyleValue.h>
#include <LibWeb/CSS/StyleValues/FunctionStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridAutoFlowStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTemplateAreaStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackPlacementStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackSizeListStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpacityValueStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpenTypeTaggedStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/PositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/RadialSizeStyleValue.h>
#include <LibWeb/CSS/StyleValues/RatioStyleValue.h>
#include <LibWeb/CSS/StyleValues/RectStyleValue.h>
#include <LibWeb/CSS/StyleValues/RepeatStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/ResolutionStyleValue.h>
#include <LibWeb/CSS/StyleValues/ScrollbarColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/ScrollbarGutterStyleValue.h>
#include <LibWeb/CSS/StyleValues/ShadowStyleValue.h>
#include <LibWeb/CSS/StyleValues/ShorthandStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/SuperellipseStyleValue.h>
#include <LibWeb/CSS/StyleValues/TextIndentStyleValue.h>
#include <LibWeb/CSS/StyleValues/TextUnderlinePositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/TimeStyleValue.h>
#include <LibWeb/CSS/StyleValues/TransformationStyleValue.h>
#include <LibWeb/CSS/StyleValues/TupleStyleValue.h>
#include <LibWeb/CSS/StyleValues/URLStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>
#include <LibWeb/CSS/ValueType.h>
#include <LibWeb/Dump.h>
#include <LibWeb/Infra/Strings.h>

namespace Web::CSS::Parser {

static void remove_property(Vector<PropertyID>& properties, PropertyID property_to_remove)
{
    properties.remove_first_matching([&](auto it) { return it == property_to_remove; });
}

static bool property_uses_rust_owned_whole_grammar(PropertyID property_id)
{
    switch (property_id) {
    case PropertyID::AnchorName:
    case PropertyID::AnchorScope:
    case PropertyID::AnimationName:
    case PropertyID::AspectRatio:
    case PropertyID::BackgroundPosition:
    case PropertyID::BackgroundPositionX:
    case PropertyID::BackgroundPositionY:
    case PropertyID::BackgroundRepeat:
    case PropertyID::BackgroundSize:
    case PropertyID::Border:
    case PropertyID::BorderBlock:
    case PropertyID::BorderImage:
    case PropertyID::BorderImageOutset:
    case PropertyID::BorderImageRepeat:
    case PropertyID::BorderImageSlice:
    case PropertyID::BorderImageWidth:
    case PropertyID::BorderInline:
    case PropertyID::BorderBottomLeftRadius:
    case PropertyID::BorderBottomRightRadius:
    case PropertyID::BorderEndEndRadius:
    case PropertyID::BorderEndStartRadius:
    case PropertyID::BorderRadius:
    case PropertyID::BorderStartEndRadius:
    case PropertyID::BorderStartStartRadius:
    case PropertyID::BorderTopLeftRadius:
    case PropertyID::BorderTopRightRadius:
    case PropertyID::BoxShadow:
    case PropertyID::BackdropFilter:
    case PropertyID::ColorScheme:
    case PropertyID::Columns:
    case PropertyID::Contain:
    case PropertyID::ContainerType:
    case PropertyID::Content:
    case PropertyID::CounterIncrement:
    case PropertyID::CounterReset:
    case PropertyID::CounterSet:
    case PropertyID::Cursor:
    case PropertyID::Display:
    case PropertyID::Filter:
    case PropertyID::Flex:
    case PropertyID::FlexFlow:
    case PropertyID::FontFamily:
    case PropertyID::FontFeatureSettings:
    case PropertyID::FontLanguageOverride:
    case PropertyID::FontVariant:
    case PropertyID::FontVariationSettings:
    case PropertyID::GridAutoColumns:
    case PropertyID::GridAutoFlow:
    case PropertyID::GridAutoRows:
    case PropertyID::GridColumnEnd:
    case PropertyID::GridColumnStart:
    case PropertyID::GridRowEnd:
    case PropertyID::GridRowStart:
    case PropertyID::GridTemplateAreas:
    case PropertyID::GridTemplateColumns:
    case PropertyID::GridTemplateRows:
    case PropertyID::ListStyle:
    case PropertyID::MaskPosition:
    case PropertyID::MaskRepeat:
    case PropertyID::MaskSize:
    case PropertyID::MathDepth:
    case PropertyID::OverflowClipMargin:
    case PropertyID::OverflowClipMarginBlock:
    case PropertyID::OverflowClipMarginBlockEnd:
    case PropertyID::OverflowClipMarginBlockStart:
    case PropertyID::OverflowClipMarginBottom:
    case PropertyID::OverflowClipMarginInline:
    case PropertyID::OverflowClipMarginInlineEnd:
    case PropertyID::OverflowClipMarginInlineStart:
    case PropertyID::OverflowClipMarginLeft:
    case PropertyID::OverflowClipMarginRight:
    case PropertyID::OverflowClipMarginTop:
    case PropertyID::PaintOrder:
    case PropertyID::PlaceContent:
    case PropertyID::PlaceItems:
    case PropertyID::PlaceSelf:
    case PropertyID::PositionAnchor:
    case PropertyID::PositionArea:
    case PropertyID::PositionTryFallbacks:
    case PropertyID::PositionTryOrder:
    case PropertyID::PositionVisibility:
    case PropertyID::Quotes:
    case PropertyID::Rotate:
    case PropertyID::Scale:
    case PropertyID::ScrollTimeline:
    case PropertyID::ScrollTimelineName:
    case PropertyID::ScrollbarColor:
    case PropertyID::ScrollbarGutter:
    case PropertyID::ShapeOutside:
    case PropertyID::StrokeDasharray:
    case PropertyID::TextDecoration:
    case PropertyID::TextDecorationLine:
    case PropertyID::TextIndent:
    case PropertyID::TextShadow:
    case PropertyID::TextUnderlinePosition:
    case PropertyID::TextWrap:
    case PropertyID::TextWrapMode:
    case PropertyID::TextWrapStyle:
    case PropertyID::TimelineScope:
    case PropertyID::TouchAction:
    case PropertyID::TransformOrigin:
    case PropertyID::TransitionBehavior:
    case PropertyID::TransitionProperty:
    case PropertyID::Translate:
    case PropertyID::ViewTimeline:
    case PropertyID::ViewTimelineName:
    case PropertyID::ViewTransitionName:
    case PropertyID::WhiteSpace:
    case PropertyID::WhiteSpaceTrim:
    case PropertyID::WillChange:
        return true;
    default:
        return false;
    }
}

static FontStyleKeyword font_style_keyword_from_rust(FFI::CssFontStyleKind font_style)
{
    switch (font_style) {
    case FFI::CssFontStyleKind::Normal:
        return FontStyleKeyword::Normal;
    case FFI::CssFontStyleKind::Italic:
        return FontStyleKeyword::Italic;
    case FFI::CssFontStyleKind::Left:
        return FontStyleKeyword::Left;
    case FFI::CssFontStyleKind::Right:
        return FontStyleKeyword::Right;
    case FFI::CssFontStyleKind::Oblique:
        return FontStyleKeyword::Oblique;
    }
    VERIFY_NOT_REACHED();
}

RefPtr<StyleValue const> Parser::parse_all_as_single_keyword_value(TokenStream<ComponentValue>& tokens, Keyword keyword)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto keyword_value = parse_specific_keyword_value(tokens, keyword);
    tokens.discard_whitespace();

    if (tokens.has_next_token() || !keyword_value)
        return {};

    transaction.commit();
    return keyword_value;
}

RefPtr<StyleValueList const> Parser::parse_simple_comma_separated_value_list(PropertyID property_id, TokenStream<ComponentValue>& tokens)
{
    return parse_comma_separated_value_list(tokens, [this, property_id](auto& tokens) -> RefPtr<StyleValue const> {
        auto transaction = tokens.begin_transaction();
        if (auto value = parse_css_value_for_property(property_id, tokens)) {
            transaction.commit();
            return value;
        }
        return nullptr;
    });
}

RefPtr<StyleValue const> Parser::parse_coordinating_value_list_shorthand(TokenStream<ComponentValue>& tokens, PropertyID shorthand_id, Vector<PropertyID> const& longhand_ids, Vector<PropertyID> const& reset_only_longhand_ids = {})
{
    auto unwrap_single_coordinating_value_list_item = [](PropertyID property_id, RefPtr<StyleValue const>& parsed_value) {
        if (first_is_one_of(property_id,
                PropertyID::AnimationName,
                PropertyID::ScrollTimelineName,
                PropertyID::TransitionBehavior,
                PropertyID::TransitionProperty,
                PropertyID::ViewTimelineName)
            && parsed_value->is_value_list()
            && parsed_value->as_value_list().size() == 1) {
            parsed_value = parsed_value->as_value_list().values()[0];
        }
    };

    {
        auto rust_transaction = tokens.begin_transaction();
        auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
        if (auto rust_items = RustComponentValueParser::parse_coordinating_value_list_shorthand(longhand_ids, source.bytes_as_string_view()); rust_items.has_value()) {
            Vector<HashMap<PropertyID, NonnullRefPtr<StyleValue const>>> parsed_layers;

            for (auto const& item : rust_items.value()) {
                if (item.layer_index >= parsed_layers.size())
                    parsed_layers.resize(item.layer_index + 1);

                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(item.value.bytes_as_string_view(), "utf-8"sv);
                TokenStream<ComponentValue> value_tokens { component_values };
                auto parsed_value = parse_css_value_for_property(item.property_id, value_tokens);
                value_tokens.discard_whitespace();
                if (!parsed_value || value_tokens.has_next_token())
                    return {};

                unwrap_single_coordinating_value_list_item(item.property_id, parsed_value);

                parsed_layers[item.layer_index].set(item.property_id, parsed_value.release_nonnull());
            }

            if (parsed_layers.is_empty())
                return {};

            StyleValueVector longhand_values {};
            for (auto const& longhand_id : longhand_ids) {
                StyleValueVector layer_values;
                for (auto const& parsed_layer : parsed_layers) {
                    layer_values.append(*parsed_layer.get(longhand_id).value_or_lazy_evaluated([&]() -> ValueComparingNonnullRefPtr<StyleValue const> {
                        return property_initial_value(longhand_id)->as_value_list().values()[0];
                    }));
                }
                longhand_values.append(StyleValueList::create(move(layer_values), StyleValueList::Separator::Comma));
            }

            Vector<PropertyID> longhand_ids_including_reset_only_longhands;
            longhand_ids_including_reset_only_longhands.extend(longhand_ids);
            longhand_ids_including_reset_only_longhands.extend(reset_only_longhand_ids);

            for (auto reset_only_longhand_id : reset_only_longhand_ids)
                longhand_values.append(property_initial_value(reset_only_longhand_id));

            while (tokens.has_next_token())
                tokens.discard_a_token();
            rust_transaction.commit();
            return ShorthandStyleValue::create(shorthand_id, longhand_ids_including_reset_only_longhands, longhand_values);
        }
    }

    HashMap<PropertyID, StyleValueVector> longhand_vectors;

    auto transaction = tokens.begin_transaction();

    do {
        Vector<PropertyID> remaining_longhands {};
        remaining_longhands.extend(longhand_ids);

        HashMap<PropertyID, NonnullRefPtr<StyleValue const>> parsed_values;

        while (tokens.has_next_token() && !tokens.next_token().is(Token::Type::Comma)) {
            auto property_and_value = parse_css_value_for_properties(remaining_longhands, tokens);

            if (!property_and_value.has_value())
                return {};

            remove_property(remaining_longhands, property_and_value->property);

            auto parsed_value = property_and_value->style_value;
            unwrap_single_coordinating_value_list_item(property_and_value->property, parsed_value);

            parsed_values.set(property_and_value->property, parsed_value.release_nonnull());
        }

        if (parsed_values.is_empty())
            return {};

        for (auto const& longhand_id : longhand_ids)
            longhand_vectors.ensure(longhand_id).append(*parsed_values.get(longhand_id).value_or_lazy_evaluated([&]() -> ValueComparingNonnullRefPtr<StyleValue const> {
                return property_initial_value(longhand_id)->as_value_list().values()[0];
            }));

        if (tokens.has_next_token()) {
            if (tokens.next_token().is(Token::Type::Comma))
                tokens.discard_a_token();
            else
                return {};
        }
    } while (tokens.has_next_token());

    transaction.commit();

    Vector<PropertyID> longhand_ids_including_reset_only_longhands;
    longhand_ids_including_reset_only_longhands.extend(longhand_ids);
    longhand_ids_including_reset_only_longhands.extend(reset_only_longhand_ids);
    StyleValueVector longhand_values {};

    for (auto const& longhand_id : longhand_ids)
        longhand_values.append(StyleValueList::create(move(*longhand_vectors.get(longhand_id)), StyleValueList::Separator::Comma));

    for (auto reset_only_longhand_id : reset_only_longhand_ids)
        longhand_values.append(property_initial_value(reset_only_longhand_id));

    return ShorthandStyleValue::create(shorthand_id, longhand_ids_including_reset_only_longhands, longhand_values);
}

RefPtr<StyleValue const> Parser::parse_css_value_for_property(PropertyID property_id, TokenStream<ComponentValue>& tokens)
{
    return parse_css_value_for_properties({ &property_id, 1 }, tokens)
        .map([](auto&& it) { return it.style_value; })
        .value_or(nullptr);
}

Optional<Parser::PropertyAndValue> Parser::parse_css_value_for_properties(ReadonlySpan<PropertyID> property_ids, TokenStream<ComponentValue>& tokens)
{
    auto any_property_accepts_type = [](ReadonlySpan<PropertyID> property_ids, ValueType value_type) -> Optional<PropertyID> {
        return RustComponentValueParser::property_accepting_type(property_ids, value_type);
    };
    auto property_numeric_metadata = [](ReadonlySpan<PropertyID> property_ids, ValueType value_type) -> Optional<RustComponentValueParser::PropertyNumericMetadata> {
        return RustComponentValueParser::property_numeric_metadata(property_ids, value_type);
    };
    tokens.discard_whitespace();
    auto& peek_token = tokens.next_token();

    auto parse_for_type = [&](ValueType const type) -> Optional<PropertyAndValue> {
        if (auto property = any_property_accepts_type(property_ids, type); property.has_value()) {
            auto context_guard = push_temporary_value_parsing_context(*property);
            if (auto maybe_parsed_value = parse_value(type, tokens))
                return PropertyAndValue { *property, maybe_parsed_value };
        }
        return OptionalNone {};
    };

    {
        auto generated_transaction = tokens.begin_transaction();
        auto has_view_timeline_inset_property = [&] {
            for (auto property_id : property_ids) {
                if (property_id == PropertyID::ViewTimelineInset)
                    return true;
            }
            return false;
        };
        auto source = property_ids.size() == 1
            ? serialize_component_values_for_reparsing(tokens.remaining_tokens())
            : has_view_timeline_inset_property()
            ? serialize_component_values_for_reparsing(tokens.remaining_tokens())
            : [&] {
                  auto component_value_source = peek_token.original_source_text();
                  return component_value_source.is_empty() ? peek_token.to_string() : component_value_source;
              }();
        if (auto rust_style_value = RustComponentValueParser::parse_style_value_for_property(property_ids, source.bytes_as_string_view()); rust_style_value.has_value()) {
            auto parse_rust_numeric_value = [&]() -> RefPtr<StyleValue const> {
                if (!rust_style_value->value_type.has_value())
                    return nullptr;

                auto metadata = RustComponentValueParser::property_numeric_metadata({ &rust_style_value->property_id, 1 }, *rust_style_value->value_type);
                if (!metadata.has_value())
                    return nullptr;

                switch (*rust_style_value->value_type) {
                case ValueType::Integer:
                    return parse_integer_value(tokens, metadata->range);
                case ValueType::Number:
                    return parse_number_value(tokens, metadata->range);
                case ValueType::Angle:
                case ValueType::AnglePercentage:
                    if (metadata->percentages_resolve_to_value_type) {
                        VERIFY(metadata->percentage_range.has_value());
                        return parse_angle_percentage_value(tokens, metadata->range, metadata->percentage_range.value());
                    }
                    return parse_angle_value(tokens, metadata->range);
                case ValueType::Flex:
                    return parse_flex_value(tokens, metadata->range);
                case ValueType::Frequency:
                case ValueType::FrequencyPercentage:
                    if (metadata->percentages_resolve_to_value_type) {
                        VERIFY(metadata->percentage_range.has_value());
                        return parse_frequency_percentage_value(tokens, metadata->range, metadata->percentage_range.value());
                    }
                    return parse_frequency_value(tokens, metadata->range);
                case ValueType::Length:
                case ValueType::LengthPercentage:
                    if (metadata->percentages_resolve_to_value_type) {
                        VERIFY(metadata->percentage_range.has_value());
                        return parse_length_percentage_value(tokens, metadata->range, metadata->percentage_range.value());
                    }
                    return parse_length_value(tokens, metadata->range);
                case ValueType::Resolution:
                    return parse_resolution_value(tokens, metadata->range);
                case ValueType::Time:
                case ValueType::TimePercentage:
                    if (metadata->percentages_resolve_to_value_type) {
                        VERIFY(metadata->percentage_range.has_value());
                        return parse_time_percentage_value(tokens, metadata->range, metadata->percentage_range.value());
                    }
                    return parse_time_value(tokens, metadata->range);
                case ValueType::Percentage:
                    return parse_percentage_value(tokens, metadata->range);
                case ValueType::OpacityValue:
                    return parse_opacity_value_value(tokens);
                default:
                    return nullptr;
                }
            };
            auto materialize_rust_numeric_value = [&]() -> RefPtr<StyleValue const> {
                if (!rust_style_value->value_type.has_value() || !rust_style_value->numeric_value.has_value())
                    return nullptr;

                auto metadata = RustComponentValueParser::property_numeric_metadata({ &rust_style_value->property_id, 1 }, *rust_style_value->value_type);
                if (!metadata.has_value())
                    return nullptr;

                switch (*rust_style_value->value_type) {
                case ValueType::Integer:
                    if (!metadata->range.contains(*rust_style_value->numeric_value))
                        return nullptr;
                    return IntegerStyleValue::create(static_cast<i32>(*rust_style_value->numeric_value));
                case ValueType::Number:
                    if (!metadata->range.contains(*rust_style_value->numeric_value))
                        return nullptr;
                    return NumberStyleValue::create(*rust_style_value->numeric_value);
                case ValueType::Percentage:
                    if (!metadata->range.contains(*rust_style_value->numeric_value))
                        return nullptr;
                    return PercentageStyleValue::create(Percentage(*rust_style_value->numeric_value));
                case ValueType::OpacityValue:
                    if (!metadata->range.contains(*rust_style_value->numeric_value))
                        return nullptr;
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Percentage)
                        return OpacityValueStyleValue::create(PercentageStyleValue::create(Percentage(*rust_style_value->numeric_value)));
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Number)
                        return OpacityValueStyleValue::create(NumberStyleValue::create(*rust_style_value->numeric_value));
                    return nullptr;
                case ValueType::Angle:
                case ValueType::AnglePercentage: {
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Percentage) {
                        if (!metadata->percentage_range.has_value() || !metadata->percentage_range->contains(*rust_style_value->numeric_value))
                            return nullptr;
                        return PercentageStyleValue::create(Percentage(*rust_style_value->numeric_value));
                    }
                    if (!rust_style_value->dimension_unit.has_value())
                        return nullptr;
                    auto angle_unit = string_to_angle_unit(*rust_style_value->dimension_unit);
                    if (!angle_unit.has_value())
                        return nullptr;
                    Angle angle { *rust_style_value->numeric_value, angle_unit.release_value() };
                    if (!metadata->range.contains(angle.raw_value()))
                        return nullptr;
                    return AngleStyleValue::create(angle);
                }
                case ValueType::Flex: {
                    if (!rust_style_value->dimension_unit.has_value())
                        return nullptr;
                    auto flex_unit = string_to_flex_unit(*rust_style_value->dimension_unit);
                    if (!flex_unit.has_value())
                        return nullptr;
                    Flex flex { *rust_style_value->numeric_value, flex_unit.release_value() };
                    if (!metadata->range.contains(flex.raw_value()))
                        return nullptr;
                    return FlexStyleValue::create(flex);
                }
                case ValueType::Frequency:
                case ValueType::FrequencyPercentage: {
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Percentage) {
                        if (!metadata->percentage_range.has_value() || !metadata->percentage_range->contains(*rust_style_value->numeric_value))
                            return nullptr;
                        return PercentageStyleValue::create(Percentage(*rust_style_value->numeric_value));
                    }
                    if (!rust_style_value->dimension_unit.has_value())
                        return nullptr;
                    auto frequency_unit = string_to_frequency_unit(*rust_style_value->dimension_unit);
                    if (!frequency_unit.has_value())
                        return nullptr;
                    Frequency frequency { *rust_style_value->numeric_value, frequency_unit.release_value() };
                    if (!metadata->range.contains(frequency.raw_value()))
                        return nullptr;
                    return FrequencyStyleValue::create(frequency);
                }
                case ValueType::Length:
                case ValueType::LengthPercentage: {
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Percentage) {
                        if (!metadata->percentage_range.has_value() || !metadata->percentage_range->contains(*rust_style_value->numeric_value))
                            return nullptr;
                        return PercentageStyleValue::create(Percentage(*rust_style_value->numeric_value));
                    }
                    if (!rust_style_value->dimension_unit.has_value())
                        return nullptr;
                    auto length_unit = string_to_length_unit(*rust_style_value->dimension_unit);
                    if (!length_unit.has_value())
                        return nullptr;
                    Length length { *rust_style_value->numeric_value, length_unit.release_value() };
                    if (!metadata->range.contains(length.raw_value()))
                        return nullptr;
                    return LengthStyleValue::create(length);
                }
                case ValueType::Resolution: {
                    if (!rust_style_value->dimension_unit.has_value())
                        return nullptr;
                    auto resolution_unit = string_to_resolution_unit(*rust_style_value->dimension_unit);
                    if (!resolution_unit.has_value())
                        return nullptr;
                    Resolution resolution { *rust_style_value->numeric_value, resolution_unit.release_value() };
                    if (!metadata->range.contains(resolution.raw_value()))
                        return nullptr;
                    return ResolutionStyleValue::create(resolution);
                }
                case ValueType::Time:
                case ValueType::TimePercentage: {
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Percentage) {
                        if (!metadata->percentage_range.has_value() || !metadata->percentage_range->contains(*rust_style_value->numeric_value))
                            return nullptr;
                        return PercentageStyleValue::create(Percentage(*rust_style_value->numeric_value));
                    }
                    if (!rust_style_value->dimension_unit.has_value())
                        return nullptr;
                    auto time_unit = string_to_time_unit(*rust_style_value->dimension_unit);
                    if (!time_unit.has_value())
                        return nullptr;
                    Time time { *rust_style_value->numeric_value, time_unit.release_value() };
                    if (!metadata->range.contains(time.to_seconds()))
                        return nullptr;
                    return TimeStyleValue::create(move(time));
                }
                default:
                    return nullptr;
                }
            };
            auto materialize_rust_ratio_value = [&]() -> RefPtr<StyleValue const> {
                if (!rust_style_value->value_type.has_value()
                    || *rust_style_value->value_type != ValueType::Ratio
                    || rust_style_value->primitive_kind != FFI::CssPrimitiveValueKind::Ratio
                    || !rust_style_value->numeric_value.has_value()
                    || !rust_style_value->secondary_numeric_value.has_value())
                    return nullptr;

                return RatioStyleValue::create(
                    NumberStyleValue::create(*rust_style_value->numeric_value),
                    NumberStyleValue::create(*rust_style_value->secondary_numeric_value));
            };
            auto materialize_rust_scroll_function_value = [&]() -> RefPtr<StyleValue const> {
                StyleValueTuple tuple;
                tuple.resize_with_default_value(2, nullptr);

                switch (rust_style_value->scroll_function_scroller) {
                case FFI::CssScrollFunctionScrollerKind::None:
                case FFI::CssScrollFunctionScrollerKind::Nearest:
                    break;
                case FFI::CssScrollFunctionScrollerKind::Root:
                    tuple[TupleStyleValue::Indices::ScrollFunction::Scroller] = KeywordStyleValue::create(Keyword::Root);
                    break;
                case FFI::CssScrollFunctionScrollerKind::Self_:
                    tuple[TupleStyleValue::Indices::ScrollFunction::Scroller] = KeywordStyleValue::create(Keyword::Self);
                    break;
                }

                switch (rust_style_value->scroll_function_axis) {
                case FFI::CssScrollFunctionAxisKind::None:
                case FFI::CssScrollFunctionAxisKind::Block:
                    break;
                case FFI::CssScrollFunctionAxisKind::Inline:
                    tuple[TupleStyleValue::Indices::ScrollFunction::Axis] = KeywordStyleValue::create(Keyword::Inline);
                    break;
                case FFI::CssScrollFunctionAxisKind::X:
                    tuple[TupleStyleValue::Indices::ScrollFunction::Axis] = KeywordStyleValue::create(Keyword::X);
                    break;
                case FFI::CssScrollFunctionAxisKind::Y:
                    tuple[TupleStyleValue::Indices::ScrollFunction::Axis] = KeywordStyleValue::create(Keyword::Y);
                    break;
                }

                return FunctionStyleValue::create("scroll"_fly_string, TupleStyleValue::create(move(tuple)));
            };
            auto materialize_rust_view_function_value = [&]() -> RefPtr<StyleValue const> {
                if (!tokens.next_token().is_function("view"sv))
                    return nullptr;

                auto context_guard = push_temporary_value_parsing_context(FunctionContext { "view"sv });

                StyleValueTuple tuple;
                tuple.resize_with_default_value(2, nullptr);

                switch (rust_style_value->scroll_function_axis) {
                case FFI::CssScrollFunctionAxisKind::None:
                case FFI::CssScrollFunctionAxisKind::Block:
                    break;
                case FFI::CssScrollFunctionAxisKind::Inline:
                    tuple[TupleStyleValue::Indices::ViewFunction::Axis] = KeywordStyleValue::create(Keyword::Inline);
                    break;
                case FFI::CssScrollFunctionAxisKind::X:
                    tuple[TupleStyleValue::Indices::ViewFunction::Axis] = KeywordStyleValue::create(Keyword::X);
                    break;
                case FFI::CssScrollFunctionAxisKind::Y:
                    tuple[TupleStyleValue::Indices::ViewFunction::Axis] = KeywordStyleValue::create(Keyword::Y);
                    break;
                }

                switch (rust_style_value->view_function_inset) {
                case FFI::CssViewFunctionInsetKind::None:
                case FFI::CssViewFunctionInsetKind::Default:
                    break;
                case FFI::CssViewFunctionInsetKind::NonDefault: {
                    auto argument_tokens = TokenStream { tokens.next_token().function().value };
                    if (rust_style_value->view_function_inset_position == FFI::CssViewFunctionInsetPosition::AfterAxis) {
                        argument_tokens.discard_whitespace();
                        argument_tokens.discard_a_token();
                    }

                    auto inset_value = parse_view_timeline_inset_value(argument_tokens);
                    if (!inset_value)
                        return nullptr;

                    tuple[TupleStyleValue::Indices::ViewFunction::Inset] = inset_value.release_nonnull();
                    break;
                }
                }

                return FunctionStyleValue::create("view"_fly_string, TupleStyleValue::create(move(tuple)));
            };
            auto keyword_from_scroll_function_axis = [](FFI::CssScrollFunctionAxisKind axis) -> Optional<Keyword> {
                switch (axis) {
                case FFI::CssScrollFunctionAxisKind::None:
                    return {};
                case FFI::CssScrollFunctionAxisKind::Block:
                    return Keyword::Block;
                case FFI::CssScrollFunctionAxisKind::Inline:
                    return Keyword::Inline;
                case FFI::CssScrollFunctionAxisKind::X:
                    return Keyword::X;
                case FFI::CssScrollFunctionAxisKind::Y:
                    return Keyword::Y;
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_white_space_trim = [](FFI::CssWhiteSpaceTrimValue const& white_space_trim) -> RefPtr<StyleValue const> {
                switch (white_space_trim.kind) {
                case FFI::CssWhiteSpaceTrimValueKind::Invalid:
                    return nullptr;
                case FFI::CssWhiteSpaceTrimValueKind::None:
                    return KeywordStyleValue::create(Keyword::None);
                case FFI::CssWhiteSpaceTrimValueKind::List: {
                    StyleValueVector values;
                    if (white_space_trim.has_discard_before)
                        values.append(KeywordStyleValue::create(Keyword::DiscardBefore));
                    if (white_space_trim.has_discard_after)
                        values.append(KeywordStyleValue::create(Keyword::DiscardAfter));
                    if (white_space_trim.has_discard_inner)
                        values.append(KeywordStyleValue::create(Keyword::DiscardInner));
                    return StyleValueList::create(move(values), StyleValueList::Separator::Space);
                }
                }
                VERIFY_NOT_REACHED();
            };
            auto discard_rust_owned_property_value_tokens = [&] {
                if (property_ids.size() > 1) {
                    tokens.discard_a_token();
                    return;
                }

                while (tokens.has_next_token())
                    tokens.discard_a_token();
            };
            auto discard_rust_view_timeline_inset_value_tokens = [&] {
                if (property_ids.size() > 1) {
                    for (size_t i = 0; i < rust_style_value->view_timeline_insets.size(); ++i) {
                        tokens.discard_whitespace();
                        tokens.discard_a_token();
                    }
                    return;
                }

                while (tokens.has_next_token())
                    tokens.discard_a_token();
            };
            auto repetition_from_rust = [](u8 repetition) {
                switch (repetition) {
                case 0:
                    return Repetition::NoRepeat;
                case 1:
                    return Repetition::Repeat;
                case 2:
                    return Repetition::Round;
                case 3:
                    return Repetition::Space;
                }

                VERIFY_NOT_REACHED();
            };
            auto horizontal_text_underline_position_from_rust = [](FFI::CssTextUnderlinePositionHorizontal horizontal) {
                switch (horizontal) {
                case FFI::CssTextUnderlinePositionHorizontal::Invalid:
                    break;
                case FFI::CssTextUnderlinePositionHorizontal::Auto:
                    return TextUnderlinePositionHorizontal::Auto;
                case FFI::CssTextUnderlinePositionHorizontal::FromFont:
                    return TextUnderlinePositionHorizontal::FromFont;
                case FFI::CssTextUnderlinePositionHorizontal::Under:
                    return TextUnderlinePositionHorizontal::Under;
                }

                VERIFY_NOT_REACHED();
            };
            auto vertical_text_underline_position_from_rust = [](FFI::CssTextUnderlinePositionVertical vertical) {
                switch (vertical) {
                case FFI::CssTextUnderlinePositionVertical::Invalid:
                    break;
                case FFI::CssTextUnderlinePositionVertical::Auto:
                    return TextUnderlinePositionVertical::Auto;
                case FFI::CssTextUnderlinePositionVertical::Left:
                    return TextUnderlinePositionVertical::Left;
                case FFI::CssTextUnderlinePositionVertical::Right:
                    return TextUnderlinePositionVertical::Right;
                }

                VERIFY_NOT_REACHED();
            };
            auto paint_order_keyword_from_rust = [](FFI::CssPaintOrderKeyword keyword) {
                switch (keyword) {
                case FFI::CssPaintOrderKeyword::Invalid:
                    break;
                case FFI::CssPaintOrderKeyword::Fill:
                    return Keyword::Fill;
                case FFI::CssPaintOrderKeyword::Stroke:
                    return Keyword::Stroke;
                case FFI::CssPaintOrderKeyword::Markers:
                    return Keyword::Markers;
                }

                VERIFY_NOT_REACHED();
            };
            auto position_try_order_keyword_from_rust = [](FFI::CssPositionTryOrderValue value) -> Optional<Keyword> {
                switch (value) {
                case FFI::CssPositionTryOrderValue::Invalid:
                    return {};
                case FFI::CssPositionTryOrderValue::Normal:
                    return Keyword::Normal;
                case FFI::CssPositionTryOrderValue::MostWidth:
                    return Keyword::MostWidth;
                case FFI::CssPositionTryOrderValue::MostHeight:
                    return Keyword::MostHeight;
                case FFI::CssPositionTryOrderValue::MostBlockSize:
                    return Keyword::MostBlockSize;
                case FFI::CssPositionTryOrderValue::MostInlineSize:
                    return Keyword::MostInlineSize;
                }

                VERIFY_NOT_REACHED();
            };
            auto display_box_from_rust = [](u8 value) {
                enum : u8 {
                    Contents,
                    None,
                };
                switch (value) {
                case Contents:
                    return DisplayBox::Contents;
                case None:
                    return DisplayBox::None;
                }
                VERIFY_NOT_REACHED();
            };
            auto display_inside_from_rust = [](RustComponentValueParser::RustDisplayInside value) {
                switch (value) {
                case RustComponentValueParser::RustDisplayInside::Flow:
                    return DisplayInside::Flow;
                case RustComponentValueParser::RustDisplayInside::FlowRoot:
                    return DisplayInside::FlowRoot;
                case RustComponentValueParser::RustDisplayInside::Table:
                    return DisplayInside::Table;
                case RustComponentValueParser::RustDisplayInside::Flex:
                    return DisplayInside::Flex;
                case RustComponentValueParser::RustDisplayInside::Grid:
                    return DisplayInside::Grid;
                case RustComponentValueParser::RustDisplayInside::Ruby:
                    return DisplayInside::Ruby;
                case RustComponentValueParser::RustDisplayInside::Math:
                    return DisplayInside::Math;
                }
                VERIFY_NOT_REACHED();
            };
            auto display_internal_from_rust = [](u8 value) {
                enum : u8 {
                    TableRowGroup,
                    TableHeaderGroup,
                    TableFooterGroup,
                    TableRow,
                    TableCell,
                    TableColumnGroup,
                    TableColumn,
                    TableCaption,
                    RubyBase,
                    RubyText,
                    RubyBaseContainer,
                    RubyTextContainer,
                };
                switch (value) {
                case TableRowGroup:
                    return DisplayInternal::TableRowGroup;
                case TableHeaderGroup:
                    return DisplayInternal::TableHeaderGroup;
                case TableFooterGroup:
                    return DisplayInternal::TableFooterGroup;
                case TableRow:
                    return DisplayInternal::TableRow;
                case TableCell:
                    return DisplayInternal::TableCell;
                case TableColumnGroup:
                    return DisplayInternal::TableColumnGroup;
                case TableColumn:
                    return DisplayInternal::TableColumn;
                case TableCaption:
                    return DisplayInternal::TableCaption;
                case RubyBase:
                    return DisplayInternal::RubyBase;
                case RubyText:
                    return DisplayInternal::RubyText;
                case RubyBaseContainer:
                    return DisplayInternal::RubyBaseContainer;
                case RubyTextContainer:
                    return DisplayInternal::RubyTextContainer;
                }
                VERIFY_NOT_REACHED();
            };
            auto display_outside_from_rust = [](u8 value) {
                enum : u8 {
                    Block,
                    Inline,
                    RunIn,
                };
                switch (value) {
                case Block:
                    return DisplayOutside::Block;
                case Inline:
                    return DisplayOutside::Inline;
                case RunIn:
                    return DisplayOutside::RunIn;
                }
                VERIFY_NOT_REACHED();
            };
            auto text_wrap_mode_keyword_from_rust = [](FFI::CssTextWrapModeValue value) -> Optional<Keyword> {
                switch (value) {
                case FFI::CssTextWrapModeValue::Invalid:
                    return {};
                case FFI::CssTextWrapModeValue::Wrap:
                    return Keyword::Wrap;
                case FFI::CssTextWrapModeValue::Nowrap:
                    return Keyword::Nowrap;
                }
                VERIFY_NOT_REACHED();
            };
            auto text_wrap_style_keyword_from_rust = [](FFI::CssTextWrapStyleValue value) -> Optional<Keyword> {
                switch (value) {
                case FFI::CssTextWrapStyleValue::Invalid:
                    return {};
                case FFI::CssTextWrapStyleValue::Auto:
                    return Keyword::Auto;
                case FFI::CssTextWrapStyleValue::Balance:
                    return Keyword::Balance;
                case FFI::CssTextWrapStyleValue::Stable:
                    return Keyword::Stable;
                case FFI::CssTextWrapStyleValue::Pretty:
                    return Keyword::Pretty;
                }
                VERIFY_NOT_REACHED();
            };
            auto touch_action_keyword_from_rust = [](FFI::CssTouchActionKeyword keyword) {
                switch (keyword) {
                case FFI::CssTouchActionKeyword::Invalid:
                    break;
                case FFI::CssTouchActionKeyword::PanX:
                    return Keyword::PanX;
                case FFI::CssTouchActionKeyword::PanLeft:
                    return Keyword::PanLeft;
                case FFI::CssTouchActionKeyword::PanRight:
                    return Keyword::PanRight;
                case FFI::CssTouchActionKeyword::PanY:
                    return Keyword::PanY;
                case FFI::CssTouchActionKeyword::PanUp:
                    return Keyword::PanUp;
                case FFI::CssTouchActionKeyword::PanDown:
                    return Keyword::PanDown;
                }

                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_font_variant_alternates_value = [&]() -> RefPtr<StyleValue const> {
                StyleValueVector values;
                values.ensure_capacity(rust_style_value->font_variant_alternates.size());

                for (auto const& value : rust_style_value->font_variant_alternates) {
                    if (value.kind == FFI::CssFontVariantAlternatesValueKind::HistoricalForms) {
                        values.append(KeywordStyleValue::create(Keyword::HistoricalForms));
                        continue;
                    }

                    StyleValueVector feature_value_names;
                    feature_value_names.ensure_capacity(value.feature_value_names.size());
                    for (auto const& feature_value_name : value.feature_value_names)
                        feature_value_names.append(CustomIdentStyleValue::create(feature_value_name));

                    FlyString function_name;
                    switch (value.kind) {
                    case FFI::CssFontVariantAlternatesValueKind::Stylistic:
                        function_name = "stylistic"_fly_string;
                        break;
                    case FFI::CssFontVariantAlternatesValueKind::Styleset:
                        function_name = "styleset"_fly_string;
                        break;
                    case FFI::CssFontVariantAlternatesValueKind::CharacterVariant:
                        function_name = "character-variant"_fly_string;
                        break;
                    case FFI::CssFontVariantAlternatesValueKind::Swash:
                        function_name = "swash"_fly_string;
                        break;
                    case FFI::CssFontVariantAlternatesValueKind::Ornaments:
                        function_name = "ornaments"_fly_string;
                        break;
                    case FFI::CssFontVariantAlternatesValueKind::Annotation:
                        function_name = "annotation"_fly_string;
                        break;
                    case FFI::CssFontVariantAlternatesValueKind::HistoricalForms:
                        VERIFY_NOT_REACHED();
                    }

                    values.append(FunctionStyleValue::create(move(function_name), StyleValueList::create(move(feature_value_names), StyleValueList::Separator::Comma)));
                }

                return StyleValueList::create(move(values), StyleValueList::Separator::Space);
            };
            auto materialize_rust_font_variant_east_asian_value = [&]() -> RefPtr<StyleValue const> {
                StyleValueTuple tuple;
                tuple.resize_with_default_value(3, nullptr);

                for (auto const& value : rust_style_value->font_variant_east_asian) {
                    auto maybe_keyword = keyword_from_string(value.value);
                    if (!maybe_keyword.has_value())
                        return nullptr;
                    auto style_value = KeywordStyleValue::create(*maybe_keyword);
                    switch (value.kind) {
                    case FFI::CssFontVariantEastAsianValueKind::Variant:
                        tuple[TupleStyleValue::Indices::FontVariantEastAsian::Variant] = style_value;
                        break;
                    case FFI::CssFontVariantEastAsianValueKind::Width:
                        tuple[TupleStyleValue::Indices::FontVariantEastAsian::Width] = style_value;
                        break;
                    case FFI::CssFontVariantEastAsianValueKind::Ruby:
                        tuple[TupleStyleValue::Indices::FontVariantEastAsian::Ruby] = style_value;
                        break;
                    }
                }

                return TupleStyleValue::create(tuple);
            };
            auto materialize_rust_font_variant_ligatures_value = [&]() -> RefPtr<StyleValue const> {
                StyleValueTuple tuple;
                tuple.resize_with_default_value(4, nullptr);

                for (auto const& value : rust_style_value->font_variant_ligatures) {
                    auto maybe_keyword = keyword_from_string(value.value);
                    if (!maybe_keyword.has_value())
                        return nullptr;
                    auto style_value = KeywordStyleValue::create(*maybe_keyword);
                    switch (value.kind) {
                    case FFI::CssFontVariantLigaturesValueKind::Common:
                        tuple[TupleStyleValue::Indices::FontVariantLigatures::Common] = style_value;
                        break;
                    case FFI::CssFontVariantLigaturesValueKind::Discretionary:
                        tuple[TupleStyleValue::Indices::FontVariantLigatures::Discretionary] = style_value;
                        break;
                    case FFI::CssFontVariantLigaturesValueKind::Historical:
                        tuple[TupleStyleValue::Indices::FontVariantLigatures::Historical] = style_value;
                        break;
                    case FFI::CssFontVariantLigaturesValueKind::Contextual:
                        tuple[TupleStyleValue::Indices::FontVariantLigatures::Contextual] = style_value;
                        break;
                    }
                }

                return TupleStyleValue::create(tuple);
            };
            auto materialize_rust_font_variant_numeric_value = [&]() -> RefPtr<StyleValue const> {
                StyleValueTuple tuple;
                tuple.resize_with_default_value(5, nullptr);

                for (auto const& value : rust_style_value->font_variant_numeric) {
                    auto maybe_keyword = keyword_from_string(value.value);
                    if (!maybe_keyword.has_value())
                        return nullptr;
                    auto style_value = KeywordStyleValue::create(*maybe_keyword);
                    switch (value.kind) {
                    case FFI::CssFontVariantNumericValueKind::Figure:
                        tuple[TupleStyleValue::Indices::FontVariantNumeric::Figure] = style_value;
                        break;
                    case FFI::CssFontVariantNumericValueKind::Spacing:
                        tuple[TupleStyleValue::Indices::FontVariantNumeric::Spacing] = style_value;
                        break;
                    case FFI::CssFontVariantNumericValueKind::Fraction:
                        tuple[TupleStyleValue::Indices::FontVariantNumeric::Fraction] = style_value;
                        break;
                    case FFI::CssFontVariantNumericValueKind::Ordinal:
                        tuple[TupleStyleValue::Indices::FontVariantNumeric::Ordinal] = style_value;
                        break;
                    case FFI::CssFontVariantNumericValueKind::SlashedZero:
                        tuple[TupleStyleValue::Indices::FontVariantNumeric::SlashedZero] = style_value;
                        break;
                    }
                }

                return TupleStyleValue::create(tuple);
            };
            auto materialize_rust_font_variant_value = [&]() -> RefPtr<StyleValue const> {
                RefPtr<StyleValue const> alternates_value;
                RefPtr<StyleValue const> caps_value;
                RefPtr<StyleValue const> emoji_value;
                RefPtr<StyleValue const> position_value;
                RefPtr<StyleValue const> east_asian_value;
                RefPtr<StyleValue const> ligatures_value;
                RefPtr<StyleValue const> numeric_value;

                auto keyword_style_value_from_string = [](FlyString const& value) -> RefPtr<StyleValue const> {
                    auto maybe_keyword = keyword_from_string(value);
                    if (!maybe_keyword.has_value())
                        return nullptr;
                    return KeywordStyleValue::create(*maybe_keyword);
                };

                if (!rust_style_value->font_variant_alternates.is_empty())
                    alternates_value = materialize_rust_font_variant_alternates_value();
                if (rust_style_value->font_variant_caps.has_value())
                    caps_value = keyword_style_value_from_string(*rust_style_value->font_variant_caps);
                if (rust_style_value->font_variant_emoji.has_value())
                    emoji_value = keyword_style_value_from_string(*rust_style_value->font_variant_emoji);
                if (rust_style_value->font_variant_position.has_value())
                    position_value = keyword_style_value_from_string(*rust_style_value->font_variant_position);
                if (rust_style_value->font_variant_ligatures_none)
                    ligatures_value = KeywordStyleValue::create(Keyword::None);
                if (!rust_style_value->font_variant_east_asian.is_empty())
                    east_asian_value = materialize_rust_font_variant_east_asian_value();
                if (!rust_style_value->font_variant_ligatures.is_empty())
                    ligatures_value = materialize_rust_font_variant_ligatures_value();
                if (!rust_style_value->font_variant_numeric.is_empty())
                    numeric_value = materialize_rust_font_variant_numeric_value();

                if ((rust_style_value->font_variant_caps.has_value() && !caps_value)
                    || (rust_style_value->font_variant_emoji.has_value() && !emoji_value)
                    || (rust_style_value->font_variant_position.has_value() && !position_value)
                    || (!rust_style_value->font_variant_alternates.is_empty() && !alternates_value)
                    || (!rust_style_value->font_variant_east_asian.is_empty() && !east_asian_value)
                    || (!rust_style_value->font_variant_ligatures.is_empty() && !ligatures_value)
                    || (!rust_style_value->font_variant_numeric.is_empty() && !numeric_value))
                    return nullptr;

                auto normal_value = KeywordStyleValue::create(Keyword::Normal);
                if (!alternates_value)
                    alternates_value = normal_value;
                if (!caps_value)
                    caps_value = normal_value;
                if (!emoji_value)
                    emoji_value = normal_value;
                if (!position_value)
                    position_value = normal_value;
                if (!east_asian_value)
                    east_asian_value = normal_value;
                if (!ligatures_value)
                    ligatures_value = normal_value;
                if (!numeric_value)
                    numeric_value = normal_value;

                return ShorthandStyleValue::create(PropertyID::FontVariant,
                    { PropertyID::FontVariantAlternates,
                        PropertyID::FontVariantCaps,
                        PropertyID::FontVariantEastAsian,
                        PropertyID::FontVariantEmoji,
                        PropertyID::FontVariantLigatures,
                        PropertyID::FontVariantNumeric,
                        PropertyID::FontVariantPosition },
                    {
                        alternates_value.release_nonnull(),
                        caps_value.release_nonnull(),
                        east_asian_value.release_nonnull(),
                        emoji_value.release_nonnull(),
                        ligatures_value.release_nonnull(),
                        numeric_value.release_nonnull(),
                        position_value.release_nonnull(),
                    });
            };
            auto parse_rust_source_as_property = [&](PropertyID property_id, String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_css_value_for_property(property_id, value_tokens);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_integer_in_range = [&](String const& source, NumericRange const& range) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_integer_value(value_tokens, range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_number = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_number_value(value_tokens, infinite_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_number_in_range = [&](String const& source, NumericRange const& range) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_number_value(value_tokens, range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_percentage = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_percentage_value(value_tokens, infinite_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto materialize_rust_easing_function = [&]() -> RefPtr<StyleValue const> {
                auto materialize_easing_number = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
                    if (!value.numeric_value.has_value())
                        return parse_rust_source_as_number_in_range(value.source_or_unit, range);
                    if (value.primitive_kind != FFI::CssPrimitiveValueKind::Number || !range.contains(*value.numeric_value))
                        return nullptr;
                    return NumberStyleValue::create(*value.numeric_value);
                };
                auto materialize_easing_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                    if (!value.numeric_value.has_value())
                        return parse_rust_source_as_percentage(value.source_or_unit);
                    if (value.primitive_kind != FFI::CssPrimitiveValueKind::Percentage)
                        return nullptr;
                    return PercentageStyleValue::create(Percentage { *value.numeric_value });
                };
                auto materialize_easing_integer = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
                    if (!value.numeric_value.has_value())
                        return parse_rust_source_as_integer_in_range(value.source_or_unit, range);
                    if (value.primitive_kind != FFI::CssPrimitiveValueKind::Integer || !range.contains(*value.numeric_value))
                        return nullptr;
                    return IntegerStyleValue::create(static_cast<i32>(*value.numeric_value));
                };

                switch (rust_style_value->easing_function_kind) {
                case 0:
                    return EasingStyleValue::create(EasingStyleValue::Steps { IntegerStyleValue::create(1), rust_style_value->easing_function_step_position });
                case 1: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "linear"sv });
                    Vector<EasingStyleValue::Linear::Stop> stops;
                    for (auto const& stop : rust_style_value->linear_easing_stops) {
                        auto output = materialize_easing_number(stop.output, infinite_range);
                        if (!output)
                            return nullptr;

                        RefPtr<StyleValue const> first_input;
                        if (stop.first_stop_length.has_value()) {
                            first_input = materialize_easing_percentage(*stop.first_stop_length);
                            if (!first_input)
                                return nullptr;
                        }

                        auto output_value = output.release_nonnull();
                        stops.append({ output_value, first_input });
                        if (stop.second_stop_length.has_value()) {
                            auto second_input = materialize_easing_percentage(*stop.second_stop_length);
                            if (!second_input)
                                return nullptr;
                            stops.append({ output_value, second_input.release_nonnull() });
                        }
                    }
                    if (stops.is_empty())
                        return nullptr;
                    return EasingStyleValue::create(EasingStyleValue::Linear { move(stops) });
                }
                case 2: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "cubic-bezier"sv });
                    if (rust_style_value->easing_function_values.size() != 4)
                        return nullptr;
                    auto x1 = materialize_easing_number(rust_style_value->easing_function_values[0], { .min = 0, .max = 1 });
                    auto y1 = materialize_easing_number(rust_style_value->easing_function_values[1], infinite_range);
                    auto x2 = materialize_easing_number(rust_style_value->easing_function_values[2], { .min = 0, .max = 1 });
                    auto y2 = materialize_easing_number(rust_style_value->easing_function_values[3], infinite_range);
                    if (!x1 || !y1 || !x2 || !y2)
                        return nullptr;
                    return EasingStyleValue::create(EasingStyleValue::CubicBezier {
                        x1.release_nonnull(),
                        y1.release_nonnull(),
                        x2.release_nonnull(),
                        y2.release_nonnull(),
                    });
                }
                case 3: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "steps"sv });
                    if (rust_style_value->easing_function_values.size() != 1)
                        return nullptr;
                    auto position = rust_style_value->easing_function_step_position;
                    auto min_intervals = position == StepPosition::JumpNone ? 2.0 : 1.0;
                    auto intervals = materialize_easing_integer(rust_style_value->easing_function_values[0], NumericRange { .min = min_intervals, .max = AK::NumericLimits<i32>::max() });
                    if (!intervals)
                        return nullptr;
                    return EasingStyleValue::create(EasingStyleValue::Steps { intervals.release_nonnull(), position });
                }
                default:
                    return nullptr;
                }
            };
            auto parse_rust_source_as_image = [&](String const& source) -> RefPtr<AbstractImageStyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_image_value(value_tokens);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto materialize_rust_image = [&](RustComponentValueParser::RustImageKind kind, String const& source, Optional<URL> const& typed_url) -> RefPtr<AbstractImageStyleValue const> {
                switch (kind) {
                case RustComponentValueParser::RustImageKind::Url: {
                    auto url = typed_url.has_value()
                        ? typed_url
                        : RustComponentValueParser::parse_a_url_function(source.bytes_as_string_view(), "utf-8"sv);
                    if (!url.has_value() || url->url().starts_with('#'))
                        return nullptr;
                    return ImageStyleValue::create(url.release_value());
                }
                case RustComponentValueParser::RustImageKind::Gradient:
                case RustComponentValueParser::RustImageKind::ImageSet:
                    return parse_rust_source_as_image(source);
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_image_from_original_tokens = [&](RustComponentValueParser::RustImageKind kind, String const& source, Optional<URL> const& typed_url) -> RefPtr<AbstractImageStyleValue const> {
                switch (kind) {
                case RustComponentValueParser::RustImageKind::Url:
                    return materialize_rust_image(kind, source, typed_url);
                case RustComponentValueParser::RustImageKind::Gradient:
                case RustComponentValueParser::RustImageKind::ImageSet:
                    // AD-HOC: Re-parsing substituted component values through Rust
                    // would lose C++-side attr() taint metadata until that
                    // metadata is carried over FFI.
                    return parse_image_value(tokens);
                }
                VERIFY_NOT_REACHED();
            };
            auto parse_rust_basic_shape_group = [&](String const& source) {
                return RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
            };
            auto parse_rust_basic_shape_fill_rule_argument = [&](String const& source) -> Optional<Gfx::WindingRule> {
                auto component_values = parse_rust_basic_shape_group(source);
                TokenStream tokens { component_values };

                tokens.discard_whitespace();
                auto& maybe_ident = tokens.consume_a_token();
                tokens.discard_whitespace();

                if (tokens.has_next_token())
                    return {};
                if (maybe_ident.is_ident("nonzero"sv))
                    return Gfx::WindingRule::Nonzero;
                if (maybe_ident.is_ident("evenodd"sv))
                    return Gfx::WindingRule::EvenOdd;
                return {};
            };
            auto materialize_rust_basic_shape = [&](RustComponentValueParser::RustBasicShapeKind kind, Vector<String> const& argument_groups, Optional<u8> fill_rule_value, Vector<RustComponentValueParser::RustBasicShapeRectangleComponent> const& rectangle_components, Vector<RustComponentValueParser::RustNestedPrimitiveValue> const& rectangle_border_radius_horizontal_radii, Vector<RustComponentValueParser::RustNestedPrimitiveValue> const& rectangle_border_radius_vertical_radii, bool radial_shape_is_typed, Vector<RustComponentValueParser::RustBasicShapeRadiusComponent> const& radial_shape_radius, Optional<RustComponentValueParser::RustPosition> const& radial_shape_position, Vector<RustComponentValueParser::RustNestedPrimitiveValue> const& polygon_coordinates, Optional<String> const& path_data_string) -> RefPtr<StyleValue const> {
                auto materialize_rust_fill_rule = [](Optional<u8> fill_rule_value) -> Optional<Gfx::WindingRule> {
                    if (!fill_rule_value.has_value() || *fill_rule_value == 0)
                        return Gfx::WindingRule::Nonzero;
                    if (*fill_rule_value == 1)
                        return Gfx::WindingRule::EvenOdd;
                    return {};
                };
                auto materialize_rust_basic_shape_length_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
                    if (!value.numeric_value.has_value()) {
                        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(value.source_or_unit.bytes_as_string_view(), "utf-8"sv);
                        TokenStream value_tokens { component_values };
                        auto parsed_value = parse_length_percentage_value(value_tokens, range, range);
                        value_tokens.discard_whitespace();
                        if (!parsed_value || value_tokens.has_next_token())
                            return nullptr;
                        return parsed_value.release_nonnull();
                    }
                    if (value.primitive_kind == FFI::CssPrimitiveValueKind::Length) {
                        auto length_unit = string_to_length_unit(value.source_or_unit);
                        if (!length_unit.has_value())
                            return nullptr;
                        Length length { *value.numeric_value, length_unit.release_value() };
                        if (!range.contains(length.raw_value()))
                            return nullptr;
                        return LengthStyleValue::create(length);
                    }
                    if (value.primitive_kind == FFI::CssPrimitiveValueKind::Percentage) {
                        if (!range.contains(*value.numeric_value))
                            return nullptr;
                        return PercentageStyleValue::create(Percentage { *value.numeric_value });
                    }
                    return nullptr;
                };
                auto materialize_rust_basic_shape_rectangle_component = [&](RustComponentValueParser::RustBasicShapeRectangleComponent const& component, NumericRange const& range) -> RefPtr<StyleValue const> {
                    if (component.is_auto)
                        return KeywordStyleValue::create(Keyword::Auto);
                    return materialize_rust_basic_shape_length_percentage(component.value, range);
                };
                auto materialize_rust_basic_shape_border_radius = [&]() -> RefPtr<StyleValue const> {
                    if (rectangle_border_radius_horizontal_radii.is_empty())
                        return BorderRadiusRectStyleValue::create_zero();

                    auto materialize_radius_values = [&](Vector<RustComponentValueParser::RustNestedPrimitiveValue> const& radii) -> Optional<StyleValueVector> {
                        StyleValueVector values;
                        values.ensure_capacity(radii.size());
                        for (auto const& radius : radii) {
                            auto value = materialize_rust_basic_shape_length_percentage(radius, non_negative_range);
                            if (!value)
                                return {};
                            values.append(value.release_nonnull());
                        }
                        return values;
                    };
                    auto top_left = [](StyleValueVector& radii) { return radii[0]; };
                    auto top_right = [](StyleValueVector& radii) {
                        switch (radii.size()) {
                        case 4:
                        case 3:
                        case 2:
                            return radii[1];
                        case 1:
                            return radii[0];
                        default:
                            VERIFY_NOT_REACHED();
                        }
                    };
                    auto bottom_right = [](StyleValueVector& radii) {
                        switch (radii.size()) {
                        case 4:
                        case 3:
                            return radii[2];
                        case 2:
                        case 1:
                            return radii[0];
                        default:
                            VERIFY_NOT_REACHED();
                        }
                    };
                    auto bottom_left = [](StyleValueVector& radii) {
                        switch (radii.size()) {
                        case 4:
                            return radii[3];
                        case 3:
                        case 2:
                            return radii[1];
                        case 1:
                            return radii[0];
                        default:
                            VERIFY_NOT_REACHED();
                        }
                    };

                    auto maybe_horizontal_radii = materialize_radius_values(rectangle_border_radius_horizontal_radii);
                    auto maybe_vertical_radii = rectangle_border_radius_vertical_radii.is_empty()
                        ? Optional<StyleValueVector> {}
                        : materialize_radius_values(rectangle_border_radius_vertical_radii);
                    if (!maybe_horizontal_radii.has_value() || (!rectangle_border_radius_vertical_radii.is_empty() && !maybe_vertical_radii.has_value()))
                        return nullptr;

                    auto& horizontal_radii = *maybe_horizontal_radii;
                    auto& vertical_radii = maybe_vertical_radii.has_value() ? *maybe_vertical_radii : horizontal_radii;
                    auto top_left_radius = BorderRadiusStyleValue::create(top_left(horizontal_radii), top_left(vertical_radii));
                    auto top_right_radius = BorderRadiusStyleValue::create(top_right(horizontal_radii), top_right(vertical_radii));
                    auto bottom_right_radius = BorderRadiusStyleValue::create(bottom_right(horizontal_radii), bottom_right(vertical_radii));
                    auto bottom_left_radius = BorderRadiusStyleValue::create(bottom_left(horizontal_radii), bottom_left(vertical_radii));
                    return BorderRadiusRectStyleValue::create(top_left_radius, top_right_radius, bottom_right_radius, bottom_left_radius);
                };
                auto materialize_rust_basic_shape_radial_extent = [](RustComponentValueParser::RustBasicShapeRadialExtent extent) {
                    switch (extent) {
                    case RustComponentValueParser::RustBasicShapeRadialExtent::ClosestCorner:
                        return RadialExtent::ClosestCorner;
                    case RustComponentValueParser::RustBasicShapeRadialExtent::ClosestSide:
                        return RadialExtent::ClosestSide;
                    case RustComponentValueParser::RustBasicShapeRadialExtent::FarthestCorner:
                        return RadialExtent::FarthestCorner;
                    case RustComponentValueParser::RustBasicShapeRadialExtent::FarthestSide:
                        return RadialExtent::FarthestSide;
                    }
                    VERIFY_NOT_REACHED();
                };
                auto materialize_rust_basic_shape_radial_size = [&]() -> RefPtr<RadialSizeStyleValue const> {
                    Vector<RadialSizeStyleValue::Component> components;
                    components.ensure_capacity(radial_shape_radius.size());
                    for (auto const& component : radial_shape_radius) {
                        if (component.is_radial_extent) {
                            components.append(materialize_rust_basic_shape_radial_extent(component.radial_extent));
                            continue;
                        }

                        auto length_percentage = materialize_rust_basic_shape_length_percentage(component.length_percentage, non_negative_range);
                        if (!length_percentage)
                            return nullptr;
                        components.append(length_percentage.release_nonnull());
                    }

                    if (components.is_empty()) {
                        if (kind == RustComponentValueParser::RustBasicShapeKind::Circle)
                            components.append(RadialExtent::ClosestSide);
                        else {
                            components.append(RadialExtent::ClosestSide);
                            components.append(RadialExtent::ClosestSide);
                        }
                    }

                    return RadialSizeStyleValue::create(move(components));
                };
                auto materialize_rust_basic_shape_position_edge = [](RustComponentValueParser::RustPositionEdge edge) -> Optional<PositionEdge> {
                    switch (edge) {
                    case RustComponentValueParser::RustPositionEdge::None:
                        return {};
                    case RustComponentValueParser::RustPositionEdge::Center:
                        return PositionEdge::Center;
                    case RustComponentValueParser::RustPositionEdge::Left:
                        return PositionEdge::Left;
                    case RustComponentValueParser::RustPositionEdge::Right:
                        return PositionEdge::Right;
                    case RustComponentValueParser::RustPositionEdge::Top:
                        return PositionEdge::Top;
                    case RustComponentValueParser::RustPositionEdge::Bottom:
                        return PositionEdge::Bottom;
                    }
                    VERIFY_NOT_REACHED();
                };
                auto materialize_rust_basic_shape_position_component = [&](RustComponentValueParser::RustPositionComponent const& component) -> RefPtr<StyleValue const> {
                    RefPtr<StyleValue const> offset;
                    if (component.offset.has_value()) {
                        offset = materialize_rust_basic_shape_length_percentage(*component.offset, infinite_range);
                        if (!offset)
                            return nullptr;
                    }
                    return EdgeStyleValue::create(materialize_rust_basic_shape_position_edge(component.edge), offset);
                };
                auto materialize_rust_basic_shape_position = [&]() -> RefPtr<PositionStyleValue const> {
                    if (!radial_shape_position.has_value())
                        return nullptr;

                    auto x = materialize_rust_basic_shape_position_component(radial_shape_position->x);
                    if (!x)
                        return nullptr;
                    auto y = materialize_rust_basic_shape_position_component(radial_shape_position->y);
                    if (!y)
                        return nullptr;
                    return PositionStyleValue::create(x->as_edge(), y->as_edge());
                };
                auto parse_optional_round_border_radius = [&](TokenStream<ComponentValue>& arguments_tokens) -> RefPtr<StyleValue const> {
                    NonnullRefPtr<StyleValue const> border_radius = BorderRadiusRectStyleValue::create_zero();
                    arguments_tokens.discard_whitespace();
                    if (arguments_tokens.next_token().is_ident("round"sv)) {
                        arguments_tokens.discard_a_token();
                        auto parsed_border_radius = parse_border_radius_rect_value(arguments_tokens);
                        if (!parsed_border_radius)
                            return nullptr;
                        border_radius = parsed_border_radius.release_nonnull();
                        arguments_tokens.discard_whitespace();
                    }
                    if (arguments_tokens.has_next_token())
                        return nullptr;
                    return border_radius;
                };

                switch (kind) {
                case RustComponentValueParser::RustBasicShapeKind::Inset: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "inset"sv });
                    if (!rectangle_components.is_empty()) {
                        if (rectangle_components.size() > 4)
                            return nullptr;

                        auto top = materialize_rust_basic_shape_rectangle_component(rectangle_components[0], infinite_range);
                        if (!top)
                            return nullptr;

                        RefPtr<StyleValue const> right;
                        if (rectangle_components.size() > 1)
                            right = materialize_rust_basic_shape_rectangle_component(rectangle_components[1], infinite_range);
                        else
                            right = top;
                        if (!right)
                            return nullptr;

                        RefPtr<StyleValue const> bottom;
                        if (rectangle_components.size() > 2)
                            bottom = materialize_rust_basic_shape_rectangle_component(rectangle_components[2], infinite_range);
                        else
                            bottom = top;
                        if (!bottom)
                            return nullptr;

                        RefPtr<StyleValue const> left;
                        if (rectangle_components.size() > 3)
                            left = materialize_rust_basic_shape_rectangle_component(rectangle_components[3], infinite_range);
                        else
                            left = right;
                        if (!left)
                            return nullptr;

                        auto border_radius = materialize_rust_basic_shape_border_radius();
                        if (!border_radius)
                            return nullptr;

                        return BasicShapeStyleValue::create(Inset { top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), border_radius.release_nonnull() });
                    }

                    if (argument_groups.size() != 1)
                        return nullptr;
                    auto component_values = parse_rust_basic_shape_group(argument_groups[0]);
                    TokenStream arguments_tokens { component_values };

                    // inset() = inset( <length-percentage>{1,4} [ round <'border-radius'> ]? )
                    arguments_tokens.discard_whitespace();
                    auto top = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
                    if (!top)
                        return nullptr;

                    arguments_tokens.discard_whitespace();
                    auto right = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
                    if (!right)
                        right = top;

                    arguments_tokens.discard_whitespace();
                    auto bottom = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
                    if (!bottom)
                        bottom = top;

                    arguments_tokens.discard_whitespace();
                    auto left = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
                    if (!left)
                        left = right;

                    auto border_radius = parse_optional_round_border_radius(arguments_tokens);
                    if (!border_radius)
                        return nullptr;

                    return BasicShapeStyleValue::create(Inset { top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), border_radius.release_nonnull() });
                }
                case RustComponentValueParser::RustBasicShapeKind::Xywh: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "xywh"sv });
                    if (!rectangle_components.is_empty()) {
                        if (rectangle_components.size() != 4)
                            return nullptr;
                        auto x = materialize_rust_basic_shape_rectangle_component(rectangle_components[0], infinite_range);
                        auto y = materialize_rust_basic_shape_rectangle_component(rectangle_components[1], infinite_range);
                        auto width = materialize_rust_basic_shape_rectangle_component(rectangle_components[2], non_negative_range);
                        auto height = materialize_rust_basic_shape_rectangle_component(rectangle_components[3], non_negative_range);
                        auto border_radius = materialize_rust_basic_shape_border_radius();
                        if (!x || !y || !width || !height || !border_radius)
                            return nullptr;

                        return BasicShapeStyleValue::create(Xywh { x.release_nonnull(), y.release_nonnull(), width.release_nonnull(), height.release_nonnull(), border_radius.release_nonnull() });
                    }

                    if (argument_groups.size() != 1)
                        return nullptr;
                    auto component_values = parse_rust_basic_shape_group(argument_groups[0]);
                    TokenStream arguments_tokens { component_values };

                    // xywh() = xywh( <length-percentage>{2} <length-percentage [0,∞]>{2} [ round <'border-radius'> ]? )
                    arguments_tokens.discard_whitespace();
                    auto x = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
                    if (!x)
                        return nullptr;

                    arguments_tokens.discard_whitespace();
                    auto y = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
                    if (!y)
                        return nullptr;

                    arguments_tokens.discard_whitespace();
                    auto width = parse_length_percentage_value(arguments_tokens, non_negative_range, non_negative_range);
                    if (!width)
                        return nullptr;

                    arguments_tokens.discard_whitespace();
                    auto height = parse_length_percentage_value(arguments_tokens, non_negative_range, non_negative_range);
                    if (!height)
                        return nullptr;

                    auto border_radius = parse_optional_round_border_radius(arguments_tokens);
                    if (!border_radius)
                        return nullptr;

                    return BasicShapeStyleValue::create(Xywh { x.release_nonnull(), y.release_nonnull(), width.release_nonnull(), height.release_nonnull(), border_radius.release_nonnull() });
                }
                case RustComponentValueParser::RustBasicShapeKind::Rect: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "rect"sv });
                    if (!rectangle_components.is_empty()) {
                        if (rectangle_components.size() != 4)
                            return nullptr;

                        auto top = materialize_rust_basic_shape_rectangle_component(rectangle_components[0], infinite_range);
                        auto right = materialize_rust_basic_shape_rectangle_component(rectangle_components[1], infinite_range);
                        auto bottom = materialize_rust_basic_shape_rectangle_component(rectangle_components[2], infinite_range);
                        auto left = materialize_rust_basic_shape_rectangle_component(rectangle_components[3], infinite_range);
                        auto border_radius = materialize_rust_basic_shape_border_radius();
                        if (!top || !right || !bottom || !left || !border_radius)
                            return nullptr;

                        return BasicShapeStyleValue::create(Rect { top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), border_radius.release_nonnull() });
                    }

                    if (argument_groups.size() != 1)
                        return nullptr;
                    auto component_values = parse_rust_basic_shape_group(argument_groups[0]);
                    TokenStream arguments_tokens { component_values };

                    auto parse_length_percentage_or_auto = [this](TokenStream<ComponentValue>& tokens) -> RefPtr<StyleValue const> {
                        tokens.discard_whitespace();
                        if (auto value = parse_length_percentage_value(tokens, infinite_range, infinite_range))
                            return value;
                        if (tokens.consume_a_token().is_ident("auto"sv))
                            return KeywordStyleValue::create(Keyword::Auto);
                        return {};
                    };

                    // rect() = rect( [ <length-percentage> | auto ]{4} [ round <'border-radius'> ]? )
                    auto top = parse_length_percentage_or_auto(arguments_tokens);
                    auto right = parse_length_percentage_or_auto(arguments_tokens);
                    auto bottom = parse_length_percentage_or_auto(arguments_tokens);
                    auto left = parse_length_percentage_or_auto(arguments_tokens);
                    if (!top || !right || !bottom || !left)
                        return nullptr;

                    auto border_radius = parse_optional_round_border_radius(arguments_tokens);
                    if (!border_radius)
                        return nullptr;

                    return BasicShapeStyleValue::create(Rect { top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), border_radius.release_nonnull() });
                }
                case RustComponentValueParser::RustBasicShapeKind::Circle:
                case RustComponentValueParser::RustBasicShapeKind::Ellipse: {
                    auto is_circle = kind == RustComponentValueParser::RustBasicShapeKind::Circle;
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { is_circle ? "circle"sv : "ellipse"sv });
                    if (radial_shape_is_typed) {
                        if ((is_circle && radial_shape_radius.size() > 1) || (!is_circle && radial_shape_radius.size() == 1))
                            return nullptr;

                        auto radius = materialize_rust_basic_shape_radial_size();
                        if (!radius)
                            return nullptr;

                        RefPtr<PositionStyleValue const> position;
                        if (radial_shape_position.has_value()) {
                            position = materialize_rust_basic_shape_position();
                            if (!position)
                                return nullptr;
                        }

                        if (is_circle)
                            return BasicShapeStyleValue::create(Circle { radius.release_nonnull(), position });
                        return BasicShapeStyleValue::create(Ellipse { radius.release_nonnull(), position });
                    }

                    if (argument_groups.size() != 1)
                        return nullptr;
                    auto component_values = parse_rust_basic_shape_group(argument_groups[0]);
                    TokenStream arguments_tokens { component_values };

                    // circle() = circle( <radial-size>? [ at <position> ]? )
                    // ellipse() = ellipse( <radial-size>? [ at <position> ]? )
                    auto radius = parse_radial_size(arguments_tokens);
                    if (is_circle && radius && radius->components().size() != 1)
                        return nullptr;
                    if (!is_circle && radius && radius->components().size() != 2)
                        return nullptr;

                    if (!radius) {
                        if (is_circle)
                            radius = RadialSizeStyleValue::create({ RadialExtent::ClosestSide });
                        else
                            radius = RadialSizeStyleValue::create({ RadialExtent::ClosestSide, RadialExtent::ClosestSide });
                    }

                    RefPtr<PositionStyleValue const> position;
                    arguments_tokens.discard_whitespace();
                    if (arguments_tokens.next_token().is_ident("at"sv)) {
                        arguments_tokens.discard_a_token();
                        arguments_tokens.discard_whitespace();
                        auto maybe_position = parse_position_value(arguments_tokens);
                        if (maybe_position.is_null())
                            return nullptr;
                        position = maybe_position;
                    }

                    arguments_tokens.discard_whitespace();
                    if (arguments_tokens.has_next_token())
                        return nullptr;

                    if (is_circle)
                        return BasicShapeStyleValue::create(Circle { radius.release_nonnull(), position });
                    return BasicShapeStyleValue::create(Ellipse { radius.release_nonnull(), position });
                }
                case RustComponentValueParser::RustBasicShapeKind::Polygon: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "polygon"sv });
                    if (!polygon_coordinates.is_empty()) {
                        auto fill_rule = materialize_rust_fill_rule(fill_rule_value);
                        if (!fill_rule.has_value() || polygon_coordinates.size() % 2 != 0)
                            return nullptr;

                        Vector<Polygon::Point> points;
                        points.ensure_capacity(polygon_coordinates.size() / 2);
                        for (size_t i = 0; i < polygon_coordinates.size(); i += 2) {
                            auto x_pos = materialize_rust_basic_shape_length_percentage(polygon_coordinates[i], infinite_range);
                            auto y_pos = materialize_rust_basic_shape_length_percentage(polygon_coordinates[i + 1], infinite_range);
                            if (!x_pos || !y_pos)
                                return nullptr;
                            points.append(Polygon::Point { x_pos.release_nonnull(), y_pos.release_nonnull() });
                        }

                        return BasicShapeStyleValue::create(Polygon { *fill_rule, move(points) });
                    }

                    if (argument_groups.is_empty())
                        return nullptr;

                    auto fill_rule = parse_rust_basic_shape_fill_rule_argument(argument_groups[0]);
                    size_t first_point_index = 0;
                    if (fill_rule.has_value())
                        first_point_index = 1;
                    else
                        fill_rule = Gfx::WindingRule::Nonzero;
                    if (first_point_index >= argument_groups.size())
                        return nullptr;

                    Vector<Polygon::Point> points;
                    for (size_t i = first_point_index; i < argument_groups.size(); ++i) {
                        auto component_values = parse_rust_basic_shape_group(argument_groups[i]);
                        TokenStream argument_tokens { component_values };

                        argument_tokens.discard_whitespace();
                        auto x_pos = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
                        if (!x_pos)
                            return nullptr;

                        argument_tokens.discard_whitespace();
                        auto y_pos = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
                        if (!y_pos)
                            return nullptr;

                        argument_tokens.discard_whitespace();
                        if (argument_tokens.has_next_token())
                            return nullptr;

                        points.append(Polygon::Point { x_pos.release_nonnull(), y_pos.release_nonnull() });
                    }

                    return BasicShapeStyleValue::create(Polygon { fill_rule.release_value(), move(points) });
                }
                case RustComponentValueParser::RustBasicShapeKind::Path: {
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "path"sv });
                    if (path_data_string.has_value()) {
                        auto fill_rule = materialize_rust_fill_rule(fill_rule_value);
                        if (!fill_rule.has_value())
                            return nullptr;

                        auto path_data = SVG::AttributeParser::parse_path_data(*path_data_string);
                        if (path_data.instructions().is_empty())
                            return nullptr;

                        return BasicShapeStyleValue::create(Path { *fill_rule, move(path_data) });
                    }

                    if (argument_groups.is_empty() || argument_groups.size() > 2)
                        return nullptr;

                    Gfx::WindingRule fill_rule { Gfx::WindingRule::Nonzero };
                    if (argument_groups.size() == 2) {
                        auto maybe_fill_rule = parse_rust_basic_shape_fill_rule_argument(argument_groups[0]);
                        if (!maybe_fill_rule.has_value())
                            return nullptr;
                        fill_rule = maybe_fill_rule.release_value();
                    }

                    auto component_values = parse_rust_basic_shape_group(argument_groups.last());
                    TokenStream path_argument_tokens { component_values };
                    path_argument_tokens.discard_whitespace();
                    auto& maybe_string = path_argument_tokens.consume_a_token();
                    path_argument_tokens.discard_whitespace();
                    if (!maybe_string.is(Token::Type::String) || path_argument_tokens.has_next_token())
                        return nullptr;

                    auto path_data = SVG::AttributeParser::parse_path_data(maybe_string.token().string().to_string());
                    if (path_data.instructions().is_empty())
                        return nullptr;

                    return BasicShapeStyleValue::create(Path { fill_rule, move(path_data) });
                }
                }
                VERIFY_NOT_REACHED();
            };
            auto parse_rust_source_as_value_type = [&](StringView source, ValueType value_type) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_value(value_type, value_tokens);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto materialize_rust_counter_style = [&](Optional<RustComponentValueParser::CounterStyle> const& maybe_counter_style) -> NonnullRefPtr<StyleValue const> {
                if (!maybe_counter_style.has_value())
                    return CounterStyleStyleValue::create("decimal"_fly_string);

                auto counter_style = *maybe_counter_style;
                if (counter_style.kind == FFI::CssCounterStyleKind::Name) {
                    auto counter_style_name = counter_style.name;

                    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
                    // Counter style names are case-sensitive. However, the names defined in this specification are ASCII lowercased
                    // on parse wherever they are used as counter styles, e.g. in the list-style set of properties, in the
                    // @counter-style rule, and in the counter() functions.

                    // NB: The "names defined in this specification" are defined in the `CounterStyleNameKeyword` enum
                    auto const& keyword = keyword_from_string(counter_style_name);
                    if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
                        counter_style_name = counter_style_name.to_ascii_lowercase();

                    return CounterStyleStyleValue::create(counter_style_name);
                }

                VERIFY(counter_style.kind == FFI::CssCounterStyleKind::SymbolsFunction);
                auto symbols_type = [&] {
                    switch (counter_style.symbols_type) {
                    case FFI::CssCounterStyleSymbolsType::Cyclic:
                        return SymbolsType::Cyclic;
                    case FFI::CssCounterStyleSymbolsType::Numeric:
                        return SymbolsType::Numeric;
                    case FFI::CssCounterStyleSymbolsType::Alphabetic:
                        return SymbolsType::Alphabetic;
                    case FFI::CssCounterStyleSymbolsType::Symbolic:
                        return SymbolsType::Symbolic;
                    case FFI::CssCounterStyleSymbolsType::Fixed:
                        return SymbolsType::Fixed;
                    }
                    VERIFY_NOT_REACHED();
                }();
                return CounterStyleStyleValue::create(CounterStyleStyleValue::SymbolsFunction { symbols_type, move(counter_style.symbols) });
            };
            auto materialize_rust_counter = [&](RustComponentValueParser::RustCounterFunctionKind function, FlyString const& name, FlyString const& join_string, Optional<RustComponentValueParser::CounterStyle> const& counter_style) -> RefPtr<StyleValue const> {
                auto counter_style_value = materialize_rust_counter_style(counter_style);
                switch (function) {
                case RustComponentValueParser::RustCounterFunctionKind::Counter:
                    return CounterStyleValue::create_counter(name, counter_style_value);
                case RustComponentValueParser::RustCounterFunctionKind::Counters:
                    return CounterStyleValue::create_counters(name, join_string, counter_style_value);
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_content_value = [&]() -> RefPtr<StyleValue const> {
                if (rust_style_value->content_keyword.has_value())
                    return KeywordStyleValue::create(*rust_style_value->content_keyword);

                StyleValueVector content_values;
                StyleValueVector alt_text_values;
                content_values.ensure_capacity(rust_style_value->content_events.size());
                alt_text_values.ensure_capacity(rust_style_value->content_events.size());

                for (auto const& event : rust_style_value->content_events) {
                    RefPtr<StyleValue const> value;
                    switch (event.kind) {
                    case RustComponentValueParser::RustContentEventKind::Normal:
                    case RustComponentValueParser::RustContentEventKind::None:
                        return nullptr;
                    case RustComponentValueParser::RustContentEventKind::ItemQuote: {
                        auto keyword = keyword_from_string(event.source);
                        if (!keyword.has_value())
                            return nullptr;
                        value = KeywordStyleValue::create(*keyword);
                        content_values.append(value.release_nonnull());
                        break;
                    }
                    case RustComponentValueParser::RustContentEventKind::ItemString:
                        content_values.append(StringStyleValue::create(event.source));
                        break;
                    case RustComponentValueParser::RustContentEventKind::ItemImage:
                        value = materialize_rust_image(event.image_kind, event.source, event.image_url);
                        if (!value)
                            return nullptr;
                        content_values.append(value.release_nonnull());
                        break;
                    case RustComponentValueParser::RustContentEventKind::ItemCounter:
                        value = materialize_rust_counter(event.counter_function, event.counter_name, event.counter_join_string, event.counter_style);
                        if (!value)
                            return nullptr;
                        content_values.append(value.release_nonnull());
                        break;
                    case RustComponentValueParser::RustContentEventKind::AltTextString:
                        alt_text_values.append(StringStyleValue::create(event.source));
                        break;
                    case RustComponentValueParser::RustContentEventKind::AltTextCounter:
                        value = materialize_rust_counter(event.counter_function, event.counter_name, event.counter_join_string, event.counter_style);
                        if (!value)
                            return nullptr;
                        alt_text_values.append(value.release_nonnull());
                        break;
                    case RustComponentValueParser::RustContentEventKind::CounterJoinString:
                    case RustComponentValueParser::RustContentEventKind::CounterStyleName:
                    case RustComponentValueParser::RustContentEventKind::CounterStyleSymbols:
                    case RustComponentValueParser::RustContentEventKind::CounterStyleSymbol:
                        return nullptr;
                    }
                }

                if (content_values.is_empty())
                    return nullptr;

                RefPtr<StyleValueList> alt_text;
                if (!alt_text_values.is_empty())
                    alt_text = StyleValueList::create(move(alt_text_values), StyleValueList::Separator::Space);

                return ContentStyleValue::create(StyleValueList::create(move(content_values), StyleValueList::Separator::Space), move(alt_text));
            };
            auto materialize_rust_shape_outside_value = [&]() -> RefPtr<StyleValue const> {
                if (rust_style_value->shape_outside_is_none)
                    return KeywordStyleValue::create(Keyword::None);

                if (rust_style_value->shape_outside_image_source.has_value()) {
                    if (!rust_style_value->shape_outside_image_source_kind.has_value())
                        return nullptr;
                    return materialize_rust_image(*rust_style_value->shape_outside_image_source_kind, *rust_style_value->shape_outside_image_source, rust_style_value->shape_outside_image_source_url);
                }

                RefPtr<StyleValue const> basic_shape_value;
                RefPtr<StyleValue const> shape_box_value;
                if (rust_style_value->shape_outside_basic_shape_kind.has_value()) {
                    basic_shape_value = materialize_rust_basic_shape(*rust_style_value->shape_outside_basic_shape_kind, rust_style_value->shape_outside_basic_shape_argument_groups, rust_style_value->shape_outside_basic_shape_fill_rule, rust_style_value->shape_outside_basic_shape_rectangle_components, rust_style_value->shape_outside_basic_shape_rectangle_border_radius_horizontal_radii, rust_style_value->shape_outside_basic_shape_rectangle_border_radius_vertical_radii, rust_style_value->shape_outside_basic_shape_radial_shape_is_typed, rust_style_value->shape_outside_basic_shape_radial_shape_radius, rust_style_value->shape_outside_basic_shape_radial_shape_position, rust_style_value->shape_outside_basic_shape_polygon_coordinates, rust_style_value->shape_outside_basic_shape_path_data);
                    if (!basic_shape_value)
                        return nullptr;
                }
                if (rust_style_value->shape_outside_shape_box.has_value())
                    shape_box_value = KeywordStyleValue::create(to_keyword(*rust_style_value->shape_outside_shape_box));

                if (basic_shape_value && !shape_box_value)
                    return basic_shape_value;

                if (!basic_shape_value && shape_box_value)
                    return shape_box_value;

                if (basic_shape_value && shape_box_value)
                    return StyleValueList::create({ basic_shape_value.release_nonnull(), shape_box_value.release_nonnull() }, StyleValueList::Separator::Space);

                return nullptr;
            };
            auto parse_rust_source_as_url = [&](String const& source) -> Optional<URL> {
                return RustComponentValueParser::parse_a_url_function(source.bytes_as_string_view(), "utf-8"sv);
            };
            auto parse_rust_source_as_non_negative_number = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_number_value(value_tokens, non_negative_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_number_percentage = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_number_percentage_value(value_tokens, infinite_range, infinite_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_non_negative_number_percentage = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_number_percentage_value(value_tokens, non_negative_range, non_negative_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_color = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_color_value(value_tokens);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto materialize_rust_style_color = [&](RustComponentValueParser::RustStyleColor const& color, auto parse_source) -> RefPtr<StyleValue const> {
                if (!color.is_simple) {
                    if (!color.source.has_value())
                        return nullptr;
                    return parse_source(*color.source);
                }

                switch (color.kind) {
                case FFI::CssParsedColorKind::Invalid:
                    return nullptr;
                case FFI::CssParsedColorKind::Rgba: {
                    Optional<FlyString> name;
                    if (color.name.has_value())
                        name = FlyString::from_utf8_without_validation(color.name->bytes());
                    return ColorStyleValue::create_from_color({ color.red, color.green, color.blue, color.alpha }, ColorSyntax::Legacy, move(name));
                }
                case FFI::CssParsedColorKind::Keyword: {
                    if (!color.name.has_value())
                        return nullptr;
                    auto keyword = keyword_from_string(*color.name);
                    if (!keyword.has_value())
                        return nullptr;
                    return KeywordStyleValue::create(*keyword);
                }
                }
                VERIFY_NOT_REACHED();
            };
            auto parse_rust_source_as_length_percentage = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_length_percentage_value(value_tokens, infinite_range, infinite_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_length = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_length_value(value_tokens, infinite_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_non_negative_length_percentage = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_length_percentage_value(value_tokens, non_negative_range, non_negative_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_non_negative_length = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_length_value(value_tokens, non_negative_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto parse_rust_source_as_border_image_outset = [&](String const& source) -> RefPtr<StyleValue const> {
                if (auto value = parse_rust_source_as_non_negative_number(source))
                    return value;
                return parse_rust_source_as_non_negative_length(source);
            };
            auto parse_rust_source_as_border_image_width = [&](String const& source) -> RefPtr<StyleValue const> {
                if (source.equals_ignoring_ascii_case("auto"sv))
                    return KeywordStyleValue::create(Keyword::Auto);
                if (auto value = parse_rust_source_as_non_negative_number(source))
                    return value;
                return parse_rust_source_as_non_negative_length_percentage(source);
            };
            auto materialize_rust_style_value_list = [&](auto const& sources, auto parse_source) -> RefPtr<StyleValue const> {
                if (sources.is_empty())
                    return nullptr;
                StyleValueVector values;
                for (auto const& source : sources) {
                    auto value = parse_source(source);
                    if (!value)
                        return nullptr;
                    values.append(value.release_nonnull());
                }
                if (values.size() == 1)
                    return values[0];
                return StyleValueList::create(move(values), StyleValueList::Separator::Space);
            };
            auto parse_rust_source_as_angle = [&](String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source, "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_angle_value(value_tokens, infinite_range);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return nullptr;
                return value.release_nonnull();
            };
            auto materialize_rust_nested_length = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value()) {
                    if (range.min >= 0)
                        return parse_rust_source_as_non_negative_length(value.source_or_unit);
                    return parse_rust_source_as_length(value.source_or_unit);
                }
                if (value.primitive_kind != FFI::CssPrimitiveValueKind::Length)
                    return nullptr;
                auto length_unit = string_to_length_unit(value.source_or_unit);
                if (!length_unit.has_value())
                    return nullptr;
                Length length { *value.numeric_value, length_unit.release_value() };
                if (!range.contains(length.raw_value()))
                    return nullptr;
                return LengthStyleValue::create(length);
            };
            auto materialize_rust_nested_length_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value()) {
                    if (range.min >= 0)
                        return parse_rust_source_as_non_negative_length_percentage(value.source_or_unit);
                    return parse_rust_source_as_length_percentage(value.source_or_unit);
                }
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Length)
                    return materialize_rust_nested_length(value, range);
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Percentage) {
                    if (!range.contains(*value.numeric_value))
                        return nullptr;
                    return PercentageStyleValue::create(Percentage { *value.numeric_value });
                }
                return nullptr;
            };
            auto materialize_rust_position_edge = [](RustComponentValueParser::RustPositionEdge edge) -> Optional<PositionEdge> {
                switch (edge) {
                case RustComponentValueParser::RustPositionEdge::None:
                    return {};
                case RustComponentValueParser::RustPositionEdge::Center:
                    return PositionEdge::Center;
                case RustComponentValueParser::RustPositionEdge::Left:
                    return PositionEdge::Left;
                case RustComponentValueParser::RustPositionEdge::Right:
                    return PositionEdge::Right;
                case RustComponentValueParser::RustPositionEdge::Top:
                    return PositionEdge::Top;
                case RustComponentValueParser::RustPositionEdge::Bottom:
                    return PositionEdge::Bottom;
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_position_component = [&](RustComponentValueParser::RustPositionComponent const& component) -> RefPtr<StyleValue const> {
                RefPtr<StyleValue const> offset;
                if (component.offset.has_value()) {
                    offset = materialize_rust_nested_length_percentage(*component.offset, infinite_range);
                    if (!offset)
                        return nullptr;
                }
                return EdgeStyleValue::create(materialize_rust_position_edge(component.edge), offset);
            };
            auto materialize_rust_position = [&](RustComponentValueParser::RustPosition const& position) -> RefPtr<StyleValue const> {
                auto x = materialize_rust_position_component(position.x);
                if (!x)
                    return nullptr;
                auto y = materialize_rust_position_component(position.y);
                if (!y)
                    return nullptr;
                return PositionStyleValue::create(x->as_edge(), y->as_edge());
            };
            auto materialize_rust_view_timeline_insets = [&](ReadonlySpan<RustComponentValueParser::RustViewTimelineInset> insets) -> RefPtr<StyleValue const> {
                if (insets.is_empty())
                    return nullptr;

                StyleValueVector inset_values;
                inset_values.ensure_capacity(insets.size());
                for (auto const& inset : insets) {
                    if (inset.is_auto) {
                        inset_values.append(KeywordStyleValue::create(Keyword::Auto));
                        continue;
                    }

                    auto value = materialize_rust_nested_length_percentage(inset.length_percentage, infinite_range);
                    if (!value)
                        return nullptr;
                    inset_values.append(value.release_nonnull());
                }

                // https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
                // If the second value is omitted, it is set to the first.
                if (inset_values.size() == 1)
                    return StyleValueList::create({ inset_values[0], inset_values[0] }, StyleValueList::Separator::Space);

                return StyleValueList::create(move(inset_values), StyleValueList::Separator::Space);
            };
            auto materialize_rust_flex_basis = [&](RustComponentValueParser::RustStyleValue const& value) -> RefPtr<StyleValue const> {
                if (!value.flex_basis_kind.has_value())
                    return nullptr;

                switch (*value.flex_basis_kind) {
                case RustComponentValueParser::RustFlexBasisKind::Auto:
                    return KeywordStyleValue::create(Keyword::Auto);
                case RustComponentValueParser::RustFlexBasisKind::Content:
                    return KeywordStyleValue::create(Keyword::Content);
                case RustComponentValueParser::RustFlexBasisKind::FitContent:
                    return KeywordStyleValue::create(Keyword::FitContent);
                case RustComponentValueParser::RustFlexBasisKind::MinContent:
                    return KeywordStyleValue::create(Keyword::MinContent);
                case RustComponentValueParser::RustFlexBasisKind::MaxContent:
                    return KeywordStyleValue::create(Keyword::MaxContent);
                case RustComponentValueParser::RustFlexBasisKind::FitContentFunction: {
                    if (!value.flex_basis.has_value())
                        return nullptr;
                    auto argument = materialize_rust_nested_length_percentage(*value.flex_basis, non_negative_range);
                    if (!argument)
                        return nullptr;
                    return FunctionStyleValue::create("fit-content"_fly_string, argument.release_nonnull());
                }
                case RustComponentValueParser::RustFlexBasisKind::LengthPercentage:
                    if (!value.flex_basis.has_value())
                        return nullptr;
                    return materialize_rust_nested_length_percentage(*value.flex_basis, non_negative_range);
                case RustComponentValueParser::RustFlexBasisKind::Source:
                    if (!value.flex_basis_source.has_value())
                        return nullptr;
                    return parse_rust_source_as_property(PropertyID::FlexBasis, *value.flex_basis_source);
                }

                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_nested_transform_origin_component = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Keyword) {
                    auto maybe_keyword = keyword_from_string(value.source_or_unit.bytes_as_string_view());
                    if (!maybe_keyword.has_value())
                        return nullptr;
                    switch (*maybe_keyword) {
                    case Keyword::Bottom:
                    case Keyword::Center:
                    case Keyword::Left:
                    case Keyword::Right:
                    case Keyword::Top:
                        return KeywordStyleValue::create(*maybe_keyword);
                    default:
                        return nullptr;
                    }
                }
                return materialize_rust_nested_length_percentage(value, infinite_range);
            };
            auto materialize_rust_nested_background_size_component = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Keyword) {
                    auto maybe_keyword = keyword_from_string(value.source_or_unit.bytes_as_string_view());
                    if (!maybe_keyword.has_value() || *maybe_keyword != Keyword::Auto)
                        return nullptr;
                    return KeywordStyleValue::create(Keyword::Auto);
                }
                return materialize_rust_nested_length_percentage(value, non_negative_range);
            };
            auto materialize_rust_fit_content = [&]() -> RefPtr<StyleValue const> {
                switch (rust_style_value->fit_content_kind) {
                case RustComponentValueParser::RustFitContentKind::Keyword:
                    return KeywordStyleValue::create(Keyword::FitContent);
                case RustComponentValueParser::RustFitContentKind::Function: {
                    if (!rust_style_value->fit_content_argument.has_value())
                        return nullptr;
                    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "fit-content"sv });
                    auto argument = materialize_rust_nested_length_percentage(*rust_style_value->fit_content_argument, infinite_range);
                    if (!argument)
                        return nullptr;
                    return FunctionStyleValue::create("fit-content"_fly_string, argument.release_nonnull());
                }
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_text_decoration_line = [](u8 bits) -> RefPtr<StyleValue const> {
                if (bits == (1 << 0))
                    return KeywordStyleValue::create(Keyword::None);

                StyleValueVector style_values;
                auto append_line = [&](u8 bit, TextDecorationLine line) {
                    if (bits & bit)
                        style_values.append(KeywordStyleValue::create(to_keyword(line)));
                };
                append_line(1 << 1, TextDecorationLine::Underline);
                append_line(1 << 2, TextDecorationLine::Overline);
                append_line(1 << 3, TextDecorationLine::LineThrough);
                append_line(1 << 4, TextDecorationLine::Blink);
                append_line(1 << 5, TextDecorationLine::SpellingError);
                append_line(1 << 6, TextDecorationLine::GrammarError);
                if (style_values.is_empty())
                    return nullptr;
                return StyleValueList::create(move(style_values), StyleValueList::Separator::Space);
            };
            auto text_decoration_style_keyword_from_rust = [](RustComponentValueParser::RustTextDecorationStyle style) {
                switch (style) {
                case RustComponentValueParser::RustTextDecorationStyle::Solid:
                    return Keyword::Solid;
                case RustComponentValueParser::RustTextDecorationStyle::Double:
                    return Keyword::Double;
                case RustComponentValueParser::RustTextDecorationStyle::Dotted:
                    return Keyword::Dotted;
                case RustComponentValueParser::RustTextDecorationStyle::Dashed:
                    return Keyword::Dashed;
                case RustComponentValueParser::RustTextDecorationStyle::Wavy:
                    return Keyword::Wavy;
                }
                VERIFY_NOT_REACHED();
            };
            auto list_style_position_keyword_from_rust = [](RustComponentValueParser::RustListStylePosition position) {
                switch (position) {
                case RustComponentValueParser::RustListStylePosition::Inside:
                    return Keyword::Inside;
                case RustComponentValueParser::RustListStylePosition::Outside:
                    return Keyword::Outside;
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_list_style_image = [&]() -> RefPtr<StyleValue const> {
                if (!rust_style_value->list_style_image_kind.has_value())
                    return property_initial_value(PropertyID::ListStyleImage);
                switch (*rust_style_value->list_style_image_kind) {
                case RustComponentValueParser::RustListStyleImageKind::None:
                    return KeywordStyleValue::create(Keyword::None);
                case RustComponentValueParser::RustListStyleImageKind::Source:
                    if (!rust_style_value->list_style_image_source.has_value() || !rust_style_value->list_style_image_source_kind.has_value())
                        return nullptr;
                    return materialize_rust_image(*rust_style_value->list_style_image_source_kind, *rust_style_value->list_style_image_source, rust_style_value->list_style_image_source_url);
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_list_style_type = [&]() -> RefPtr<StyleValue const> {
                if (!rust_style_value->list_style_type_kind.has_value())
                    return property_initial_value(PropertyID::ListStyleType);
                switch (*rust_style_value->list_style_type_kind) {
                case RustComponentValueParser::RustListStyleTypeKind::None:
                    return KeywordStyleValue::create(Keyword::None);
                case RustComponentValueParser::RustListStyleTypeKind::String:
                    if (!rust_style_value->list_style_type_string.has_value())
                        return nullptr;
                    return StringStyleValue::create(*rust_style_value->list_style_type_string);
                case RustComponentValueParser::RustListStyleTypeKind::CounterStyleName:
                case RustComponentValueParser::RustListStyleTypeKind::CounterStyleSymbols:
                case RustComponentValueParser::RustListStyleTypeKind::CounterStyleSymbol:
                    if (!rust_style_value->list_style_type_counter_style.has_value())
                        return nullptr;
                    return materialize_rust_counter_style(rust_style_value->list_style_type_counter_style);
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_keyword_list = [](Vector<String> const& keywords) -> RefPtr<StyleValue const> {
                if (keywords.is_empty())
                    return nullptr;

                auto keyword_from_rust = [](String const& keyword_source) -> Optional<Keyword> {
                    auto keyword = keyword_from_string(keyword_source);
                    if (!keyword.has_value())
                        return {};
                    return keyword;
                };

                if (keywords.size() == 1) {
                    auto keyword = keyword_from_rust(keywords.first());
                    if (!keyword.has_value())
                        return nullptr;
                    return KeywordStyleValue::create(*keyword);
                }

                StyleValueVector values;
                values.ensure_capacity(keywords.size());
                for (auto const& keyword_source : keywords) {
                    auto keyword = keyword_from_rust(keyword_source);
                    if (!keyword.has_value())
                        return nullptr;
                    values.append(KeywordStyleValue::create(*keyword));
                }
                return StyleValueList::create(move(values), StyleValueList::Separator::Space);
            };
            auto materialize_rust_text_decoration_thickness = [&]() -> RefPtr<StyleValue const> {
                if (!rust_style_value->text_decoration_thickness_kind.has_value())
                    return nullptr;

                switch (*rust_style_value->text_decoration_thickness_kind) {
                case RustComponentValueParser::RustTextDecorationThicknessKind::Auto:
                    return KeywordStyleValue::create(Keyword::Auto);
                case RustComponentValueParser::RustTextDecorationThicknessKind::FromFont:
                    return KeywordStyleValue::create(Keyword::FromFont);
                case RustComponentValueParser::RustTextDecorationThicknessKind::LengthPercentage:
                    if (!rust_style_value->text_decoration_thickness.has_value())
                        return nullptr;
                    return materialize_rust_nested_length_percentage(*rust_style_value->text_decoration_thickness, infinite_range);
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_rect = [&]() -> RefPtr<StyleValue const> {
                if (rust_style_value->rect_sides.size() != 4)
                    return nullptr;

                auto context_guard = push_temporary_value_parsing_context(FunctionContext { "rect"sv });
                auto materialize_rust_rect_side = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                    if (value.primitive_kind == FFI::CssPrimitiveValueKind::Keyword) {
                        auto keyword = keyword_from_string(value.source_or_unit.bytes_as_string_view());
                        if (!keyword.has_value() || *keyword != Keyword::Auto)
                            return nullptr;
                        return KeywordStyleValue::create(Keyword::Auto);
                    }
                    return materialize_rust_nested_length(value, infinite_range);
                };

                auto top = materialize_rust_rect_side(rust_style_value->rect_sides[0]);
                auto right = materialize_rust_rect_side(rust_style_value->rect_sides[1]);
                auto bottom = materialize_rust_rect_side(rust_style_value->rect_sides[2]);
                auto left = materialize_rust_rect_side(rust_style_value->rect_sides[3]);
                if (!top || !right || !bottom || !left)
                    return nullptr;

                return RectStyleValue::create(top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull());
            };
            auto materialize_rust_nested_angle = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value())
                    return parse_rust_source_as_angle(value.source_or_unit);
                if (value.primitive_kind != FFI::CssPrimitiveValueKind::Angle)
                    return nullptr;
                auto angle_unit = string_to_angle_unit(value.source_or_unit);
                if (!angle_unit.has_value())
                    return nullptr;
                return AngleStyleValue::create(Angle { *value.numeric_value, angle_unit.release_value() });
            };
            auto materialize_rust_nested_integer = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value())
                    return parse_rust_source_as_integer_in_range(value.source_or_unit, range);
                if (value.primitive_kind != FFI::CssPrimitiveValueKind::Integer)
                    return nullptr;
                if (*value.numeric_value < AK::NumericLimits<i32>::min() || *value.numeric_value > AK::NumericLimits<i32>::max())
                    return nullptr;
                if (!range.contains(*value.numeric_value))
                    return nullptr;
                return IntegerStyleValue::create(static_cast<i32>(*value.numeric_value));
            };
            auto materialize_rust_nested_number = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value())
                    return parse_rust_source_as_number(value.source_or_unit);
                if (value.primitive_kind != FFI::CssPrimitiveValueKind::Number)
                    return nullptr;
                return NumberStyleValue::create(*value.numeric_value);
            };
            auto materialize_rust_nested_non_negative_number = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value())
                    return parse_rust_source_as_non_negative_number(value.source_or_unit);
                if (value.primitive_kind != FFI::CssPrimitiveValueKind::Number)
                    return nullptr;
                if (*value.numeric_value < 0)
                    return nullptr;
                return NumberStyleValue::create(*value.numeric_value);
            };
            auto materialize_rust_nested_non_negative_number_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value())
                    return parse_rust_source_as_non_negative_number_percentage(value.source_or_unit);
                if (*value.numeric_value < 0)
                    return nullptr;
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Number)
                    return NumberStyleValue::create(*value.numeric_value);
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Percentage)
                    return PercentageStyleValue::create(Percentage { *value.numeric_value });
                return nullptr;
            };
            auto materialize_rust_nested_number_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value())
                    return parse_rust_source_as_number_percentage(value.source_or_unit);
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Number)
                    return NumberStyleValue::create(*value.numeric_value);
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Percentage)
                    return PercentageStyleValue::create(Percentage { *value.numeric_value });
                return nullptr;
            };
            auto materialize_rust_transformation = [&](RustComponentValueParser::RustTransformation const& transformation, PropertyID property_id) -> RefPtr<StyleValue const> {
                StyleValueVector arguments;
                arguments.ensure_capacity(transformation.arguments.size());
                for (auto const& argument : transformation.arguments) {
                    RefPtr<StyleValue const> value;
                    switch (argument.parameter_type) {
                    case TransformFunctionParameterType::Angle:
                        value = materialize_rust_nested_angle(argument.value);
                        break;
                    case TransformFunctionParameterType::Length:
                        value = materialize_rust_nested_length(argument.value, infinite_range);
                        break;
                    case TransformFunctionParameterType::LengthNone:
                        if (!argument.value.numeric_value.has_value() && argument.value.source_or_unit.equals_ignoring_ascii_case("none"sv))
                            value = KeywordStyleValue::create(Keyword::None);
                        else
                            value = materialize_rust_nested_length(argument.value, infinite_range);
                        break;
                    case TransformFunctionParameterType::LengthPercentage:
                        value = materialize_rust_nested_length_percentage(argument.value, infinite_range);
                        break;
                    case TransformFunctionParameterType::Number:
                        value = materialize_rust_nested_number(argument.value);
                        break;
                    case TransformFunctionParameterType::NumberPercentage:
                        value = materialize_rust_nested_number_percentage(argument.value);
                        break;
                    }
                    if (!value)
                        return nullptr;
                    arguments.append(value.release_nonnull());
                }
                return TransformationStyleValue::create(property_id, transformation.function, move(arguments));
            };
            auto materialize_rust_nested_non_negative_number_length_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
                if (!value.numeric_value.has_value()) {
                    if (auto number_value = parse_rust_source_as_non_negative_number(value.source_or_unit))
                        return number_value;
                    return parse_rust_source_as_non_negative_length_percentage(value.source_or_unit);
                }
                if (value.primitive_kind == FFI::CssPrimitiveValueKind::Number) {
                    if (*value.numeric_value < 0)
                        return nullptr;
                    return NumberStyleValue::create(*value.numeric_value);
                }
                return materialize_rust_nested_length_percentage(value, non_negative_range);
            };
            auto border_image_repeat_keyword_from_rust = [](RustComponentValueParser::RustBorderImageRepeat repeat) {
                switch (repeat) {
                case RustComponentValueParser::RustBorderImageRepeat::Stretch:
                    return Keyword::Stretch;
                case RustComponentValueParser::RustBorderImageRepeat::Repeat:
                    return Keyword::Repeat;
                case RustComponentValueParser::RustBorderImageRepeat::Round:
                    return Keyword::Round;
                case RustComponentValueParser::RustBorderImageRepeat::Space:
                    return Keyword::Space;
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_border_image_slice = [&](Vector<RustComponentValueParser::RustNestedPrimitiveValue> const& values) -> RefPtr<StyleValue const> {
                if (values.size() != 4)
                    return nullptr;
                auto top = materialize_rust_nested_non_negative_number_percentage(values[0]);
                auto right = materialize_rust_nested_non_negative_number_percentage(values[1]);
                auto bottom = materialize_rust_nested_non_negative_number_percentage(values[2]);
                auto left = materialize_rust_nested_non_negative_number_percentage(values[3]);
                if (!top || !right || !bottom || !left)
                    return nullptr;
                return BorderImageSliceStyleValue::create(top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), rust_style_value->border_image_slice_fill);
            };
            auto materialize_rust_border_image_outset = [&](RustComponentValueParser::RustBorderImageOutset const& outset) -> RefPtr<StyleValue const> {
                if (outset.value.primitive_kind == FFI::CssPrimitiveValueKind::Number)
                    return materialize_rust_nested_non_negative_number(outset.value);
                if (outset.value.primitive_kind == FFI::CssPrimitiveValueKind::Invalid)
                    return parse_rust_source_as_border_image_outset(outset.value.source_or_unit);
                return materialize_rust_nested_length(outset.value, non_negative_range);
            };
            auto materialize_rust_border_image_width = [&](RustComponentValueParser::RustBorderImageWidth const& width) -> RefPtr<StyleValue const> {
                if (width.is_auto)
                    return KeywordStyleValue::create(Keyword::Auto);
                if (width.value.primitive_kind == FFI::CssPrimitiveValueKind::Number)
                    return materialize_rust_nested_non_negative_number(width.value);
                if (width.value.primitive_kind == FFI::CssPrimitiveValueKind::Invalid)
                    return parse_rust_source_as_border_image_width(width.value.source_or_unit);
                return materialize_rust_nested_length_percentage(width.value, non_negative_range);
            };
            auto materialize_rust_border_image_repeat = [&](Vector<RustComponentValueParser::RustBorderImageRepeat> const& repeats) -> RefPtr<StyleValue const> {
                if (repeats.is_empty())
                    return nullptr;
                StyleValueVector values;
                for (auto repeat : repeats)
                    values.append(KeywordStyleValue::create(border_image_repeat_keyword_from_rust(repeat)));
                if (values.size() == 1)
                    return values[0];
                return StyleValueList::create(move(values), StyleValueList::Separator::Space);
            };
            auto rust_keyword_style_value = [](FlyString const& keyword_string) -> RefPtr<StyleValue const> {
                auto maybe_keyword = keyword_from_string(keyword_string);
                if (!maybe_keyword.has_value())
                    return nullptr;
                return KeywordStyleValue::create(maybe_keyword.release_value());
            };
            auto materialize_rust_position_area = [&](RustComponentValueParser::RustPositionArea const& position_area) -> RefPtr<StyleValue const> {
                auto first_value = rust_keyword_style_value(position_area.first_keyword);
                if (!first_value)
                    return nullptr;

                if (!position_area.second_keyword.has_value())
                    return first_value.release_nonnull();

                auto second_value = rust_keyword_style_value(*position_area.second_keyword);
                if (!second_value)
                    return nullptr;

                StyleValueVector values;
                values.ensure_capacity(2);
                values.append(first_value.release_nonnull());
                values.append(second_value.release_nonnull());
                return StyleValueList::create(move(values), StyleValueList::Separator::Space);
            };
            auto materialize_rust_position_try_fallback = [&](RustComponentValueParser::RustPositionTryFallback const& fallback) -> RefPtr<StyleValue const> {
                if (fallback.kind == RustComponentValueParser::RustPositionTryFallbackKind::PositionArea)
                    return materialize_rust_position_area(fallback.position_area);

                StyleValueVector values;
                if (fallback.dashed_ident.has_value())
                    values.append(CustomIdentStyleValue::create(*fallback.dashed_ident));

                StyleValueVector try_tactics;
                try_tactics.ensure_capacity(fallback.try_tactics.size());
                for (auto const& try_tactic : fallback.try_tactics) {
                    auto maybe_keyword = keyword_from_string(try_tactic);
                    if (!maybe_keyword.has_value())
                        return nullptr;
                    try_tactics.unchecked_append(KeywordStyleValue::create(maybe_keyword.release_value()));
                }
                if (!try_tactics.is_empty())
                    values.append(StyleValueList::create(move(try_tactics), StyleValueList::Separator::Space));

                if (values.is_empty())
                    return nullptr;
                return StyleValueList::create(move(values), StyleValueList::Separator::Space);
            };
            auto materialize_rust_grid_track_placement = [&](RustComponentValueParser::RustGridTrackPlacement const& grid_track_placement) -> RefPtr<GridTrackPlacementStyleValue const> {
                RefPtr<StyleValue const> line_number;
                if (grid_track_placement.line_number.has_value()) {
                    line_number = materialize_rust_nested_integer(*grid_track_placement.line_number, infinite_integer_range);
                    if (!line_number)
                        return nullptr;
                }

                switch (grid_track_placement.kind) {
                case RustComponentValueParser::RustGridTrackPlacementKind::Auto:
                    return GridTrackPlacementStyleValue::create(GridTrackPlacement::make_auto());
                case RustComponentValueParser::RustGridTrackPlacementKind::Line:
                    return GridTrackPlacementStyleValue::create(GridTrackPlacement::make_line(line_number, grid_track_placement.name));
                case RustComponentValueParser::RustGridTrackPlacementKind::Span:
                    return GridTrackPlacementStyleValue::create(GridTrackPlacement::make_span(line_number ? line_number.release_nonnull() : IntegerStyleValue::create(1), grid_track_placement.name));
                }

                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_grid_track_breadth = [&](RustComponentValueParser::RustGridTrackBreadthKind kind, RustComponentValueParser::RustNestedPrimitiveValue const& value) -> Optional<GridSize> {
                switch (kind) {
                case RustComponentValueParser::RustGridTrackBreadthKind::Invalid:
                    return {};
                case RustComponentValueParser::RustGridTrackBreadthKind::LengthPercentage: {
                    auto fixed_breadth = materialize_rust_nested_length_percentage(value, non_negative_range);
                    if (!fixed_breadth)
                        return {};
                    return GridSize { fixed_breadth.release_nonnull() };
                }
                case RustComponentValueParser::RustGridTrackBreadthKind::Flex: {
                    RefPtr<StyleValue const> flex_value;
                    if (value.numeric_value.has_value()) {
                        if (value.primitive_kind != FFI::CssPrimitiveValueKind::Number || *value.numeric_value < 0)
                            return {};
                        flex_value = FlexStyleValue::create(Flex::make_fr(*value.numeric_value));
                    } else {
                        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(value.source_or_unit.bytes_as_string_view(), "utf-8"sv);
                        TokenStream value_tokens { component_values };
                        flex_value = parse_flex_value(value_tokens, non_negative_range);
                        value_tokens.discard_whitespace();
                        if (!flex_value || value_tokens.has_next_token())
                            return {};
                    }
                    return GridSize { flex_value.release_nonnull() };
                }
                case RustComponentValueParser::RustGridTrackBreadthKind::MinContent:
                    return GridSize(KeywordStyleValue::create(Keyword::MinContent));
                case RustComponentValueParser::RustGridTrackBreadthKind::MaxContent:
                    return GridSize(KeywordStyleValue::create(Keyword::MaxContent));
                case RustComponentValueParser::RustGridTrackBreadthKind::Auto:
                    return GridSize::make_auto();
                }
                VERIFY_NOT_REACHED();
            };
            auto materialize_rust_grid_track_size = [&](RustComponentValueParser::RustGridTrackSizeListEvent const& event) -> Optional<ExplicitGridTrack> {
                switch (event.kind) {
                case RustComponentValueParser::RustGridTrackSizeListEventKind::Breadth: {
                    auto value = materialize_rust_grid_track_breadth(event.breadth_kind, event.value);
                    if (!value.has_value())
                        return {};
                    return ExplicitGridTrack(value.release_value());
                }
                case RustComponentValueParser::RustGridTrackSizeListEventKind::MinMax: {
                    auto min_value = materialize_rust_grid_track_breadth(event.breadth_kind, event.value);
                    auto max_value = materialize_rust_grid_track_breadth(event.secondary_breadth_kind, event.secondary_value);
                    if (!min_value.has_value() || !max_value.has_value())
                        return {};
                    return ExplicitGridTrack(GridMinMax(min_value.release_value(), max_value.release_value()));
                }
                case RustComponentValueParser::RustGridTrackSizeListEventKind::FitContent: {
                    auto length_percentage = materialize_rust_nested_length_percentage(event.value, non_negative_range);
                    if (!length_percentage)
                        return {};
                    return ExplicitGridTrack(GridSize(FunctionStyleValue::create("fit-content"_fly_string, length_percentage.release_nonnull())));
                }
                default:
                    return {};
                }
            };
            auto materialize_rust_grid_track_size_list = [&](auto& materialize_rust_grid_track_size_list, Vector<RustComponentValueParser::RustGridTrackSizeListEvent> const& events, size_t& index, bool stop_at_repeat_end) -> Optional<GridTrackSizeList> {
                GridTrackSizeList grid_track_size_list;

                while (index < events.size()) {
                    auto const& event = events[index++];
                    switch (event.kind) {
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::None:
                        return {};
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::LineNames: {
                        GridLineNames line_names;
                        for (auto name : event.source.bytes_as_string_view().split_view('\0')) {
                            if (!name.is_empty())
                                line_names.append(FlyString::from_utf8_without_validation(name.bytes()));
                        }
                        if (!line_names.is_empty())
                            grid_track_size_list.append(move(line_names));
                        break;
                    }
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::Breadth:
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::MinMax:
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::FitContent: {
                        auto explicit_grid_track = materialize_rust_grid_track_size(event);
                        if (!explicit_grid_track.has_value())
                            return {};
                        grid_track_size_list.append(explicit_grid_track.release_value());
                        break;
                    }
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::RepeatBegin: {
                        auto repeat_type = [&] {
                            switch (event.repeat_type) {
                            case RustComponentValueParser::RustGridRepeatType::AutoFill:
                                return GridRepeatType::AutoFill;
                            case RustComponentValueParser::RustGridRepeatType::AutoFit:
                                return GridRepeatType::AutoFit;
                            case RustComponentValueParser::RustGridRepeatType::Fixed:
                                return GridRepeatType::Fixed;
                            }
                            VERIFY_NOT_REACHED();
                        }();

                        RefPtr<StyleValue const> repeat_count;
                        if (repeat_type == GridRepeatType::Fixed) {
                            repeat_count = materialize_rust_nested_integer(event.value, NumericRange { .min = 1, .max = AK::NumericLimits<i32>::max() });
                            if (!repeat_count)
                                return {};
                        }

                        auto nested_list = materialize_rust_grid_track_size_list(materialize_rust_grid_track_size_list, events, index, true);
                        if (!nested_list.has_value())
                            return {};

                        grid_track_size_list.append(ExplicitGridTrack(GridRepeat(nested_list.release_value(), GridRepeatParams { repeat_type, repeat_count })));
                        break;
                    }
                    case RustComponentValueParser::RustGridTrackSizeListEventKind::RepeatEnd:
                        if (stop_at_repeat_end)
                            return grid_track_size_list;
                        return {};
                    }
                }

                if (stop_at_repeat_end)
                    return {};
                return grid_track_size_list;
            };
            auto materialize_rust_grid_template_areas = [](RustComponentValueParser::RustStyleValue const& rust_style_value) -> RefPtr<GridTemplateAreaStyleValue const> {
                if (rust_style_value.grid_template_areas_is_none)
                    return GridTemplateAreaStyleValue::create({}, 0, 0);

                Vector<Vector<String>> grid_area_rows;
                grid_area_rows.ensure_capacity(rust_style_value.grid_template_area_rows.size());
                for (auto const& row_source : rust_style_value.grid_template_area_rows) {
                    Vector<String> row;
                    for (auto cell : row_source.bytes_as_string_view().split_view('\0')) {
                        if (!cell.is_empty())
                            row.append(String::from_utf8_without_validation(cell.bytes()));
                    }
                    grid_area_rows.append(move(row));
                }

                HashMap<String, GridArea> grid_areas;
                for (size_t y = 0; y < grid_area_rows.size(); y++) {
                    for (size_t x = 0; x < grid_area_rows[y].size(); x++) {
                        auto const& name = grid_area_rows[y][x];
                        if (name == "."sv || grid_areas.contains(name))
                            continue;

                        size_t x_end = x;
                        while (x_end < grid_area_rows[y].size() && grid_area_rows[y][x_end] == name)
                            x_end++;
                        size_t y_end = y;
                        while (y_end < grid_area_rows.size() && grid_area_rows[y_end][x] == name)
                            y_end++;

                        grid_areas.set(name, { y, y_end, x, x_end });
                    }
                }

                auto column_count = grid_area_rows.is_empty() ? 0 : grid_area_rows[0].size();
                return GridTemplateAreaStyleValue::create(move(grid_areas), grid_area_rows.size(), column_count);
            };
            auto materialize_rust_simple_filter_function = [](RustComponentValueParser::RustSimpleFilterFunction function) {
                switch (function) {
                case RustComponentValueParser::RustSimpleFilterFunction::Brightness:
                    return Gfx::ColorFilterType::Brightness;
                case RustComponentValueParser::RustSimpleFilterFunction::Contrast:
                    return Gfx::ColorFilterType::Contrast;
                case RustComponentValueParser::RustSimpleFilterFunction::Grayscale:
                    return Gfx::ColorFilterType::Grayscale;
                case RustComponentValueParser::RustSimpleFilterFunction::Invert:
                    return Gfx::ColorFilterType::Invert;
                case RustComponentValueParser::RustSimpleFilterFunction::Opacity:
                    return Gfx::ColorFilterType::Opacity;
                case RustComponentValueParser::RustSimpleFilterFunction::Saturate:
                    return Gfx::ColorFilterType::Saturate;
                case RustComponentValueParser::RustSimpleFilterFunction::Sepia:
                    return Gfx::ColorFilterType::Sepia;
                }
                VERIFY_NOT_REACHED();
            };
            switch (rust_style_value->kind) {
            case FFI::CssStyleValueKind::Invalid:
                break;
            case FFI::CssStyleValueKind::BackgroundSize: {
                if (!rust_style_value->background_sizes.is_empty()) {
                    StyleValueVector values;
                    values.ensure_capacity(rust_style_value->background_sizes.size());
                    for (auto const& background_size : rust_style_value->background_sizes) {
                        if (background_size.keyword.has_value()) {
                            if (!first_is_one_of(*background_size.keyword, Keyword::Cover, Keyword::Contain))
                                break;
                            values.append(KeywordStyleValue::create(*background_size.keyword));
                            continue;
                        }

                        if (!background_size.width.has_value())
                            break;
                        auto width = materialize_rust_nested_background_size_component(*background_size.width);
                        auto height = background_size.height.has_value()
                            ? materialize_rust_nested_background_size_component(*background_size.height)
                            : KeywordStyleValue::create(Keyword::Auto);
                        if (!width || !height)
                            break;

                        values.append(BackgroundSizeStyleValue::create(width.release_nonnull(), height.release_nonnull()));
                    }

                    if (values.size() == rust_style_value->background_sizes.size()) {
                        discard_rust_owned_property_value_tokens();
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Comma) };
                    }
                }
                break;
            }
            case FFI::CssStyleValueKind::Keyword:
                if (rust_style_value->keyword.has_value()) {
                    tokens.discard_a_token();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(*rust_style_value->keyword) };
                }
                break;
            case FFI::CssStyleValueKind::CustomIdent:
                if (rust_style_value->custom_ident.has_value()) {
                    tokens.discard_a_token();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, CustomIdentStyleValue::create(*rust_style_value->custom_ident) };
                }
                break;
            case FFI::CssStyleValueKind::Color:
                tokens.discard_a_token();
                generated_transaction.commit();
                return PropertyAndValue {
                    rust_style_value->property_id,
                    ColorStyleValue::create_from_color(
                        { rust_style_value->color_red, rust_style_value->color_green, rust_style_value->color_blue, rust_style_value->color_alpha },
                        ColorSyntax::Legacy,
                        rust_style_value->string)
                };
            case FFI::CssStyleValueKind::Url:
                if (rust_style_value->string.has_value()) {
                    auto maybe_url = rust_style_value->url.has_value()
                        ? rust_style_value->url
                        : RustComponentValueParser::parse_a_url_function(rust_style_value->string->bytes_as_string_view(), "utf-8"sv);
                    if (maybe_url.has_value()) {
                        tokens.discard_a_token();
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, URLStyleValue::create(maybe_url.release_value()) };
                    }
                }
                break;
            case FFI::CssStyleValueKind::CounterStyleName:
                if (rust_style_value->string.has_value()) {
                    auto counter_style_name = rust_style_value->string.release_value();

                    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
                    // Counter style names are case-sensitive. However, the names defined in this specification are ASCII lowercased
                    // on parse wherever they are used as counter styles, e.g. in the list-style set of properties, in the
                    // @counter-style rule, and in the counter() functions.

                    // NB: The "names defined in this specification" are defined in the `CounterStyleNameKeyword` enum
                    auto const& keyword = keyword_from_string(counter_style_name);
                    if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
                        counter_style_name = counter_style_name.to_ascii_lowercase();

                    tokens.discard_a_token();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, CounterStyleStyleValue::create(counter_style_name) };
                }
                break;
            case FFI::CssStyleValueKind::FontVariantAlternates:
                if (!rust_style_value->font_variant_alternates.is_empty()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, materialize_rust_font_variant_alternates_value().release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::FontVariantEastAsian:
                if (!rust_style_value->font_variant_east_asian.is_empty()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, materialize_rust_font_variant_east_asian_value().release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::FontVariantLigatures:
                if (!rust_style_value->font_variant_ligatures.is_empty()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, materialize_rust_font_variant_ligatures_value().release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::FontVariantNumeric:
                if (!rust_style_value->font_variant_numeric.is_empty()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, materialize_rust_font_variant_numeric_value().release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::EasingFunction:
                if (auto value = materialize_rust_easing_function()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::FitContent:
                if (auto value = materialize_rust_fit_content()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::Image:
                if (rust_style_value->image_kind.has_value() && rust_style_value->image_source.has_value()) {
                    if (auto value = materialize_rust_image_from_original_tokens(*rust_style_value->image_kind, *rust_style_value->image_source, rust_style_value->image_url)) {
                        if (*rust_style_value->image_kind == RustComponentValueParser::RustImageKind::Url)
                            discard_rust_owned_property_value_tokens();
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, value };
                    }
                }
                break;
            case FFI::CssStyleValueKind::BasicShape:
                if (auto value = materialize_rust_basic_shape(rust_style_value->basic_shape_kind, rust_style_value->basic_shape_argument_groups, rust_style_value->basic_shape_fill_rule, rust_style_value->basic_shape_rectangle_components, rust_style_value->basic_shape_rectangle_border_radius_horizontal_radii, rust_style_value->basic_shape_rectangle_border_radius_vertical_radii, rust_style_value->basic_shape_radial_shape_is_typed, rust_style_value->basic_shape_radial_shape_radius, rust_style_value->basic_shape_radial_shape_position, rust_style_value->basic_shape_polygon_coordinates, rust_style_value->basic_shape_path_data)) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::Rect:
                if (auto value = materialize_rust_rect()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::AnchorNameOrScope:
                switch (rust_style_value->anchor_name_or_scope_kind) {
                case FFI::CssAnchorNameOrScopeValueKind::Invalid:
                    break;
                case FFI::CssAnchorNameOrScopeValueKind::All:
                    if (rust_style_value->property_id != PropertyID::AnchorScope)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::All) };
                case FFI::CssAnchorNameOrScopeValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssAnchorNameOrScopeValueKind::List: {
                    StyleValueVector names;
                    names.ensure_capacity(rust_style_value->anchor_names.size());
                    for (auto const& name : rust_style_value->anchor_names)
                        names.unchecked_append(CustomIdentStyleValue::create(name));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(names), StyleValueList::Separator::Comma) };
                }
                }
                break;
            case FFI::CssStyleValueKind::AnimationName:
                switch (rust_style_value->animation_name_kind) {
                case FFI::CssAnimationNameValueKind::Invalid:
                    break;
                case FFI::CssAnimationNameValueKind::List: {
                    StyleValueVector names;
                    VERIFY(rust_style_value->animation_name_item_kinds.size() == rust_style_value->animation_names.size());
                    names.ensure_capacity(rust_style_value->animation_names.size());
                    for (size_t i = 0; i < rust_style_value->animation_names.size(); ++i) {
                        switch (rust_style_value->animation_name_item_kinds[i]) {
                        case FFI::CssAnimationNameItemKind::None:
                            names.unchecked_append(KeywordStyleValue::create(Keyword::None));
                            break;
                        case FFI::CssAnimationNameItemKind::CustomIdent:
                            names.unchecked_append(CustomIdentStyleValue::create(rust_style_value->animation_names[i]));
                            break;
                        case FFI::CssAnimationNameItemKind::String:
                            names.unchecked_append(StringStyleValue::create(rust_style_value->animation_names[i]));
                            break;
                        }
                    }
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(names), StyleValueList::Separator::Comma) };
                }
                }
                break;
            case FFI::CssStyleValueKind::AspectRatio:
                if (rust_style_value->aspect_ratio_numerator.has_value()) {
                    auto numerator = materialize_rust_nested_non_negative_number(*rust_style_value->aspect_ratio_numerator);
                    auto denominator = rust_style_value->aspect_ratio_denominator.has_value()
                        ? materialize_rust_nested_non_negative_number(*rust_style_value->aspect_ratio_denominator)
                        : NumberStyleValue::create(1);
                    if (!numerator || !denominator)
                        break;
                    auto ratio = RatioStyleValue::create(numerator.release_nonnull(), denominator.release_nonnull());
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    if (rust_style_value->aspect_ratio_has_auto) {
                        return PropertyAndValue { rust_style_value->property_id,
                            StyleValueList::create(
                                StyleValueVector { KeywordStyleValue::create(Keyword::Auto), ratio },
                                StyleValueList::Separator::Space) };
                    }
                    return PropertyAndValue { rust_style_value->property_id, ratio };
                }
                if (rust_style_value->aspect_ratio_has_auto) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Auto) };
                }
                break;
            case FFI::CssStyleValueKind::ColorScheme:
                switch (rust_style_value->color_scheme_kind) {
                case FFI::CssColorSchemeValueKind::Invalid:
                    break;
                case FFI::CssColorSchemeValueKind::Normal:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, ColorSchemeStyleValue::normal() };
                case FFI::CssColorSchemeValueKind::List:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, ColorSchemeStyleValue::create(move(rust_style_value->color_scheme_schemes), rust_style_value->color_scheme_only) };
                }
                break;
            case FFI::CssStyleValueKind::Contain: {
                auto append_keyword = [](StyleValueVector& values, Keyword keyword) {
                    values.append(KeywordStyleValue::create(keyword));
                };
                StyleValueVector values;
                switch (rust_style_value->contain.kind) {
                case FFI::CssContainValueKind::Invalid:
                    break;
                case FFI::CssContainValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssContainValueKind::Strict:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Strict) };
                case FFI::CssContainValueKind::Content:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Content) };
                case FFI::CssContainValueKind::List:
                    if (rust_style_value->contain.is_size)
                        append_keyword(values, Keyword::Size);
                    if (rust_style_value->contain.is_inline_size)
                        append_keyword(values, Keyword::InlineSize);
                    if (rust_style_value->contain.has_layout)
                        append_keyword(values, Keyword::Layout);
                    if (rust_style_value->contain.has_style)
                        append_keyword(values, Keyword::Style);
                    if (rust_style_value->contain.has_paint)
                        append_keyword(values, Keyword::Paint);
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Space) };
                }
                break;
            }
            case FFI::CssStyleValueKind::ContainerType: {
                auto append_keyword = [](StyleValueVector& values, Keyword keyword) {
                    values.append(KeywordStyleValue::create(keyword));
                };
                StyleValueVector values;
                switch (rust_style_value->container_type) {
                case FFI::CssContainerTypeValueKind::Invalid:
                    break;
                case FFI::CssContainerTypeValueKind::Normal:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Normal) };
                case FFI::CssContainerTypeValueKind::Size:
                    append_keyword(values, Keyword::Size);
                    break;
                case FFI::CssContainerTypeValueKind::InlineSize:
                    append_keyword(values, Keyword::InlineSize);
                    break;
                case FFI::CssContainerTypeValueKind::ScrollState:
                    append_keyword(values, Keyword::ScrollState);
                    break;
                case FFI::CssContainerTypeValueKind::SizeAndScrollState:
                    append_keyword(values, Keyword::Size);
                    append_keyword(values, Keyword::ScrollState);
                    break;
                case FFI::CssContainerTypeValueKind::InlineSizeAndScrollState:
                    append_keyword(values, Keyword::InlineSize);
                    append_keyword(values, Keyword::ScrollState);
                    break;
                }
                if (!values.is_empty()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Space) };
                }
                break;
            }
            case FFI::CssStyleValueKind::Counter:
                if (rust_style_value->counter_function.has_value()) {
                    auto value = materialize_rust_counter(*rust_style_value->counter_function, rust_style_value->counter_name, rust_style_value->counter_join_string, rust_style_value->counter_style);
                    if (!value)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::CounterStyle:
                if (rust_style_value->counter_style.has_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, materialize_rust_counter_style(rust_style_value->counter_style) };
                }
                break;
            case FFI::CssStyleValueKind::CounterDefinitions:
                if (!rust_style_value->counter_definitions.is_empty()) {
                    VERIFY(rust_style_value->counter_definitions.size() == rust_style_value->counter_definition_values.size());

                    for (size_t i = 0; i < rust_style_value->counter_definitions.size(); ++i) {
                        auto value = materialize_rust_nested_integer(rust_style_value->counter_definition_values[i], infinite_integer_range);
                        if (!value)
                            return {};
                        rust_style_value->counter_definitions[i].value = value.release_nonnull();
                    }

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, CounterDefinitionsStyleValue::create(move(rust_style_value->counter_definitions)) };
                }
                break;
            case FFI::CssStyleValueKind::Display:
                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                switch (rust_style_value->display_kind) {
                case RustComponentValueParser::RustDisplayValueKind::Invalid:
                    break;
                case RustComponentValueParser::RustDisplayValueKind::Box:
                    return PropertyAndValue { rust_style_value->property_id, DisplayStyleValue::create(Display { display_box_from_rust(rust_style_value->display_value) }) };
                case RustComponentValueParser::RustDisplayValueKind::Internal:
                    return PropertyAndValue { rust_style_value->property_id, DisplayStyleValue::create(Display { display_internal_from_rust(rust_style_value->display_value) }) };
                case RustComponentValueParser::RustDisplayValueKind::OutsideAndInside:
                    return PropertyAndValue {
                        rust_style_value->property_id,
                        DisplayStyleValue::create(Display {
                            display_outside_from_rust(rust_style_value->display_value),
                            display_inside_from_rust(rust_style_value->display_inside),
                            rust_style_value->display_list_item == RustComponentValueParser::RustDisplayListItem::Yes ? Display::ListItem::Yes : Display::ListItem::No })
                    };
                }
                break;
            case FFI::CssStyleValueKind::FontFamily:
                if (!rust_style_value->font_family.is_empty()) {
                    StyleValueVector values;
                    values.ensure_capacity(rust_style_value->font_family.size());
                    for (auto const& family_value : rust_style_value->font_family) {
                        switch (family_value.kind) {
                        case FFI::CssFontFamilyValueKind::Generic: {
                            auto maybe_keyword = keyword_from_string(family_value.value);
                            if (!maybe_keyword.has_value() || !keyword_to_generic_font_family(*maybe_keyword).has_value())
                                break;
                            values.append(KeywordStyleValue::create(*maybe_keyword));
                            break;
                        }
                        case FFI::CssFontFamilyValueKind::FamilyName:
                            if (family_value.is_string)
                                values.append(StringStyleValue::create(family_value.value));
                            else
                                values.append(CustomIdentStyleValue::create(family_value.value));
                            break;
                        }
                    }
                    if (values.size() != rust_style_value->font_family.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::FontFeatureSettings:
                if (rust_style_value->open_type_settings_kind == FFI::CssOpenTypeSettingsKind::Normal) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Normal) };
                }
                if (rust_style_value->open_type_settings_kind == FFI::CssOpenTypeSettingsKind::TagValues) {
                    StyleValueVector feature_tags;
                    feature_tags.ensure_capacity(rust_style_value->open_type_tag_values.size());
                    for (auto const& tag_value : rust_style_value->open_type_tag_values) {
                        RefPtr<StyleValue const> value;
                        switch (tag_value.value_kind) {
                        case FFI::CssOpenTypeTaggedValueKind::Implicit:
                        case FFI::CssOpenTypeTaggedValueKind::On:
                            // "If the value is omitted, a value of 1 is assumed."
                            // A value of on is synonymous with 1 and off is synonymous with 0.
                            value = IntegerStyleValue::create(1);
                            break;
                        case FFI::CssOpenTypeTaggedValueKind::Off:
                            // A value of on is synonymous with 1 and off is synonymous with 0.
                            value = IntegerStyleValue::create(0);
                            break;
                        case FFI::CssOpenTypeTaggedValueKind::Value: {
                            VERIFY(tag_value.value.has_value());
                            auto component_values = RustComponentValueParser::parse_a_list_of_component_values(*tag_value.value, "utf-8"sv);
                            TokenStream value_tokens { component_values };
                            value = parse_integer_value(value_tokens, non_negative_integer_range);
                            value_tokens.discard_whitespace();
                            if (!value || value_tokens.has_next_token())
                                break;
                            break;
                        }
                        }
                        if (!value)
                            break;

                        feature_tags.append(OpenTypeTaggedStyleValue::create(OpenTypeTaggedStyleValue::Mode::FontFeatureSettings, tag_value.tag, value.release_nonnull()));
                    }
                    if (feature_tags.size() != rust_style_value->open_type_tag_values.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(feature_tags), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::FontLanguageOverride:
                switch (rust_style_value->font_language_override_kind) {
                case FFI::CssFontLanguageOverrideKind::Normal:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Normal) };
                case FFI::CssFontLanguageOverrideKind::String:
                    if (rust_style_value->font_language_override.has_value()) {
                        discard_rust_owned_property_value_tokens();
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, StringStyleValue::create(rust_style_value->font_language_override.release_value()) };
                    }
                    break;
                }
                break;
            case FFI::CssStyleValueKind::FontStyle: {
                auto font_style_keyword = font_style_keyword_from_rust(rust_style_value->font_style.kind);
                if (rust_style_value->font_style.has_angle) {
                    if (!rust_style_value->font_style_angle.has_value())
                        break;
                    auto component_values = RustComponentValueParser::parse_a_list_of_component_values(*rust_style_value->font_style_angle, "utf-8"sv);
                    TokenStream angle_tokens { component_values };
                    auto angle_value = parse_angle_value(angle_tokens, { .min = -90, .max = 90 });
                    angle_tokens.discard_whitespace();
                    if (!angle_value || angle_tokens.has_next_token())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, FontStyleStyleValue::create(font_style_keyword, angle_value.release_nonnull()) };
                }
                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { rust_style_value->property_id, FontStyleStyleValue::create(font_style_keyword) };
            }
            case FFI::CssStyleValueKind::FontVariant:
                if (auto value = materialize_rust_font_variant_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::FontVariationSettings:
                if (rust_style_value->open_type_settings_kind == FFI::CssOpenTypeSettingsKind::Normal) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Normal) };
                }
                if (rust_style_value->open_type_settings_kind == FFI::CssOpenTypeSettingsKind::TagValues) {
                    StyleValueVector axis_tags;
                    axis_tags.ensure_capacity(rust_style_value->open_type_tag_values.size());
                    for (auto const& tag_value : rust_style_value->open_type_tag_values) {
                        if (tag_value.value_kind != FFI::CssOpenTypeTaggedValueKind::Value || !tag_value.value.has_value())
                            break;

                        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(*tag_value.value, "utf-8"sv);
                        TokenStream value_tokens { component_values };
                        auto number = parse_number_value(value_tokens, infinite_range);
                        value_tokens.discard_whitespace();
                        if (!number || value_tokens.has_next_token())
                            break;

                        axis_tags.append(OpenTypeTaggedStyleValue::create(OpenTypeTaggedStyleValue::Mode::FontVariationSettings, tag_value.tag, number.release_nonnull()));
                    }
                    if (axis_tags.size() != rust_style_value->open_type_tag_values.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(axis_tags), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::BorderRadius:
                if (rust_style_value->border_radius_horizontal_radii.is_empty())
                    break;
                if (rust_style_value->property_id == PropertyID::BorderRadius) {
                    auto parse_radius_values = [&](Vector<RustComponentValueParser::RustNestedPrimitiveValue> const& radii) -> Optional<StyleValueVector> {
                        StyleValueVector values;
                        values.ensure_capacity(radii.size());
                        for (auto const& radius : radii) {
                            auto value = materialize_rust_nested_length_percentage(radius, non_negative_range);
                            if (!value)
                                return {};
                            values.append(value.release_nonnull());
                        }
                        return values;
                    };
                    auto top_left = [](StyleValueVector& radii) { return radii[0]; };
                    auto top_right = [](StyleValueVector& radii) {
                        switch (radii.size()) {
                        case 4:
                        case 3:
                        case 2:
                            return radii[1];
                        case 1:
                            return radii[0];
                        default:
                            VERIFY_NOT_REACHED();
                        }
                    };
                    auto bottom_right = [](StyleValueVector& radii) {
                        switch (radii.size()) {
                        case 4:
                        case 3:
                            return radii[2];
                        case 2:
                        case 1:
                            return radii[0];
                        default:
                            VERIFY_NOT_REACHED();
                        }
                    };
                    auto bottom_left = [](StyleValueVector& radii) {
                        switch (radii.size()) {
                        case 4:
                            return radii[3];
                        case 3:
                        case 2:
                            return radii[1];
                        case 1:
                            return radii[0];
                        default:
                            VERIFY_NOT_REACHED();
                        }
                    };

                    auto maybe_horizontal_radii = parse_radius_values(rust_style_value->border_radius_horizontal_radii);
                    auto maybe_vertical_radii = rust_style_value->border_radius_vertical_radii.is_empty()
                        ? Optional<StyleValueVector> {}
                        : parse_radius_values(rust_style_value->border_radius_vertical_radii);
                    if (!maybe_horizontal_radii.has_value() || (!rust_style_value->border_radius_vertical_radii.is_empty() && !maybe_vertical_radii.has_value()))
                        break;

                    auto& horizontal_radii = *maybe_horizontal_radii;
                    auto& vertical_radii = maybe_vertical_radii.has_value() ? *maybe_vertical_radii : horizontal_radii;
                    auto top_left_radius = BorderRadiusStyleValue::create(top_left(horizontal_radii), top_left(vertical_radii));
                    auto top_right_radius = BorderRadiusStyleValue::create(top_right(horizontal_radii), top_right(vertical_radii));
                    auto bottom_right_radius = BorderRadiusStyleValue::create(bottom_right(horizontal_radii), bottom_right(vertical_radii));
                    auto bottom_left_radius = BorderRadiusStyleValue::create(bottom_left(horizontal_radii), bottom_left(vertical_radii));

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::BorderRadius,
                            { PropertyID::BorderTopLeftRadius, PropertyID::BorderTopRightRadius, PropertyID::BorderBottomRightRadius, PropertyID::BorderBottomLeftRadius },
                            { top_left_radius, top_right_radius, bottom_right_radius, bottom_left_radius }) };
                } else {
                    auto horizontal = materialize_rust_nested_length_percentage(rust_style_value->border_radius_horizontal_radii[0], non_negative_range);
                    auto vertical = rust_style_value->border_radius_vertical_radii.is_empty()
                        ? horizontal
                        : materialize_rust_nested_length_percentage(rust_style_value->border_radius_vertical_radii[0], non_negative_range);
                    if (!horizontal || !vertical)
                        break;

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, BorderRadiusStyleValue::create(horizontal.release_nonnull(), vertical.release_nonnull()) };
                }
            case FFI::CssStyleValueKind::BorderImageSlice:
                if (auto value = materialize_rust_border_image_slice(rust_style_value->border_image_slices)) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::BorderImageOutset:
                if (auto value = materialize_rust_style_value_list(rust_style_value->border_image_outsets, materialize_rust_border_image_outset)) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::Border: {
                auto width_property = PropertyID::BorderWidth;
                auto style_property = PropertyID::BorderStyle;
                auto color_property = PropertyID::BorderColor;
                switch (rust_style_value->property_id) {
                case PropertyID::Border:
                    break;
                case PropertyID::BorderBlock:
                    width_property = PropertyID::BorderBlockWidth;
                    style_property = PropertyID::BorderBlockStyle;
                    color_property = PropertyID::BorderBlockColor;
                    break;
                case PropertyID::BorderInline:
                    width_property = PropertyID::BorderInlineWidth;
                    style_property = PropertyID::BorderInlineStyle;
                    color_property = PropertyID::BorderInlineColor;
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }

                auto const make_single_value_shorthand = [&](PropertyID property_id, Vector<PropertyID> const& longhands, ValueComparingNonnullRefPtr<StyleValue const> const& value) {
                    Vector<ValueComparingNonnullRefPtr<StyleValue const>> longhand_values;
                    longhand_values.resize_with_default_value(longhands.size(), value);

                    return ShorthandStyleValue::create(property_id, longhands, longhand_values);
                };
                RefPtr<StyleValue const> width;
                if (rust_style_value->border_width_keyword.has_value())
                    width = make_single_value_shorthand(width_property, longhands_for_shorthand(width_property), KeywordStyleValue::create(to_keyword(*rust_style_value->border_width_keyword)));
                else if (rust_style_value->border_width_length.has_value()) {
                    auto width_value = materialize_rust_nested_length(*rust_style_value->border_width_length, non_negative_range);
                    if (!width_value)
                        break;
                    width = make_single_value_shorthand(width_property, longhands_for_shorthand(width_property), width_value.release_nonnull());
                } else {
                    width = property_initial_value(width_property);
                }
                if (!width)
                    break;

                RefPtr<StyleValue const> style = rust_style_value->border_style.has_value()
                    ? make_single_value_shorthand(style_property, longhands_for_shorthand(style_property), KeywordStyleValue::create(to_keyword(*rust_style_value->border_style)))
                    : property_initial_value(style_property);

                RefPtr<StyleValue const> color;
                if (rust_style_value->border_color.has_value()) {
                    auto color_value = materialize_rust_style_color(*rust_style_value->border_color, [&](String const& source) {
                        return parse_rust_source_as_property(color_property, source);
                    });
                    if (!color_value)
                        break;
                    color = make_single_value_shorthand(color_property, longhands_for_shorthand(color_property), color_value.release_nonnull());
                } else {
                    color = property_initial_value(color_property);
                }
                if (!color)
                    break;
                auto width_nonnull = width.release_nonnull();
                auto style_nonnull = style.release_nonnull();
                auto color_nonnull = color.release_nonnull();

                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();

                if (first_is_one_of(rust_style_value->property_id, PropertyID::BorderBlock, PropertyID::BorderInline)) {
                    return PropertyAndValue {
                        rust_style_value->property_id,
                        ShorthandStyleValue::create(rust_style_value->property_id,
                            { width_property, style_property, color_property },
                            { width_nonnull, style_nonnull, color_nonnull })
                    };
                }

                return PropertyAndValue {
                    rust_style_value->property_id,
                    ShorthandStyleValue::create(PropertyID::Border,
                        { width_property, style_property, color_property, PropertyID::BorderImage },
                        { width_nonnull, style_nonnull, color_nonnull, property_initial_value(PropertyID::BorderImage) })
                };
            }
            case FFI::CssStyleValueKind::BorderImage: {
                RefPtr<StyleValue const> source;
                if (rust_style_value->border_image_source_kind.has_value()) {
                    switch (*rust_style_value->border_image_source_kind) {
                    case RustComponentValueParser::RustBorderImageSourceKind::None:
                        source = KeywordStyleValue::create(Keyword::None);
                        break;
                    case RustComponentValueParser::RustBorderImageSourceKind::Source:
                        if (!rust_style_value->border_image_source_source.has_value() || !rust_style_value->border_image_source_source_kind.has_value())
                            break;
                        source = materialize_rust_image(*rust_style_value->border_image_source_source_kind, *rust_style_value->border_image_source_source, rust_style_value->border_image_source_source_url);
                        break;
                    }
                } else {
                    source = property_initial_value(PropertyID::BorderImageSource);
                }
                auto slice = rust_style_value->border_image_shorthand_has_slice
                    ? materialize_rust_border_image_slice(rust_style_value->border_image_slices)
                    : property_initial_value(PropertyID::BorderImageSlice);
                auto width = rust_style_value->border_image_shorthand_has_width
                    ? materialize_rust_style_value_list(rust_style_value->border_image_widths, materialize_rust_border_image_width)
                    : property_initial_value(PropertyID::BorderImageWidth);
                auto outset = rust_style_value->border_image_shorthand_has_outset
                    ? materialize_rust_style_value_list(rust_style_value->border_image_outsets, materialize_rust_border_image_outset)
                    : property_initial_value(PropertyID::BorderImageOutset);
                auto repeat = rust_style_value->border_image_shorthand_has_repeat
                    ? materialize_rust_border_image_repeat(rust_style_value->border_image_repeats)
                    : property_initial_value(PropertyID::BorderImageRepeat);
                if (!source || !slice || !width || !outset || !repeat)
                    break;

                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue {
                    rust_style_value->property_id,
                    ShorthandStyleValue::create(PropertyID::BorderImage,
                        { PropertyID::BorderImageSource, PropertyID::BorderImageSlice, PropertyID::BorderImageWidth, PropertyID::BorderImageOutset, PropertyID::BorderImageRepeat },
                        { source.release_nonnull(), slice.release_nonnull(), width.release_nonnull(), outset.release_nonnull(), repeat.release_nonnull() })
                };
            }
            case FFI::CssStyleValueKind::BorderImageRepeat:
                if (auto value = materialize_rust_border_image_repeat(rust_style_value->border_image_repeats)) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::BorderImageWidth:
                if (auto value = materialize_rust_style_value_list(rust_style_value->border_image_widths, materialize_rust_border_image_width)) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::Columns:
                if (rust_style_value->column_count.has_value() || rust_style_value->column_count_is_auto || rust_style_value->column_width.has_value() || rust_style_value->column_width_is_auto || rust_style_value->column_height.has_value() || rust_style_value->column_height_is_auto) {
                    RefPtr<StyleValue const> column_count;
                    if (rust_style_value->column_count.has_value())
                        column_count = materialize_rust_nested_integer(*rust_style_value->column_count, NumericRange { .min = 1, .max = AK::NumericLimits<i32>::max() });
                    else if (rust_style_value->column_count_is_auto)
                        column_count = KeywordStyleValue::create(Keyword::Auto);
                    else
                        column_count = property_initial_value(PropertyID::ColumnCount);

                    RefPtr<StyleValue const> column_width;
                    if (rust_style_value->column_width.has_value())
                        column_width = materialize_rust_nested_length(*rust_style_value->column_width, non_negative_range);
                    else if (rust_style_value->column_width_is_auto)
                        column_width = KeywordStyleValue::create(Keyword::Auto);
                    else
                        column_width = property_initial_value(PropertyID::ColumnWidth);

                    RefPtr<StyleValue const> column_height;
                    if (rust_style_value->column_height.has_value())
                        column_height = materialize_rust_nested_length(*rust_style_value->column_height, non_negative_range);
                    else if (rust_style_value->column_height_is_auto)
                        column_height = KeywordStyleValue::create(Keyword::Auto);
                    else
                        column_height = property_initial_value(PropertyID::ColumnHeight);

                    if (!column_count || !column_width || !column_height)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::Columns,
                            { PropertyID::ColumnCount, PropertyID::ColumnWidth, PropertyID::ColumnHeight },
                            { column_count.release_nonnull(), column_width.release_nonnull(), column_height.release_nonnull() }) };
                }
                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { rust_style_value->property_id,
                    ShorthandStyleValue::create(PropertyID::Columns,
                        { PropertyID::ColumnCount, PropertyID::ColumnWidth, PropertyID::ColumnHeight },
                        { property_initial_value(PropertyID::ColumnCount), property_initial_value(PropertyID::ColumnWidth), property_initial_value(PropertyID::ColumnHeight) }) };
            case FFI::CssStyleValueKind::Content:
                if (auto value = materialize_rust_content_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::Cursor:
                if (rust_style_value->cursor_predefined.has_value()) {
                    StyleValueVector cursors;
                    cursors.ensure_capacity(rust_style_value->cursor_images.size() + 1);
                    for (auto const& cursor_image : rust_style_value->cursor_images) {
                        auto image = materialize_rust_image(cursor_image.image_kind, cursor_image.image_source, cursor_image.image_url);
                        if (!image)
                            break;

                        RefPtr<StyleValue const> x;
                        RefPtr<StyleValue const> y;
                        if (cursor_image.x.has_value() && cursor_image.y.has_value()) {
                            x = materialize_rust_nested_number(*cursor_image.x);
                            y = materialize_rust_nested_number(*cursor_image.y);
                            if (!x || !y)
                                break;
                        } else if (cursor_image.x.has_value() || cursor_image.y.has_value()) {
                            break;
                        }

                        cursors.unchecked_append(CursorStyleValue::create(image.release_nonnull(), move(x), move(y)));
                    }

                    if (cursors.size() != rust_style_value->cursor_images.size())
                        break;

                    auto keyword = keyword_from_string(rust_style_value->cursor_predefined->bytes_as_string_view());
                    if (!keyword.has_value() || !keyword_to_cursor_predefined(*keyword).has_value())
                        break;

                    cursors.unchecked_append(KeywordStyleValue::create(*keyword));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    if (cursors.size() == 1)
                        return PropertyAndValue { rust_style_value->property_id, *cursors.first() };

                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(cursors), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::Flex:
                if (rust_style_value->flex_shorthand_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::Flex,
                            { PropertyID::FlexGrow, PropertyID::FlexShrink, PropertyID::FlexBasis },
                            { NumberStyleValue::create(0), NumberStyleValue::create(0), KeywordStyleValue::create(Keyword::Auto) }) };
                }
                if (rust_style_value->flex_grow.has_value() && rust_style_value->flex_shrink.has_value() && rust_style_value->flex_basis_kind.has_value()) {
                    auto flex_grow = materialize_rust_nested_non_negative_number(*rust_style_value->flex_grow);
                    auto flex_shrink = materialize_rust_nested_non_negative_number(*rust_style_value->flex_shrink);
                    auto flex_basis = materialize_rust_flex_basis(*rust_style_value);
                    if (!flex_grow
                        && rust_style_value->flex_grow->primitive_kind == FFI::CssPrimitiveValueKind::Invalid
                        && rust_style_value->flex_shrink->primitive_kind == FFI::CssPrimitiveValueKind::Number
                        && rust_style_value->flex_shrink->numeric_value.has_value()
                        && *rust_style_value->flex_shrink->numeric_value == 1
                        && *rust_style_value->flex_basis_kind == RustComponentValueParser::RustFlexBasisKind::LengthPercentage
                        && rust_style_value->flex_basis.has_value()
                        && rust_style_value->flex_basis->primitive_kind == FFI::CssPrimitiveValueKind::Percentage
                        && rust_style_value->flex_basis->numeric_value.has_value()
                        && *rust_style_value->flex_basis->numeric_value == 0) {
                        // NOTE: The spec says that flex-basis should be 0 here, but other engines currently use 0%.
                        // https://github.com/w3c/csswg-drafts/issues/5742
                        flex_grow = NumberStyleValue::create(1);
                        flex_basis = parse_rust_source_as_property(PropertyID::FlexBasis, rust_style_value->flex_grow->source_or_unit);
                    }
                    if (!flex_grow || !flex_shrink || !flex_basis)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::Flex,
                            { PropertyID::FlexGrow, PropertyID::FlexShrink, PropertyID::FlexBasis },
                            { flex_grow.release_nonnull(), flex_shrink.release_nonnull(), flex_basis.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::FlexFlow:
                if (rust_style_value->flex_direction.has_value() || rust_style_value->flex_wrap.has_value()) {
                    RefPtr<StyleValue const> flex_direction = rust_style_value->flex_direction.has_value()
                        ? KeywordStyleValue::create(to_keyword(*rust_style_value->flex_direction))
                        : property_initial_value(PropertyID::FlexDirection);
                    RefPtr<StyleValue const> flex_wrap = rust_style_value->flex_wrap.has_value()
                        ? KeywordStyleValue::create(to_keyword(*rust_style_value->flex_wrap))
                        : property_initial_value(PropertyID::FlexWrap);
                    if (!flex_direction || !flex_wrap)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::FlexFlow,
                            { PropertyID::FlexDirection, PropertyID::FlexWrap },
                            { flex_direction.release_nonnull(), flex_wrap.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::FilterValueList:
                if (rust_style_value->filter_value_list_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (!rust_style_value->filter_value_list_events.is_empty()) {
                    Vector<FilterValue> filter_value_list;
                    filter_value_list.ensure_capacity(rust_style_value->filter_value_list_events.size());
                    for (auto const& event : rust_style_value->filter_value_list_events) {
                        switch (event.kind) {
                        case RustComponentValueParser::RustFilterValueListEventKind::None:
                            break;
                        case RustComponentValueParser::RustFilterValueListEventKind::Url: {
                            auto url = event.url.has_value() ? event.url : parse_rust_source_as_url(event.source);
                            if (!url.has_value())
                                break;
                            filter_value_list.append(url.release_value());
                            break;
                        }
                        case RustComponentValueParser::RustFilterValueListEventKind::DropShadowRadius:
                        case RustComponentValueParser::RustFilterValueListEventKind::DropShadowColor:
                            VERIFY_NOT_REACHED();
                        case RustComponentValueParser::RustFilterValueListEventKind::Blur: {
                            auto radius = event.has_value
                                ? materialize_rust_nested_length(event.value, non_negative_range)
                                : LengthStyleValue::create(Length::make_px(0));
                            if (!radius)
                                break;
                            filter_value_list.append(FilterOperation::Blur { radius.release_nonnull() });
                            break;
                        }
                        case RustComponentValueParser::RustFilterValueListEventKind::DropShadow: {
                            auto offset_x = materialize_rust_nested_length(event.value, infinite_range);
                            auto offset_y = materialize_rust_nested_length(event.secondary_value, infinite_range);
                            if (!offset_x || !offset_y)
                                break;

                            RefPtr<StyleValue const> radius;
                            if (event.drop_shadow_radius.has_value()) {
                                radius = materialize_rust_nested_length(*event.drop_shadow_radius, infinite_range);
                                if (!radius)
                                    break;
                            }

                            RefPtr<StyleValue const> color;
                            if (event.drop_shadow_color.has_value()) {
                                color = materialize_rust_style_color(*event.drop_shadow_color, parse_rust_source_as_color);
                                if (!color)
                                    break;
                            }

                            filter_value_list.append(FilterOperation::DropShadow { offset_x.release_nonnull(), offset_y.release_nonnull(), radius, color });
                            break;
                        }
                        case RustComponentValueParser::RustFilterValueListEventKind::HueRotate: {
                            auto angle = event.has_value
                                ? materialize_rust_nested_angle(event.value)
                                : AngleStyleValue::create(Angle::make_degrees(0));
                            if (!angle)
                                break;
                            filter_value_list.append(FilterOperation::HueRotate { angle.release_nonnull() });
                            break;
                        }
                        case RustComponentValueParser::RustFilterValueListEventKind::Simple: {
                            auto amount = event.has_value
                                ? materialize_rust_nested_non_negative_number_percentage(event.value)
                                : NumberStyleValue::create(1);
                            if (!amount)
                                break;

                            auto operation = materialize_rust_simple_filter_function(event.simple_function);
                            if (first_is_one_of(operation, Gfx::ColorFilterType::Grayscale, Gfx::ColorFilterType::Invert, Gfx::ColorFilterType::Opacity, Gfx::ColorFilterType::Sepia)) {
                                if (amount->is_percentage() && amount->as_percentage().percentage().value() > 100)
                                    amount = PercentageStyleValue::create(Percentage { 100 });
                                if (amount->is_number() && amount->as_number().number() > 1)
                                    amount = NumberStyleValue::create(1);
                            }

                            filter_value_list.append(FilterOperation::Color { operation, amount.release_nonnull() });
                            break;
                        }
                        }
                    }

                    if (filter_value_list.size() != rust_style_value->filter_value_list_events.size())
                        break;

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, FilterValueListStyleValue::create(move(filter_value_list)) };
                }
                break;
            case FFI::CssStyleValueKind::GridAutoFlow: {
                auto axis = rust_style_value->grid_auto_flow_axis == 1
                    ? GridAutoFlowStyleValue::Axis::Column
                    : GridAutoFlowStyleValue::Axis::Row;
                auto dense = rust_style_value->grid_auto_flow_dense == 1
                    ? GridAutoFlowStyleValue::Dense::Yes
                    : GridAutoFlowStyleValue::Dense::No;
                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { rust_style_value->property_id, GridAutoFlowStyleValue::create(axis, dense) };
            }
            case FFI::CssStyleValueKind::GridTemplateAreas:
                if (rust_style_value->grid_template_areas_is_none || !rust_style_value->grid_template_area_rows.is_empty()) {
                    auto value = materialize_rust_grid_template_areas(*rust_style_value);
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::GridAutoTrackSizes:
                if (!rust_style_value->grid_track_size_list_is_none || !rust_style_value->grid_track_size_list_events.is_empty()) {
                    size_t event_index = 0;
                    auto value = rust_style_value->grid_track_size_list_is_none
                        ? Optional<GridTrackSizeList> { GridTrackSizeList {} }
                        : materialize_rust_grid_track_size_list(materialize_rust_grid_track_size_list, rust_style_value->grid_track_size_list_events, event_index, false);
                    if (!value.has_value() || event_index != rust_style_value->grid_track_size_list_events.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, GridTrackSizeListStyleValue::create(value.release_value()) };
                }
                break;
            case FFI::CssStyleValueKind::GridTrackPlacement:
                if (rust_style_value->grid_track_placement.has_value()) {
                    auto value = materialize_rust_grid_track_placement(*rust_style_value->grid_track_placement);
                    if (!value)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::GridTrackSizeList:
                if (rust_style_value->grid_track_size_list_is_none || !rust_style_value->grid_track_size_list_events.is_empty()) {
                    size_t event_index = 0;
                    auto value = rust_style_value->grid_track_size_list_is_none
                        ? Optional<GridTrackSizeList> { GridTrackSizeList::make_none() }
                        : materialize_rust_grid_track_size_list(materialize_rust_grid_track_size_list, rust_style_value->grid_track_size_list_events, event_index, false);
                    if (!value.has_value() || event_index != rust_style_value->grid_track_size_list_events.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, GridTrackSizeListStyleValue::create(value.release_value()) };
                }
                break;
            case FFI::CssStyleValueKind::ListStyle:
                if (rust_style_value->list_style_position.has_value() || rust_style_value->list_style_image_kind.has_value() || rust_style_value->list_style_type_kind.has_value()) {
                    RefPtr<StyleValue const> list_position = rust_style_value->list_style_position.has_value()
                        ? KeywordStyleValue::create(list_style_position_keyword_from_rust(*rust_style_value->list_style_position))
                        : property_initial_value(PropertyID::ListStylePosition);
                    auto list_image = materialize_rust_list_style_image();
                    auto list_type = materialize_rust_list_style_type();
                    if (!list_position || !list_image || !list_type)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::ListStyle,
                            { PropertyID::ListStylePosition, PropertyID::ListStyleImage, PropertyID::ListStyleType },
                            { list_position.release_nonnull(), list_image.release_nonnull(), list_type.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::MathDepth:
                if (rust_style_value->color_red == 0) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::AutoAdd) };
                }
                if (rust_style_value->math_depth_integer.has_value()) {
                    auto integer_value = materialize_rust_nested_integer(*rust_style_value->math_depth_integer, infinite_integer_range);
                    if (!integer_value)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    if (rust_style_value->color_red == 1) {
                        return PropertyAndValue { rust_style_value->property_id,
                            FunctionStyleValue::create("add"_fly_string, integer_value.release_nonnull()) };
                    }
                    return PropertyAndValue { rust_style_value->property_id, integer_value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::TransformLonghand:
                if (rust_style_value->transform_longhand_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (rust_style_value->transform_longhand_function.has_value()) {
                    auto const& arguments = rust_style_value->transform_longhand_arguments;
                    auto materialize_rotation = [&](TransformFunction function) -> RefPtr<StyleValue const> {
                        if (arguments.size() != 1)
                            return nullptr;
                        auto angle = materialize_rust_nested_angle(arguments[0].value);
                        if (!angle)
                            return nullptr;
                        return TransformationStyleValue::create(rust_style_value->property_id, function, { angle.release_nonnull() });
                    };
                    auto materialize_translate = [&]() -> RefPtr<StyleValue const> {
                        if (arguments.size() != 2)
                            return nullptr;
                        auto x = materialize_rust_nested_length_percentage(arguments[0].value, infinite_range);
                        auto y = materialize_rust_nested_length_percentage(arguments[1].value, infinite_range);
                        if (!x || !y)
                            return nullptr;
                        return TransformationStyleValue::create(rust_style_value->property_id, TransformFunction::Translate, { x.release_nonnull(), y.release_nonnull() });
                    };
                    auto materialize_translate3d = [&]() -> RefPtr<StyleValue const> {
                        if (arguments.size() != 3)
                            return nullptr;
                        auto x = materialize_rust_nested_length_percentage(arguments[0].value, infinite_range);
                        auto y = materialize_rust_nested_length_percentage(arguments[1].value, infinite_range);
                        auto z = materialize_rust_nested_length(arguments[2].value, infinite_range);
                        if (!x || !y || !z)
                            return nullptr;
                        return TransformationStyleValue::create(rust_style_value->property_id, TransformFunction::Translate3d, { x.release_nonnull(), y.release_nonnull(), z.release_nonnull() });
                    };
                    auto materialize_scale = [&]() -> RefPtr<StyleValue const> {
                        if (arguments.size() != 1 && arguments.size() != 2)
                            return nullptr;
                        auto x = materialize_rust_nested_number_percentage(arguments[0].value);
                        auto y = arguments.size() == 1 ? x : materialize_rust_nested_number_percentage(arguments[1].value);
                        if (!x || !y)
                            return nullptr;
                        return TransformationStyleValue::create(rust_style_value->property_id, TransformFunction::Scale, { x.release_nonnull(), y.release_nonnull() });
                    };
                    auto materialize_scale3d = [&]() -> RefPtr<StyleValue const> {
                        if (arguments.size() != 3)
                            return nullptr;
                        auto x = materialize_rust_nested_number_percentage(arguments[0].value);
                        auto y = materialize_rust_nested_number_percentage(arguments[1].value);
                        auto z = materialize_rust_nested_number_percentage(arguments[2].value);
                        if (!x || !y || !z)
                            return nullptr;
                        return TransformationStyleValue::create(rust_style_value->property_id, TransformFunction::Scale3d, { x.release_nonnull(), y.release_nonnull(), z.release_nonnull() });
                    };
                    auto materialize_rotate3d = [&]() -> RefPtr<StyleValue const> {
                        if (arguments.size() != 4)
                            return nullptr;
                        auto x = materialize_rust_nested_number(arguments[0].value);
                        auto y = materialize_rust_nested_number(arguments[1].value);
                        auto z = materialize_rust_nested_number(arguments[2].value);
                        auto angle = materialize_rust_nested_angle(arguments[3].value);
                        if (!x || !y || !z || !angle)
                            return nullptr;
                        return TransformationStyleValue::create(rust_style_value->property_id, TransformFunction::Rotate3d, { x.release_nonnull(), y.release_nonnull(), z.release_nonnull(), angle.release_nonnull() });
                    };

                    RefPtr<StyleValue const> value;
                    switch (*rust_style_value->transform_longhand_function) {
                    case RustComponentValueParser::RustTransformLonghandFunction::Rotate:
                        value = materialize_rotation(TransformFunction::Rotate);
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::RotateX:
                        value = materialize_rotation(TransformFunction::RotateX);
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::RotateY:
                        value = materialize_rotation(TransformFunction::RotateY);
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::RotateZ:
                        value = materialize_rotation(TransformFunction::RotateZ);
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::Rotate3d:
                        value = materialize_rotate3d();
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::Translate:
                        value = materialize_translate();
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::Translate3d:
                        value = materialize_translate3d();
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::Scale:
                        value = materialize_scale();
                        break;
                    case RustComponentValueParser::RustTransformLonghandFunction::Scale3d:
                        value = materialize_scale3d();
                        break;
                    }

                    if (value) {
                        discard_rust_owned_property_value_tokens();
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, value };
                    }
                }
                break;
            case FFI::CssStyleValueKind::Transformation:
                if (!rust_style_value->transformations.is_empty()) {
                    StyleValueVector transformations;
                    transformations.ensure_capacity(rust_style_value->transformations.size());
                    for (auto const& transformation : rust_style_value->transformations) {
                        auto value = materialize_rust_transformation(transformation, rust_style_value->property_id);
                        if (!value)
                            break;
                        transformations.append(value.release_nonnull());
                    }
                    if (transformations.size() != rust_style_value->transformations.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(transformations), StyleValueList::Separator::Space) };
                }
                break;
            case FFI::CssStyleValueKind::TransformOrigin:
                if (rust_style_value->transform_origin_x.has_value() && rust_style_value->transform_origin_y.has_value() && rust_style_value->transform_origin_z.has_value()) {
                    auto x_value = materialize_rust_nested_transform_origin_component(*rust_style_value->transform_origin_x);
                    auto y_value = materialize_rust_nested_transform_origin_component(*rust_style_value->transform_origin_y);
                    auto z_value = materialize_rust_nested_length(*rust_style_value->transform_origin_z, infinite_range);
                    if (!x_value || !y_value || !z_value)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        StyleValueList::create(
                            StyleValueVector { x_value.release_nonnull(), y_value.release_nonnull(), z_value.release_nonnull() },
                            StyleValueList::Separator::Space) };
                }
                break;
            case FFI::CssStyleValueKind::CornerShape:
                if (rust_style_value->corner_shape_keyword.has_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(*rust_style_value->corner_shape_keyword) };
                }
                if (rust_style_value->corner_shape_superellipse_parameter.has_value()) {
                    auto parameter = materialize_rust_nested_number(*rust_style_value->corner_shape_superellipse_parameter);
                    if (!parameter)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, SuperellipseStyleValue::create(parameter.release_nonnull()) };
                }
                break;
            case FFI::CssStyleValueKind::Paint:
                if (rust_style_value->paint_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (rust_style_value->paint_color.has_value()) {
                    auto color = materialize_rust_style_color(*rust_style_value->paint_color, parse_rust_source_as_color);
                    if (!color)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, color.release_nonnull() };
                }
                if (rust_style_value->paint_url_source.has_value()) {
                    auto maybe_url = rust_style_value->paint_url.has_value()
                        ? rust_style_value->paint_url
                        : RustComponentValueParser::parse_a_url_function(rust_style_value->paint_url_source->bytes_as_string_view(), "utf-8"sv);
                    if (!maybe_url.has_value())
                        break;

                    RefPtr<StyleValue const> paint_fallback;
                    if (rust_style_value->paint_fallback_color.has_value()) {
                        paint_fallback = materialize_rust_style_color(*rust_style_value->paint_fallback_color, parse_rust_source_as_color);
                        if (!paint_fallback)
                            break;
                    }

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, URLStyleValue::create(maybe_url.release_value(), paint_fallback) };
                }
                break;
            case FFI::CssStyleValueKind::PaintOrder:
                switch (rust_style_value->paint_order.kind) {
                case FFI::CssPaintOrderValueKind::Invalid:
                    break;
                case FFI::CssPaintOrderValueKind::Normal:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Normal) };
                case FFI::CssPaintOrderValueKind::Keyword:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(paint_order_keyword_from_rust(rust_style_value->paint_order.first)) };
                case FFI::CssPaintOrderValueKind::Pair:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        StyleValueList::create({
                                                   KeywordStyleValue::create(paint_order_keyword_from_rust(rust_style_value->paint_order.first)),
                                                   KeywordStyleValue::create(paint_order_keyword_from_rust(rust_style_value->paint_order.second)),
                                               },
                            StyleValueList::Separator::Space) };
                }
                break;
            case FFI::CssStyleValueKind::PlaceContent:
                if (!rust_style_value->place_align_keywords.is_empty() && !rust_style_value->place_justify_keywords.is_empty()) {
                    auto align_content = materialize_rust_keyword_list(rust_style_value->place_align_keywords);
                    auto justify_content = materialize_rust_keyword_list(rust_style_value->place_justify_keywords);
                    if (!align_content || !justify_content)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::PlaceContent,
                            { PropertyID::AlignContent, PropertyID::JustifyContent },
                            { align_content.release_nonnull(), justify_content.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::PlaceItems:
                if (!rust_style_value->place_align_keywords.is_empty() && !rust_style_value->place_justify_keywords.is_empty()) {
                    auto align_items = materialize_rust_keyword_list(rust_style_value->place_align_keywords);
                    auto justify_items = materialize_rust_keyword_list(rust_style_value->place_justify_keywords);
                    if (!align_items || !justify_items)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::PlaceItems,
                            { PropertyID::AlignItems, PropertyID::JustifyItems },
                            { align_items.release_nonnull(), justify_items.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::PlaceSelf:
                if (!rust_style_value->place_align_keywords.is_empty() && !rust_style_value->place_justify_keywords.is_empty()) {
                    auto align_self = materialize_rust_keyword_list(rust_style_value->place_align_keywords);
                    auto justify_self = materialize_rust_keyword_list(rust_style_value->place_justify_keywords);
                    if (!align_self || !justify_self)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::PlaceSelf,
                            { PropertyID::AlignSelf, PropertyID::JustifySelf },
                            { align_self.release_nonnull(), justify_self.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::Position:
                if (rust_style_value->value_type.has_value()) {
                    if (first_is_one_of(rust_style_value->property_id, PropertyID::BackgroundPositionX, PropertyID::BackgroundPositionY)) {
                        StyleValueVector values;
                        values.ensure_capacity(rust_style_value->position_components.size());
                        for (auto const& component : rust_style_value->position_components) {
                            auto value = materialize_rust_position_component(component);
                            if (!value)
                                break;
                            values.append(value.release_nonnull());
                        }

                        if (values.size() == rust_style_value->position_components.size()) {
                            discard_rust_owned_property_value_tokens();
                            generated_transaction.commit();
                            return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Comma) };
                        }
                    } else if (*rust_style_value->value_type == ValueType::BackgroundPosition) {
                        StyleValueVector background_position_x_values;
                        StyleValueVector background_position_y_values;
                        background_position_x_values.ensure_capacity(rust_style_value->positions.size());
                        background_position_y_values.ensure_capacity(rust_style_value->positions.size());

                        for (auto const& position : rust_style_value->positions) {
                            auto x = materialize_rust_position_component(position.x);
                            if (!x)
                                break;
                            auto y = materialize_rust_position_component(position.y);
                            if (!y)
                                break;

                            background_position_x_values.append(x.release_nonnull());
                            background_position_y_values.append(y.release_nonnull());
                        }

                        if (background_position_x_values.size() == rust_style_value->positions.size()) {
                            discard_rust_owned_property_value_tokens();
                            generated_transaction.commit();
                            return PropertyAndValue { rust_style_value->property_id,
                                ShorthandStyleValue::create(PropertyID::BackgroundPosition,
                                    { PropertyID::BackgroundPositionX, PropertyID::BackgroundPositionY },
                                    { StyleValueList::create(move(background_position_x_values), StyleValueList::Separator::Comma),
                                        StyleValueList::create(move(background_position_y_values), StyleValueList::Separator::Comma) }) };
                        }
                    } else if (*rust_style_value->value_type == ValueType::Position && rust_style_value->property_id != PropertyID::MaskPosition) {
                        if (rust_style_value->positions.size() == 1) {
                            if (auto value = materialize_rust_position(rust_style_value->positions[0])) {
                                discard_rust_owned_property_value_tokens();
                                generated_transaction.commit();
                                return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                            }
                        }
                    } else if (*rust_style_value->value_type == ValueType::Position) {
                        StyleValueVector values;
                        values.ensure_capacity(rust_style_value->positions.size());
                        for (auto const& position : rust_style_value->positions) {
                            auto value = materialize_rust_position(position);
                            if (!value)
                                break;

                            values.append(value.release_nonnull());
                        }

                        if (values.size() == rust_style_value->positions.size()) {
                            discard_rust_owned_property_value_tokens();
                            generated_transaction.commit();
                            return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Comma) };
                        }
                    }
                }
                break;
            case FFI::CssStyleValueKind::PositionAnchor:
                switch (rust_style_value->position_anchor_kind) {
                case FFI::CssPositionAnchorValueKind::Invalid:
                    break;
                case FFI::CssPositionAnchorValueKind::Normal:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Normal) };
                case FFI::CssPositionAnchorValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssPositionAnchorValueKind::Auto:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Auto) };
                case FFI::CssPositionAnchorValueKind::AnchorName:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, CustomIdentStyleValue::create(rust_style_value->position_anchor_name) };
                }
                break;
            case FFI::CssStyleValueKind::PositionArea:
                if (rust_style_value->position_area_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (rust_style_value->position_area.has_value()) {
                    auto value = materialize_rust_position_area(*rust_style_value->position_area);
                    if (!value)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::PositionTryFallbacks:
                if (rust_style_value->position_try_fallbacks_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (!rust_style_value->position_try_fallbacks.is_empty()) {
                    StyleValueVector fallbacks;
                    fallbacks.ensure_capacity(rust_style_value->position_try_fallbacks.size());
                    for (auto const& fallback : rust_style_value->position_try_fallbacks) {
                        auto value = materialize_rust_position_try_fallback(fallback);
                        if (!value)
                            break;
                        fallbacks.append(value.release_nonnull());
                    }
                    if (fallbacks.size() != rust_style_value->position_try_fallbacks.size())
                        break;

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(fallbacks), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::PositionTryOrder:
                if (auto keyword = position_try_order_keyword_from_rust(rust_style_value->position_try_order); keyword.has_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(keyword.release_value()) };
                }
                break;
            case FFI::CssStyleValueKind::PositionVisibility:
                switch (rust_style_value->position_visibility.kind) {
                case FFI::CssPositionVisibilityValueKind::Invalid:
                    break;
                case FFI::CssPositionVisibilityValueKind::Always:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Always) };
                case FFI::CssPositionVisibilityValueKind::List: {
                    StyleValueVector values;
                    if (rust_style_value->position_visibility.has_anchors_valid)
                        values.append(KeywordStyleValue::create(Keyword::AnchorsValid));
                    if (rust_style_value->position_visibility.has_anchors_visible)
                        values.append(KeywordStyleValue::create(Keyword::AnchorsVisible));
                    if (rust_style_value->position_visibility.has_no_overflow)
                        values.append(KeywordStyleValue::create(Keyword::NoOverflow));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Space) };
                }
                }
                break;
            case FFI::CssStyleValueKind::Quotes:
                switch (rust_style_value->quotes_kind) {
                case FFI::CssQuotesValueKind::Invalid:
                    break;
                case FFI::CssQuotesValueKind::Auto:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Auto) };
                case FFI::CssQuotesValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssQuotesValueKind::List: {
                    StyleValueVector string_values;
                    string_values.ensure_capacity(rust_style_value->quotes_strings.size());
                    for (auto const& string : rust_style_value->quotes_strings)
                        string_values.unchecked_append(StringStyleValue::create(string));

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(string_values), StyleValueList::Separator::Space) };
                }
                }
                break;
            case FFI::CssStyleValueKind::OverflowClipMargin:
                if (rust_style_value->overflow_clip_margin.has_value()) {
                    auto value = materialize_rust_nested_length(*rust_style_value->overflow_clip_margin, non_negative_range);
                    if (!value)
                        break;
                    auto const& longhands = longhands_for_shorthand(rust_style_value->property_id);
                    if (longhands.is_empty()) {
                        discard_rust_owned_property_value_tokens();
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, value };
                    }

                    Vector<ValueComparingNonnullRefPtr<StyleValue const>> longhand_values;
                    longhand_values.resize_with_default_value(longhands.size(), value.release_nonnull());

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(rust_style_value->property_id, longhands, longhand_values) };
                }
                break;
            case FFI::CssStyleValueKind::RepeatStyle:
                VERIFY(rust_style_value->repeat_x_values.size() == rust_style_value->repeat_y_values.size());
                if (!rust_style_value->repeat_x_values.is_empty()) {
                    StyleValueVector values;
                    values.ensure_capacity(rust_style_value->repeat_x_values.size());
                    for (size_t i = 0; i < rust_style_value->repeat_x_values.size(); ++i) {
                        values.append(RepeatStyleStyleValue::create(
                            repetition_from_rust(rust_style_value->repeat_x_values[i]),
                            repetition_from_rust(rust_style_value->repeat_y_values[i])));
                    }
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::ScrollFunction:
                if (auto value = materialize_rust_scroll_function_value()) {
                    tokens.discard_a_token();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::ScrollbarColor:
                if (rust_style_value->scrollbar_color_kind == 1) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Auto) };
                }
                if (rust_style_value->scrollbar_color_kind == 2) {
                    if (!rust_style_value->scrollbar_thumb_color.has_value() || !rust_style_value->scrollbar_track_color.has_value())
                        break;
                    auto thumb_color = materialize_rust_style_color(*rust_style_value->scrollbar_thumb_color, parse_rust_source_as_color);
                    auto track_color = materialize_rust_style_color(*rust_style_value->scrollbar_track_color, parse_rust_source_as_color);
                    if (!thumb_color || !track_color)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, ScrollbarColorStyleValue::create(thumb_color.release_nonnull(), track_color.release_nonnull()) };
                }
                break;
            case FFI::CssStyleValueKind::ScrollbarGutter:
                switch (rust_style_value->scrollbar_gutter) {
                case FFI::CssScrollbarGutterValueKind::Invalid:
                    break;
                case FFI::CssScrollbarGutterValueKind::Auto:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, ScrollbarGutterStyleValue::create(ScrollbarGutter::Auto) };
                case FFI::CssScrollbarGutterValueKind::Stable:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, ScrollbarGutterStyleValue::create(ScrollbarGutter::Stable) };
                case FFI::CssScrollbarGutterValueKind::BothEdges:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, ScrollbarGutterStyleValue::create(ScrollbarGutter::BothEdges) };
                }
                break;
            case FFI::CssStyleValueKind::StrokeDasharray:
                if (rust_style_value->stroke_dasharray_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (!rust_style_value->stroke_dasharray_values.is_empty()) {
                    Vector<ValueComparingNonnullRefPtr<StyleValue const>> dashes;
                    dashes.ensure_capacity(rust_style_value->stroke_dasharray_values.size());
                    for (auto const& dash_value : rust_style_value->stroke_dasharray_values) {
                        auto value = materialize_rust_nested_non_negative_number_length_percentage(dash_value);
                        if (!value)
                            break;
                        dashes.append(value.release_nonnull());
                    }
                    if (dashes.size() != rust_style_value->stroke_dasharray_values.size())
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(dashes), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::Shadow: {
                auto shadow_type = rust_style_value->property_id == PropertyID::TextShadow
                    ? ShadowStyleValue::ShadowType::Text
                    : ShadowStyleValue::ShadowType::Normal;
                if (rust_style_value->shadow_is_none) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                }
                if (!rust_style_value->shadows.is_empty()) {
                    StyleValueVector shadows;
                    shadows.ensure_capacity(rust_style_value->shadows.size());
                    for (auto const& shadow : rust_style_value->shadows) {
                        RefPtr<StyleValue const> color;
                        if (shadow.color.has_value()) {
                            color = materialize_rust_style_color(*shadow.color, parse_rust_source_as_color);
                            if (!color)
                                break;
                        }

                        auto offset_x = materialize_rust_nested_length(shadow.offset_x, infinite_range);
                        auto offset_y = materialize_rust_nested_length(shadow.offset_y, infinite_range);
                        if (!offset_x || !offset_y)
                            break;

                        RefPtr<StyleValue const> blur_radius;
                        if (shadow.blur_radius.has_value()) {
                            blur_radius = materialize_rust_nested_length(*shadow.blur_radius, non_negative_range);
                            if (!blur_radius)
                                break;
                        }

                        RefPtr<StyleValue const> spread_distance;
                        if (shadow.spread_distance.has_value()) {
                            spread_distance = materialize_rust_nested_length(*shadow.spread_distance, infinite_range);
                            if (!spread_distance)
                                break;
                        }

                        auto placement = shadow.placement == RustComponentValueParser::RustShadowPlacement::Inner ? ShadowPlacement::Inner : ShadowPlacement::Outer;
                        shadows.append(ShadowStyleValue::create(shadow_type, color, offset_x.release_nonnull(), offset_y.release_nonnull(), blur_radius, spread_distance, placement));
                    }
                    if (shadows.size() != rust_style_value->shadows.size())
                        break;

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(shadows), StyleValueList::Separator::Comma) };
                }
                break;
            }
            case FFI::CssStyleValueKind::ShapeOutside:
                if (auto value = materialize_rust_shape_outside_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::TextDecoration:
                if (rust_style_value->text_decoration_line_bits.has_value() || rust_style_value->text_decoration_thickness_kind.has_value() || rust_style_value->text_decoration_style.has_value() || rust_style_value->text_decoration_color.has_value()) {
                    auto decoration_line = rust_style_value->text_decoration_line_bits.has_value()
                        ? materialize_rust_text_decoration_line(*rust_style_value->text_decoration_line_bits)
                        : property_initial_value(PropertyID::TextDecorationLine);
                    auto decoration_thickness = rust_style_value->text_decoration_thickness_kind.has_value()
                        ? materialize_rust_text_decoration_thickness()
                        : property_initial_value(PropertyID::TextDecorationThickness);
                    RefPtr<StyleValue const> decoration_style = rust_style_value->text_decoration_style.has_value()
                        ? KeywordStyleValue::create(text_decoration_style_keyword_from_rust(*rust_style_value->text_decoration_style))
                        : property_initial_value(PropertyID::TextDecorationStyle);
                    auto decoration_color = rust_style_value->text_decoration_color.has_value()
                        ? materialize_rust_style_color(*rust_style_value->text_decoration_color, [&](String const& source) {
                              return parse_rust_source_as_property(PropertyID::TextDecorationColor, source);
                          })
                        : property_initial_value(PropertyID::TextDecorationColor);
                    if (!decoration_line || !decoration_thickness || !decoration_style || !decoration_color)
                        break;
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id,
                        ShorthandStyleValue::create(PropertyID::TextDecoration,
                            { PropertyID::TextDecorationLine, PropertyID::TextDecorationThickness, PropertyID::TextDecorationStyle, PropertyID::TextDecorationColor },
                            { decoration_line.release_nonnull(), decoration_thickness.release_nonnull(), decoration_style.release_nonnull(), decoration_color.release_nonnull() }) };
                }
                break;
            case FFI::CssStyleValueKind::TextDecorationLine:
                if (auto value = materialize_rust_text_decoration_line(rust_style_value->color_red)) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value.release_nonnull() };
                }
                break;
            case FFI::CssStyleValueKind::ScrollTimeline: {
                VERIFY(rust_style_value->timeline_name_item_kinds.size() == rust_style_value->timeline_names.size());
                VERIFY(rust_style_value->timeline_names.size() == rust_style_value->scroll_timeline_axes.size());

                StyleValueVector names;
                names.ensure_capacity(rust_style_value->timeline_names.size());
                for (size_t i = 0; i < rust_style_value->timeline_names.size(); ++i) {
                    switch (rust_style_value->timeline_name_item_kinds[i]) {
                    case FFI::CssTimelineNameItemKind::None:
                        names.unchecked_append(KeywordStyleValue::create(Keyword::None));
                        break;
                    case FFI::CssTimelineNameItemKind::DashedIdent:
                        names.unchecked_append(CustomIdentStyleValue::create(rust_style_value->timeline_names[i]));
                        break;
                    }
                }

                StyleValueVector axes;
                axes.ensure_capacity(rust_style_value->scroll_timeline_axes.size());
                for (auto axis : rust_style_value->scroll_timeline_axes) {
                    auto keyword = keyword_from_scroll_function_axis(axis);
                    if (!keyword.has_value())
                        break;
                    axes.unchecked_append(KeywordStyleValue::create(keyword.release_value()));
                }

                if (axes.size() != rust_style_value->scroll_timeline_axes.size())
                    break;

                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { PropertyID::ScrollTimeline,
                    ShorthandStyleValue::create(PropertyID::ScrollTimeline,
                        { PropertyID::ScrollTimelineName, PropertyID::ScrollTimelineAxis },
                        { StyleValueList::create(move(names), StyleValueList::Separator::Comma), StyleValueList::create(move(axes), StyleValueList::Separator::Comma) }) };
            }
            case FFI::CssStyleValueKind::TimelineName:
                switch (rust_style_value->timeline_name_kind) {
                case FFI::CssTimelineNameValueKind::Invalid:
                    break;
                case FFI::CssTimelineNameValueKind::List: {
                    StyleValueVector names;
                    VERIFY(rust_style_value->timeline_name_item_kinds.size() == rust_style_value->timeline_names.size());
                    names.ensure_capacity(rust_style_value->timeline_names.size());
                    for (size_t i = 0; i < rust_style_value->timeline_names.size(); ++i) {
                        switch (rust_style_value->timeline_name_item_kinds[i]) {
                        case FFI::CssTimelineNameItemKind::None:
                            names.unchecked_append(KeywordStyleValue::create(Keyword::None));
                            break;
                        case FFI::CssTimelineNameItemKind::DashedIdent:
                            names.unchecked_append(CustomIdentStyleValue::create(rust_style_value->timeline_names[i]));
                            break;
                        }
                    }
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(names), StyleValueList::Separator::Comma) };
                }
                }
                break;
            case FFI::CssStyleValueKind::TimelineScope:
                switch (rust_style_value->timeline_scope_kind) {
                case FFI::CssTimelineScopeValueKind::Invalid:
                    break;
                case FFI::CssTimelineScopeValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssTimelineScopeValueKind::All:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::All) };
                case FFI::CssTimelineScopeValueKind::List: {
                    StyleValueVector names;
                    names.ensure_capacity(rust_style_value->timeline_scope_names.size());
                    for (auto const& name : rust_style_value->timeline_scope_names)
                        names.unchecked_append(CustomIdentStyleValue::create(name));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(names), StyleValueList::Separator::Comma) };
                }
                }
                break;
            case FFI::CssStyleValueKind::TextWrap: {
                if (rust_style_value->text_wrap.kind != FFI::CssTextWrapValueKind::Valid)
                    break;
                auto text_wrap_mode = property_initial_value(PropertyID::TextWrapMode);
                if (auto keyword = text_wrap_mode_keyword_from_rust(rust_style_value->text_wrap.mode); keyword.has_value())
                    text_wrap_mode = KeywordStyleValue::create(keyword.release_value());

                auto text_wrap_style = property_initial_value(PropertyID::TextWrapStyle);
                if (auto keyword = text_wrap_style_keyword_from_rust(rust_style_value->text_wrap.style); keyword.has_value())
                    text_wrap_style = KeywordStyleValue::create(keyword.release_value());

                Vector<ValueComparingNonnullRefPtr<StyleValue const>> longhand_values;
                longhand_values.append(text_wrap_mode);
                longhand_values.append(text_wrap_style);
                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { rust_style_value->property_id,
                    ShorthandStyleValue::create(
                        PropertyID::TextWrap,
                        { PropertyID::TextWrapMode, PropertyID::TextWrapStyle },
                        move(longhand_values)) };
            }
            case FFI::CssStyleValueKind::TextWrapMode:
                if (auto keyword = text_wrap_mode_keyword_from_rust(rust_style_value->text_wrap_mode); keyword.has_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(keyword.release_value()) };
                }
                break;
            case FFI::CssStyleValueKind::TextWrapStyle:
                if (auto keyword = text_wrap_style_keyword_from_rust(rust_style_value->text_wrap_style); keyword.has_value()) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(keyword.release_value()) };
                }
                break;
            case FFI::CssStyleValueKind::TextIndent: {
                RefPtr<StyleValue const> length_percentage;
                if (rust_style_value->numeric_value.has_value())
                    length_percentage = materialize_rust_numeric_value();
                else
                    length_percentage = parse_length_percentage_value(tokens, infinite_range, infinite_range);
                if (!length_percentage)
                    break;
                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue {
                    rust_style_value->property_id,
                    TextIndentStyleValue::create(
                        length_percentage.release_nonnull(),
                        rust_style_value->text_indent_has_hanging ? TextIndentStyleValue::Hanging::Yes : TextIndentStyleValue::Hanging::No,
                        rust_style_value->text_indent_has_each_line ? TextIndentStyleValue::EachLine::Yes : TextIndentStyleValue::EachLine::No)
                };
            }
            case FFI::CssStyleValueKind::TextUnderlinePosition:
                if (rust_style_value->text_underline_position_horizontal != FFI::CssTextUnderlinePositionHorizontal::Invalid
                    && rust_style_value->text_underline_position_vertical != FFI::CssTextUnderlinePositionVertical::Invalid) {
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue {
                        rust_style_value->property_id,
                        TextUnderlinePositionStyleValue::create(
                            horizontal_text_underline_position_from_rust(rust_style_value->text_underline_position_horizontal),
                            vertical_text_underline_position_from_rust(rust_style_value->text_underline_position_vertical))
                    };
                }
                break;
            case FFI::CssStyleValueKind::TouchAction:
                switch (rust_style_value->touch_action.kind) {
                case FFI::CssTouchActionValueKind::Invalid:
                    break;
                case FFI::CssTouchActionValueKind::Auto:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Auto) };
                case FFI::CssTouchActionValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssTouchActionValueKind::Manipulation:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Manipulation) };
                case FFI::CssTouchActionValueKind::List: {
                    StyleValueVector values;
                    values.append(KeywordStyleValue::create(touch_action_keyword_from_rust(rust_style_value->touch_action.first)));
                    if (rust_style_value->touch_action.second != FFI::CssTouchActionKeyword::Invalid)
                        values.append(KeywordStyleValue::create(touch_action_keyword_from_rust(rust_style_value->touch_action.second)));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Space) };
                }
                }
                break;
            case FFI::CssStyleValueKind::TransitionBehavior:
                if (!rust_style_value->transition_behaviors.is_empty()) {
                    StyleValueVector behaviors;
                    behaviors.ensure_capacity(rust_style_value->transition_behaviors.size());
                    for (auto behavior : rust_style_value->transition_behaviors) {
                        switch (behavior) {
                        case FFI::CssTransitionBehaviorItemKind::Normal:
                            behaviors.unchecked_append(KeywordStyleValue::create(Keyword::Normal));
                            break;
                        case FFI::CssTransitionBehaviorItemKind::AllowDiscrete:
                            behaviors.unchecked_append(KeywordStyleValue::create(Keyword::AllowDiscrete));
                            break;
                        }
                    }
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(behaviors), StyleValueList::Separator::Comma) };
                }
                break;
            case FFI::CssStyleValueKind::TransitionProperty:
                switch (rust_style_value->transition_property_kind) {
                case FFI::CssTransitionPropertyValueKind::Invalid:
                    break;
                case FFI::CssTransitionPropertyValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create({ KeywordStyleValue::create(Keyword::None) }, StyleValueList::Separator::Comma) };
                case FFI::CssTransitionPropertyValueKind::List: {
                    StyleValueVector transition_properties;
                    transition_properties.ensure_capacity(rust_style_value->transition_properties.size());
                    for (auto const& property : rust_style_value->transition_properties)
                        transition_properties.unchecked_append(CustomIdentStyleValue::create(property));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(transition_properties), StyleValueList::Separator::Comma) };
                }
                }
                break;
            case FFI::CssStyleValueKind::ViewTimeline: {
                VERIFY(rust_style_value->timeline_name_item_kinds.size() == rust_style_value->timeline_names.size());
                VERIFY(rust_style_value->timeline_names.size() == rust_style_value->scroll_timeline_axes.size());
                VERIFY(rust_style_value->timeline_names.size() == rust_style_value->view_timeline_inset_counts.size());

                StyleValueVector names;
                names.ensure_capacity(rust_style_value->timeline_names.size());
                for (size_t i = 0; i < rust_style_value->timeline_names.size(); ++i) {
                    switch (rust_style_value->timeline_name_item_kinds[i]) {
                    case FFI::CssTimelineNameItemKind::None:
                        names.unchecked_append(KeywordStyleValue::create(Keyword::None));
                        break;
                    case FFI::CssTimelineNameItemKind::DashedIdent:
                        names.unchecked_append(CustomIdentStyleValue::create(rust_style_value->timeline_names[i]));
                        break;
                    }
                }

                StyleValueVector axes;
                axes.ensure_capacity(rust_style_value->scroll_timeline_axes.size());
                for (auto axis : rust_style_value->scroll_timeline_axes) {
                    auto keyword = keyword_from_scroll_function_axis(axis);
                    if (!keyword.has_value())
                        break;
                    axes.unchecked_append(KeywordStyleValue::create(keyword.release_value()));
                }
                if (axes.size() != rust_style_value->scroll_timeline_axes.size())
                    break;

                StyleValueVector insets;
                insets.ensure_capacity(rust_style_value->view_timeline_inset_counts.size());
                size_t inset_offset = 0;
                for (auto inset_count : rust_style_value->view_timeline_inset_counts) {
                    auto inset = materialize_rust_view_timeline_insets(rust_style_value->view_timeline_insets.span().slice(inset_offset, inset_count));
                    if (!inset)
                        break;
                    insets.unchecked_append(inset.release_nonnull());
                    inset_offset += inset_count;
                }
                if (insets.size() != rust_style_value->view_timeline_inset_counts.size())
                    break;
                VERIFY(inset_offset == rust_style_value->view_timeline_insets.size());

                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { PropertyID::ViewTimeline,
                    ShorthandStyleValue::create(PropertyID::ViewTimeline,
                        { PropertyID::ViewTimelineName, PropertyID::ViewTimelineAxis, PropertyID::ViewTimelineInset },
                        { StyleValueList::create(move(names), StyleValueList::Separator::Comma),
                            StyleValueList::create(move(axes), StyleValueList::Separator::Comma),
                            StyleValueList::create(move(insets), StyleValueList::Separator::Comma) }) };
            }
            case FFI::CssStyleValueKind::ViewTimelineInset:
                if (auto value = materialize_rust_view_timeline_insets(rust_style_value->view_timeline_insets)) {
                    discard_rust_view_timeline_inset_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::ViewFunction:
                if (auto value = materialize_rust_view_function_value()) {
                    tokens.discard_a_token();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, value };
                }
                break;
            case FFI::CssStyleValueKind::ViewTransitionName:
                switch (rust_style_value->view_transition_name_kind) {
                case FFI::CssViewTransitionNameValueKind::Invalid:
                    break;
                case FFI::CssViewTransitionNameValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssViewTransitionNameValueKind::CustomIdent:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, CustomIdentStyleValue::create(rust_style_value->view_transition_name) };
                }
                break;
            case FFI::CssStyleValueKind::WhiteSpace: {
                auto white_space_collapse_keyword = keyword_from_string(rust_style_value->white_space_collapse);
                auto text_wrap_mode_keyword = text_wrap_mode_keyword_from_rust(rust_style_value->text_wrap_mode);
                auto white_space_trim = materialize_white_space_trim(rust_style_value->white_space_trim);
                if (!white_space_collapse_keyword.has_value() || !text_wrap_mode_keyword.has_value() || !white_space_trim)
                    break;

                discard_rust_owned_property_value_tokens();
                generated_transaction.commit();
                return PropertyAndValue { PropertyID::WhiteSpace,
                    ShorthandStyleValue::create(PropertyID::WhiteSpace,
                        { PropertyID::WhiteSpaceCollapse, PropertyID::TextWrapMode, PropertyID::WhiteSpaceTrim },
                        { KeywordStyleValue::create(white_space_collapse_keyword.release_value()),
                            KeywordStyleValue::create(text_wrap_mode_keyword.release_value()),
                            white_space_trim.release_nonnull() }) };
            }
            case FFI::CssStyleValueKind::WhiteSpaceTrim:
                switch (rust_style_value->white_space_trim.kind) {
                case FFI::CssWhiteSpaceTrimValueKind::Invalid:
                    break;
                case FFI::CssWhiteSpaceTrimValueKind::None:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::None) };
                case FFI::CssWhiteSpaceTrimValueKind::List: {
                    StyleValueVector values;
                    if (rust_style_value->white_space_trim.has_discard_before)
                        values.append(KeywordStyleValue::create(Keyword::DiscardBefore));
                    if (rust_style_value->white_space_trim.has_discard_after)
                        values.append(KeywordStyleValue::create(Keyword::DiscardAfter));
                    if (rust_style_value->white_space_trim.has_discard_inner)
                        values.append(KeywordStyleValue::create(Keyword::DiscardInner));
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(values), StyleValueList::Separator::Space) };
                }
                }
                break;
            case FFI::CssStyleValueKind::WillChange:
                switch (rust_style_value->will_change_kind) {
                case FFI::CssWillChangeValueKind::Invalid:
                    break;
                case FFI::CssWillChangeValueKind::Auto:
                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, KeywordStyleValue::create(Keyword::Auto) };
                case FFI::CssWillChangeValueKind::List: {
                    StyleValueVector features;
                    VERIFY(rust_style_value->will_change_feature_kinds.size() == rust_style_value->will_change_features.size());
                    features.ensure_capacity(rust_style_value->will_change_features.size());
                    for (size_t i = 0; i < rust_style_value->will_change_features.size(); ++i) {
                        switch (rust_style_value->will_change_feature_kinds[i]) {
                        case FFI::CssWillChangeFeatureKind::ScrollPosition:
                            features.unchecked_append(KeywordStyleValue::create(Keyword::ScrollPosition));
                            break;
                        case FFI::CssWillChangeFeatureKind::Contents:
                            features.unchecked_append(KeywordStyleValue::create(Keyword::Contents));
                            break;
                        case FFI::CssWillChangeFeatureKind::CustomIdent:
                            features.unchecked_append(CustomIdentStyleValue::create(rust_style_value->will_change_features[i]));
                            break;
                        }
                    }

                    discard_rust_owned_property_value_tokens();
                    generated_transaction.commit();
                    return PropertyAndValue { rust_style_value->property_id, StyleValueList::create(move(features), StyleValueList::Separator::Comma) };
                }
                }
                break;
            case FFI::CssStyleValueKind::Anchor:
            case FFI::CssStyleValueKind::AnchorSize:
            case FFI::CssStyleValueKind::MathFunction:
            case FFI::CssStyleValueKind::Primitive:
            case FFI::CssStyleValueKind::ValueType:
                if (rust_style_value->value_type.has_value()) {
                    auto context_guard = push_temporary_value_parsing_context(rust_style_value->property_id);

                    RefPtr<StyleValue const> maybe_parsed_value;
                    if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Keyword && rust_style_value->keyword.has_value()) {
                        tokens.discard_a_token();
                        maybe_parsed_value = KeywordStyleValue::create(*rust_style_value->keyword);
                    } else if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::CustomIdent && rust_style_value->custom_ident.has_value()) {
                        tokens.discard_a_token();
                        maybe_parsed_value = CustomIdentStyleValue::create(*rust_style_value->custom_ident);
                    } else if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::String && rust_style_value->string.has_value()) {
                        tokens.discard_a_token();
                        maybe_parsed_value = StringStyleValue::create(*rust_style_value->string);
                    } else if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Number
                        && rust_style_value->numeric_value.has_value()
                        && !first_is_one_of(*rust_style_value->value_type, ValueType::Integer, ValueType::Number, ValueType::Angle, ValueType::AnglePercentage, ValueType::Flex, ValueType::Frequency, ValueType::FrequencyPercentage, ValueType::Length, ValueType::LengthPercentage, ValueType::Resolution, ValueType::Time, ValueType::TimePercentage, ValueType::Percentage, ValueType::OpacityValue)) {
                        tokens.discard_a_token();
                        maybe_parsed_value = NumberStyleValue::create(*rust_style_value->numeric_value);
                    } else if (rust_style_value->primitive_kind == FFI::CssPrimitiveValueKind::Ratio) {
                        tokens.discard_a_token();
                        if (rust_style_value->ratio_has_denominator) {
                            tokens.discard_whitespace();
                            tokens.discard_a_token();
                            tokens.discard_whitespace();
                            tokens.discard_a_token();
                        }
                        maybe_parsed_value = materialize_rust_ratio_value();
                    } else if (rust_style_value->numeric_value.has_value() && first_is_one_of(*rust_style_value->value_type, ValueType::Integer, ValueType::Number, ValueType::Angle, ValueType::AnglePercentage, ValueType::Flex, ValueType::Frequency, ValueType::FrequencyPercentage, ValueType::Length, ValueType::LengthPercentage, ValueType::Resolution, ValueType::Time, ValueType::TimePercentage, ValueType::Percentage, ValueType::OpacityValue)) {
                        tokens.discard_a_token();
                        maybe_parsed_value = materialize_rust_numeric_value();
                    } else if (first_is_one_of(*rust_style_value->value_type, ValueType::Integer, ValueType::Number, ValueType::Angle, ValueType::AnglePercentage, ValueType::Flex, ValueType::Frequency, ValueType::FrequencyPercentage, ValueType::Length, ValueType::LengthPercentage, ValueType::Resolution, ValueType::Time, ValueType::TimePercentage, ValueType::Percentage, ValueType::OpacityValue)) {
                        maybe_parsed_value = parse_rust_numeric_value();
                    } else if (rust_style_value->kind == FFI::CssStyleValueKind::ValueType && rust_style_value->string.has_value()) {
                        maybe_parsed_value = parse_rust_source_as_value_type(rust_style_value->string->bytes_as_string_view(), *rust_style_value->value_type);
                        if (maybe_parsed_value)
                            discard_rust_owned_property_value_tokens();
                    } else {
                        maybe_parsed_value = parse_value(*rust_style_value->value_type, tokens);
                    }

                    if (maybe_parsed_value) {
                        generated_transaction.commit();
                        return PropertyAndValue { rust_style_value->property_id, maybe_parsed_value };
                    }
                }
                break;
            }
        }
    }

    if (property_ids.size() == 1 && property_uses_rust_owned_whole_grammar(property_ids[0]))
        return OptionalNone {};

    if (peek_token.is(Token::Type::Ident)) {
        // NOTE: We do not try to parse "CSS-wide keywords" here. https://www.w3.org/TR/css-values-4/#common-keywords
        //       These are only valid on their own, and so should be parsed directly in `parse_css_value()`.
        if (auto property_keyword = RustComponentValueParser::parse_property_keyword_value(property_ids, peek_token.token().ident()); property_keyword.has_value()) {
            tokens.discard_a_token();
            return PropertyAndValue { property_keyword->property_id, KeywordStyleValue::create(property_keyword->keyword) };
        }

        // Custom idents
        auto original_source_text = peek_token.original_source_text();
        auto source = original_source_text.is_empty() ? peek_token.to_string() : original_source_text;
        if (auto property_custom_ident = RustComponentValueParser::parse_property_custom_ident_value(property_ids, source.bytes_as_string_view()); property_custom_ident.has_value()) {
            tokens.discard_a_token();
            return PropertyAndValue { property_custom_ident->property_id, CustomIdentStyleValue::create(property_custom_ident->custom_ident) };
        }
    }

    if (auto parsed = parse_for_type(ValueType::Color); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::CornerShape); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::Counter); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::CounterStyle); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::DashedIdent); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::EasingFunction); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontStyle); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontKerningValue); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontOpticalSizingValue); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontWeightAbsolute); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontWidthCss3); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantAlternates); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantCapsValue); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantEastAsian); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantEmojiValue); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantLigatures); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantNumeric); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::FontVariantPositionValue); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::Image); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::Position); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::BackgroundPosition); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::BasicShape); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::Ratio); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::OpacityValue); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::OpentypeTag); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::Rect); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::ScrollFunction); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::String); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::TransformFunction); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::TransformList); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::Url); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::ViewFunction); parsed.has_value())
        return parsed.release_value();
    if (auto parsed = parse_for_type(ValueType::ViewTimelineInset); parsed.has_value())
        return parsed.release_value();

    // <integer>/<number> come before <length>, so that 0 is not interpreted as a <length> in case both are allowed.
    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Integer); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (auto value = parse_integer_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Number); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (auto value = parse_number_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Angle); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (metadata->percentages_resolve_to_value_type) {
            VERIFY(metadata->percentage_range.has_value());
            if (auto value = parse_angle_percentage_value(tokens, metadata->range, metadata->percentage_range.value()))
                return PropertyAndValue { metadata->property_id, value };
        }

        if (auto value = parse_angle_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Flex); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (auto value = parse_flex_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Frequency); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (metadata->percentages_resolve_to_value_type) {
            VERIFY(metadata->percentage_range.has_value());
            if (auto value = parse_frequency_percentage_value(tokens, metadata->range, metadata->percentage_range.value()))
                return PropertyAndValue { metadata->property_id, value };
        }

        if (auto value = parse_frequency_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto parsed = parse_for_type(ValueType::FitContent); parsed.has_value())
        return parsed.release_value();

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Length); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (metadata->percentages_resolve_to_value_type) {
            VERIFY(metadata->percentage_range.has_value());
            if (auto value = parse_length_percentage_value(tokens, metadata->range, metadata->percentage_range.value()))
                return PropertyAndValue { metadata->property_id, value };
        }

        if (auto value = parse_length_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Resolution); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);

        if (auto value = parse_resolution_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Time); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);
        if (metadata->percentages_resolve_to_value_type) {
            VERIFY(metadata->percentage_range.has_value());
            if (auto value = parse_time_percentage_value(tokens, metadata->range, metadata->percentage_range.value()))
                return PropertyAndValue { metadata->property_id, value };
        }

        if (auto value = parse_time_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    // <percentage> is checked after the <foo-percentage> types.
    if (auto metadata = property_numeric_metadata(property_ids, ValueType::Percentage); metadata.has_value()) {
        auto context_guard = push_temporary_value_parsing_context(metadata->property_id);

        if (auto value = parse_percentage_value(tokens, metadata->range))
            return PropertyAndValue { metadata->property_id, value };
    }

    if (auto parsed = parse_for_type(ValueType::Paint); parsed.has_value())
        return parsed.release_value();

    if (auto parsed = parse_for_type(ValueType::Anchor); parsed.has_value())
        return parsed.release_value();

    return OptionalNone {};
}

Parser::ParseErrorOr<NonnullRefPtr<StyleValue const>> Parser::parse_css_value(PropertyID property_id, TokenStream<ComponentValue>& tokens, Optional<String> original_source_text)
{
    auto context_guard = push_temporary_value_parsing_context(property_id);

    SubstitutionFunctionsPresence substitution_presence;
    {
        // NB: This transaction is intentionally never committed. This loop just examines the tokens and doesn't want
        //     to permanently consume anything.
        auto transaction = tokens.begin_transaction();
        while (tokens.has_next_token()) {
            auto const& token = tokens.consume_a_token();

            if (token.is(Token::Type::Semicolon))
                return ParseError::SyntaxError;

            // https://drafts.csswg.org/css-values-5/#resolve-property
            // If a property value contains one or more arbitrary substitution functions, and all of those functions are
            // themselves syntactically valid according to their argument grammars, the entire value’s grammar must be
            // assumed to be valid at parse time.
            if (collect_arbitrary_substitution_function_presence(token, substitution_presence).is_error())
                return ParseError::SyntaxError;
        }
    }

    auto parse_all_as = [](auto& tokens, auto&& callback) -> ParseErrorOr<NonnullRefPtr<StyleValue const>> {
        tokens.discard_whitespace();
        auto parsed_value = callback(tokens);
        tokens.discard_whitespace();
        if (parsed_value && !tokens.has_next_token())
            return parsed_value.release_nonnull();
        return ParseError::SyntaxError;
    };

    {
        auto builtin_transaction = tokens.begin_transaction();
        auto builtin = parse_all_as(tokens, [this](auto& tokens) { return parse_builtin_value(tokens); });
        if (!builtin.is_error()) {
            builtin_transaction.commit();
            return builtin.release_value();
        }
    }

    if (property_id == PropertyID::Custom || substitution_presence.has_any()) {
        return parse_all_as(tokens, [&](TokenStream<ComponentValue>& tokens) -> RefPtr<StyleValue const> {
            if (tokens.is_empty())
                return UnresolvedStyleValue::create({}, substitution_presence, move(original_source_text));

            if (auto component_values = parse_declaration_value(tokens); component_values.has_value())
                return UnresolvedStyleValue::create(component_values.release_value(), substitution_presence, move(original_source_text));

            return nullptr;
        });
    }

    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return ParseError::SyntaxError;

    if (property_uses_rust_owned_whole_grammar(property_id))
        return parse_all_as(tokens, [this, property_id](auto& tokens) { return parse_css_value_for_property(property_id, tokens); });

    // Special-case property handling
    switch (property_id) {
    case PropertyID::All:
        // NOTE: The 'all' property, unlike some other shorthands, doesn't support directly listing sub-property
        //       values, only the CSS-wide keywords - this is handled above, and thus, if we have gotten to here, there
        //       is an invalid value which is a syntax error.
        return ParseError::SyntaxError;
    case PropertyID::Animation:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_animation_value(tokens); });
    case PropertyID::Background:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_background_value(tokens); });
    case PropertyID::Font:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_font_value(tokens); });
    case PropertyID::GridArea:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_grid_area_shorthand_value(tokens); });
    case PropertyID::GridColumn:
    case PropertyID::GridRow:
        return parse_all_as(tokens, [this, property_id](auto& tokens) { return parse_grid_track_placement_shorthand_value(property_id, tokens); });
    case PropertyID::Grid:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_grid_shorthand_value(tokens); });
    case PropertyID::GridTemplate:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_grid_track_size_list_shorthand_value(PropertyID::GridTemplate, tokens); });
    case PropertyID::Mask:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_mask_value(tokens); });
    case PropertyID::Transition:
        return parse_all_as(tokens, [this](auto& tokens) { return parse_transition_value(tokens); });
    default:
        break;
    }

    if (property_multiplicity(property_id) == PropertyMultiplicity::CoordinatingList
        && !property_is_shorthand(property_id)
        && !first_is_one_of(property_id,
            PropertyID::AnimationName,
            PropertyID::ScrollTimelineName,
            PropertyID::TransitionBehavior,
            PropertyID::TransitionProperty,
            PropertyID::ViewTimelineName))
        return parse_all_as(tokens, [this, property_id](auto& tokens) { return parse_simple_comma_separated_value_list(property_id, tokens); });

    if (property_is_positional_value_list_shorthand(property_id))
        return parse_all_as(tokens, [this, property_id](auto& tokens) { return parse_positional_value_list_shorthand(property_id, tokens); });

    {
        auto transaction = tokens.begin_transaction();
        StyleValueVector parsed_values;
        while (auto parsed_value = parse_css_value_for_property(property_id, tokens)) {
            parsed_values.append(parsed_value.release_nonnull());
            tokens.discard_whitespace();
            if (!tokens.has_next_token())
                break;
        }

        tokens.discard_whitespace();
        if (!tokens.has_next_token()) {
            if (parsed_values.size() == 1) {
                transaction.commit();
                return *parsed_values.take_first();
            }

            if (!parsed_values.is_empty() && parsed_values.size() <= property_maximum_value_count(property_id)) {
                transaction.commit();
                return StyleValueList::create(move(parsed_values), StyleValueList::Separator::Space);
            }
        }
    }

    // We have more values than the property claims to allow. Check if it's a shorthand.
    auto unassigned_properties = longhands_for_shorthand(property_id);
    if (unassigned_properties.is_empty())
        return ParseError::SyntaxError;

    OrderedHashMap<UnderlyingType<PropertyID>, Vector<ValueComparingNonnullRefPtr<StyleValue const>>> assigned_values;

    while (tokens.has_next_token() && !unassigned_properties.is_empty()) {
        auto property_and_value = parse_css_value_for_properties(unassigned_properties, tokens);
        if (property_and_value.has_value()) {
            auto property = property_and_value->property;
            auto value = property_and_value->style_value;
            auto& values = assigned_values.ensure(to_underlying(property));
            if (values.size() + 1 == property_maximum_value_count(property)) {
                // We're done with this property, move on to the next one.
                unassigned_properties.remove_first_matching([&](auto& unassigned_property) { return unassigned_property == property; });
            }

            values.append(value.release_nonnull());
            continue;
        }

        // No property matched, so we're done.
        if constexpr (CSS_PARSER_DEBUG) {
            dbgln("No property (from {} properties) matched {}", unassigned_properties.size(), tokens.next_token().to_debug_string());
            for (auto id : unassigned_properties)
                dbgln("    {}", string_from_property_id(id));
        }
        break;
    }

    for (auto& property : unassigned_properties)
        assigned_values.ensure(to_underlying(property)).append(property_initial_value(property));

    tokens.discard_whitespace();
    if (tokens.has_next_token())
        return ParseError::SyntaxError;

    Vector<PropertyID> longhand_properties;
    longhand_properties.ensure_capacity(assigned_values.size());
    for (auto& it : assigned_values)
        longhand_properties.unchecked_append(static_cast<PropertyID>(it.key));

    StyleValueVector longhand_values;
    longhand_values.ensure_capacity(assigned_values.size());
    for (auto& it : assigned_values) {
        if (it.value.size() == 1)
            longhand_values.unchecked_append(it.value.take_first());
        else
            longhand_values.unchecked_append(StyleValueList::create(move(it.value), StyleValueList::Separator::Space));
    }

    return { ShorthandStyleValue::create(property_id, move(longhand_properties), move(longhand_values)) };
}

RefPtr<StyleValue const> Parser::parse_positional_value_list_shorthand(PropertyID property_id, TokenStream<ComponentValue>& tokens)
{
    auto const& longhands = longhands_for_shorthand(property_id);

    auto create_shorthand_value = [&](Vector<ValueComparingNonnullRefPtr<StyleValue const>> const& parsed_values) -> RefPtr<StyleValue const> {
        if (parsed_values.is_empty() || parsed_values.size() > longhands.size())
            return nullptr;

        switch (longhands.size()) {
        case 2: {
            switch (parsed_values.size()) {
            case 1:
                return ShorthandStyleValue::create(property_id, longhands, { parsed_values[0], parsed_values[0] });
            case 2:
                return ShorthandStyleValue::create(property_id, longhands, parsed_values);
            default:
                VERIFY_NOT_REACHED();
            }
        }
        case 4: {
            switch (parsed_values.size()) {
            case 1:
                return ShorthandStyleValue::create(property_id, longhands, { parsed_values[0], parsed_values[0], parsed_values[0], parsed_values[0] });
            case 2:
                return ShorthandStyleValue::create(property_id, longhands, { parsed_values[0], parsed_values[1], parsed_values[0], parsed_values[1] });
            case 3:
                return ShorthandStyleValue::create(property_id, longhands, { parsed_values[0], parsed_values[1], parsed_values[2], parsed_values[1] });
            case 4:
                return ShorthandStyleValue::create(property_id, longhands, parsed_values);
            default:
                VERIFY_NOT_REACHED();
            }
        }
        default:
            TODO();
        }
    };

    {
        auto rust_transaction = tokens.begin_transaction();
        auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
        if (auto rust_items = RustComponentValueParser::parse_positional_value_list_shorthand(property_id, source.bytes_as_string_view()); rust_items.has_value()) {
            Vector<ValueComparingNonnullRefPtr<StyleValue const>> parsed_values;

            for (auto const& item : rust_items.value()) {
                if (item.index != parsed_values.size())
                    return {};

                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(item.value.bytes_as_string_view(), "utf-8"sv);
                TokenStream<ComponentValue> value_tokens { component_values };
                auto parsed_value = parse_css_value_for_property(property_id, value_tokens);
                value_tokens.discard_whitespace();
                if (!parsed_value || value_tokens.has_next_token())
                    return {};

                parsed_values.append(parsed_value.release_nonnull());
            }

            if (auto shorthand_value = create_shorthand_value(parsed_values)) {
                while (tokens.has_next_token())
                    tokens.discard_a_token();
                rust_transaction.commit();
                return shorthand_value;
            }
        }
    }

    Vector<ValueComparingNonnullRefPtr<StyleValue const>> parsed_values;

    while (auto parsed_value = parse_css_value_for_property(property_id, tokens))
        parsed_values.append(parsed_value.release_nonnull());

    return create_shorthand_value(parsed_values);
}

// https://drafts.csswg.org/css-animations-1/#animation
RefPtr<StyleValue const> Parser::parse_animation_value(TokenStream<ComponentValue>& tokens)
{
    // [<'animation-duration'> || <easing-function> || <'animation-delay'> || <single-animation-iteration-count> || <single-animation-direction> || <single-animation-fill-mode> || <single-animation-play-state> || [ none | <keyframes-name> ] || <single-animation-timeline>]#
    // NB: While it isn't in the spec the CSSWG resolved to include `animation-timeline` as a reset-only sub-property
    //     of the `animation` shorthand so we shouldn't actually allow <single-animation-timeline>.
    //     https://github.com/w3c/csswg-drafts/issues/6946#issuecomment-1233190360

    Vector<PropertyID> longhand_ids {
        PropertyID::AnimationDuration,
        PropertyID::AnimationTimingFunction,
        PropertyID::AnimationDelay,
        PropertyID::AnimationIterationCount,
        PropertyID::AnimationDirection,
        PropertyID::AnimationFillMode,
        PropertyID::AnimationPlayState,
        PropertyID::AnimationName
    };

    // FIXME: The animation-trigger properties are reset-only sub-properties of the animation shorthand.
    return parse_coordinating_value_list_shorthand(tokens, PropertyID::Animation, longhand_ids, { PropertyID::AnimationTimeline });
}

RefPtr<StyleValue const> Parser::parse_background_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();

    auto make_background_shorthand = [](auto background_color, auto background_image, auto background_position, auto background_size, auto background_repeat, auto background_attachment, auto background_origin, auto background_clip) {
        return ShorthandStyleValue::create(PropertyID::Background,
            { PropertyID::BackgroundColor, PropertyID::BackgroundImage, PropertyID::BackgroundPosition, PropertyID::BackgroundSize, PropertyID::BackgroundRepeat, PropertyID::BackgroundAttachment, PropertyID::BackgroundOrigin, PropertyID::BackgroundClip },
            { move(background_color), move(background_image), move(background_position), move(background_size), move(background_repeat), move(background_attachment), move(background_origin), move(background_clip) });
    };

    StyleValueVector background_images;
    StyleValueVector background_position_xs;
    StyleValueVector background_position_ys;
    StyleValueVector background_sizes;
    StyleValueVector background_repeats;
    StyleValueVector background_attachments;
    StyleValueVector background_clips;
    StyleValueVector background_origins;
    RefPtr<StyleValue const> background_color;

    auto initial_background_image = property_initial_value(PropertyID::BackgroundImage)->as_value_list().values()[0];
    auto initial_background_position_x = property_initial_value(PropertyID::BackgroundPositionX)->as_value_list().values()[0];
    auto initial_background_position_y = property_initial_value(PropertyID::BackgroundPositionY)->as_value_list().values()[0];
    auto initial_background_size = property_initial_value(PropertyID::BackgroundSize)->as_value_list().values()[0];
    auto initial_background_repeat = property_initial_value(PropertyID::BackgroundRepeat)->as_value_list().values()[0];
    auto initial_background_attachment = property_initial_value(PropertyID::BackgroundAttachment)->as_value_list().values()[0];
    auto initial_background_clip = property_initial_value(PropertyID::BackgroundClip)->as_value_list().values()[0];
    auto initial_background_origin = property_initial_value(PropertyID::BackgroundOrigin)->as_value_list().values()[0];
    auto initial_background_color = property_initial_value(PropertyID::BackgroundColor);

    // Per-layer values
    RefPtr<StyleValue const> background_image;
    RefPtr<StyleValue const> background_position_x;
    RefPtr<StyleValue const> background_position_y;
    RefPtr<StyleValue const> background_size;
    RefPtr<StyleValue const> background_repeat;
    RefPtr<StyleValue const> background_attachment;
    RefPtr<StyleValue const> background_clip;
    RefPtr<StyleValue const> background_origin;

    auto background_layer_is_valid = [&](bool allow_background_color) -> bool {
        if (allow_background_color) {
            if (background_color)
                return true;
        } else {
            if (background_color)
                return false;
        }
        return background_image || background_position_x || background_position_y || background_size || background_repeat || background_attachment || background_clip || background_origin;
    };

    auto complete_background_layer = [&]() {
        background_images.append(background_image ? background_image.release_nonnull() : initial_background_image);
        background_position_xs.append(background_position_x ? background_position_x.release_nonnull() : initial_background_position_x);
        background_position_ys.append(background_position_y ? background_position_y.release_nonnull() : initial_background_position_y);
        background_sizes.append(background_size ? background_size.release_nonnull() : initial_background_size);
        background_repeats.append(background_repeat ? background_repeat.release_nonnull() : initial_background_repeat);
        background_attachments.append(background_attachment ? background_attachment.release_nonnull() : initial_background_attachment);

        if (!background_origin && !background_clip) {
            background_origin = initial_background_origin;
            background_clip = initial_background_clip;
        } else if (!background_clip) {
            background_clip = background_origin;
        }
        background_origins.append(background_origin.release_nonnull());
        background_clips.append(background_clip.release_nonnull());

        background_image = nullptr;
        background_position_x = nullptr;
        background_position_y = nullptr;
        background_size = nullptr;
        background_repeat = nullptr;
        background_attachment = nullptr;
        background_clip = nullptr;
        background_origin = nullptr;
    };

    {
        auto rust_transaction = tokens.begin_transaction();
        auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
        if (auto rust_items = RustComponentValueParser::parse_layer_shorthand(PropertyID::Background, source.bytes_as_string_view()); rust_items.has_value()) {
            auto parse_single_layer_value = [this](PropertyID property_id, String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source.bytes_as_string_view(), "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_css_value_for_property(property_id, value_tokens);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return {};
                if (value->is_value_list() && value->as_value_list().size() == 1)
                    return value->as_value_list().values()[0];
                return value;
            };

            bool failed_to_materialize_rust_background = false;
            size_t current_layer_index = 0;
            for (auto const& item : rust_items.value()) {
                while (item.layer_index > current_layer_index) {
                    if (!background_layer_is_valid(false)) {
                        failed_to_materialize_rust_background = true;
                        break;
                    }
                    complete_background_layer();
                    ++current_layer_index;
                }
                if (failed_to_materialize_rust_background)
                    break;

                switch (item.property_id) {
                case PropertyID::BackgroundAttachment:
                    background_attachment = parse_single_layer_value(item.property_id, item.value);
                    if (!background_attachment)
                        failed_to_materialize_rust_background = true;
                    break;
                case PropertyID::BackgroundColor:
                    background_color = parse_single_layer_value(item.property_id, item.value);
                    if (!background_color)
                        failed_to_materialize_rust_background = true;
                    break;
                case PropertyID::BackgroundImage:
                    background_image = parse_single_layer_value(item.property_id, item.value);
                    if (!background_image)
                        failed_to_materialize_rust_background = true;
                    break;
                case PropertyID::BackgroundClip:
                case PropertyID::BackgroundOrigin: {
                    auto value = parse_single_layer_value(item.property_id, item.value);
                    if (!value) {
                        failed_to_materialize_rust_background = true;
                        break;
                    }
                    if (!background_origin)
                        background_origin = value.release_nonnull();
                    else
                        background_clip = value.release_nonnull();
                    break;
                }
                case PropertyID::BackgroundPosition: {
                    auto component_values = RustComponentValueParser::parse_a_list_of_component_values(item.value.bytes_as_string_view(), "utf-8"sv);
                    TokenStream value_tokens { component_values };
                    auto position = parse_position_value(value_tokens, PositionParsingMode::BackgroundPosition);
                    value_tokens.discard_whitespace();
                    if (!position || value_tokens.has_next_token()) {
                        failed_to_materialize_rust_background = true;
                        break;
                    }
                    background_position_x = position->as_position().edge_x();
                    background_position_y = position->as_position().edge_y();
                    break;
                }
                case PropertyID::BackgroundRepeat: {
                    background_repeat = parse_single_layer_value(item.property_id, item.value);
                    if (!background_repeat)
                        failed_to_materialize_rust_background = true;
                    break;
                }
                case PropertyID::BackgroundSize: {
                    background_size = parse_single_layer_value(item.property_id, item.value);
                    if (!background_size)
                        failed_to_materialize_rust_background = true;
                    break;
                }
                default:
                    failed_to_materialize_rust_background = true;
                    break;
                }
            }

            if (!failed_to_materialize_rust_background && background_layer_is_valid(true)) {
                complete_background_layer();

                if (!background_color)
                    background_color = initial_background_color;
                while (tokens.has_next_token())
                    tokens.discard_a_token();
                rust_transaction.commit();
                transaction.commit();
                return make_background_shorthand(
                    background_color.release_nonnull(),
                    StyleValueList::create(move(background_images), StyleValueList::Separator::Comma),
                    ShorthandStyleValue::create(PropertyID::BackgroundPosition,
                        { PropertyID::BackgroundPositionX, PropertyID::BackgroundPositionY },
                        { StyleValueList::create(move(background_position_xs), StyleValueList::Separator::Comma),
                            StyleValueList::create(move(background_position_ys), StyleValueList::Separator::Comma) }),
                    StyleValueList::create(move(background_sizes), StyleValueList::Separator::Comma),
                    StyleValueList::create(move(background_repeats), StyleValueList::Separator::Comma),
                    StyleValueList::create(move(background_attachments), StyleValueList::Separator::Comma),
                    StyleValueList::create(move(background_origins), StyleValueList::Separator::Comma),
                    StyleValueList::create(move(background_clips), StyleValueList::Separator::Comma));
            }
        }
    }

    return nullptr;
}

// https://drafts.csswg.org/css-fonts-4/#font-prop
RefPtr<StyleValue const> Parser::parse_font_value(TokenStream<ComponentValue>& tokens)
{
    // [ [ <'font-style'> || <font-variant-css2> || <'font-weight'> || <font-width-css3> ]? <'font-size'> [ / <'line-height'> ]? <'font-family'># ] | <system-family-name>
    //
    // FIXME: Handle <system-family-name>. (caption, icon, menu, message-box, small-caption, status-bar)
    auto transaction = tokens.begin_transaction();
    auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
    auto rust_items = RustComponentValueParser::parse_font_shorthand(source.bytes_as_string_view());
    if (!rust_items.has_value())
        return nullptr;

    RefPtr<StyleValue const> font_style;
    RefPtr<StyleValue const> font_variant;
    RefPtr<StyleValue const> font_weight;
    RefPtr<StyleValue const> font_width;
    RefPtr<StyleValue const> font_size;
    RefPtr<StyleValue const> line_height;
    RefPtr<StyleValue const> font_families;

    auto parse_value = [this](PropertyID property_id, String const& source) -> RefPtr<StyleValue const> {
        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source.bytes_as_string_view(), "utf-8"sv);
        TokenStream value_tokens { component_values };
        auto value = parse_css_value_for_property(property_id, value_tokens);
        value_tokens.discard_whitespace();
        if (!value || value_tokens.has_next_token())
            return {};
        return value;
    };

    for (auto const& item : rust_items.value()) {
        auto value = parse_value(item.property_id, item.value);
        if (!value)
            return nullptr;

        switch (item.property_id) {
        case PropertyID::FontSize: {
            if (font_size)
                return nullptr;
            font_size = value.release_nonnull();
            break;
        }
        case PropertyID::LineHeight: {
            if (line_height)
                return nullptr;
            line_height = value.release_nonnull();
            break;
        }
        case PropertyID::FontFamily: {
            if (font_families)
                return nullptr;
            font_families = value.release_nonnull();
            break;
        }
        case PropertyID::FontStyle: {
            if (font_style)
                return nullptr;
            font_style = value.release_nonnull();
            break;
        }
        case PropertyID::FontVariant: {
            if (font_variant)
                return nullptr;
            font_variant = value.release_nonnull();
            break;
        }
        case PropertyID::FontWeight: {
            if (font_weight)
                return nullptr;
            font_weight = value.release_nonnull();
            break;
        }
        case PropertyID::FontWidth: {
            if (font_width)
                return nullptr;
            font_width = value.release_nonnull();
            break;
        }
        default:
            VERIFY_NOT_REACHED();
        }
    }

    if (!font_size || !font_families)
        return nullptr;

    if (!font_style)
        font_style = property_initial_value(PropertyID::FontStyle);
    if (!font_variant)
        font_variant = property_initial_value(PropertyID::FontVariant);
    if (!font_weight)
        font_weight = property_initial_value(PropertyID::FontWeight);
    if (!font_width)
        font_width = property_initial_value(PropertyID::FontWidth);
    if (!line_height)
        line_height = property_initial_value(PropertyID::LineHeight);

    while (tokens.has_next_token())
        tokens.discard_a_token();
    transaction.commit();
    return ShorthandStyleValue::create(PropertyID::Font,
        {
            // Set explicitly https://drafts.csswg.org/css-fonts/#set-explicitly
            PropertyID::FontFamily,
            PropertyID::FontSize,
            PropertyID::FontWidth,
            PropertyID::FontStyle,
            PropertyID::FontVariant,
            PropertyID::FontWeight,
            PropertyID::LineHeight,

            // Reset implicitly https://drafts.csswg.org/css-fonts/#reset-implicitly
            PropertyID::FontFeatureSettings,
            PropertyID::FontKerning,
            PropertyID::FontLanguageOverride,
            PropertyID::FontOpticalSizing,
            // FIXME: PropertyID::FontSizeAdjust,
            PropertyID::FontVariationSettings,
        },
        {
            // Set explicitly
            font_families.release_nonnull(),
            font_size.release_nonnull(),
            font_width.release_nonnull(),
            font_style.release_nonnull(),
            font_variant.release_nonnull(),
            font_weight.release_nonnull(),
            line_height.release_nonnull(),

            // Reset implicitly
            property_initial_value(PropertyID::FontFeatureSettings),   // font-feature-settings
            property_initial_value(PropertyID::FontKerning),           // font-kerning,
            property_initial_value(PropertyID::FontLanguageOverride),  // font-language-override
            property_initial_value(PropertyID::FontOpticalSizing),     // font-optical-sizing,
                                                                       // FIXME: font-size-adjust,
            property_initial_value(PropertyID::FontVariationSettings), // font-variation-settings
        });
}

RefPtr<StyleValue const> Parser::parse_mask_value(TokenStream<ComponentValue>& tokens)
{
    // https://drafts.fxtf.org/css-masking-1/#the-mask
    // <mask-layer>#
    //
    // <mask-layer> =
    //   <mask-reference> ||
    //   <position> [ / <bg-size> ]? ||
    //   <repeat-style> ||
    //   <geometry-box> ||
    //   [ <geometry-box> | no-clip ] ||
    //   <compositing-operator> ||
    //   <masking-mode>
    auto transaction = tokens.begin_transaction();

    auto make_mask_shorthand = [](auto mask_image, auto mask_position, auto mask_size, auto mask_repeat, auto mask_origin, auto mask_clip, auto mask_composite, auto mask_mode) {
        return ShorthandStyleValue::create(PropertyID::Mask,
            { PropertyID::MaskImage, PropertyID::MaskPosition, PropertyID::MaskSize, PropertyID::MaskRepeat, PropertyID::MaskOrigin, PropertyID::MaskClip, PropertyID::MaskComposite, PropertyID::MaskMode },
            { move(mask_image), move(mask_position), move(mask_size), move(mask_repeat), move(mask_origin), move(mask_clip), move(mask_composite), move(mask_mode) });
    };

    StyleValueVector mask_images;
    StyleValueVector mask_positions;
    StyleValueVector mask_sizes;
    StyleValueVector mask_repeats;
    StyleValueVector mask_origins;
    StyleValueVector mask_clips;
    StyleValueVector mask_composites;
    StyleValueVector mask_modes;

    auto initial_mask_image = property_initial_value(PropertyID::MaskImage)->as_value_list().values()[0];
    auto initial_mask_position = property_initial_value(PropertyID::MaskPosition)->as_value_list().values()[0];
    auto initial_mask_size = property_initial_value(PropertyID::MaskSize)->as_value_list().values()[0];
    auto initial_mask_repeat = property_initial_value(PropertyID::MaskRepeat)->as_value_list().values()[0];
    auto initial_mask_origin = property_initial_value(PropertyID::MaskOrigin)->as_value_list().values()[0];
    auto initial_mask_clip = property_initial_value(PropertyID::MaskClip)->as_value_list().values()[0];
    auto initial_mask_composite = property_initial_value(PropertyID::MaskComposite)->as_value_list().values()[0];
    auto initial_mask_mode = property_initial_value(PropertyID::MaskMode)->as_value_list().values()[0];

    // Per-layer values
    RefPtr<StyleValue const> mask_image;
    RefPtr<StyleValue const> mask_position;
    RefPtr<StyleValue const> mask_size;
    RefPtr<StyleValue const> mask_repeat;
    RefPtr<StyleValue const> mask_origin;
    RefPtr<StyleValue const> mask_clip;
    RefPtr<StyleValue const> mask_composite;
    RefPtr<StyleValue const> mask_mode;

    bool has_multiple_layers = false;
    auto mask_layer_is_valid = [&]() -> bool {
        return mask_image || mask_position || mask_size || mask_repeat || mask_origin || mask_clip || mask_composite || mask_mode;
    };

    auto complete_mask_layer = [&]() {
        mask_images.append(mask_image ? mask_image.release_nonnull() : initial_mask_image);
        mask_positions.append(mask_position ? mask_position.release_nonnull() : initial_mask_position);
        mask_sizes.append(mask_size ? mask_size.release_nonnull() : initial_mask_size);
        mask_repeats.append(mask_repeat ? mask_repeat.release_nonnull() : initial_mask_repeat);
        mask_composites.append(mask_composite ? mask_composite.release_nonnull() : initial_mask_composite);
        mask_modes.append(mask_mode ? mask_mode.release_nonnull() : initial_mask_mode);

        if (!mask_origin)
            mask_origin = initial_mask_origin;
        if (!mask_clip)
            mask_clip = mask_origin;
        mask_origins.append(mask_origin.release_nonnull());
        mask_clips.append(mask_clip.release_nonnull());

        mask_image = nullptr;
        mask_position = nullptr;
        mask_size = nullptr;
        mask_repeat = nullptr;
        mask_origin = nullptr;
        mask_clip = nullptr;
        mask_composite = nullptr;
        mask_mode = nullptr;
    };

    {
        auto rust_transaction = tokens.begin_transaction();
        auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
        if (auto rust_items = RustComponentValueParser::parse_layer_shorthand(PropertyID::Mask, source.bytes_as_string_view()); rust_items.has_value()) {
            auto parse_single_layer_value = [this](PropertyID property_id, String const& source) -> RefPtr<StyleValue const> {
                auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source.bytes_as_string_view(), "utf-8"sv);
                TokenStream value_tokens { component_values };
                auto value = parse_css_value_for_property(property_id, value_tokens);
                value_tokens.discard_whitespace();
                if (!value || value_tokens.has_next_token())
                    return {};
                if (value->is_value_list() && value->as_value_list().size() == 1)
                    return value->as_value_list().values()[0];
                return value;
            };

            bool failed_to_materialize_rust_mask = false;
            size_t current_layer_index = 0;
            has_multiple_layers = rust_items->last().layer_index > 0;
            for (auto const& item : rust_items.value()) {
                while (item.layer_index > current_layer_index) {
                    if (!mask_layer_is_valid()) {
                        failed_to_materialize_rust_mask = true;
                        break;
                    }
                    complete_mask_layer();
                    ++current_layer_index;
                }
                if (failed_to_materialize_rust_mask)
                    break;

                switch (item.property_id) {
                case PropertyID::MaskImage:
                    mask_image = parse_single_layer_value(item.property_id, item.value);
                    if (!mask_image)
                        failed_to_materialize_rust_mask = true;
                    break;
                case PropertyID::MaskPosition: {
                    auto component_values = RustComponentValueParser::parse_a_list_of_component_values(item.value.bytes_as_string_view(), "utf-8"sv);
                    TokenStream value_tokens { component_values };
                    mask_position = parse_position_value(value_tokens);
                    value_tokens.discard_whitespace();
                    if (!mask_position || value_tokens.has_next_token())
                        failed_to_materialize_rust_mask = true;
                    break;
                }
                case PropertyID::MaskSize: {
                    mask_size = parse_single_layer_value(item.property_id, item.value);
                    if (!mask_size)
                        failed_to_materialize_rust_mask = true;
                    break;
                }
                case PropertyID::MaskRepeat: {
                    mask_repeat = parse_single_layer_value(item.property_id, item.value);
                    if (!mask_repeat)
                        failed_to_materialize_rust_mask = true;
                    break;
                }
                case PropertyID::MaskOrigin:
                case PropertyID::MaskClip: {
                    auto value = parse_single_layer_value(item.property_id, item.value);
                    if (!value) {
                        failed_to_materialize_rust_mask = true;
                        break;
                    }
                    if (value->is_keyword() && value->as_keyword().keyword() == Keyword::NoClip)
                        mask_clip = value.release_nonnull();
                    else if (!mask_origin)
                        mask_origin = value.release_nonnull();
                    else
                        mask_clip = value.release_nonnull();
                    break;
                }
                case PropertyID::MaskComposite:
                    mask_composite = parse_single_layer_value(item.property_id, item.value);
                    if (!mask_composite)
                        failed_to_materialize_rust_mask = true;
                    break;
                case PropertyID::MaskMode:
                    mask_mode = parse_single_layer_value(item.property_id, item.value);
                    if (!mask_mode)
                        failed_to_materialize_rust_mask = true;
                    break;
                default:
                    failed_to_materialize_rust_mask = true;
                    break;
                }
            }

            if (!failed_to_materialize_rust_mask && mask_layer_is_valid()) {
                while (tokens.has_next_token())
                    tokens.discard_a_token();
                rust_transaction.commit();
                transaction.commit();

                if (has_multiple_layers) {
                    complete_mask_layer();
                    return make_mask_shorthand(
                        StyleValueList::create(move(mask_images), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_positions), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_sizes), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_repeats), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_origins), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_clips), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_composites), StyleValueList::Separator::Comma),
                        StyleValueList::create(move(mask_modes), StyleValueList::Separator::Comma));
                }

                if (!mask_image)
                    mask_image = initial_mask_image;
                if (!mask_position)
                    mask_position = initial_mask_position;
                if (!mask_size)
                    mask_size = initial_mask_size;
                if (!mask_repeat)
                    mask_repeat = initial_mask_repeat;
                if (!mask_origin)
                    mask_origin = initial_mask_origin;
                if (!mask_clip)
                    mask_clip = mask_origin;
                if (!mask_composite)
                    mask_composite = initial_mask_composite;
                if (!mask_mode)
                    mask_mode = initial_mask_mode;

                return make_mask_shorthand(
                    mask_image.release_nonnull(),
                    mask_position.release_nonnull(),
                    mask_size.release_nonnull(),
                    mask_repeat.release_nonnull(),
                    mask_origin.release_nonnull(),
                    mask_clip.release_nonnull(),
                    mask_composite.release_nonnull(),
                    mask_mode.release_nonnull());
            }
        }
    }

    return nullptr;
}

// https://drafts.csswg.org/css-transitions-2/#transition-shorthand-property
RefPtr<StyleValue const> Parser::parse_transition_value(TokenStream<ComponentValue>& tokens)
{
    // [ [ none | <single-transition-property> ] || <time> || <easing-function> || <time> || <transition-behavior-value> ]#
    Vector<PropertyID> longhand_ids {
        PropertyID::TransitionProperty,
        PropertyID::TransitionDuration,
        PropertyID::TransitionTimingFunction,
        PropertyID::TransitionDelay,
        PropertyID::TransitionBehavior
    };

    return parse_coordinating_value_list_shorthand(tokens, PropertyID::Transition, longhand_ids);
}

RefPtr<StyleValue const> Parser::parse_grid_track_placement_shorthand_value(PropertyID property_id, TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
    auto rust_items = RustComponentValueParser::parse_grid_placement_shorthand(property_id, source.bytes_as_string_view());
    if (!rust_items.has_value())
        return nullptr;

    Vector<PropertyID> longhands;
    StyleValueVector longhand_values;
    longhands.ensure_capacity(rust_items->size());
    longhand_values.ensure_capacity(rust_items->size());

    for (auto const& item : rust_items.value()) {
        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(item.value.bytes_as_string_view(), "utf-8"sv);
        TokenStream placement_tokens { component_values };
        auto value = parse_css_value_for_property(item.property_id, placement_tokens);
        placement_tokens.discard_whitespace();
        if (!value || placement_tokens.has_next_token())
            return nullptr;

        longhands.unchecked_append(item.property_id);
        longhand_values.unchecked_append(value.release_nonnull());
    }

    while (tokens.has_next_token())
        tokens.discard_a_token();
    transaction.commit();
    return ShorthandStyleValue::create(property_id, move(longhands), move(longhand_values));
}

// https://www.w3.org/TR/css-grid-2/#explicit-grid-shorthand
// 7.4. Explicit Grid Shorthand: the grid-template property
RefPtr<StyleValue const> Parser::parse_grid_track_size_list_shorthand_value(PropertyID property_id, TokenStream<ComponentValue>& tokens, bool include_grid_auto_properties)
{
    auto transaction = tokens.begin_transaction();
    auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
    auto rust_items = RustComponentValueParser::parse_grid_template_shorthand(property_id, source.bytes_as_string_view());
    if (!rust_items.has_value())
        return nullptr;

    Vector<PropertyID> sub_properties;
    Vector<ValueComparingNonnullRefPtr<StyleValue const>> values;
    if (include_grid_auto_properties) {
        sub_properties.append(PropertyID::GridAutoFlow);
        sub_properties.append(PropertyID::GridAutoRows);
        sub_properties.append(PropertyID::GridAutoColumns);
    }
    sub_properties.append(PropertyID::GridTemplateAreas);
    sub_properties.append(PropertyID::GridTemplateRows);
    sub_properties.append(PropertyID::GridTemplateColumns);

    values.ensure_capacity(sub_properties.size());
    for (auto property_id : sub_properties)
        values.unchecked_append(property_initial_value(property_id));

    auto materialize_source_as_property = [&](PropertyID property_id, String const& source) -> RefPtr<StyleValue const> {
        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(source.bytes_as_string_view(), "utf-8"sv);
        TokenStream property_tokens { component_values };
        auto value = parse_css_value_for_property(property_id, property_tokens);
        property_tokens.discard_whitespace();
        if (!value || property_tokens.has_next_token())
            return nullptr;
        return value;
    };

    for (auto const& item : rust_items.value()) {
        auto value = materialize_source_as_property(item.property_id, item.value);
        if (!value)
            return nullptr;

        auto index = sub_properties.find_first_index(item.property_id);
        if (!index.has_value())
            return nullptr;
        values[index.value()] = value.release_nonnull();
    }

    while (tokens.has_next_token())
        tokens.discard_a_token();
    transaction.commit();
    return ShorthandStyleValue::create(property_id, move(sub_properties), move(values));
}

RefPtr<StyleValue const> Parser::parse_grid_area_shorthand_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto source = serialize_component_values_for_reparsing(tokens.remaining_tokens());
    auto rust_items = RustComponentValueParser::parse_grid_placement_shorthand(PropertyID::GridArea, source.bytes_as_string_view());
    if (!rust_items.has_value())
        return nullptr;

    Vector<PropertyID> longhands;
    StyleValueVector longhand_values;
    longhands.ensure_capacity(rust_items->size());
    longhand_values.ensure_capacity(rust_items->size());

    for (auto const& item : rust_items.value()) {
        auto component_values = RustComponentValueParser::parse_a_list_of_component_values(item.value.bytes_as_string_view(), "utf-8"sv);
        TokenStream placement_tokens { component_values };
        auto value = parse_css_value_for_property(item.property_id, placement_tokens);
        placement_tokens.discard_whitespace();
        if (!value || placement_tokens.has_next_token())
            return nullptr;

        longhands.unchecked_append(item.property_id);
        longhand_values.unchecked_append(value.release_nonnull());
    }

    while (tokens.has_next_token())
        tokens.discard_a_token();
    transaction.commit();
    return ShorthandStyleValue::create(PropertyID::GridArea, move(longhands), move(longhand_values));
}

RefPtr<StyleValue const> Parser::parse_grid_shorthand_value(TokenStream<ComponentValue>& tokens)
{
    return parse_grid_track_size_list_shorthand_value(PropertyID::Grid, tokens, true);
}

}
