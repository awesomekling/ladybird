/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

impl ComponentValueParser {
    pub(super) fn new(component_values: Vec<ComponentValue>) -> Self {
        Self {
            component_values,
            index: 0,
            boolean_expression: None,
            declared_namespaces: Vec::new(),
            pseudo_class_context: Vec::new(),
        }
    }

    pub(super) fn with_declared_namespaces(
        component_values: Vec<ComponentValue>,
        declared_namespaces: Vec<String>,
    ) -> Self {
        Self {
            declared_namespaces,
            ..Self::new(component_values)
        }
    }

    pub(super) fn next_component_value(&self) -> Option<&ComponentValue> {
        self.component_values.get(self.index)
    }

    pub(super) fn consume_the_next_component_value(&mut self) -> Option<ComponentValue> {
        let component_value = self.next_component_value()?.clone();
        self.index += 1;
        Some(component_value)
    }

    pub(super) fn consume_ident_matching(&mut self, expected: &str) -> bool {
        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && value.eq_ignore_ascii_case(expected)
        {
            self.index += 1;
            return true;
        }

        false
    }

    pub(super) fn consume_an_ident(&mut self) -> Option<String> {
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        else {
            return None;
        };
        let value = value.clone();
        self.index += 1;
        Some(value)
    }

    pub(super) fn consume_a_comma(&mut self) -> bool {
        if matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            }))
        ) {
            self.index += 1;
            return true;
        }

        false
    }

    pub(super) fn consume_a_delim(&mut self, expected: char) -> bool {
        if matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Delim { value },
                ..
            })) if *value == expected as u32
        ) {
            self.index += 1;
            return true;
        }

        false
    }

    pub(super) fn remaining_component_values(&self) -> &[ComponentValue] {
        &self.component_values[self.index..]
    }

    pub(super) fn discard_whitespace(&mut self) {
        while matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Whitespace,
                ..
            }))
        ) {
            self.index += 1;
        }
    }

    pub(super) fn has_next_component_value(&mut self) -> bool {
        self.discard_whitespace();
        self.next_component_value().is_some()
    }

    pub(super) fn parse_a_selector_list(
        &mut self,
        selector_type: SelectorType,
        parsing_mode: SelectorParsingMode,
    ) -> Option<Vec<SelectorSyntax>> {
        let mut selectors = Vec::new();

        loop {
            let selector_parts = self.consume_a_list_of_component_values_until_comma();
            let mut parser = ComponentValueParser::with_declared_namespaces(
                selector_parts.clone(),
                self.declared_namespaces.clone(),
            );
            parser.pseudo_class_context = self.pseudo_class_context.clone();

            if let Some(selector) = parser.parse_complex_selector(selector_type) {
                selectors.push(selector);
            } else if parsing_mode == SelectorParsingMode::Forgiving {
                let combinator = match selector_type {
                    SelectorType::Standalone => SelectorCombinator::None,
                    SelectorType::Relative => SelectorCombinator::Descendant,
                };
                selectors.push(create_invalid_selector_syntax(combinator, selector_parts));
            } else {
                return None;
            }

            self.discard_whitespace();
            if !matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                }))
            ) {
                break;
            }

            self.index += 1;
        }

        if selectors.is_empty() && parsing_mode != SelectorParsingMode::Forgiving {
            return None;
        }

        Some(selectors)
    }

    pub(super) fn consume_a_list_of_component_values_until_comma(&mut self) -> Vec<ComponentValue> {
        let start = self.index;
        while let Some(component_value) = self.next_component_value() {
            if matches!(
                component_value,
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                })
            ) {
                break;
            }
            self.index += 1;
        }
        self.component_values[start..self.index].to_vec()
    }

    pub(super) fn parse_complex_selector(&mut self, selector_type: SelectorType) -> Option<SelectorSyntax> {
        let mut compound_selectors = Vec::new();
        let mut first_combinator = self.parse_selector_combinator();

        match selector_type {
            SelectorType::Standalone => {
                if first_combinator.is_some_and(|combinator| combinator != SelectorCombinator::Descendant) {
                    return None;
                }
                first_combinator = Some(SelectorCombinator::None);
            }
            SelectorType::Relative => {
                first_combinator = Some(first_combinator.unwrap_or(SelectorCombinator::Descendant));
            }
        }

        let mut first_selector = self.parse_compound_selector()?;
        if first_selector.simple_selectors.is_empty() {
            return None;
        }
        first_selector.combinator = first_combinator.unwrap_or(SelectorCombinator::None);
        compound_selectors.push(first_selector);

        while self.index < self.component_values.len() {
            let Some(combinator) = self.parse_selector_combinator() else {
                break;
            };
            let mut compound_selector = self.parse_compound_selector()?;
            if compound_selector.simple_selectors.is_empty() {
                if self.index < self.component_values.len() || combinator != SelectorCombinator::Descendant {
                    return None;
                }
                break;
            }
            compound_selector.combinator = combinator;
            compound_selectors.push(compound_selector);
        }

        if compound_selectors.is_empty() || self.index < self.component_values.len() {
            return None;
        }

        validate_pseudo_element_chain(&compound_selectors)?;
        Some(SelectorSyntax { compound_selectors })
    }

    pub(super) fn parse_compound_selector(&mut self) -> Option<CompoundSelectorSyntax> {
        let mut simple_selectors = Vec::new();

        while self.index < self.component_values.len() {
            let Some(simple_selector) = self.parse_simple_selector()? else {
                break;
            };
            if matches!(simple_selector, SimpleSelectorSyntax::TagName(_)) && !simple_selectors.is_empty() {
                return None;
            }
            simple_selectors.push(simple_selector);
        }

        Some(CompoundSelectorSyntax {
            combinator: SelectorCombinator::None,
            simple_selectors,
        })
    }

    pub(super) fn parse_selector_combinator(&mut self) -> Option<SelectorCombinator> {
        let saved_index = self.index;
        let had_initial_whitespace = matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Whitespace,
                ..
            }))
        );
        self.discard_whitespace();

        if component_value_is_delim(self.next_component_value(), '>') {
            self.index += 1;
            self.discard_whitespace();
            return Some(SelectorCombinator::ImmediateChild);
        }
        if component_value_is_delim(self.next_component_value(), '+') {
            self.index += 1;
            self.discard_whitespace();
            return Some(SelectorCombinator::NextSibling);
        }
        if component_value_is_delim(self.next_component_value(), '~') {
            self.index += 1;
            self.discard_whitespace();
            return Some(SelectorCombinator::SubsequentSibling);
        }
        if component_value_is_delim(self.next_component_value(), '|')
            && component_value_is_delim(self.component_values.get(self.index + 1), '|')
        {
            self.index += 2;
            self.discard_whitespace();
            return Some(SelectorCombinator::Column);
        }

        if had_initial_whitespace {
            return Some(SelectorCombinator::Descendant);
        }

        self.index = saved_index;
        None
    }

    pub(super) fn parse_simple_selector(&mut self) -> Option<Option<SimpleSelectorSyntax>> {
        if selector_component_value_ends_selector(self.next_component_value()) {
            return Some(None);
        }

        if let Some(qualified_name) = self.parse_selector_qualified_name(AllowWildcardName::Yes) {
            if qualified_name.name == "*" {
                return Some(Some(SimpleSelectorSyntax::Universal(qualified_name)));
            }
            return Some(Some(SimpleSelectorSyntax::TagName(qualified_name)));
        }

        if self.next_is_pseudo_element() {
            return Some(Some(self.parse_pseudo_element_simple_selector()?));
        }

        if matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        ) {
            return Some(Some(self.parse_pseudo_class_simple_selector()?));
        }

        if component_value_is_one_of_delims(self.next_component_value(), &['>', '+', '~', '|']) {
            return Some(None);
        }

        let first_value = self.consume_the_next_component_value()?;
        match first_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Delim { value },
                ..
            }) if value == u32::from(b'&') => Some(Some(SimpleSelectorSyntax::Nesting)),
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Delim { value },
                ..
            }) if value == u32::from(b'.') => {
                if selector_component_value_ends_selector(self.next_component_value()) {
                    return None;
                }
                let Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) = self.consume_the_next_component_value()
                else {
                    return None;
                };
                Some(Some(SimpleSelectorSyntax::Class(value)))
            }
            ComponentValue::PreservedToken(Token {
                token_type:
                    TokenType::Hash {
                        hash_type: crate::css_tokenizer::CssHashType::Id,
                        value,
                    },
                ..
            }) => Some(Some(SimpleSelectorSyntax::Id(value))),
            ComponentValue::SimpleBlock(block) if is_square_block(&block) => {
                Some(Some(self.parse_attribute_simple_selector(block)?))
            }
            _ => None,
        }
    }

    pub(super) fn parse_selector_qualified_name(
        &mut self,
        allow_wildcard_name: AllowWildcardName,
    ) -> Option<QualifiedNameSyntax> {
        let saved_index = self.index;
        let first_token = self.consume_the_next_component_value()?;

        if component_value_is_delim(Some(&first_token), '|') {
            if selector_component_value_is_name(self.next_component_value()) {
                let name_token = self.consume_the_next_component_value()?;
                if allow_wildcard_name == AllowWildcardName::No && component_value_is_delim(Some(&name_token), '*') {
                    self.index = saved_index;
                    return None;
                }
                return Some(QualifiedNameSyntax {
                    namespace_type: NamespaceType::None,
                    namespace: String::new(),
                    name: selector_component_value_name(&name_token)?,
                });
            }
            self.index = saved_index;
            return None;
        }

        if !selector_component_value_is_name(Some(&first_token)) {
            self.index = saved_index;
            return None;
        }

        if component_value_is_delim(self.next_component_value(), '|')
            && selector_component_value_is_name(self.component_values.get(self.index + 1))
        {
            self.index += 1;
            let namespace = selector_component_value_name(&first_token)?;
            let name_token = self.consume_the_next_component_value()?;
            let name = selector_component_value_name(&name_token)?;

            if allow_wildcard_name == AllowWildcardName::No && name == "*" {
                self.index = saved_index;
                return None;
            }

            let namespace_type = if namespace == "*" {
                NamespaceType::Any
            } else {
                NamespaceType::Named
            };
            if namespace_type == NamespaceType::Named
                && !self
                    .declared_namespaces
                    .iter()
                    .any(|declared_namespace| declared_namespace.eq_ignore_ascii_case(&namespace))
            {
                self.index = saved_index;
                return None;
            }

            return Some(QualifiedNameSyntax {
                namespace_type,
                namespace,
                name,
            });
        }

        let name = selector_component_value_name(&first_token)?;
        if allow_wildcard_name == AllowWildcardName::No && name == "*" {
            self.index = saved_index;
            return None;
        }

        Some(QualifiedNameSyntax {
            namespace_type: NamespaceType::Default,
            namespace: String::new(),
            name,
        })
    }

    pub(super) fn parse_attribute_simple_selector(&self, block: SimpleBlock) -> Option<SimpleSelectorSyntax> {
        let mut parser = ComponentValueParser::with_declared_namespaces(block.value, self.declared_namespaces.clone());
        parser.discard_whitespace();
        let qualified_name = parser.parse_selector_qualified_name(AllowWildcardName::No)?;

        let mut attribute = AttributeSelectorSyntax {
            match_type: AttributeMatchType::HasAttribute,
            qualified_name,
            value: String::new(),
            case_type: AttributeCaseType::DefaultMatch,
        };

        parser.discard_whitespace();
        if parser.next_component_value().is_none() {
            return Some(SimpleSelectorSyntax::Attribute(attribute));
        }

        attribute.match_type = parser.parse_attribute_match_type()?;
        parser.discard_whitespace();
        attribute.value = match parser.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value } | TokenType::String { value },
                ..
            }) => value,
            _ => return None,
        };

        parser.discard_whitespace();
        if let Some(component_value) = parser.consume_the_next_component_value() {
            let ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) = component_value
            else {
                return None;
            };
            if value.eq_ignore_ascii_case("i") {
                attribute.case_type = AttributeCaseType::CaseInsensitiveMatch;
            } else if value.eq_ignore_ascii_case("s") {
                attribute.case_type = AttributeCaseType::CaseSensitiveMatch;
            } else {
                return None;
            }
        }

        parser.discard_whitespace();
        if parser.next_component_value().is_some() {
            return None;
        }

        Some(SimpleSelectorSyntax::Attribute(attribute))
    }

    pub(super) fn parse_attribute_match_type(&mut self) -> Option<AttributeMatchType> {
        let first_delim = self.consume_the_next_component_value()?;
        if component_value_is_delim(Some(&first_delim), '=') {
            return Some(AttributeMatchType::ExactValueMatch);
        }

        let match_type = if component_value_is_delim(Some(&first_delim), '~') {
            AttributeMatchType::ContainsWord
        } else if component_value_is_delim(Some(&first_delim), '*') {
            AttributeMatchType::ContainsString
        } else if component_value_is_delim(Some(&first_delim), '|') {
            AttributeMatchType::StartsWithSegment
        } else if component_value_is_delim(Some(&first_delim), '^') {
            AttributeMatchType::StartsWithString
        } else if component_value_is_delim(Some(&first_delim), '$') {
            AttributeMatchType::EndsWithString
        } else {
            return None;
        };

        if !component_value_is_delim(self.next_component_value(), '=') {
            return None;
        }
        self.index += 1;
        Some(match_type)
    }

    pub(super) fn parse_pseudo_class_simple_selector(&mut self) -> Option<SimpleSelectorSyntax> {
        if selector_component_value_ends_selector(self.next_component_value()) {
            return None;
        }
        if !matches!(
            self.consume_the_next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        ) {
            return None;
        }
        if selector_component_value_ends_selector(self.next_component_value()) {
            return None;
        }

        match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) => {
                let pseudo_class_id = pseudo_class_id_from_string(&value)?;
                if !pseudo_class_metadata(pseudo_class_id).is_valid_as_identifier {
                    return None;
                }
                Some(SimpleSelectorSyntax::PseudoClass(PseudoClassSelectorSyntax {
                    pseudo_class_id,
                    an_plus_b_pattern: None,
                    is_forgiving: false,
                    argument_selector_list: Vec::new(),
                    languages: Vec::new(),
                    ident: None,
                    levels: Vec::new(),
                }))
            }
            ComponentValue::Function(function) => self.parse_pseudo_class_function(function),
            _ => None,
        }
    }

    pub(super) fn parse_pseudo_class_function(&mut self, function: Function) -> Option<SimpleSelectorSyntax> {
        let pseudo_class_id = pseudo_class_id_from_string(&function.name)?;
        let metadata = pseudo_class_metadata(pseudo_class_id);
        if !metadata.is_valid_as_function || function.value.is_empty() {
            return None;
        }

        // "The :has() pseudo-class cannot be nested; :has() is not valid within :has()."
        // https://drafts.csswg.org/selectors/#relational
        if pseudo_class_id == PseudoClassId::Has && self.pseudo_class_context.contains(&PseudoClassId::Has) {
            return None;
        }

        self.pseudo_class_context.push(pseudo_class_id);
        let selector = self.parse_pseudo_class_function_value(pseudo_class_id, metadata.parameter_type, function.value);
        self.pseudo_class_context.pop();
        selector
    }

    pub(super) fn parse_pseudo_class_function_value(
        &mut self,
        pseudo_class_id: PseudoClassId,
        parameter_type: PseudoClassParameterType,
        function_values: Vec<ComponentValue>,
    ) -> Option<SimpleSelectorSyntax> {
        let mut pseudo_class = PseudoClassSelectorSyntax {
            pseudo_class_id,
            an_plus_b_pattern: None,
            is_forgiving: false,
            argument_selector_list: Vec::new(),
            languages: Vec::new(),
            ident: None,
            levels: Vec::new(),
        };

        match parameter_type {
            PseudoClassParameterType::AnPlusB => {
                let mut parser = ComponentValueParser::new(function_values);
                pseudo_class.an_plus_b_pattern = Some(parser.parse_a_n_plus_b_pattern()?);
                parser.discard_whitespace();
                if parser.next_component_value().is_some() {
                    return None;
                }
            }
            PseudoClassParameterType::AnPlusBOf => {
                let mut parser =
                    ComponentValueParser::with_declared_namespaces(function_values, self.declared_namespaces.clone());
                parser.pseudo_class_context = self.pseudo_class_context.clone();
                pseudo_class.an_plus_b_pattern = Some(parser.parse_a_n_plus_b_pattern()?);
                parser.discard_whitespace();
                if parser.next_component_value().is_some() {
                    if !parser.consume_ident_matching("of") {
                        return None;
                    }
                    parser.discard_whitespace();
                    pseudo_class.argument_selector_list =
                        parser.parse_a_selector_list(SelectorType::Standalone, SelectorParsingMode::Normal)?;
                    parser.discard_whitespace();
                    if parser.next_component_value().is_some() {
                        return None;
                    }
                }
            }
            PseudoClassParameterType::CompoundSelector => {
                let mut parser =
                    ComponentValueParser::with_declared_namespaces(function_values, self.declared_namespaces.clone());
                parser.pseudo_class_context = self.pseudo_class_context.clone();
                let mut compound_selector = parser.parse_compound_selector()?;
                parser.discard_whitespace();
                if parser.next_component_value().is_some() {
                    return None;
                }
                compound_selector.combinator = SelectorCombinator::None;
                pseudo_class.argument_selector_list.push(SelectorSyntax {
                    compound_selectors: vec![compound_selector],
                });
            }
            PseudoClassParameterType::ForgivingRelativeSelectorList
            | PseudoClassParameterType::ForgivingSelectorList
            | PseudoClassParameterType::RelativeSelectorList
            | PseudoClassParameterType::SelectorList => {
                let mut parser =
                    ComponentValueParser::with_declared_namespaces(function_values, self.declared_namespaces.clone());
                parser.pseudo_class_context = self.pseudo_class_context.clone();
                let selector_type = match parameter_type {
                    PseudoClassParameterType::ForgivingSelectorList | PseudoClassParameterType::SelectorList => {
                        SelectorType::Standalone
                    }
                    _ => SelectorType::Relative,
                };
                let parsing_mode = match parameter_type {
                    PseudoClassParameterType::ForgivingRelativeSelectorList
                    | PseudoClassParameterType::ForgivingSelectorList => SelectorParsingMode::Forgiving,
                    _ => SelectorParsingMode::Normal,
                };
                pseudo_class.is_forgiving = parsing_mode == SelectorParsingMode::Forgiving;
                pseudo_class.argument_selector_list = parser.parse_a_selector_list(selector_type, parsing_mode)?;
            }
            PseudoClassParameterType::Ident => {
                let mut parser = ComponentValueParser::new(function_values);
                parser.discard_whitespace();
                pseudo_class.ident = Some(parser.consume_an_ident()?);
                parser.discard_whitespace();
                if parser.next_component_value().is_some() {
                    return None;
                }
            }
            PseudoClassParameterType::LanguageRanges => {
                for group in split_component_values_by_comma(function_values) {
                    let mut parser = ComponentValueParser::new(group);
                    parser.discard_whitespace();
                    let ComponentValue::PreservedToken(Token {
                        token_type: TokenType::Ident { value } | TokenType::String { value },
                        ..
                    }) = parser.consume_the_next_component_value()?
                    else {
                        return None;
                    };
                    parser.discard_whitespace();
                    if parser.next_component_value().is_some() {
                        return None;
                    }
                    pseudo_class.languages.push(value);
                }
            }
            PseudoClassParameterType::LevelList => {
                for group in split_component_values_by_comma(function_values) {
                    let mut parser = ComponentValueParser::new(group);
                    parser.discard_whitespace();
                    let level = parse_integer_component_value(parser.consume_the_next_component_value()?)?;
                    parser.discard_whitespace();
                    if parser.next_component_value().is_some() {
                        return None;
                    }
                    pseudo_class.levels.push(i64::from(level));
                }
            }
            PseudoClassParameterType::None => return None,
        }

        Some(SimpleSelectorSyntax::PseudoClass(pseudo_class))
    }

    pub(super) fn parse_pseudo_element_simple_selector(&mut self) -> Option<SimpleSelectorSyntax> {
        if selector_component_value_ends_selector(self.next_component_value()) {
            return None;
        }
        if !matches!(
            self.consume_the_next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        ) {
            return None;
        }

        let started_with_double_colon = matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        );
        if started_with_double_colon {
            self.index += 1;
        }

        let (pseudo_name, is_function, function_values) = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) => (value, false, Vec::new()),
            ComponentValue::Function(function) => (function.name, true, function.value),
            _ => return None,
        };

        let mut is_aliased_pseudo = false;
        let mut pseudo_element_id = pseudo_element_id_from_string(&pseudo_name);
        if pseudo_element_id.is_none() {
            pseudo_element_id = aliased_pseudo_element_id_from_string(&pseudo_name);
            is_aliased_pseudo = pseudo_element_id.is_some();
        }

        if let Some(pseudo_element_id) = pseudo_element_id {
            if self.pseudo_class_context.contains(&PseudoClassId::Has)
                && !is_has_allowed_pseudo_element(pseudo_element_id)
            {
                return None;
            }

            if !started_with_double_colon {
                if is_legacy_single_colon_pseudo_element(pseudo_element_id) {
                    return Some(SimpleSelectorSyntax::PseudoElement(PseudoElementSelectorSyntax {
                        pseudo_element_id,
                        name: None,
                        value: PseudoElementSelectorValue::Empty,
                    }));
                }
                return None;
            }

            let metadata = pseudo_element_metadata(pseudo_element_id);
            let value = if is_function {
                if !metadata.is_valid_as_function {
                    return None;
                }
                self.parse_pseudo_element_function_value(metadata.parameter_type, function_values)?
            } else {
                if !metadata.is_valid_as_identifier {
                    return None;
                }
                PseudoElementSelectorValue::Empty
            };

            return Some(SimpleSelectorSyntax::PseudoElement(PseudoElementSelectorSyntax {
                pseudo_element_id,
                name: is_aliased_pseudo.then(|| pseudo_name.to_ascii_lowercase()),
                value,
            }));
        }

        // https://drafts.csswg.org/selectors-4/#compat
        // All other pseudo-elements whose names begin with the string “-webkit-” (matched ASCII case-insensitively)
        // and that are not functional notations must be treated as valid at parse time. (That is, ::-webkit-asdf is
        // valid at parse time, but ::-webkit-jkl() is not.) If they’re not otherwise recognized and supported, they
        // must be treated as matching nothing, and are unknown -webkit- pseudo-elements.
        if !is_function
            && pseudo_name
                .as_bytes()
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"-webkit-"))
        {
            if self.pseudo_class_context.contains(&PseudoClassId::Has) {
                return None;
            }
            return Some(SimpleSelectorSyntax::PseudoElement(PseudoElementSelectorSyntax {
                pseudo_element_id: PseudoElementId::UnknownWebKit,
                name: Some(pseudo_name.to_ascii_lowercase()),
                value: PseudoElementSelectorValue::Empty,
            }));
        }

        None
    }

    pub(super) fn parse_pseudo_element_function_value(
        &mut self,
        parameter_type: PseudoElementParameterType,
        function_values: Vec<ComponentValue>,
    ) -> Option<PseudoElementSelectorValue> {
        let mut parser =
            ComponentValueParser::with_declared_namespaces(function_values, self.declared_namespaces.clone());
        parser.pseudo_class_context = self.pseudo_class_context.clone();
        parser.discard_whitespace();

        match parameter_type {
            PseudoElementParameterType::None => {
                if parser.next_component_value().is_some() {
                    return None;
                }
                Some(PseudoElementSelectorValue::Empty)
            }
            PseudoElementParameterType::CompoundSelector => {
                let mut compound_selector = parser.parse_compound_selector()?;
                parser.discard_whitespace();
                if parser.next_component_value().is_some() {
                    return None;
                }
                compound_selector.combinator = SelectorCombinator::None;
                Some(PseudoElementSelectorValue::CompoundSelector(Box::new(SelectorSyntax {
                    compound_selectors: vec![compound_selector],
                })))
            }
            PseudoElementParameterType::IdentList => {
                let mut idents = Vec::new();
                while parser.next_component_value().is_some() {
                    idents.push(parser.consume_an_ident()?);
                    parser.discard_whitespace();
                }
                if idents.is_empty() {
                    return None;
                }
                Some(PseudoElementSelectorValue::IdentList(idents))
            }
            PseudoElementParameterType::PTNameSelector => {
                let value = if component_value_is_delim(parser.next_component_value(), '*') {
                    parser.index += 1;
                    PseudoElementSelectorValue::PTNameSelector {
                        is_universal: true,
                        value: String::new(),
                    }
                } else {
                    PseudoElementSelectorValue::PTNameSelector {
                        is_universal: false,
                        value: parser.consume_an_ident()?,
                    }
                };
                parser.discard_whitespace();
                if parser.next_component_value().is_some() {
                    return None;
                }
                Some(value)
            }
        }
    }

    pub(super) fn next_is_pseudo_element(&self) -> bool {
        if !matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        ) {
            return false;
        }
        if matches!(
            self.component_values.get(self.index + 1),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        ) {
            return true;
        }
        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.component_values.get(self.index + 1)
            && let Some(pseudo_element_id) = pseudo_element_id_from_string(value)
        {
            return is_legacy_single_colon_pseudo_element(pseudo_element_id);
        }

        false
    }

    pub(super) fn parse_a_n_plus_b_pattern(&mut self) -> Option<ANPlusBPattern> {
        let saved_index = self.index;
        self.discard_whitespace();

        if component_value_is_ident(self.next_component_value(), "odd") {
            self.index += 1;
            return Some(ANPlusBPattern {
                step_size: 2,
                offset: 1,
            });
        }
        if component_value_is_ident(self.next_component_value(), "even") {
            self.index += 1;
            return Some(ANPlusBPattern {
                step_size: 2,
                offset: 0,
            });
        }
        if let Some(integer) = self
            .next_component_value()
            .cloned()
            .and_then(parse_integer_component_value)
        {
            self.index += 1;
            return Some(ANPlusBPattern {
                step_size: 0,
                offset: integer,
            });
        }

        let result = self.parse_a_n_plus_b_pattern_with_leading_sign();
        if result.is_none() {
            self.index = saved_index;
        }
        result
    }

    pub(super) fn parse_a_n_plus_b_pattern_with_leading_sign(&mut self) -> Option<ANPlusBPattern> {
        let sign = if component_value_is_delim(self.next_component_value(), '+') {
            self.index += 1;
            1
        } else {
            1
        };
        self.parse_a_n_plus_b_pattern_after_optional_plus(sign)
    }

    pub(super) fn parse_a_n_plus_b_pattern_after_optional_plus(&mut self, sign: i32) -> Option<ANPlusBPattern> {
        let first = self.consume_the_next_component_value()?;

        if let Some((step_size, offset)) = parse_an_plus_b_dimension(&first) {
            if offset == i32::MIN {
                self.discard_whitespace();
                let offset = -parse_signless_integer_component_value(self.consume_the_next_component_value()?)?;
                return Some(ANPlusBPattern { step_size, offset });
            }
            if let Some(b) = self.parse_optional_an_plus_b_offset() {
                return Some(ANPlusBPattern { step_size, offset: b });
            }
            return Some(ANPlusBPattern { step_size, offset });
        }

        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = first
        else {
            return None;
        };

        if value.eq_ignore_ascii_case("n") || value.eq_ignore_ascii_case("-n") {
            let step_size = if value.starts_with('-') { -1 } else { sign };
            let offset = self.parse_optional_an_plus_b_offset().unwrap_or(0);
            return Some(ANPlusBPattern { step_size, offset });
        }

        if value.eq_ignore_ascii_case("n-") || value.eq_ignore_ascii_case("-n-") {
            self.discard_whitespace();
            let offset = -parse_signless_integer_component_value(self.consume_the_next_component_value()?)?;
            let step_size = if value.starts_with('-') { -1 } else { sign };
            return Some(ANPlusBPattern { step_size, offset });
        }

        if let Some(offset) = parse_ndashdigit_ident(&value, "n-") {
            return Some(ANPlusBPattern {
                step_size: sign,
                offset,
            });
        }

        if let Some(offset) = parse_ndashdigit_ident(&value, "-n-") {
            return Some(ANPlusBPattern { step_size: -1, offset });
        }

        None
    }

    pub(super) fn parse_optional_an_plus_b_offset(&mut self) -> Option<i32> {
        self.discard_whitespace();
        if let Some(integer) = self
            .next_component_value()
            .cloned()
            .and_then(parse_signed_integer_component_value)
        {
            self.index += 1;
            return Some(integer);
        }

        let saved_index = self.index;
        let sign = if component_value_is_delim(self.next_component_value(), '+') {
            self.index += 1;
            1
        } else if component_value_is_delim(self.next_component_value(), '-') {
            self.index += 1;
            -1
        } else {
            return None;
        };
        self.discard_whitespace();
        let Some(integer) = self
            .consume_the_next_component_value()
            .and_then(parse_signless_integer_component_value)
        else {
            self.index = saved_index;
            return None;
        };
        Some(sign * integer)
    }

    pub(super) fn parse_a_boolean_expression(&mut self, test_kind: BooleanExpressionTestKind) -> Option<()> {
        self.boolean_expression = self.parse_boolean_expression(test_kind);
        self.boolean_expression.as_ref()?;
        Some(())
    }

    pub(super) fn parse_media_condition(&mut self) -> Option<BooleanExpression> {
        // <media-condition> = <media-not> | <media-and> | <media-or> | <media-in-parens>
        self.parse_boolean_expression(BooleanExpressionTestKind::MediaFeature)
    }

    pub(super) fn parse_media_condition_without_or(&mut self) -> Option<BooleanExpression> {
        // <media-condition-without-or> = <media-not> | <media-and> | <media-in-parens>
        let expression = self.parse_media_condition()?;
        if matches!(expression, BooleanExpression::Or(_)) {
            return None;
        }
        Some(expression)
    }

    pub(super) fn parse_media_query_modifier(&mut self) -> MediaQueryModifier {
        // [ not | only ]?
        if component_value_is_ident(self.next_component_value(), "not") {
            self.index += 1;
            return MediaQueryModifier::Not;
        }
        if component_value_is_ident(self.next_component_value(), "only") {
            self.index += 1;
            return MediaQueryModifier::Only;
        }
        MediaQueryModifier::None
    }

    pub(super) fn parse_media_type(&mut self) -> Option<String> {
        // <media-type> = <ident>
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        else {
            return None;
        };

        // https://drafts.csswg.org/mediaqueries-3/#error-handling
        // "However, an exception is made for media types ‘layer’, ‘not’, ‘and’, ‘only’, and ‘or’.
        // Even though they do match the IDENT production, they must not be treated as unknown media
        // types, but rather trigger the malformed query clause."
        if ["layer", "not", "and", "only", "or"]
            .iter()
            .any(|reserved| value.eq_ignore_ascii_case(reserved))
        {
            return None;
        }

        let media_type = value.clone();
        self.index += 1;
        Some(media_type)
    }

    // https://drafts.csswg.org/css-values-5/#typedef-boolean-expr
    pub(super) fn parse_boolean_expression(
        &mut self,
        test_kind: BooleanExpressionTestKind,
    ) -> Option<BooleanExpression> {
        // <boolean-expr[ <test> ]> = not <boolean-expr-group> | <boolean-expr-group>
        //                            [ [ and <boolean-expr-group> ]*
        //                            | [ or <boolean-expr-group> ]* ]
        let saved_index = self.index;
        self.discard_whitespace();

        // `not <boolean-expr-group>`
        if component_value_is_ident(self.next_component_value(), "not") {
            self.index += 1;
            self.discard_whitespace();

            let child = self.parse_boolean_expression_group(test_kind)?;
            self.discard_whitespace();
            return Some(BooleanExpression::Not(Box::new(child)));
        }

        // `<boolean-expr-group>
        //   [ [ and <boolean-expr-group> ]*
        //   | [ or <boolean-expr-group> ]* ]`
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Combinator {
            And,
            Or,
        }

        let mut children = Vec::new();
        let mut combinator = None;

        while self.next_component_value().is_some() {
            if !children.is_empty() {
                let maybe_combinator = if component_value_is_ident(self.next_component_value(), "and") {
                    Some(Combinator::And)
                } else if component_value_is_ident(self.next_component_value(), "or") {
                    Some(Combinator::Or)
                } else {
                    None
                };

                let maybe_combinator = maybe_combinator?;
                if let Some(combinator) = combinator {
                    if maybe_combinator != combinator {
                        self.index = saved_index;
                        return None;
                    }
                } else {
                    combinator = Some(maybe_combinator);
                }
                self.index += 1;
            }

            self.discard_whitespace();
            children.push(self.parse_boolean_expression_group(test_kind)?);
            self.discard_whitespace();
        }

        if children.is_empty() {
            self.index = saved_index;
            return None;
        }

        if children.len() == 1 {
            return children.pop();
        }

        match combinator.expect("multiple children must have a combinator") {
            Combinator::And => Some(BooleanExpression::And(children)),
            Combinator::Or => Some(BooleanExpression::Or(children)),
        }
    }

    pub(super) fn parse_boolean_expression_group(
        &mut self,
        test_kind: BooleanExpressionTestKind,
    ) -> Option<BooleanExpression> {
        // <boolean-expr-group> = <test> | ( <boolean-expr[ <test> ]> ) | <general-enclosed>

        // `( <boolean-expr[ <test> ]> )`
        if let Some(ComponentValue::SimpleBlock(block)) = self.next_component_value().cloned()
            && is_paren_block(&block)
        {
            let saved_index = self.index;
            self.index += 1;
            let mut child_parser = ComponentValueParser::new(block.value);
            if let Some(expression) = child_parser.parse_boolean_expression(test_kind)
                && !child_parser.has_next_component_value()
            {
                return Some(BooleanExpression::Parens(Box::new(expression)));
            }
            self.index = saved_index;
        }

        // `<test>`
        if let Some(test) = self.parse_test(test_kind) {
            return Some(BooleanExpression::Test(test));
        }

        // `<general-enclosed>`
        if let Some(general_enclosed) = self.parse_general_enclosed() {
            return Some(BooleanExpression::GeneralEnclosed(general_enclosed));
        }

        None
    }

    pub(super) fn parse_test(&mut self, test_kind: BooleanExpressionTestKind) -> Option<BooleanExpressionTest> {
        match test_kind {
            BooleanExpressionTestKind::SupportsFeature => self.parse_supports_feature(),
            BooleanExpressionTestKind::MediaFeature => self.parse_media_feature(),
            BooleanExpressionTestKind::IfTest => self.parse_if_test(),
        }
    }

    // https://drafts.csswg.org/css-conditional-5/#typedef-supports-feature
    pub(super) fn parse_supports_feature(&mut self) -> Option<BooleanExpressionTest> {
        // <supports-feature> = <supports-selector-fn> | <supports-font-tech-fn>
        //                    | <supports-font-format-fn> | <supports-env-fn>
        //                    | <supports-decl>
        let (feature, component_value) = self.parse_supports_feature_syntax()?;
        Some(BooleanExpressionTest::SupportsFeature(feature, vec![component_value]))
    }

    // https://drafts.csswg.org/css-conditional-5/#typedef-supports-feature
    pub(super) fn parse_supports_feature_syntax(&mut self) -> Option<(SupportsFeature, ComponentValue)> {
        // <supports-feature> = <supports-selector-fn> | <supports-font-tech-fn>
        //                    | <supports-font-format-fn> | <supports-env-fn>
        //                    | <supports-decl>
        let component_value = self.next_component_value()?.clone();

        // `<supports-decl> = ( <declaration> )`
        if let ComponentValue::SimpleBlock(block) = &component_value
            && is_paren_block(block)
            && component_values_start_like_a_declaration(&block.value)
        {
            self.index += 1;
            return Some((SupportsFeature::Declaration, component_value));
        }

        let ComponentValue::Function(function) = &component_value else {
            return None;
        };

        // `<supports-selector-fn> = selector( <complex-selector> )`
        if function.name.eq_ignore_ascii_case("selector") {
            self.index += 1;
            return Some((SupportsFeature::Selector, component_value));
        }

        // `<supports-font-tech-fn> = font-tech( <font-tech> )`
        // `<supports-font-format-fn> = font-format( <font-format> )`
        // `<supports-env-fn> = env( <ident> )`
        if function.name.eq_ignore_ascii_case("font-tech")
            || function.name.eq_ignore_ascii_case("font-format")
            || function.name.eq_ignore_ascii_case("env")
        {
            let mut parser = ComponentValueParser::new(function.value.clone());
            parser.discard_whitespace();
            let ident = parser.consume_the_next_component_value();
            parser.discard_whitespace();
            if let Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) = ident
                && parser.next_component_value().is_none()
            {
                let feature = if function.name.eq_ignore_ascii_case("font-tech") {
                    SupportsFeature::FontTech(value)
                } else if function.name.eq_ignore_ascii_case("font-format") {
                    SupportsFeature::FontFormat(value)
                } else {
                    SupportsFeature::Env(value)
                };
                self.index += 1;
                return Some((feature, component_value));
            }
        }

        None
    }

    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-feature
    pub(super) fn parse_media_feature(&mut self) -> Option<BooleanExpressionTest> {
        // <media-feature> = [ <mf-plain> | <mf-boolean> | <mf-range> ]
        let component_value = self.next_component_value()?.clone();
        if let ComponentValue::SimpleBlock(block) = &component_value
            && is_paren_block(block)
            && let Some(kind) = component_values_parse_as_media_feature(&block.value)
        {
            self.index += 1;
            return Some(BooleanExpressionTest::MediaFeature(Box::new(MediaFeatureTest {
                component_value,
                kind,
            })));
        }

        None
    }

    // https://drafts.csswg.org/css-values-5/#typedef-if-condition
    pub(super) fn parse_if_test(&mut self) -> Option<BooleanExpressionTest> {
        // <if-test> =
        //   supports( [ <ident> : <declaration-value> ] | <supports-condition> ) |
        //   media( <media-feature> | <media-condition> ) |
        //   style( <style-query> )
        let component_value = self.next_component_value()?.clone();
        let ComponentValue::Function(function) = &component_value else {
            return None;
        };

        if function.name.eq_ignore_ascii_case("supports")
            || function.name.eq_ignore_ascii_case("media")
            || function.name.eq_ignore_ascii_case("style")
        {
            self.index += 1;
            return Some(BooleanExpressionTest::IfTest(vec![component_value]));
        }

        None
    }

    // https://drafts.csswg.org/css-page-3/#syntax-page-selector
    pub(super) fn parse_a_page_selector_list(&mut self) -> Option<Vec<PageSelector>> {
        // <page-selector-list> = <page-selector>#
        // <page-selector> = [ <ident-token>? <pseudo-page>* ]!
        // <pseudo-page> = : [ left | right | first | blank ]
        let mut selector_list = Vec::new();

        self.discard_whitespace();
        while self.has_next_component_value() {
            let name = if let Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) = self.next_component_value()
            {
                let name = value.clone();
                self.index += 1;
                Some(name)
            } else {
                None
            };

            let mut pseudo_classes = Vec::new();
            while matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Colon,
                    ..
                }))
            ) {
                self.index += 1;
                let Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) = self.next_component_value()
                else {
                    return None;
                };

                let pseudo_class = page_pseudo_class_from_string(value)?;
                self.index += 1;
                pseudo_classes.push(pseudo_class);
            }

            if name.is_none() && pseudo_classes.is_empty() {
                return None;
            }
            selector_list.push(PageSelector { name, pseudo_classes });

            self.discard_whitespace();
            if matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                }))
            ) {
                self.index += 1;
                self.discard_whitespace();
                if !self.has_next_component_value() {
                    return None;
                }
            } else if self.has_next_component_value() {
                return None;
            }
        }

        Some(selector_list)
    }

    // https://drafts.csswg.org/css-animations-1/#typedef-keyframe-selector
    pub(super) fn parse_a_keyframe_selector_list(&mut self) -> Option<Vec<KeyframeSelector>> {
        // <keyframe-selector> = from | to | <percentage [0,100]>
        //
        // The <<keyframe-selector>> for a <<keyframe-block>> consists of a comma-separated list of percentage values or
        // the keywords ''from'' or ''to''.
        let mut selector_list = Vec::new();

        self.discard_whitespace();
        loop {
            let selector = match self.next_component_value() {
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) if value.eq_ignore_ascii_case("from") => Some(0.0),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) if value.eq_ignore_ascii_case("to") => Some(100.0),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Percentage { number },
                    ..
                })) if (0.0..=100.0).contains(&number.value()) => Some(number.value()),
                _ => None,
            }?;
            self.index += 1;
            selector_list.push(selector);

            self.discard_whitespace();
            if matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                }))
            ) {
                self.index += 1;
                self.discard_whitespace();
                if !self.has_next_component_value() {
                    return None;
                }
            } else {
                break;
            }
        }

        if self.has_next_component_value() {
            return None;
        }

        Some(selector_list)
    }

    // https://drafts.csswg.org/css-animations-1/#typedef-keyframes-name
    pub(super) fn parse_a_keyframes_name(&mut self) -> Option<String> {
        // <keyframes-name> = <custom-ident> | <string>
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => value.clone(),
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, &["none"]) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-values-4/#custom-idents
    pub(super) fn parse_a_custom_ident(&mut self, blacklist: &[&str]) -> Option<String> {
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, blacklist) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-values-4/#typedef-dashed-ident
    pub(super) fn parse_a_dashed_ident(&mut self) -> Option<String> {
        // The <dashed-ident> production is a <custom-ident>, with all the case-sensitivity that implies, with the
        // additional restriction that it must start with two dashes (U+002D HYPHEN-MINUS).
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if value.starts_with("--") && is_valid_custom_ident(value, &[]) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-values-4/#url-value
    pub(super) fn parse_a_url_function(&mut self) -> Option<UrlFunction> {
        let url_function = self.parse_a_url_function_component()?;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(url_function)
    }

    pub(super) fn parse_a_url_function_component(&mut self) -> Option<UrlFunction> {
        // <url> = <url()> | <src()>
        // <url()> = url( <string> <url-modifier>* ) | <url-token>
        // <src()> = src( <string> <url-modifier>* )
        let url_function = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Url { value },
                ..
            }) => UrlFunction {
                function_type: CssUrlFunctionType::Url,
                url: value,
                request_url_modifiers: Vec::new(),
            },
            ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("url") => {
                parse_url_or_src_function_contents(CssUrlFunctionType::Url, &function.value)?
            }
            ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("src") => {
                parse_url_or_src_function_contents(CssUrlFunctionType::Src, &function.value)?
            }
            _ => return None,
        };

        Some(url_function)
    }

    // https://drafts.csswg.org/css-cascade-5/#at-import
    pub(super) fn parse_an_import_url(&mut self) -> Option<UrlFunction> {
        // @import [ <url> | <string> ]
        let url_function = self.parse_an_import_url_prefix()?;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(url_function)
    }

    pub(super) fn parse_an_import_url_prefix(&mut self) -> Option<UrlFunction> {
        // @import [ <url> | <string> ]
        self.discard_whitespace();

        let url_function = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => {
                let value = value.clone();
                self.index += 1;
                UrlFunction {
                    function_type: CssUrlFunctionType::Url,
                    url: value,
                    request_url_modifiers: Vec::new(),
                }
            }
            _ => self.parse_a_url_function_component()?,
        };

        Some(url_function)
    }

    // https://drafts.csswg.org/css-fonts/#font-face-src-parsing
    pub(super) fn parse_a_font_source(&mut self) -> Option<FontSource> {
        // <font-src> = <url> [ format(<font-format>)]? [ tech( <font-tech>#)]? | local(<family-name>)
        self.discard_whitespace();

        // local(<family-name>)
        if let Some(ComponentValue::Function(function)) = self.next_component_value()
            && function.name.eq_ignore_ascii_case("local")
        {
            let mut function_parser = ComponentValueParser::new(function.value.clone());
            let family_name = function_parser.parse_a_family_name()?;
            function_parser.discard_whitespace();
            if function_parser.has_next_component_value() {
                return None;
            }

            self.index += 1;
            self.discard_whitespace();
            if self.has_next_component_value() {
                return None;
            }
            return Some(FontSource::Local(family_name));
        }

        // <url> [ format(<font-format>)]? [ tech( <font-tech>#)]?
        let url_function = self.parse_a_url_function_component()?;
        let mut format = None;
        let mut tech = Vec::new();

        self.discard_whitespace();

        // [ format(<font-format>)]?
        if let Some(ComponentValue::Function(function)) = self.next_component_value()
            && function.name.eq_ignore_ascii_case("format")
        {
            let (parsed_format, parsed_tech) = parse_font_format_function(function)?;
            format = Some(parsed_format);
            tech.extend(parsed_tech);
            self.index += 1;
        }

        self.discard_whitespace();

        // [ tech( <font-tech>#)]?
        if let Some(ComponentValue::Function(function)) = self.next_component_value()
            && function.name.eq_ignore_ascii_case("tech")
        {
            tech.extend(parse_font_tech_function(function)?);
            self.index += 1;
        }

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(FontSource::Url {
            url_function,
            format,
            tech,
        })
    }

    // https://drafts.csswg.org/css-fonts/#propdef-font-language-override
    pub(super) fn parse_a_font_language_override(&mut self) -> Option<FontLanguageOverride> {
        // normal | <string>
        self.discard_whitespace();

        let font_language_override = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if value.eq_ignore_ascii_case("normal") => FontLanguageOverride::Normal,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => FontLanguageOverride::String(parse_font_language_override_string_value(&value)?),
            _ => return None,
        };

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(font_language_override)
    }

    // https://drafts.csswg.org/css-fonts/#propdef-font-feature-settings
    pub(super) fn parse_a_font_feature_settings(&mut self, filtered_input: &str) -> Option<OpenTypeSettings> {
        // normal | <feature-tag-value>#
        self.discard_whitespace();

        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && value.eq_ignore_ascii_case("normal")
        {
            self.index += 1;
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(OpenTypeSettings::Normal);
        }

        // <feature-tag-value>#
        let tag_values =
            parse_comma_separated_component_values(self.remaining_component_values().to_vec(), |component_values| {
                parse_feature_tag_value(component_values, filtered_input)
            })?;
        self.index = self.component_values.len();

        Some(OpenTypeSettings::TagValues(tag_values))
    }

    // https://drafts.csswg.org/css-fonts/#propdef-font-variation-settings
    pub(super) fn parse_a_font_variation_settings(&mut self, filtered_input: &str) -> Option<OpenTypeSettings> {
        // normal | [ <opentype-tag> <number> ]#
        self.discard_whitespace();

        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && value.eq_ignore_ascii_case("normal")
        {
            self.index += 1;
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(OpenTypeSettings::Normal);
        }

        // [ <opentype-tag> <number>]#
        let tag_values =
            parse_comma_separated_component_values(self.remaining_component_values().to_vec(), |component_values| {
                parse_variation_tag_value(component_values, filtered_input)
            })?;
        self.index = self.component_values.len();

        Some(OpenTypeSettings::TagValues(tag_values))
    }

    // https://drafts.csswg.org/css-fonts-4/#font-style-prop
    pub(super) fn parse_a_font_style(&mut self) -> Option<FontStyle> {
        // normal | italic | left | right | oblique <angle [-90deg,90deg]>?
        self.discard_whitespace();

        if self.consume_ident_matching("normal") {
            return Some(FontStyle::Normal);
        }
        if self.consume_ident_matching("italic") {
            return Some(FontStyle::Italic);
        }
        if self.consume_ident_matching("left") {
            return Some(FontStyle::Left);
        }
        if self.consume_ident_matching("right") {
            return Some(FontStyle::Right);
        }
        if self.consume_ident_matching("oblique") {
            self.discard_whitespace();
            if self
                .next_component_value()
                .is_some_and(component_value_parse_as_font_style_angle)
            {
                self.index += 1;
                return Some(FontStyle::Oblique { has_angle: true });
            }
            return Some(FontStyle::Oblique { has_angle: false });
        }

        None
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-alternates
    pub(super) fn parse_a_font_variant_alternates(&mut self) -> Option<Vec<FontVariantAlternatesValue>> {
        // [ stylistic(<feature-value-name>) || historical-forms || styleset(<feature-value-name>#) || character-variant(<feature-value-name>#) || swash(<feature-value-name>) || ornaments(<feature-value-name>) || annotation(<feature-value-name>) ]
        // <feature-value-name> = <ident>
        let mut stylistic = None;
        let mut historical_forms = None;
        let mut styleset = None;
        let mut character_variant = None;
        let mut swash = None;
        let mut ornaments = None;
        let mut annotation = None;

        loop {
            self.discard_whitespace();

            if self.consume_ident_matching("historical-forms") {
                if historical_forms.is_some() {
                    return None;
                }
                historical_forms = Some(FontVariantAlternatesValue {
                    kind: CssFontVariantAlternatesValueKind::HistoricalForms,
                    feature_value_names: Vec::new(),
                });
                continue;
            }

            let Some(ComponentValue::Function(function)) = self.next_component_value() else {
                break;
            };

            let kind = if function.name.eq_ignore_ascii_case("stylistic") {
                if stylistic.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Stylistic
            } else if function.name.eq_ignore_ascii_case("styleset") {
                if styleset.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Styleset
            } else if function.name.eq_ignore_ascii_case("character-variant") {
                if character_variant.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::CharacterVariant
            } else if function.name.eq_ignore_ascii_case("swash") {
                if swash.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Swash
            } else if function.name.eq_ignore_ascii_case("ornaments") {
                if ornaments.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Ornaments
            } else if function.name.eq_ignore_ascii_case("annotation") {
                if annotation.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Annotation
            } else {
                break;
            };

            let feature_value_names = parse_font_variant_alternates_feature_value_names(function.value.clone())?;
            if !matches!(
                kind,
                CssFontVariantAlternatesValueKind::Styleset | CssFontVariantAlternatesValueKind::CharacterVariant
            ) && feature_value_names.len() != 1
            {
                return None;
            }

            self.index += 1;
            let value = FontVariantAlternatesValue {
                kind,
                feature_value_names,
            };
            match kind {
                CssFontVariantAlternatesValueKind::Stylistic => stylistic = Some(value),
                CssFontVariantAlternatesValueKind::Styleset => styleset = Some(value),
                CssFontVariantAlternatesValueKind::CharacterVariant => character_variant = Some(value),
                CssFontVariantAlternatesValueKind::Swash => swash = Some(value),
                CssFontVariantAlternatesValueKind::Ornaments => ornaments = Some(value),
                CssFontVariantAlternatesValueKind::Annotation => annotation = Some(value),
                CssFontVariantAlternatesValueKind::HistoricalForms => unreachable!(),
            }
        }

        let values = [
            stylistic,
            historical_forms,
            styleset,
            character_variant,
            swash,
            ornaments,
            annotation,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant
    pub(super) fn parse_a_font_variant(&mut self) -> Option<FontVariant> {
        // normal |
        // none |
        // [
        //   [ <common-lig-values> || <discretionary-lig-values> || <historical-lig-values> || <contextual-alt-values> ] ||
        //   [ small-caps | all-small-caps | petite-caps | all-petite-caps | unicase | titling-caps ] ||
        //   [ stylistic(<feature-value-name>) || historical-forms || styleset(<feature-value-name>#) || character-variant(<feature-value-name>#) || swash(<feature-value-name>) || ornaments(<feature-value-name>) || annotation(<feature-value-name>) ] ||
        //   [ <numeric-figure-values> || <numeric-spacing-values> || <numeric-fraction-values> || ordinal || slashed-zero ] ||
        //   [ <east-asian-variant-values> || <east-asian-width-values> || ruby ] ||
        //   [ sub | super ] ||
        //   [ text | emoji | unicode ]
        // ]
        self.discard_whitespace();
        if self.consume_ident_matching("normal") {
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(FontVariant::default());
        }

        if self.consume_ident_matching("none") {
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(FontVariant {
                ligatures_none: true,
                ..FontVariant::default()
            });
        }

        let mut font_variant = FontVariant::default();
        while self.has_next_component_value() {
            let start = self.index;
            if let Some(ligatures) = self.parse_a_font_variant_ligatures() {
                if font_variant.ligatures.is_some() {
                    return None;
                }
                font_variant.ligatures = Some(ligatures);
                continue;
            }
            self.index = start;

            if let Some(alternates) = self.parse_a_font_variant_alternates() {
                if font_variant.alternates.is_some() {
                    return None;
                }
                font_variant.alternates = Some(alternates);
                continue;
            }
            self.index = start;

            if let Some(numeric) = self.parse_a_font_variant_numeric() {
                if font_variant.numeric.is_some() {
                    return None;
                }
                font_variant.numeric = Some(numeric);
                continue;
            }
            self.index = start;

            if let Some(east_asian) = self.parse_a_font_variant_east_asian() {
                if font_variant.east_asian.is_some() {
                    return None;
                }
                font_variant.east_asian = Some(east_asian);
                continue;
            }
            self.index = start;

            self.discard_whitespace();
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if value.eq_ignore_ascii_case("normal") || value.eq_ignore_ascii_case("none") {
                return None;
            }

            if matches_font_variant_caps_value(&value) {
                if font_variant.caps.is_some() {
                    return None;
                }
                font_variant.caps = Some(value);
                continue;
            }

            if matches_font_variant_emoji_value(&value) {
                if font_variant.emoji.is_some() {
                    return None;
                }
                font_variant.emoji = Some(value);
                continue;
            }

            if matches_font_variant_position_value(&value) {
                if font_variant.position.is_some() {
                    return None;
                }
                font_variant.position = Some(value);
                continue;
            }

            self.index = start;
            break;
        }

        font_variant.has_any_value().then_some(font_variant)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-east-asian
    pub(super) fn parse_a_font_variant_east_asian(&mut self) -> Option<Vec<FontVariantEastAsianValue>> {
        // [ <east-asian-variant-values> || <east-asian-width-values> || ruby ]
        // <east-asian-variant-values> = [ jis78 | jis83 | jis90 | jis04 | simplified | traditional ]
        // <east-asian-width-values>   = [ full-width | proportional-width ]
        let mut variant = false;
        let mut width = false;
        let mut ruby = false;
        let mut values = Vec::new();

        loop {
            self.discard_whitespace();
            let start = self.index;
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if value == "ruby" {
                if ruby {
                    return None;
                }
                ruby = true;
                values.push(FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Ruby,
                    value,
                });
                continue;
            }

            if matches_east_asian_width_value(&value) {
                if width {
                    return None;
                }
                width = true;
                values.push(FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Width,
                    value,
                });
                continue;
            }

            if matches_east_asian_variant_value(&value) {
                if variant {
                    return None;
                }
                variant = true;
                values.push(FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Variant,
                    value,
                });
                continue;
            }

            self.index = start;
            break;
        }

        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-numeric
    pub(super) fn parse_a_font_variant_numeric(&mut self) -> Option<Vec<FontVariantNumericValue>> {
        // [ <numeric-figure-values> || <numeric-spacing-values> || <numeric-fraction-values> || ordinal || slashed-zero]
        // <numeric-figure-values>       = [ lining-nums | oldstyle-nums ]
        // <numeric-spacing-values>      = [ proportional-nums | tabular-nums ]
        // <numeric-fraction-values>     = [ diagonal-fractions | stacked-fractions ]
        let mut figure = false;
        let mut spacing = false;
        let mut fraction = false;
        let mut ordinal = false;
        let mut slashed_zero = false;
        let mut values = Vec::new();

        loop {
            self.discard_whitespace();
            let start = self.index;
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if matches_numeric_figure_value(&value) {
                if figure {
                    return None;
                }
                figure = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Figure,
                    value,
                });
                continue;
            }

            if matches_numeric_spacing_value(&value) {
                if spacing {
                    return None;
                }
                spacing = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Spacing,
                    value,
                });
                continue;
            }

            if matches_numeric_fraction_value(&value) {
                if fraction {
                    return None;
                }
                fraction = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Fraction,
                    value,
                });
                continue;
            }

            if value == "ordinal" {
                if ordinal {
                    return None;
                }
                ordinal = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Ordinal,
                    value,
                });
                continue;
            }

            if value == "slashed-zero" {
                if slashed_zero {
                    return None;
                }
                slashed_zero = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::SlashedZero,
                    value,
                });
                continue;
            }

            self.index = start;
            break;
        }

        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-ligatures
    pub(super) fn parse_a_font_variant_ligatures(&mut self) -> Option<Vec<FontVariantLigaturesValue>> {
        // [ <common-lig-values> || <discretionary-lig-values> || <historical-lig-values> || <contextual-alt-values> ]
        // <common-lig-values>       = [ common-ligatures | no-common-ligatures ]
        // <discretionary-lig-values> = [ discretionary-ligatures | no-discretionary-ligatures ]
        // <historical-lig-values>   = [ historical-ligatures | no-historical-ligatures ]
        // <contextual-alt-values>   = [ contextual | no-contextual ]
        let mut common = false;
        let mut discretionary = false;
        let mut historical = false;
        let mut contextual = false;
        let mut values = Vec::new();

        loop {
            self.discard_whitespace();
            let start = self.index;
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if matches_common_lig_value(&value) {
                if common {
                    return None;
                }
                common = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Common,
                    value,
                });
                continue;
            }

            if matches_discretionary_lig_value(&value) {
                if discretionary {
                    return None;
                }
                discretionary = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Discretionary,
                    value,
                });
                continue;
            }

            if matches_historical_lig_value(&value) {
                if historical {
                    return None;
                }
                historical = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Historical,
                    value,
                });
                continue;
            }

            if matches_contextual_alt_value(&value) {
                if contextual {
                    return None;
                }
                contextual = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Contextual,
                    value,
                });
                continue;
            }

            self.index = start;
            break;
        }

        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#font-family-prop
    pub(super) fn parse_a_font_family_item(&mut self) -> Option<FontFamilyValue> {
        // [ <family-name> | <generic-family> ]#
        self.discard_whitespace();

        // <generic-family>
        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && matches_generic_font_family_keyword(value)
        {
            let generic_family = value.to_ascii_lowercase();
            self.index += 1;
            return Some(FontFamilyValue::Generic(generic_family));
        }

        // <family-name>
        self.parse_a_family_name().map(FontFamilyValue::FamilyName)
    }

    // https://drafts.csswg.org/css-variables-2/#typedef-custom-property-name
    pub(super) fn parse_a_custom_property_name(&mut self) -> Option<String> {
        // The <custom-property-name> production corresponds to this: it’s defined as any <dashed-ident>
        // (a valid identifier that starts with two dashes), except -- itself, which is reserved for future use by CSS.
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_a_custom_property_name_string(value) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-cascade-5/#typedef-layer-name
    pub(super) fn parse_a_layer_name(&mut self, allow_blank_layer_name: bool) -> Option<String> {
        // <layer-name> = <ident> [ '.' <ident> ]*
        self.discard_whitespace();
        if allow_blank_layer_name && !self.has_next_component_value() {
            return Some(String::new());
        }

        let mut name = self.consume_a_layer_name_part()?;
        while component_value_is_delim(self.next_component_value(), '.') {
            self.index += 1;
            let name_part = self.consume_a_layer_name_part()?;
            name.push('.');
            name.push_str(&name_part);
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-cascade-5/#at-import
    pub(super) fn parse_an_import_layer(&mut self) -> Option<String> {
        // [ layer | layer(<layer-name>) ]?
        let layer = self.parse_an_import_layer_prefix()?;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(layer)
    }

    pub(super) fn parse_an_import_layer_prefix(&mut self) -> Option<String> {
        // [ layer | layer(<layer-name>) ]?
        self.discard_whitespace();

        let layer = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if value.eq_ignore_ascii_case("layer") => String::new(),
            ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("layer") => {
                let mut function_parser = ComponentValueParser::new(function.value);
                let name = function_parser.parse_a_layer_name(false)?;
                function_parser.discard_whitespace();
                if function_parser.has_next_component_value() {
                    return None;
                }
                name
            }
            _ => return None,
        };

        Some(layer)
    }

    // https://drafts.csswg.org/css-cascade-5/#layering
    pub(super) fn parse_a_layer_name_list(&mut self) -> Option<Vec<String>> {
        // @layer <layer-name>#;
        let mut names = Vec::new();
        self.discard_whitespace();
        if !self.has_next_component_value() {
            return None;
        }

        loop {
            let name = self.parse_a_layer_name(false)?;
            names.push(name);
            self.discard_whitespace();

            if !self.has_next_component_value() {
                break;
            }

            if !component_value_is_comma(self.next_component_value()) {
                return None;
            }
            self.index += 1;
            self.discard_whitespace();
            if !self.has_next_component_value() {
                return None;
            }
        }

        Some(names)
    }

    pub(super) fn consume_a_layer_name_part(&mut self) -> Option<String> {
        // "The CSS-wide keywords are reserved for future use, and cause the rule to be invalid at parse time
        // if used as an <ident> in the <layer-name>."
        let name_part = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if !matches_css_wide_keyword(value) => value.clone(),
            _ => return None,
        };
        self.index += 1;
        Some(name_part)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style-name
    pub(super) fn parse_a_counter_style_name(&mut self) -> Option<String> {
        // <counter-style-name> is a <custom-ident> that is not an ASCII case-insensitive match for none.
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, &["none"]) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style
    pub(super) fn parse_a_counter_style(&mut self) -> Option<CounterStyle> {
        // <counter-style> = <counter-style-name> | <symbols()>
        let saved_index = self.index;
        if let Some(name) = self.parse_a_counter_style_name() {
            return Some(CounterStyle::Name(name));
        }
        self.index = saved_index;

        // <symbols()> = symbols( <symbols-type>? [ <string> | <image> ]+ )
        let ComponentValue::Function(Function { name, value, .. }) = self.consume_the_next_component_value()? else {
            return None;
        };
        if !name.eq_ignore_ascii_case("symbols") {
            return None;
        }

        let mut parser = ComponentValueParser::new(value);
        parser.discard_whitespace();

        // <symbols-type> = cyclic | numeric | alphabetic | symbolic | fixed
        // NB: <symbols-type> defaults to symbolic if not provided.
        let symbols_type = if parser.consume_ident_matching("cyclic") {
            CssCounterStyleSymbolsType::Cyclic
        } else if parser.consume_ident_matching("numeric") {
            CssCounterStyleSymbolsType::Numeric
        } else if parser.consume_ident_matching("alphabetic") {
            CssCounterStyleSymbolsType::Alphabetic
        } else if parser.consume_ident_matching("symbolic") {
            CssCounterStyleSymbolsType::Symbolic
        } else if parser.consume_ident_matching("fixed") {
            CssCounterStyleSymbolsType::Fixed
        } else {
            CssCounterStyleSymbolsType::Symbolic
        };

        // AD-HOC: In line with <symbol>, we don't support <image> here since
        // that part of the grammar is at-risk and unsupported by other engines.
        let mut symbols = Vec::new();
        loop {
            parser.discard_whitespace();
            let Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            })) = parser.next_component_value()
            else {
                break;
            };
            symbols.push(value.clone());
            parser.index += 1;
        }

        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        // https://drafts.csswg.org/css-counter-styles-3/#symbols-function
        // If the system is alphabetic or numeric, there must be at least two
        // <string>s or <image>s, or else the function is invalid.
        if symbols.is_empty()
            || (matches!(
                symbols_type,
                CssCounterStyleSymbolsType::Alphabetic | CssCounterStyleSymbolsType::Numeric
            ) && symbols.len() < 2)
        {
            return None;
        }

        Some(CounterStyle::SymbolsFunction { symbols_type, symbols })
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-system
    pub(super) fn parse_counter_style_system(&mut self) -> Option<CssCounterStyleSystemKind> {
        // cyclic | numeric | alphabetic | symbolic | additive | [fixed <integer>?] | [ extends <counter-style-name> ]
        self.discard_whitespace();

        if self.consume_ident_matching("cyclic") {
            return Some(CssCounterStyleSystemKind::Cyclic);
        }
        if self.consume_ident_matching("numeric") {
            return Some(CssCounterStyleSystemKind::Numeric);
        }
        if self.consume_ident_matching("alphabetic") {
            return Some(CssCounterStyleSystemKind::Alphabetic);
        }
        if self.consume_ident_matching("symbolic") {
            return Some(CssCounterStyleSystemKind::Symbolic);
        }
        if self.consume_ident_matching("additive") {
            return Some(CssCounterStyleSystemKind::Additive);
        }

        if self.consume_ident_matching("fixed") {
            self.discard_whitespace();
            if self.consume_integer_syntax() {
                return Some(CssCounterStyleSystemKind::FixedWithInteger);
            }
            return Some(CssCounterStyleSystemKind::Fixed);
        }

        if self.consume_ident_matching("extends") {
            self.discard_whitespace();
            if self.consume_counter_style_name_syntax() {
                return Some(CssCounterStyleSystemKind::Extends);
            }
        }

        None
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-negative
    pub(super) fn parse_counter_style_negative(&mut self) -> Option<CssCounterStyleNegativeSymbolCount> {
        // <symbol> <symbol>?
        self.discard_whitespace();
        if !self.consume_symbol_syntax() {
            return None;
        }

        self.discard_whitespace();
        if !self.consume_symbol_syntax() {
            return Some(CssCounterStyleNegativeSymbolCount::One);
        }

        Some(CssCounterStyleNegativeSymbolCount::Two)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-symbols
    pub(super) fn parse_counter_style_symbols(&mut self) -> Option<usize> {
        // <symbol>+
        let mut count = 0;
        loop {
            self.discard_whitespace();
            if !self.consume_symbol_syntax() {
                break;
            }
            count += 1;
        }

        if count == 0 {
            return None;
        }

        Some(count)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
    pub(super) fn parse_counter_style_symbol(&mut self) -> Option<()> {
        // <symbol> = <string> | <image> | <custom-ident>
        self.discard_whitespace();
        self.consume_symbol_syntax().then_some(())
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-range
    pub(super) fn parse_counter_style_range(&mut self) -> Option<(CssCounterStyleRangeKind, usize)> {
        // [ [ <integer> | infinite ]{2} ]# | auto
        self.discard_whitespace();
        if self.consume_ident_matching("auto") {
            return Some((CssCounterStyleRangeKind::Auto, 0));
        }

        let mut count = 0;
        loop {
            self.discard_whitespace();
            if !self.consume_counter_style_range_bound_syntax() {
                break;
            }

            self.discard_whitespace();
            if !self.consume_counter_style_range_bound_syntax() {
                return None;
            }

            count += 1;
            self.discard_whitespace();
            if !self.consume_comma() {
                break;
            }
            self.discard_whitespace();
            if !self.has_next_component_value() {
                return None;
            }
        }

        if count == 0 {
            return None;
        }

        Some((CssCounterStyleRangeKind::List, count))
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-additive-symbols
    pub(super) fn parse_counter_style_additive_symbols(&mut self) -> Option<usize> {
        // <additive-symbols> = <additive-tuple>#
        let mut count = 0;
        loop {
            self.discard_whitespace();
            self.parse_a_nonnegative_integer_symbol_pair()?;

            count += 1;
            self.discard_whitespace();
            if !self.consume_comma() {
                break;
            }
            self.discard_whitespace();
            if !self.has_next_component_value() {
                return None;
            }
        }

        if count == 0 {
            return None;
        }

        Some(count)
    }

    // https://drafts.csswg.org/css-page-3/#marks
    pub(super) fn parse_crop_or_cross(&mut self) -> Option<CssCropOrCrossKind> {
        // crop || cross
        self.discard_whitespace();

        let first_is_crop = if self.consume_ident_matching("crop") {
            true
        } else if self.consume_ident_matching("cross") {
            false
        } else {
            return None;
        };

        self.discard_whitespace();
        let has_both = if first_is_crop {
            self.consume_ident_matching("cross")
        } else {
            self.consume_ident_matching("crop")
        };

        if has_both {
            return Some(CssCropOrCrossKind::CropAndCross);
        }

        Some(if first_is_crop {
            CssCropOrCrossKind::Crop
        } else {
            CssCropOrCrossKind::Cross
        })
    }

    // https://drafts.csswg.org/css-fonts-4/#font-prop-desc
    pub(super) fn parse_font_weight_absolute_pair(&mut self) -> Option<usize> {
        // <font-weight-absolute>{1,2}
        let mut count = 0;
        for _ in 0..2 {
            self.discard_whitespace();
            if !self.consume_font_weight_absolute_syntax() {
                break;
            }
            count += 1;
        }

        if count == 0 {
            return None;
        }

        Some(count)
    }

    // https://drafts.csswg.org/css-page-3/#page-size-prop
    pub(super) fn parse_page_size_descriptor(&mut self) -> Option<()> {
        // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
        self.discard_whitespace();
        if self.consume_ident_matching("auto") {
            return Some(());
        }

        let saved_index = self.index;
        let mut length_count = 0;
        for _ in 0..2 {
            self.discard_whitespace();
            if !self.consume_nonnegative_length_descriptor_syntax() {
                break;
            }
            length_count += 1;
        }
        if length_count > 0 {
            return Some(());
        }
        self.index = saved_index;

        let mut page_size = false;
        let mut orientation = false;

        for _ in 0..2 {
            self.discard_whitespace();
            let Some(ident) = self.consume_an_ident() else {
                break;
            };

            if is_page_size_keyword(&ident) {
                if page_size {
                    return None;
                }
                page_size = true;
            } else if ident.eq_ignore_ascii_case("portrait") || ident.eq_ignore_ascii_case("landscape") {
                if orientation {
                    return None;
                }
                orientation = true;
            } else {
                return None;
            }
        }

        (page_size || orientation).then_some(())
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-additive-tuple
    pub(super) fn parse_a_nonnegative_integer_symbol_pair(&mut self) -> Option<CssNonnegativeIntegerSymbolPairOrder> {
        // <additive-tuple> = [ <integer [0,∞]> && <symbol> ]
        let saved_index = self.index;
        if self.consume_nonnegative_integer_syntax() {
            self.discard_whitespace();
            if self.consume_symbol_syntax() {
                return Some(CssNonnegativeIntegerSymbolPairOrder::IntegerFirst);
            }
        }
        self.index = saved_index;

        if self.consume_symbol_syntax() {
            self.discard_whitespace();
            if self.consume_nonnegative_integer_syntax() {
                return Some(CssNonnegativeIntegerSymbolPairOrder::SymbolFirst);
            }
        }
        self.index = saved_index;
        None
    }

    pub(super) fn consume_nonnegative_integer_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_nonnegative_integer = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }) => number_is_integer(*number) && number.value() >= 0.0,
            ComponentValue::Function(function) => parse_rust_owned_calculation_function(function).is_some(),
            _ => false,
        };

        if is_nonnegative_integer {
            self.index += 1;
        }
        is_nonnegative_integer
    }

    pub(super) fn consume_integer_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_integer = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }) => number_is_integer(*number),
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing math functions still happens in C++.
            ComponentValue::Function(_) => true,
            _ => false,
        };

        if is_integer {
            self.index += 1;
        }
        is_integer
    }

    pub(super) fn consume_counter_style_name_syntax(&mut self) -> bool {
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        else {
            return false;
        };

        // <counter-style-name> is a <custom-ident> that is not an ASCII
        // case-insensitive match for none.
        if !is_valid_custom_ident(value, &["none"]) {
            return false;
        }

        self.index += 1;
        true
    }

    pub(super) fn consume_counter_style_range_bound_syntax(&mut self) -> bool {
        if self.consume_ident_matching("infinite") {
            return true;
        }

        self.consume_integer_syntax()
    }

    pub(super) fn consume_font_weight_absolute_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_font_weight_absolute = component_values_parse_as_value_type(
            ValueTypeId::FontWeightAbsolute,
            std::slice::from_ref(component_value),
        ) != CssValueTypeSyntaxKind::Invalid;

        if is_font_weight_absolute {
            self.index += 1;
        }
        is_font_weight_absolute
    }

    pub(super) fn consume_nonnegative_length_descriptor_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_nonnegative_length = component_value_parse_as_nonnegative_length_descriptor(component_value);
        if is_nonnegative_length {
            self.index += 1;
        }
        is_nonnegative_length
    }

    pub(super) fn consume_symbol_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        // <symbol> = <string> | <image> | <custom-ident>
        let is_symbol = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { .. },
                ..
            }) => true,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) => is_valid_custom_ident(value, &[]),
            // AD-HOC: In line with the generated <symbol> parser, we don't
            // support <image> here since that part of the grammar is at-risk
            // and unsupported by other engines.
            _ => false,
        };

        if is_symbol {
            self.index += 1;
        }
        is_symbol
    }

    pub(super) fn consume_comma(&mut self) -> bool {
        if !matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            }))
        ) {
            return false;
        }

        self.index += 1;
        true
    }

    // https://drafts.csswg.org/css-namespaces/#syntax
    pub(super) fn parse_a_namespace_rule_prelude(&mut self) -> Option<(Option<String>, String)> {
        // @namespace <namespace-prefix>? [ <string> | <url> ] ;
        // <namespace-prefix> = <ident>
        self.discard_whitespace();

        let prefix = match self.next_component_value() {
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) => {
                let prefix = value.clone();
                self.index += 1;
                self.discard_whitespace();
                Some(prefix)
            }
            _ => None,
        };

        let namespace_uri = self.consume_namespace_uri()?;
        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some((prefix, namespace_uri))
    }

    pub(super) fn consume_namespace_uri(&mut self) -> Option<String> {
        // "A URI string parsed from the URI syntax must be treated as a literal string: as with the STRING syntax, no
        // URI-specific normalization is applied."
        // https://drafts.csswg.org/css-namespaces/#syntax
        let namespace_uri = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value } | TokenType::Url { value },
                ..
            }) => value.clone(),
            ComponentValue::Function(function)
                if function.name.eq_ignore_ascii_case("url") || function.name.eq_ignore_ascii_case("src") =>
            {
                let mut function_parser = ComponentValueParser::new(function.value.clone());
                function_parser.discard_whitespace();
                let namespace_uri = match function_parser.next_component_value()? {
                    ComponentValue::PreservedToken(Token {
                        token_type: TokenType::String { value },
                        ..
                    }) => value.clone(),
                    _ => return None,
                };
                function_parser.index += 1;
                function_parser.discard_whitespace();
                if function_parser.has_next_component_value() {
                    return None;
                }
                namespace_uri
            }
            _ => return None,
        };
        self.index += 1;
        Some(namespace_uri)
    }

    // https://drafts.csswg.org/css-fonts-4/#font-feature-values-syntax
    pub(super) fn parse_font_feature_values_feature_value(&mut self) -> Option<Vec<u32>> {
        // <feature-value-declaration> = <custom-ident> : <integer [0,∞]>+;
        self.discard_whitespace();

        let mut values = Vec::new();
        while let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        })) = self.next_component_value()
        {
            if !number_is_integer(*number) || number.value() < 0.0 || number.value() > u32::MAX as f64 {
                return None;
            }

            values.push(number.value() as u32);
            self.index += 1;
            self.discard_whitespace();
        }

        if values.is_empty() || self.has_next_component_value() {
            return None;
        }

        Some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#font-family-name-syntax
    pub(super) fn parse_a_family_name(&mut self) -> Option<FamilyName> {
        // <font-family-name> = <string> | <custom-ident>+
        self.discard_whitespace();

        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        })) = self.next_component_value()
        {
            let family_name = value.clone();
            self.index += 1;
            return Some(FamilyName {
                name: family_name,
                is_string: true,
            });
        }

        let mut parts = Vec::new();
        while let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        {
            parts.push(value.clone());
            self.index += 1;
            self.discard_whitespace();
        }

        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            // Any identifier which could be misinterpreted as a pre-defined keyword in the font-family value
            // definition, or the CSS-wide keywords, is not allowed.
            // AD-HOC: We allow all <ident>'s rather than just <custom-ident>, although we check that the whole value
            //         isn't a CSS-wide keyword, see https://github.com/w3c/csswg-drafts/issues/13692
            let part = &parts[0];
            if !is_valid_custom_ident(part, &[]) || matches_generic_font_family_keyword(part) {
                return None;
            }
        }

        Some(FamilyName {
            name: parts.join(" "),
            is_string: false,
        })
    }

    pub(super) fn parse_container_rule_prelude_item(
        &mut self,
        filtered_input: &str,
    ) -> Option<(Option<String>, Option<String>)> {
        // https://drafts.csswg.org/css-conditional-5/#container-rule
        // <container-condition> = [ <container-name>? <container-query>? ]!
        // https://drafts.csswg.org/css-conditional-5/#container-name
        // <container-name> = <custom-ident>
        self.discard_whitespace();

        let container_name = match self.next_component_value() {
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) if is_valid_custom_ident(value, &["none", "and", "not", "or"]) => {
                let container_name = value.clone();
                self.index += 1;
                self.discard_whitespace();
                Some(container_name)
            }
            _ => None,
        };

        let container_query = if self.has_next_component_value() {
            Some(serialize_component_values_for_reparsing(
                &self.component_values[self.index..],
                filtered_input,
            )?)
        } else {
            None
        };

        if container_name.is_none() && container_query.is_none() {
            return None;
        }

        Some((container_name, container_query))
    }

    pub(super) fn parse_container_rule_prelude_item_condition(
        &mut self,
    ) -> Option<(Option<String>, Option<BooleanExpression>)> {
        // https://drafts.csswg.org/css-conditional-5/#container-rule
        // <container-condition> = [ <container-name>? <container-query>? ]!
        // https://drafts.csswg.org/css-conditional-5/#container-name
        // <container-name> = <custom-ident>
        self.discard_whitespace();

        let container_name = match self.next_component_value() {
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) if is_valid_custom_ident(value, &["none", "and", "not", "or"]) => {
                let container_name = value.clone();
                self.index += 1;
                self.discard_whitespace();
                Some(container_name)
            }
            _ => None,
        };

        let container_query = if self.has_next_component_value() {
            let mut query_parser = ComponentValueParser::new(self.component_values[self.index..].to_vec());
            let query = query_parser.parse_media_condition()?;
            if query_parser.has_next_component_value() {
                return None;
            }
            self.index = self.component_values.len();
            Some(query)
        } else {
            None
        };

        if container_name.is_none() && container_query.is_none() {
            return None;
        }

        Some((container_name, container_query))
    }

    // https://drafts.csswg.org/mediaqueries-5/#typedef-general-enclosed
    pub(super) fn parse_general_enclosed(&mut self) -> Option<ComponentValue> {
        // <general-enclosed> = [ <function-token> <any-value>? ) ] | [ ( <any-value>? ) ]
        //
        // https://drafts.csswg.org/css-syntax-3/#typedef-any-value
        // "The <any-value> production is identical to <declaration-value>",
        // and <declaration-value> does not contain "<<bad-string-token>>,
        // <<bad-url-token>>, unmatched <<)-token>>, <<]-token>>, or
        // <<}-token>>".
        let component_value = self.next_component_value()?.clone();
        let contains_only_any_value = match &component_value {
            ComponentValue::Function(function) => contains_only_any_value(&function.value),
            ComponentValue::SimpleBlock(block) if is_paren_block(block) => contains_only_any_value(&block.value),
            _ => false,
        };

        if contains_only_any_value {
            self.index += 1;
            return Some(component_value);
        }

        None
    }
}

fn component_value_parse_as_font_style_angle(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Angle)) => {
            let angle_in_degrees = match unit.to_ascii_lowercase().as_str() {
                "deg" => number.value(),
                "grad" => number.value() * 0.9,
                "rad" => number.value() * 57.29577951308232,
                "turn" => number.value() * 360.0,
                _ => return false,
            };
            (-90.0..=90.0).contains(&angle_in_degrees)
        }
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(_) => true,
        _ => false,
    }
}
