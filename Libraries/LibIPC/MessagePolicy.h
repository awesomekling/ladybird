/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/String.h>
#include <AK/StringView.h>
#include <AK/Types.h>

namespace URL {

class Origin;
class URL;

}

namespace IPC {

class MessagePolicy {
public:
    virtual ~MessagePolicy() = default;

    virtual bool is_primary_connection() const = 0;
    virtual bool is_test_mode() const = 0;
    virtual bool allows_principal(URL::URL const&) const = 0;
    virtual bool allows_principal(URL::Origin const&) const = 0;
    virtual bool allows_principal(String const&) const = 0;
    virtual bool allows_site(StringView) const = 0;
    virtual bool owns_page(u64 page_id) const = 0;
    virtual bool has_transient_activation(u64 page_id) const = 0;
    virtual void did_misbehave(StringView message, StringView reason) = 0;
};

}
