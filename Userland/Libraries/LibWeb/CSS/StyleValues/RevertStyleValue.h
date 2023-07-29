/*
 * Copyright (c) 2023, Andreas Kling <kling@serenityos.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/CSS/StyleValue.h>

namespace Web::CSS {

class RevertStyleValue final : public StyleValueWithDefaultOperators<RevertStyleValue> {
public:
    static ErrorOr<ValueComparingNonnullRefPtr<RevertStyleValue>> the()
    {
        static ValueComparingNonnullRefPtr<RevertStyleValue> instance = TRY(adopt_nonnull_ref_or_enomem(new (nothrow) RevertStyleValue));
        return instance;
    }
    virtual ~RevertStyleValue() override = default;

    ErrorOr<String> to_string() const override { return "revert"_string; }

    bool properties_equal(RevertStyleValue const&) const { return true; }

private:
    RevertStyleValue()
        : StyleValueWithDefaultOperators(Type::Revert)
    {
    }
};

}
