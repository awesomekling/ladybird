/*
 * Copyright (c) 2026, Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibCore/ArgsParser.h>
#include <LibCore/ElapsedTimer.h>
#include <LibMain/Main.h>
#include <LibWeb/HTML/Parser/HTMLToken.h>
#include <LibWeb/HTML/Parser/HTMLTokenizer.h>

using Web::HTML::HTMLToken;
using Web::HTML::HTMLTokenizer;

ErrorOr<int> ladybird_main(Main::Arguments arguments)
{
    int iterations = 100;

    Core::ArgsParser args_parser;
    args_parser.set_general_help("Benchmark the HTML tokenizer.");
    args_parser.add_option(iterations, "Number of iterations", "iterations", 'n', "count");
    args_parser.parse(arguments);

    // Build a large HTML string with many named character references.
    StringBuilder input_builder;
    for (int i = 0; i < 5000; i++) {
        input_builder.append("&amp;&lt;&gt;&quot;&apos;&notindot;&CounterClockwiseContourIntegral;"sv);
        input_builder.append("<div class=\"test\">"sv);
        input_builder.append("Hello &copy; World &mdash; foo &hearts; bar"sv);
        input_builder.append("</div>"sv);
    }
    auto input = input_builder.to_string_without_validation();

    outln("Input size: {}KB", input.bytes().size() / 1024);
    outln("Iterations: {}", iterations);

    auto timer = Core::ElapsedTimer::start_new();

    u64 total_tokens = 0;
    for (int i = 0; i < iterations; i++) {
        HTMLTokenizer tokenizer { input, "UTF-8"sv };
        while (true) {
            auto maybe_token = tokenizer.next_token();
            if (!maybe_token.has_value())
                break;
            total_tokens++;
            if (maybe_token->is_end_of_file())
                break;
        }
    }

    auto elapsed_ms = timer.elapsed_milliseconds();
    outln("Total: {}ms ({:.2f}ms per iteration)", elapsed_ms, static_cast<double>(elapsed_ms) / iterations);
    outln("Tokens per iteration: {}", total_tokens / iterations);

    return 0;
}
