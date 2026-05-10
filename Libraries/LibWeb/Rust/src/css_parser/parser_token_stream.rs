/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            index: 0,
            rule_context: Vec::new(),
        }
    }

    // https://www.w3.org/TR/css-syntax-3/#urange-syntax
    pub(super) fn parse_a_unicode_range(&mut self, filtered_input: &str) -> Option<CssUnicodeRange> {
        // <urange> =
        //  u '+' <ident-token> '?'* |
        //  u <dimension-token> '?'* |
        //  u <number-token> '?'* |
        //  u <number-token> <dimension-token> |
        //  u <number-token> <number-token> |
        //  u '+' '?'+
        // (All with no whitespace in between tokens.)
        self.discard_whitespace();
        let unicode_range = self.consume_a_unicode_range(filtered_input)?;
        self.discard_whitespace();
        if !matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return None;
        }
        Some(unicode_range)
    }

    // https://www.w3.org/TR/css-syntax-3/#urange-syntax
    pub(super) fn parse_a_unicode_range_list(&mut self, filtered_input: &str) -> Option<Vec<CssUnicodeRange>> {
        let mut unicode_ranges = Vec::new();

        loop {
            self.discard_whitespace();
            unicode_ranges.push(self.consume_a_unicode_range(filtered_input)?);
            self.discard_whitespace();

            match self.next_input_token().token_type {
                TokenType::Comma => self.discard_a_token(),
                TokenType::EndOfFile => break,
                _ => return None,
            }
        }

        Some(unicode_ranges)
    }

    // https://www.w3.org/TR/css-syntax-3/#urange-syntax
    pub(super) fn consume_a_unicode_range(&mut self, filtered_input: &str) -> Option<CssUnicodeRange> {
        let u = self.consume_the_next_input_token();
        if !matches!(u.token_type, TokenType::Ident { ref value } if value.eq_ignore_ascii_case("u")) {
            return None;
        }

        let second_token = self.consume_the_next_input_token();

        //  u '+' <ident-token> '?'* |
        //  u '+' '?'+
        if token_is_delim(&second_token, '+') {
            let mut text = token_original_source(&second_token, filtered_input)?.to_string();
            let third_token = self.consume_the_next_input_token();
            if matches!(third_token.token_type, TokenType::Ident { .. }) || token_is_delim(&third_token, '?') {
                text.push_str(token_original_source(&third_token, filtered_input)?);
                while token_is_delim(self.next_input_token(), '?') {
                    text.push_str(token_original_source(
                        &self.consume_the_next_input_token(),
                        filtered_input,
                    )?);
                }
                if self.next_input_token().is_unicode_range_ending_token() {
                    return parse_unicode_range_text(&text);
                }
            }
        }

        //  u <dimension-token> '?'*
        if matches!(second_token.token_type, TokenType::Dimension { .. }) {
            let mut text = token_original_source(&second_token, filtered_input)?.to_string();
            while token_is_delim(self.next_input_token(), '?') {
                text.push_str(token_original_source(
                    &self.consume_the_next_input_token(),
                    filtered_input,
                )?);
            }
            if self.next_input_token().is_unicode_range_ending_token() {
                return parse_unicode_range_text(&text);
            }
        }

        //  u <number-token> '?'* |
        //  u <number-token> <dimension-token> |
        //  u <number-token> <number-token>
        if matches!(second_token.token_type, TokenType::Number { .. }) {
            let mut text = token_original_source(&second_token, filtered_input)?.to_string();

            if self.next_input_token().is_unicode_range_ending_token() {
                return parse_unicode_range_text(&text);
            }

            let third_token = self.consume_the_next_input_token();
            if token_is_delim(&third_token, '?') {
                text.push_str(token_original_source(&third_token, filtered_input)?);
                while token_is_delim(self.next_input_token(), '?') {
                    text.push_str(token_original_source(
                        &self.consume_the_next_input_token(),
                        filtered_input,
                    )?);
                }
                if self.next_input_token().is_unicode_range_ending_token() {
                    return parse_unicode_range_text(&text);
                }
            } else if matches!(
                third_token.token_type,
                TokenType::Dimension { .. } | TokenType::Number { .. }
            ) {
                text.push_str(token_original_source(&third_token, filtered_input)?);
                if self.next_input_token().is_unicode_range_ending_token() {
                    return parse_unicode_range_text(&text);
                }
            }
        }

        None
    }

    // https://drafts.csswg.org/css-syntax/#parse-a-stylesheets-contents
    pub(crate) fn parse_a_stylesheets_contents(&mut self) -> Vec<Rule> {
        // To parse a stylesheet’s contents from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a stylesheet’s contents from input, and return the result.
        self.consume_a_stylesheets_contents()
    }

    // https://drafts.csswg.org/css-syntax/#parse-block-contents
    pub(crate) fn parse_a_blocks_contents(&mut self) -> Vec<RuleOrListOfDeclarations> {
        // To parse a block’s contents from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a block’s contents from input, and return the result.
        self.consume_a_blocks_contents()
    }

    // https://drafts.csswg.org/css-syntax/#parse-rule
    pub(crate) fn parse_a_rule(&mut self) -> Option<Rule> {
        // To parse a rule from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. If the next token from input is an <EOF-token>, return a syntax error.
        let rule = if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return None;
        }
        // Otherwise, if the next token from input is an <at-keyword-token>, consume an at-rule from input,
        // and let rule be the return value.
        else if matches!(self.next_input_token().token_type, TokenType::AtKeyword { .. }) {
            Rule::AtRule(self.consume_an_at_rule(Nested::No)?)
        }
        // Otherwise, consume a qualified rule from input and let rule be the return value.
        // If nothing or an invalid rule error was returned, return a syntax error.
        else {
            Rule::QualifiedRule(self.consume_a_qualified_rule(None, Nested::No)?)
        };

        // 4. Discard whitespace from input.
        self.discard_whitespace();

        // 5. If the next token from input is an <EOF-token>, return rule. Otherwise, return a syntax error.
        if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return Some(rule);
        }
        None
    }

    // https://drafts.csswg.org/css-syntax/#parse-declaration
    pub(crate) fn parse_a_declaration(&mut self) -> Option<Declaration> {
        // To parse a declaration from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        self.rule_context.push(RuleContext::Style);
        let declaration = self.parse_a_declaration_with_current_context();
        self.rule_context.pop();
        declaration
    }

    pub(super) fn parse_a_declaration_with_current_context(&mut self) -> Option<Declaration> {
        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. Consume a declaration from input. If anything was returned, return it.
        // Otherwise, return a syntax error.
        self.consume_a_declaration(Nested::No)
    }

    // https://drafts.csswg.org/css-syntax/#parse-list-of-component-values
    pub(crate) fn parse_a_list_of_component_values(&mut self) -> Vec<ComponentValue> {
        // To parse a list of component values from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a list of component values from input, and return the result.
        self.consume_a_list_of_component_values(None, Nested::No)
    }

    // https://drafts.csswg.org/css-syntax/#parse-comma-separated-list-of-component-values
    pub(crate) fn parse_a_comma_separated_list_of_component_values(&mut self) -> Vec<Vec<ComponentValue>> {
        // To parse a comma-separated list of component values from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Let groups be an empty list.
        let mut groups = Vec::new();

        // 3. While input is not empty:
        let mut just_consumed_comma = false;
        while !matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            // 1. Consume a list of component values from input, with <comma-token> as the stop token,
            // and append the result to groups.
            groups.push(self.consume_a_list_of_component_values(Some(TokenType::Comma), Nested::No));

            // 2. Discard a token from input.
            just_consumed_comma = matches!(self.consume_the_next_input_token().token_type, TokenType::Comma);
        }

        // AD-HOC: Also append an empty group if there was a trailing comma.
        // Some related spec discussion: https://github.com/w3c/csswg-drafts/issues/11254
        if just_consumed_comma {
            groups.push(Vec::new());
        }

        // 4. Return groups.
        groups
    }

    pub(crate) fn parse_a_media_query_list(&mut self) -> Vec<MediaQuerySyntax> {
        // https://drafts.csswg.org/mediaqueries-5/#typedef-media-query-list
        // To parse a <media-query-list> production,
        // parse a comma-separated list of component values,
        // then parse each entry in the returned list as a <media-query>.
        // Its value is the list of <media-query>s so produced.
        let groups = self.parse_a_comma_separated_list_of_component_values();

        // AD-HOC: Ignore whitespace-only queries
        // to make `@media {..}` equivalent to `@media all {..}`.
        if groups.len() == 1 && strip_whitespace(&groups[0]).is_empty() {
            return Vec::new();
        }

        groups.into_iter().map(component_values_parse_as_media_query).collect()
    }

    // https://drafts.csswg.org/css-syntax/#parse-component-value
    pub(crate) fn parse_a_component_value(&mut self) -> Option<ComponentValue> {
        // To parse a component value from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. If input is empty, return a syntax error.
        if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return None;
        }

        // 4. Consume a component value from input and let value be the return value.
        let component_value = self.consume_a_component_value();

        // 5. Discard whitespace from input.
        self.discard_whitespace();

        // 6. If input is empty, return value. Otherwise, return a syntax error.
        if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return Some(component_value);
        }
        None
    }

    pub(super) fn next_input_token(&self) -> &Token {
        self.tokens
            .get(self.index)
            .or_else(|| self.tokens.last())
            .expect("CSS parser requires an EOF token")
    }

    pub(super) fn consume_the_next_input_token(&mut self) -> Token {
        let token = self.next_input_token().clone();
        self.index += 1;
        token
    }

    pub(super) fn discard_a_token(&mut self) {
        self.index += 1;
    }

    pub(super) fn discard_whitespace(&mut self) {
        while matches!(self.next_input_token().token_type, TokenType::Whitespace) {
            self.discard_a_token();
        }
    }

    pub(super) fn peek_token(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.index + offset)
            .or_else(|| self.tokens.last())
            .expect("CSS parser requires an EOF token")
    }

    // https://drafts.csswg.org/css-syntax/#consume-stylesheet-contents
    pub(super) fn consume_a_stylesheets_contents(&mut self) -> Vec<Rule> {
        // To consume a stylesheet’s contents from a token stream input:
        // Let rules be an initially empty list of rules.
        let mut rules = Vec::new();

        // Process input:
        loop {
            let token = self.next_input_token();

            // <whitespace-token>
            if matches!(token.token_type, TokenType::Whitespace) {
                // Discard a token from input.
                self.discard_a_token();
                continue;
            }

            // <EOF-token>
            if matches!(token.token_type, TokenType::EndOfFile) {
                // Return rules.
                return rules;
            }

            // <CDO-token>
            // <CDC-token>
            if matches!(token.token_type, TokenType::Cdo | TokenType::Cdc) {
                // Discard a token from input.
                self.discard_a_token();
                continue;
            }

            // <at-keyword-token>
            if matches!(token.token_type, TokenType::AtKeyword { .. }) {
                // Consume an at-rule from input. If anything is returned, append it to rules.
                if let Some(rule) = self.consume_an_at_rule(Nested::No) {
                    rules.push(Rule::AtRule(rule));
                }
                continue;
            }

            // anything else
            // Consume a qualified rule from input. If a rule is returned, append it to rules.
            if let Some(rule) = self.consume_a_qualified_rule(None, Nested::No) {
                rules.push(Rule::QualifiedRule(rule));
            }
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-at-rule
    pub(super) fn consume_an_at_rule(&mut self, nested: Nested) -> Option<AtRule> {
        // To consume an at-rule from a token stream input, given an optional bool nested (default false):
        // Assert: The next token is an <at-keyword-token>.
        assert!(matches!(
            self.next_input_token().token_type,
            TokenType::AtKeyword { .. }
        ));

        // Consume a token from input, and let rule be a new at-rule with its name set to the returned token’s value,
        // its prelude initially set to an empty list, and no declarations or child rules.
        let token = self.consume_the_next_input_token();
        let TokenType::AtKeyword { name } = token.token_type else {
            unreachable!("consume_an_at_rule requires an at-keyword token");
        };
        let mut rule = AtRule {
            name,
            prelude: Vec::new(),
            child_rules_and_lists_of_declarations: Vec::new(),
            is_block_rule: false,
        };

        // Process input:
        loop {
            let token = self.next_input_token();

            // <semicolon-token>
            // <EOF-token>
            if matches!(token.token_type, TokenType::Semicolon | TokenType::EndOfFile) {
                // Discard a token from input. If rule is valid in the current context, return it;
                // otherwise return nothing.
                self.discard_a_token();
                if self.is_at_rule_valid_in_the_current_context(&rule) {
                    return Some(rule);
                }
                return None;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) {
                // If nested is true:
                if nested == Nested::Yes {
                    // If rule is valid in the current context, return it.
                    if self.is_at_rule_valid_in_the_current_context(&rule) {
                        return Some(rule);
                    }
                    return None;
                }
                // Otherwise, consume a token and append the result to rule’s prelude.
                rule.prelude
                    .push(ComponentValue::PreservedToken(self.consume_the_next_input_token()));
                continue;
            }

            // <{-token>
            if matches!(token.token_type, TokenType::OpenCurly) {
                // Consume a block from input, and assign the result to rule’s child rules.
                self.rule_context.push(rule_context_type_for_at_rule(&rule.name));
                rule.child_rules_and_lists_of_declarations = self.consume_a_block();
                self.rule_context.pop();
                rule.is_block_rule = true;

                // If rule is valid in the current context, return it. Otherwise, return nothing.
                if self.is_at_rule_valid_in_the_current_context(&rule) {
                    return Some(rule);
                }
                return None;
            }

            // anything else
            // Consume a component value from input and append the returned value to rule’s prelude.
            rule.prelude.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-qualified-rule
    pub(super) fn consume_a_qualified_rule(
        &mut self,
        stop_token: Option<TokenType>,
        nested: Nested,
    ) -> Option<QualifiedRule> {
        // To consume a qualified rule, from a token stream input, given an optional token stop token
        // and an optional bool nested (default false):

        // Let rule be a new qualified rule with its prelude, declarations, and child rules all initially set to empty lists.
        let is_style_rule = self.rule_context.last().is_none_or(|context| {
            matches!(
                context,
                RuleContext::Style
                    | RuleContext::AtContainer
                    | RuleContext::AtLayer
                    | RuleContext::AtMedia
                    | RuleContext::AtSupports
            )
        });
        let selector_type = if nested == Nested::Yes
            && self
                .rule_context
                .iter()
                .any(|context| matches!(context, RuleContext::Style | RuleContext::AtFunction))
        {
            Nested::Yes
        } else {
            Nested::No
        };
        let mut rule = QualifiedRule {
            prelude: Vec::new(),
            declarations: Vec::new(),
            child_rules: Vec::new(),
            selector_type: is_style_rule.then_some(selector_type),
        };

        // NOTE: Qualified rules inside @keyframes are a keyframe rule.
        //       We'll assume all others are style rules.
        let type_of_qualified_rule = if self.rule_context.last() == Some(&RuleContext::AtKeyframes) {
            RuleContext::Keyframe
        } else {
            RuleContext::Style
        };

        // Process input:
        loop {
            let token = self.next_input_token();

            // <EOF-token>
            // stop token (if passed)
            if matches!(token.token_type, TokenType::EndOfFile)
                || stop_token
                    .as_ref()
                    .is_some_and(|stop_token| token.token_type == *stop_token)
            {
                // This is a parse error. Return nothing.
                return None;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) {
                // This is a parse error. If nested is true, return nothing.
                // Otherwise, consume a token and append the result to rule’s prelude.
                if nested == Nested::Yes {
                    return None;
                }
                rule.prelude
                    .push(ComponentValue::PreservedToken(self.consume_the_next_input_token()));
                continue;
            }

            // <{-token>
            if matches!(token.token_type, TokenType::OpenCurly) {
                // If the first two non-<whitespace-token> values of rule’s prelude are an <ident-token>
                // whose value starts with "--" followed by a <colon-token>, then:
                let mut prelude = rule
                    .prelude
                    .iter()
                    .filter(|value| !is_whitespace_component_value(value));
                let starts_like_custom_property_declaration = matches!(
                    (prelude.next(), prelude.next()),
                    (
                        Some(ComponentValue::PreservedToken(Token {
                            token_type: TokenType::Ident { value },
                            ..
                        })),
                        Some(ComponentValue::PreservedToken(Token {
                            token_type: TokenType::Colon,
                            ..
                        }))
                    ) if value.starts_with("--")
                );

                if starts_like_custom_property_declaration {
                    // If nested is true, consume the remnants of a bad declaration from input,
                    // with nested set to true, and return nothing.
                    if nested == Nested::Yes {
                        self.consume_the_remnants_of_a_bad_declaration(Nested::Yes);
                        return None;
                    }

                    // If nested is false, consume a block from input, and return nothing.
                    let _ = self.consume_a_block();
                    return None;
                }

                // Otherwise, consume a block from input, and let child rules be the result.
                self.rule_context.push(type_of_qualified_rule);
                rule.child_rules = self.consume_a_block();
                self.rule_context.pop();

                // If the first item of child rules is a list of declarations, remove it from child rules
                // and assign it to rule’s declarations.
                if matches!(
                    rule.child_rules.first(),
                    Some(RuleOrListOfDeclarations::ListOfDeclarations(_))
                ) && let RuleOrListOfDeclarations::ListOfDeclarations(declarations) = rule.child_rules.remove(0)
                {
                    rule.declarations = declarations;
                }

                // If rule is valid in the current context, return it; otherwise return an invalid rule error.
                if self.is_qualified_rule_valid_in_the_current_context() {
                    return Some(rule);
                }
                return None;
            }

            // anything else
            // Consume a component value from input and append the result to rule’s prelude.
            rule.prelude.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-block
    pub(super) fn consume_a_block(&mut self) -> Vec<RuleOrListOfDeclarations> {
        // To consume a block, from a token stream input:
        // Assert: The next token is a <{-token>.
        assert!(matches!(self.next_input_token().token_type, TokenType::OpenCurly));

        // Discard a token from input.
        self.discard_a_token();

        // Consume a block’s contents from input and let rules be the result.
        let rules = self.consume_a_blocks_contents();

        // Discard a token from input.
        self.discard_a_token();

        // Return rules.
        rules
    }

    // https://drafts.csswg.org/css-syntax/#consume-block-contents
    pub(super) fn consume_a_blocks_contents(&mut self) -> Vec<RuleOrListOfDeclarations> {
        // To consume a block’s contents from a token stream input:
        // Let rules be an empty list, containing either rules or lists of declarations.
        let mut rules = Vec::new();

        // Let decls be an empty list of declarations.
        let mut declarations = Vec::new();

        // Process input:
        loop {
            let token = self.next_input_token();

            // <whitespace-token>
            // <semicolon-token>
            if matches!(token.token_type, TokenType::Whitespace | TokenType::Semicolon) {
                // Discard a token from input.
                self.discard_a_token();
                continue;
            }

            // <EOF-token>
            // <}-token>
            if matches!(token.token_type, TokenType::EndOfFile | TokenType::CloseCurly) {
                // AD-HOC: If decls is not empty, append it to rules.
                // Spec issue: https://github.com/w3c/csswg-drafts/issues/11017
                if !declarations.is_empty() {
                    rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                }
                // Return rules.
                return rules;
            }

            // <at-keyword-token>
            if matches!(token.token_type, TokenType::AtKeyword { .. }) {
                // If decls is not empty, append it to rules, and set decls to a fresh empty list of declarations.
                if !declarations.is_empty() {
                    rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                    declarations = Vec::new();
                }

                // Consume an at-rule from input, with nested set to true.
                // If a rule was returned, append it to rules.
                if let Some(rule) = self.consume_an_at_rule(Nested::Yes) {
                    rules.push(RuleOrListOfDeclarations::Rule(Rule::AtRule(rule)));
                }
                continue;
            }

            // anything else
            // OPTIMIZATION: Look ahead to determine if this can be a declaration (ident whitespace* ':').
            // If not, skip straight to qualified rule parsing.
            let could_be_declaration = if matches!(token.token_type, TokenType::Ident { .. }) {
                let mut lookahead = 1;
                while matches!(self.peek_token(lookahead).token_type, TokenType::Whitespace) {
                    lookahead += 1;
                }
                matches!(self.peek_token(lookahead).token_type, TokenType::Colon)
            } else {
                false
            };

            if could_be_declaration {
                // Mark input.
                let mark = self.index;

                // Consume a declaration from input, with nested set to true.
                if let Some(declaration) = self.consume_a_declaration(Nested::Yes) {
                    // If anything was returned, append it to decls.
                    declarations.push(declaration);
                    continue;
                }

                // Otherwise, restore input.
                self.index = mark;
            }

            // Consume a qualified rule from input, with nested set to true, and with <semicolon-token> as the stop token.
            // If a rule was returned, append it to rules.
            // If an invalid rule error was returned, append decls to rules and set decls to a fresh empty list of declarations.
            if let Some(rule) = self.consume_a_qualified_rule(Some(TokenType::Semicolon), Nested::Yes) {
                if !declarations.is_empty() {
                    rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                    declarations = Vec::new();
                }
                rules.push(RuleOrListOfDeclarations::Rule(Rule::QualifiedRule(rule)));
            } else if !declarations.is_empty() {
                rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                declarations = Vec::new();
            }
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-declaration
    pub(super) fn consume_a_declaration(&mut self, nested: Nested) -> Option<Declaration> {
        // To consume a declaration from a token stream input, given an optional bool nested (default false):

        // Let decl be a new declaration, with an initially empty name and a value set to an empty list.
        let mut declaration = Declaration {
            name: String::new(),
            value: Vec::new(),
            important: false,
        };

        // 1. If the next token is an <ident-token>, consume a token from input and set decl’s name to the token’s value.
        if matches!(self.next_input_token().token_type, TokenType::Ident { .. }) {
            let token = self.consume_the_next_input_token();
            let TokenType::Ident { value } = token.token_type else {
                unreachable!("declaration names require ident tokens")
            };
            declaration.name = value;
        }
        // Otherwise, consume the remnants of a bad declaration from input, with nested, and return nothing.
        else {
            self.consume_the_remnants_of_a_bad_declaration(nested);
            return None;
        }

        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. If the next token is a <colon-token>, discard a token from input.
        if matches!(self.next_input_token().token_type, TokenType::Colon) {
            self.discard_a_token();
        }
        // Otherwise, consume the remnants of a bad declaration from input, with nested, and return nothing.
        else {
            self.consume_the_remnants_of_a_bad_declaration(nested);
            return None;
        }

        // 4. Discard whitespace from input.
        self.discard_whitespace();

        // 5. Consume a list of component values from input, with nested, and with <semicolon-token> as the stop token,
        // and set decl’s value to the result.
        declaration.value = self.consume_a_list_of_component_values(Some(TokenType::Semicolon), nested);

        // 6. If the last two non-<whitespace-token>s in decl’s value are a <delim-token> with the value "!"
        // followed by an <ident-token> with a value that is an ASCII case-insensitive match for "important",
        // remove them from decl’s value and set decl’s important flag.
        if let Some(important_index) = declaration
            .value
            .iter()
            .rposition(|value| is_ident_component_value(value, "important"))
        {
            let has_only_whitespace_after_important = declaration.value[important_index + 1..]
                .iter()
                .all(is_whitespace_component_value);
            if has_only_whitespace_after_important
                && let Some(bang_index) = declaration.value[..important_index]
                    .iter()
                    .rposition(is_bang_component_value)
            {
                let has_only_whitespace_between_bang_and_important = declaration.value[bang_index + 1..important_index]
                    .iter()
                    .all(is_whitespace_component_value);
                if has_only_whitespace_between_bang_and_important {
                    declaration.value.remove(important_index);
                    declaration.value.remove(bang_index);
                    declaration.important = true;
                }
            }
        }

        // 7. While the last item in decl’s value is a <whitespace-token>, remove that token.
        while declaration.value.last().is_some_and(is_whitespace_component_value) {
            declaration.value.pop();
        }

        // 8. If decl’s name is a custom property name string, then set decl’s original text to the segment
        // of the original source text string corresponding to the tokens of decl’s value.
        if declaration.name.starts_with("--") {
            if contains_an_unmatched_closing_token(&declaration.value) {
                return None;
            }
            // TODO: Preserve original text once the rule/declaration FFI surface exists.
        }
        // Otherwise, if decl’s value contains a top-level simple block with an associated token of <{-token>,
        // and also contains any other non-<whitespace-token> value, return nothing.
        else if contains_a_curly_block_and_non_whitespace(&declaration.value) {
            return None;
        }
        // Otherwise, if decl’s name is an ASCII case-insensitive match for "unicode-range", consume the value of
        // a unicode-range descriptor from the segment of the original source text string corresponding to the
        // tokens returned by the consume a list of component values call, and replace decl’s value with the result.
        else if declaration.name.eq_ignore_ascii_case("unicode-range") {
            // FIXME: Special unicode-range handling.
        }

        // 9. If decl is valid in the current context, return it; otherwise return nothing.
        if self.is_declaration_valid_in_the_current_context(&declaration) {
            return Some(declaration);
        }
        None
    }

    // https://drafts.csswg.org/css-syntax/#consume-the-remnants-of-a-bad-declaration
    pub(super) fn consume_the_remnants_of_a_bad_declaration(&mut self, nested: Nested) {
        // To consume the remnants of a bad declaration from a token stream input, given a bool nested:
        // Process input:
        loop {
            let token = self.next_input_token();

            // <eof-token>
            // <semicolon-token>
            if matches!(token.token_type, TokenType::EndOfFile | TokenType::Semicolon) {
                // Discard a token from input, and return nothing.
                self.discard_a_token();
                return;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) {
                // If nested is true, return nothing. Otherwise, discard a token.
                if nested == Nested::Yes {
                    return;
                }
                self.discard_a_token();
                continue;
            }

            // anything else
            // Consume a component value from input, and do nothing.
            self.consume_a_component_value();
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-list-of-component-values
    pub(super) fn consume_a_list_of_component_values(
        &mut self,
        stop_token: Option<TokenType>,
        nested: Nested,
    ) -> Vec<ComponentValue> {
        // To consume a list of component values from a token stream input, given an optional token stop token
        // and an optional boolean nested (default false):
        // Let values be an empty list of component values.
        let mut values = Vec::new();

        // Process input:
        loop {
            let token = self.next_input_token();

            // <eof-token>
            // stop token (if passed)
            if matches!(token.token_type, TokenType::EndOfFile)
                || stop_token
                    .as_ref()
                    .is_some_and(|stop_token| token.token_type == *stop_token)
            {
                // Return values.
                return values;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) && nested == Nested::Yes {
                // If nested is true, return values.
                return values;
            }

            // anything else
            // Consume a component value from input, and append the result to values.
            values.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-component-value
    pub(super) fn consume_a_component_value(&mut self) -> ComponentValue {
        // To consume a component value from a stream of CSS component values input:
        // Consume the next input token.
        let token = self.consume_the_next_input_token();

        match token.token_type {
            // <{-token>, <[-token>, <(-token>
            TokenType::OpenCurly | TokenType::OpenSquare | TokenType::OpenParen => {
                // Consume a simple block and return it.
                ComponentValue::SimpleBlock(self.consume_a_simple_block(token))
            }

            // <function-token>
            TokenType::Function { .. } => {
                // Consume a function and return it.
                ComponentValue::Function(self.consume_a_function(token))
            }

            // anything else
            _ => {
                // Return the current input token.
                ComponentValue::PreservedToken(token)
            }
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-simple-block
    pub(super) fn consume_a_simple_block(&mut self, token: Token) -> SimpleBlock {
        // To consume a simple block from a stream of CSS component values input:
        // The ending token is the mirror variant of the current input token.
        let ending_token_type = mirror_variant(&token.token_type);

        // Let value be an initially empty list of component values.
        let mut value = Vec::new();

        loop {
            // Repeatedly consume the next input token and process it as follows:
            let next_token = self.next_input_token();

            // ending token
            if next_token.token_type == ending_token_type {
                // Return the block.
                return SimpleBlock {
                    token,
                    value,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // <eof-token>
            if matches!(next_token.token_type, TokenType::EndOfFile) {
                // This is a parse error. Return the block.
                return SimpleBlock {
                    token,
                    value,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // anything else
            // Reconsume the current input token. Consume a component value and append the returned value to the block’s value.
            value.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-function
    pub(super) fn consume_a_function(&mut self, token: Token) -> Function {
        // To consume a function from a stream of CSS component values input:
        // Let function be a function with its name equal to the value of the current input token,
        // and with a value set to an empty list.
        let name = match &token.token_type {
            TokenType::Function { name } => name.clone(),
            _ => unreachable!("consume_a_function requires a function token"),
        };
        let mut value = Vec::new();

        loop {
            // Repeatedly consume the next input token and process it as follows:
            let next_token = self.next_input_token();

            // <)-token>
            if matches!(next_token.token_type, TokenType::CloseParen) {
                // Return the function.
                return Function {
                    name,
                    value,
                    name_token: token,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // <eof-token>
            if matches!(next_token.token_type, TokenType::EndOfFile) {
                // This is a parse error. Return the function.
                return Function {
                    name,
                    value,
                    name_token: token,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // anything else
            // Reconsume the current input token. Consume a component value and append the returned value to the function’s value.
            value.push(self.consume_a_component_value());
        }
    }
}

pub(super) fn mirror_variant(token_type: &TokenType) -> TokenType {
    match token_type {
        TokenType::OpenCurly => TokenType::CloseCurly,
        TokenType::OpenSquare => TokenType::CloseSquare,
        TokenType::OpenParen => TokenType::CloseParen,
        _ => unreachable!("CSS simple blocks must start with a grouping token"),
    }
}

pub(super) fn is_whitespace_component_value(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Whitespace,
            ..
        })
    )
}

pub(super) fn is_ident_component_value(component_value: &ComponentValue, ident: &str) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case(ident)
    )
}

pub(super) fn is_bang_component_value(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Delim { value },
            ..
        }) if *value == '!' as u32
    )
}

pub(super) fn contains_a_curly_block_and_non_whitespace(declaration_value: &[ComponentValue]) -> bool {
    let mut contains_curly_block = false;
    let mut contains_non_whitespace = false;
    for value in declaration_value {
        if let ComponentValue::SimpleBlock(block) = value
            && matches!(block.token.token_type, TokenType::OpenCurly)
        {
            if contains_non_whitespace {
                return true;
            }
            contains_curly_block = true;
            continue;
        }

        if !is_whitespace_component_value(value) {
            if contains_curly_block {
                return true;
            }
            contains_non_whitespace = true;
        }
    }
    false
}

pub(super) fn contains_an_unmatched_closing_token(component_values: &[ComponentValue]) -> bool {
    component_values.iter().any(|component_value| match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::CloseCurly | TokenType::CloseParen | TokenType::CloseSquare,
            ..
        }) => true,
        ComponentValue::Function(function) => contains_an_unmatched_closing_token(&function.value),
        ComponentValue::SimpleBlock(block) => contains_an_unmatched_closing_token(&block.value),
        _ => false,
    })
}

impl Parser {
    pub(super) fn is_declaration_valid_in_the_current_context(&self, declaration: &Declaration) -> bool {
        let Some(context) = self.rule_context.last() else {
            return false;
        };

        match context {
            RuleContext::Unknown => false,
            RuleContext::Style => true,
            RuleContext::Keyframe => {
                // https://drafts.csswg.org/css-animations-1/#keyframes
                // The <declaration-list> inside of <keyframe-block> accepts any CSS property except those defined in
                // this specification, but does accept the animation-timing-function property and interprets it specially
                // NB: animation-composition is defined in CSS Animations Level 2, so it is not excluded by this rule.
                !is_animation_property_disallowed_in_keyframe(&declaration.name)
            }
            RuleContext::AtContainer | RuleContext::AtLayer | RuleContext::AtMedia | RuleContext::AtSupports => self
                .rule_context
                .iter()
                .any(|context| matches!(context, RuleContext::Style | RuleContext::AtFunction)),
            RuleContext::FontFeatureValue => true,
            RuleContext::AtFunction => true,
            RuleContext::AtCounterStyle
            | RuleContext::AtFontFace
            | RuleContext::AtFontFeatureValues
            | RuleContext::AtPage
            | RuleContext::AtProperty
            | RuleContext::Margin => true,
            RuleContext::AtKeyframes => false,
            RuleContext::SupportsCondition => true,
        }
    }

    pub(super) fn is_at_rule_valid_in_the_current_context(&self, at_rule: &AtRule) -> bool {
        if self.rule_context.is_empty() {
            return !is_margin_rule_name(&at_rule.name);
        }

        if self
            .rule_context
            .iter()
            .any(|context| matches!(context, RuleContext::Style))
        {
            return first_is_one_of(&at_rule.name, &["container", "layer", "media", "supports"]);
        }

        if self
            .rule_context
            .iter()
            .any(|context| matches!(context, RuleContext::AtFunction))
        {
            return first_is_one_of(&at_rule.name, &["container", "media", "supports"]);
        }

        match self.rule_context.last().expect("checked non-empty context") {
            RuleContext::Unknown => false,
            RuleContext::Style => unreachable!("style context handled above"),
            RuleContext::AtContainer | RuleContext::AtLayer | RuleContext::AtMedia | RuleContext::AtSupports => {
                !first_is_one_of(&at_rule.name, &["import", "namespace"])
            }
            RuleContext::SupportsCondition => false,
            RuleContext::AtPage => is_margin_rule_name(&at_rule.name),
            RuleContext::AtCounterStyle
            | RuleContext::AtFontFace
            | RuleContext::FontFeatureValue
            | RuleContext::AtKeyframes
            | RuleContext::Keyframe
            | RuleContext::AtProperty
            | RuleContext::Margin => false,
            RuleContext::AtFontFeatureValues => is_font_feature_value_type_at_keyword(&at_rule.name),
            RuleContext::AtFunction => unreachable!("function context handled above"),
        }
    }

    pub(super) fn is_qualified_rule_valid_in_the_current_context(&self) -> bool {
        let Some(context) = self.rule_context.last() else {
            return true;
        };

        match context {
            RuleContext::Unknown => false,
            RuleContext::Style
            | RuleContext::AtContainer
            | RuleContext::AtLayer
            | RuleContext::AtMedia
            | RuleContext::AtSupports
            | RuleContext::AtKeyframes => true,
            RuleContext::SupportsCondition
            | RuleContext::AtCounterStyle
            | RuleContext::AtFontFace
            | RuleContext::AtFontFeatureValues
            | RuleContext::FontFeatureValue
            | RuleContext::AtFunction
            | RuleContext::AtPage
            | RuleContext::AtProperty
            | RuleContext::Keyframe
            | RuleContext::Margin => false,
        }
    }
}
