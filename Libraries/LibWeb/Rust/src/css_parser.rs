/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// The Rust parser is being staged bottom-up. Keep this module warning-free until
// the C++ bridge starts calling into it.
#![allow(dead_code)]

use crate::css_tokenizer::{Token, TokenType};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ComponentValue {
    PreservedToken(Token),
    Function(Function),
    SimpleBlock(SimpleBlock),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Function {
    pub(crate) name: String,
    pub(crate) value: Vec<ComponentValue>,
    pub(crate) name_token: Token,
    pub(crate) end_token: Token,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SimpleBlock {
    pub(crate) token: Token,
    pub(crate) value: Vec<ComponentValue>,
    pub(crate) end_token: Token,
}

pub(crate) struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
    }

    // https://drafts.csswg.org/css-syntax/#parse-list-of-component-values
    pub(crate) fn parse_a_list_of_component_values(&mut self) -> Vec<ComponentValue> {
        // To parse a list of component values from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a list of component values from input, and return the result.
        self.consume_a_list_of_component_values()
    }

    fn next_input_token(&self) -> &Token {
        self.tokens
            .get(self.index)
            .or_else(|| self.tokens.last())
            .expect("CSS parser requires an EOF token")
    }

    fn consume_the_next_input_token(&mut self) -> Token {
        let token = self.next_input_token().clone();
        self.index += 1;
        token
    }

    // https://drafts.csswg.org/css-syntax/#consume-list-of-component-values
    fn consume_a_list_of_component_values(&mut self) -> Vec<ComponentValue> {
        // To consume a list of component values from a stream of CSS component values input:
        // Let values be an empty list of component values.
        let mut values = Vec::new();

        loop {
            // Repeatedly consume the next input token from input:
            let token = self.next_input_token();

            // <eof-token>
            if matches!(token.token_type, TokenType::EndOfFile) {
                // Return values.
                return values;
            }

            // anything else
            // Reconsume the current input token. Consume a component value and append the returned value to values.
            values.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-component-value
    fn consume_a_component_value(&mut self) -> ComponentValue {
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
    fn consume_a_simple_block(&mut self, token: Token) -> SimpleBlock {
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
    fn consume_a_function(&mut self, token: Token) -> Function {
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

fn mirror_variant(token_type: &TokenType) -> TokenType {
    match token_type {
        TokenType::OpenCurly => TokenType::CloseCurly,
        TokenType::OpenSquare => TokenType::CloseSquare,
        TokenType::OpenParen => TokenType::CloseParen,
        _ => unreachable!("CSS simple blocks must start with a grouping token"),
    }
}

#[cfg(test)]
mod tests {
    use super::{ComponentValue, Parser};
    use crate::css_tokenizer::{self, TokenType};

    fn parse(input: &str) -> Vec<ComponentValue> {
        let mut tokens = Vec::new();
        css_tokenizer::tokenize(input.as_bytes(), |token, _| tokens.push(token.clone()));
        Parser::new(tokens).parse_a_list_of_component_values()
    }

    #[test]
    fn parses_preserved_tokens() {
        let values = parse("a, b");

        assert_eq!(values.len(), 4);
        assert!(matches!(values[0], ComponentValue::PreservedToken(_)));
        assert!(matches!(values[1], ComponentValue::PreservedToken(_)));
        assert!(matches!(values[2], ComponentValue::PreservedToken(_)));
        assert!(matches!(values[3], ComponentValue::PreservedToken(_)));
    }

    #[test]
    fn parses_simple_blocks() {
        let values = parse("{ color: rgb(1 2 3); }");

        let ComponentValue::SimpleBlock(block) = &values[0] else {
            panic!("expected a simple block");
        };
        assert!(matches!(block.token.token_type, TokenType::OpenCurly));
        assert!(matches!(block.end_token.token_type, TokenType::CloseCurly));
        assert!(
            block
                .value
                .iter()
                .any(|value| matches!(value, ComponentValue::Function(function) if function.name == "rgb"))
        );
    }

    #[test]
    fn parses_functions() {
        let values = parse("calc(1px + var(--gap))");

        let ComponentValue::Function(function) = &values[0] else {
            panic!("expected a function");
        };
        assert_eq!(function.name, "calc");
        assert!(matches!(function.end_token.token_type, TokenType::CloseParen));
        assert!(
            function
                .value
                .iter()
                .any(|value| matches!(value, ComponentValue::Function(function) if function.name == "var"))
        );
    }
}
