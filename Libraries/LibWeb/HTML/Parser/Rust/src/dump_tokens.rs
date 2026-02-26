// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

use libweb_html_tokenizer::token::TokenType;
use libweb_html_tokenizer::tokenizer::HtmlTokenizer;
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let input_data = if args.len() > 1 && args[1] != "-" {
        std::fs::read_to_string(&args[1]).expect("Failed to read file")
    } else {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .expect("Failed to read stdin");
        buf
    };

    // Convert to code points (u32).
    let code_points: Vec<u32> = input_data.chars().map(|c| c as u32).collect();

    let mut tokenizer = HtmlTokenizer::new(code_points);

    loop {
        let token = match tokenizer.next_token() {
            Some(t) => t,
            None => break,
        };

        match token.token_type {
            TokenType::Doctype => {
                print!("DOCTYPE");
                if let Some(ref dd) = token.doctype_data {
                    if !dd.missing_name {
                        print!(" name=\"{}\"", dd.name);
                    }
                    if !dd.missing_public_identifier {
                        print!(" public_id=\"{}\"", dd.public_identifier);
                    }
                    if !dd.missing_system_identifier {
                        print!(" system_id=\"{}\"", dd.system_identifier);
                    }
                    if dd.force_quirks {
                        print!(" force_quirks");
                    }
                }
                println!(
                    " @{}:{}-{}:{}",
                    token.start_position.line,
                    token.start_position.column,
                    token.end_position.line,
                    token.end_position.column
                );
            }
            TokenType::StartTag => {
                if token.self_closing {
                    print!("StartTag <{}/> [", token.tag_name);
                } else {
                    print!("StartTag <{}> [", token.tag_name);
                }
                for (i, attr) in token.attributes.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    print!("{}=\"{}\"", attr.local_name, attr.value);
                }
                println!(
                    "] @{}:{}-{}:{}",
                    token.start_position.line,
                    token.start_position.column,
                    token.end_position.line,
                    token.end_position.column
                );
            }
            TokenType::EndTag => {
                println!(
                    "EndTag </{}> @{}:{}-{}:{}",
                    token.tag_name,
                    token.start_position.line,
                    token.start_position.column,
                    token.end_position.line,
                    token.end_position.column
                );
            }
            TokenType::Comment => {
                println!(
                    "Comment \"{}\" @{}:{}-{}:{}",
                    token.comment_data,
                    token.start_position.line,
                    token.start_position.column,
                    token.end_position.line,
                    token.end_position.column
                );
            }
            TokenType::Character => {
                let cp = token.code_point;
                if cp >= 0x20 && cp < 0x7F {
                    println!(
                        "Character U+{:04X} '{}' @{}:{}",
                        cp,
                        char::from_u32(cp).unwrap(),
                        token.start_position.line,
                        token.start_position.column
                    );
                } else {
                    println!(
                        "Character U+{:04X} @{}:{}",
                        cp,
                        token.start_position.line,
                        token.start_position.column
                    );
                }
            }
            TokenType::EndOfFile => {
                println!(
                    "EndOfFile @{}:{}",
                    token.start_position.line, token.start_position.column
                );
                break;
            }
            _ => {}
        }
    }
}
