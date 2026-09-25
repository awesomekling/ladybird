/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Noncopyable.h>

namespace Web::CSS {

class StyleEngine;

// Proof that the host is draining what a style pass published: installing its rows and applying the
// effects they leave. Only StyleEffectDrain makes one, so code that takes one runs inside the drain.
//
// Reads take none. A published record and a custom property environment the engine published
// (style_record_view, borrow_engine_custom_property_environment) are immutable and readable
// anywhere, by CSSOM and layout as much as by the drain, and a read that joins the pass in flight
// reads the engine at rest. Dropping a removed element's recorded input is an input publication,
// not a drain step: it takes a StyleInputScope, or this one when the drain drops it itself. The
// animation inputs are the same (dual): the drain records them for the animations it installs, and
// its later waves sample them, so inside a drain they take this scope (see StyleInputScope).
class StyleDrainScope {
    AK_MAKE_NONCOPYABLE(StyleDrainScope);
    AK_MAKE_NONMOVABLE(StyleDrainScope);

public:
    StyleEngine& engine() const { return m_engine; }

private:
    friend class StyleEffectDrain;
    explicit StyleDrainScope(StyleEngine& engine)
        : m_engine(engine)
    {
    }

    StyleEngine& m_engine;
};

}
