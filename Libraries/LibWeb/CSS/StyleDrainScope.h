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
