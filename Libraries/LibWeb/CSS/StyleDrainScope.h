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
    // FIXME: Every caller of this runs outside the drain, and each is a host step still to be moved
    //        into the drain or the pass.
    static StyleDrainScope not_yet_drained(StyleEngine& engine) { return StyleDrainScope { engine }; }

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
