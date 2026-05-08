/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn parse_url_or_src_function_contents(
    function_type: CssUrlFunctionType,
    component_values: &[ComponentValue],
) -> Option<UrlFunction> {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value: url },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };
    parser.discard_whitespace();

    // NB: Currently <request-url-modifier> is the only kind of <url-modifier>
    // https://drafts.csswg.org/css-values-5/#request-url-modifiers
    // <request-url-modifier> = <cross-origin-modifier> | <integrity-modifier> | <referrer-policy-modifier>
    let mut request_url_modifiers = Vec::new();
    while parser.has_next_component_value() {
        let modifier = parse_request_url_modifier(&parser.consume_the_next_component_value()?)?;

        // AD-HOC: This isn't mentioned in the spec, but WPT expects modifiers to be unique (one per type).
        // Spec issue: https://github.com/w3c/csswg-drafts/issues/12151
        if request_url_modifiers
            .iter()
            .any(|existing_modifier: &UrlModifier| existing_modifier.kind() == modifier.kind())
        {
            return None;
        }
        request_url_modifiers.push(modifier);
        parser.discard_whitespace();
    }

    // AD-HOC: This isn't mentioned in the spec, but WPT expects modifiers to be sorted alphabetically.
    // Spec issue: https://github.com/w3c/csswg-drafts/issues/12151
    request_url_modifiers.sort_by_key(UrlModifier::kind);

    Some(UrlFunction {
        function_type,
        url,
        request_url_modifiers,
    })
}

pub(super) fn parse_request_url_modifier(component_value: &ComponentValue) -> Option<UrlModifier> {
    match component_value {
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("cross-origin") => {
            parse_cross_origin_modifier(function)
        }
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("integrity") => {
            parse_integrity_modifier(function)
        }
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("referrer-policy") => {
            parse_referrer_policy_modifier(function)
        }
        _ => None,
    }
}

pub(super) fn parse_cross_origin_modifier(function: &Function) -> Option<UrlModifier> {
    // <cross-origin-modifier> = cross-origin(anonymous | use-credentials)
    let ident = parse_single_ident_from_function(function)?;
    let value = match ident.as_str() {
        value if value.eq_ignore_ascii_case("anonymous") => CssUrlCrossOriginModifierValue::Anonymous,
        value if value.eq_ignore_ascii_case("use-credentials") => CssUrlCrossOriginModifierValue::UseCredentials,
        _ => return None,
    };
    Some(UrlModifier::CrossOrigin(value))
}

pub(super) fn parse_integrity_modifier(function: &Function) -> Option<UrlModifier> {
    // <integrity-modifier> = integrity(<string>)
    let mut parser = ComponentValueParser::new(function.value.clone());
    parser.discard_whitespace();
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value: integrity },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }
    Some(UrlModifier::Integrity(integrity))
}

pub(super) fn parse_referrer_policy_modifier(function: &Function) -> Option<UrlModifier> {
    // <referrer-policy-modifier> = (no-referrer | no-referrer-when-downgrade | same-origin | origin | strict-origin | origin-when-cross-origin | strict-origin-when-cross-origin | unsafe-url)
    let ident = parse_single_ident_from_function(function)?;
    let value = match ident.as_str() {
        value if value.eq_ignore_ascii_case("no-referrer") => CssUrlReferrerPolicyModifierValue::NoReferrer,
        value if value.eq_ignore_ascii_case("no-referrer-when-downgrade") => {
            CssUrlReferrerPolicyModifierValue::NoReferrerWhenDowngrade
        }
        value if value.eq_ignore_ascii_case("same-origin") => CssUrlReferrerPolicyModifierValue::SameOrigin,
        value if value.eq_ignore_ascii_case("origin") => CssUrlReferrerPolicyModifierValue::Origin,
        value if value.eq_ignore_ascii_case("strict-origin") => CssUrlReferrerPolicyModifierValue::StrictOrigin,
        value if value.eq_ignore_ascii_case("origin-when-cross-origin") => {
            CssUrlReferrerPolicyModifierValue::OriginWhenCrossOrigin
        }
        value if value.eq_ignore_ascii_case("strict-origin-when-cross-origin") => {
            CssUrlReferrerPolicyModifierValue::StrictOriginWhenCrossOrigin
        }
        value if value.eq_ignore_ascii_case("unsafe-url") => CssUrlReferrerPolicyModifierValue::UnsafeUrl,
        _ => return None,
    };
    Some(UrlModifier::ReferrerPolicy(value))
}

pub(super) fn parse_font_format_function(function: &Function) -> Option<(String, Vec<CssFontTech>)> {
    // <font-format> = [ <string> | collection | embedded-opentype | opentype | svg | truetype | woff | woff2 ]
    let mut parser = ComponentValueParser::new(function.value.clone());
    parser.discard_whitespace();
    let format_name_token = parser.consume_the_next_component_value()?;
    let (format, tech) = match format_name_token {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => (value, Vec::new()),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        }) => parse_font_format_string(&value)?,
        _ => return None,
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }
    Some((format, tech))
}

pub(super) fn parse_font_format_string(value: &str) -> Option<(String, Vec<CssFontTech>)> {
    // https://drafts.csswg.org/css-fonts-4/#font-face-src-parsing
    // There's a fixed set of strings allowed here, which we'll assume are case-insensitive:
    // format("woff2")                 -> format(woff2)
    // format("woff")                  -> format(woff)
    // format("truetype")              -> format(truetype)
    // format("opentype")              -> format(opentype)
    // format("collection")            -> format(collection)
    // format("woff2-variations")      -> format(woff2) tech(variations)
    // format("woff-variations")       -> format(woff) tech(variations)
    // format("truetype-variations")   -> format(truetype) tech(variations)
    // format("opentype-variations")   -> format(opentype) tech(variations)
    let (format, has_variations) = match value {
        value if value.eq_ignore_ascii_case("woff2") => ("woff2", false),
        value if value.eq_ignore_ascii_case("woff") => ("woff", false),
        value if value.eq_ignore_ascii_case("truetype") => ("truetype", false),
        value if value.eq_ignore_ascii_case("opentype") => ("opentype", false),
        value if value.eq_ignore_ascii_case("collection") => ("collection", false),
        value if value.eq_ignore_ascii_case("woff2-variations") => ("woff2", true),
        value if value.eq_ignore_ascii_case("woff-variations") => ("woff", true),
        value if value.eq_ignore_ascii_case("truetype-variations") => ("truetype", true),
        value if value.eq_ignore_ascii_case("opentype-variations") => ("opentype", true),
        _ => return None,
    };

    Some((
        format.to_string(),
        if has_variations {
            vec![CssFontTech::Variations]
        } else {
            Vec::new()
        },
    ))
}

pub(super) fn parse_font_tech_function(function: &Function) -> Option<Vec<CssFontTech>> {
    // <font-tech> = features-opentype | features-aat | features-graphite | variations | color-COLRv0 | color-COLRv1
    //             | color-SVG | color-sbix | color-CBDT | palettes | incremental
    let mut groups = Vec::new();
    let mut current_group = Vec::new();
    for component_value in &function.value {
        if matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            })
        ) {
            groups.push(current_group);
            current_group = Vec::new();
            continue;
        }
        current_group.push(component_value.clone());
    }
    groups.push(current_group);

    if groups.is_empty() {
        return None;
    }

    let mut tech = Vec::new();
    for group in groups {
        let mut parser = ComponentValueParser::new(group);
        parser.discard_whitespace();
        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = parser.consume_the_next_component_value()?
        else {
            return None;
        };
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        tech.push(parse_font_tech_name(&value)?);
    }

    Some(tech)
}

pub(super) fn parse_comma_separated_component_values<T, F>(
    component_values: Vec<ComponentValue>,
    mut parse_group: F,
) -> Option<Vec<T>>
where
    F: FnMut(Vec<ComponentValue>) -> Option<T>,
{
    let mut groups = Vec::new();
    let mut current_group = Vec::new();
    for component_value in component_values {
        if matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            })
        ) {
            groups.push(current_group);
            current_group = Vec::new();
            continue;
        }
        current_group.push(component_value);
    }
    groups.push(current_group);

    if groups.is_empty() {
        return None;
    }

    groups.into_iter().map(&mut parse_group).collect()
}

pub(super) fn parse_font_variant_alternates_feature_value_names(
    component_values: Vec<ComponentValue>,
) -> Option<Vec<String>> {
    let groups = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.parse_a_custom_ident(&[])
    })?;

    (!groups.is_empty()).then_some(groups)
}

// https://drafts.csswg.org/css-fonts/#typedef-opentype-tag
pub(super) fn parse_opentype_tag(parser: &mut ComponentValueParser) -> Option<String> {
    // <opentype-tag> = <string>
    // The <opentype-tag> is a case-sensitive OpenType feature tag.
    // As specified in the OpenType specification [OPENTYPE], feature tags contain four ASCII characters.
    // Tag strings longer or shorter than four characters, or containing characters outside the U+20–7E codepoint range are invalid.
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };

    (value.len() == 4 && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))).then_some(value)
}

pub(super) fn parse_feature_tag_value(
    component_values: Vec<ComponentValue>,
    filtered_input: &str,
) -> Option<OpenTypeTaggedValue> {
    // <feature-tag-value> = <opentype-tag> [ <integer [0,∞]> | on | off ]?
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let tag = parse_opentype_tag(&mut parser)?;
    parser.discard_whitespace();

    if !parser.has_next_component_value() {
        // "If the value is omitted, a value of 1 is assumed."
        return Some(OpenTypeTaggedValue {
            tag,
            value_kind: CssOpenTypeTaggedValueKind::Implicit,
            value: None,
        });
    }

    if let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    {
        // A value of on is synonymous with 1 and off is synonymous with 0.
        let value_kind = if value.eq_ignore_ascii_case("on") {
            CssOpenTypeTaggedValueKind::On
        } else if value.eq_ignore_ascii_case("off") {
            CssOpenTypeTaggedValueKind::Off
        } else {
            CssOpenTypeTaggedValueKind::Value
        };

        if value_kind != CssOpenTypeTaggedValueKind::Value {
            parser.index += 1;
            parser.discard_whitespace();
            if parser.has_next_component_value() {
                return None;
            }
            return Some(OpenTypeTaggedValue {
                tag,
                value_kind,
                value: None,
            });
        }
    }

    let value = serialize_component_values_for_reparsing(parser.remaining_component_values(), filtered_input)?;
    Some(OpenTypeTaggedValue {
        tag,
        value_kind: CssOpenTypeTaggedValueKind::Value,
        value: Some(value),
    })
}

pub(super) fn parse_variation_tag_value(
    component_values: Vec<ComponentValue>,
    filtered_input: &str,
) -> Option<OpenTypeTaggedValue> {
    // [ <opentype-tag> <number>]
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let tag = parse_opentype_tag(&mut parser)?;
    parser.discard_whitespace();

    if !parser.has_next_component_value() {
        return None;
    }

    let value = serialize_component_values_for_reparsing(parser.remaining_component_values(), filtered_input)?;
    Some(OpenTypeTaggedValue {
        tag,
        value_kind: CssOpenTypeTaggedValueKind::Value,
        value: Some(value),
    })
}

pub(super) fn parse_font_tech_name(value: &str) -> Option<CssFontTech> {
    match value {
        value if value.eq_ignore_ascii_case("avar2") => Some(CssFontTech::Avar2),
        value if value.eq_ignore_ascii_case("color-cbdt") => Some(CssFontTech::ColorCbdt),
        value if value.eq_ignore_ascii_case("color-colrv0") => Some(CssFontTech::ColorColrv0),
        value if value.eq_ignore_ascii_case("color-colrv1") => Some(CssFontTech::ColorColrv1),
        value if value.eq_ignore_ascii_case("color-sbix") => Some(CssFontTech::ColorSbix),
        value if value.eq_ignore_ascii_case("color-svg") => Some(CssFontTech::ColorSvg),
        value if value.eq_ignore_ascii_case("features-aat") => Some(CssFontTech::FeaturesAat),
        value if value.eq_ignore_ascii_case("features-graphite") => Some(CssFontTech::FeaturesGraphite),
        value if value.eq_ignore_ascii_case("features-opentype") => Some(CssFontTech::FeaturesOpentype),
        value if value.eq_ignore_ascii_case("incremental") => Some(CssFontTech::Incremental),
        value if value.eq_ignore_ascii_case("palettes") => Some(CssFontTech::Palettes),
        value if value.eq_ignore_ascii_case("variations") => Some(CssFontTech::Variations),
        _ => None,
    }
}

pub(super) fn parse_font_language_override_string_value(value: &str) -> Option<String> {
    // https://drafts.csswg.org/css-fonts/#propdef-font-language-override
    // This is `normal | <string>` but with the constraint that the string has to be 4 characters long:
    // Shorter strings are right-padded with spaces before use, and longer strings are invalid.
    let length = value.len();
    if length == 0 || length > 4 || !value.is_ascii() {
        return None;
    }

    // We're expected to always serialize without any trailing spaces, so remove them now for convenience.
    let trimmed = value.trim_end_matches(|code_point: char| code_point.is_ascii_whitespace());
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

pub(super) fn parse_single_ident_from_function(function: &Function) -> Option<String> {
    let mut parser = ComponentValueParser::new(function.value.clone());
    parser.discard_whitespace();
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value: ident },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }
    Some(ident)
}
