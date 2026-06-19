/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Span.h>
#include <AK/String.h>
#include <LibWeb/Forward.h>

namespace Web::CSS::Parser {

// https://drafts.csswg.org/css-values-5/#substitution-context
struct SubstitutionContext {
    enum class DependencyType : u8 {
        Property,
        Attribute,
        Function,
    };
    DependencyType dependency_type;
    String first;
    Optional<String> second {};

    bool is_cyclic { false };

    bool operator==(SubstitutionContext const&) const;
    String to_string() const;
};

class GuardedSubstitutionContexts {
public:
    void guard(SubstitutionContext&);
    void unguard(SubstitutionContext const&);

    // Incremented every time a cycle is detected. Lets callers tell whether resolving a value involved a cycle,
    // which matters because a cyclic value's resolution is path-dependent and must not be memoized.
    u64 cycle_detection_count() const { return m_cycle_detection_count; }

private:
    Vector<SubstitutionContext&> m_contexts;
    u64 m_cycle_detection_count { 0 };
};

enum class ArbitrarySubstitutionFunction : u8 {
    Attr,
    Env,
    If,
    Inherit,
    Var,
};
[[nodiscard]] Optional<ArbitrarySubstitutionFunction> to_arbitrary_substitution_function(FlyString const& name);

bool contains_guaranteed_invalid_value(ReadonlySpan<ComponentValue>);

[[nodiscard]] Vector<ComponentValue> substitute_arbitrary_substitution_functions(DOM::AbstractElement&, GuardedSubstitutionContexts&, ReadonlySpan<ComponentValue>, Optional<SubstitutionContext> = {});

using DeclarationValueList = Vector<ReadonlySpan<ComponentValue>>;

struct IfArgsBranch {
    ReadonlySpan<ComponentValue> condition;
    Optional<ReadonlySpan<ComponentValue>> value;
};

using IfArgs = Vector<IfArgsBranch>;
using ArbitrarySubstitutionFunctionArguments = Variant<DeclarationValueList, IfArgs>;
// The returned argument spans borrow from the input component value list.
[[nodiscard]] Optional<ArbitrarySubstitutionFunctionArguments> parse_according_to_argument_grammar(ArbitrarySubstitutionFunction, ReadonlySpan<ComponentValue>);

[[nodiscard]] Vector<ComponentValue> replace_an_arbitrary_substitution_function(DOM::AbstractElement&, GuardedSubstitutionContexts&, ArbitrarySubstitutionFunction, ArbitrarySubstitutionFunctionArguments const&);

}
