/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Noncopyable.h>

namespace Web::CSS {

class StyleEngine;

// Proof that the host publishes a style input between passes: while no batch a pass published is
// waiting for the host or being drained. An input published while a pass is in flight changes what
// that pass answers for under it, so only the style engine makes one: StyleEngine::publish_input()
// hands it to an input at once between passes, and queues the input until the pass has drained
// while one is in flight. What the drain itself tells the engine about the rows it installs takes
// the drain's StyleDrainScope instead.
//
// Appending to an input journal the next transaction takes (the host's recorded input, the engine's
// deferred element inputs), interning into the engine's catalogs (atoms, attribute value texts,
// selector queries) and granting the host style node identities to mint change no answer of a pass,
// and need no scope.
class StyleInputScope {
    AK_MAKE_NONCOPYABLE(StyleInputScope);
    AK_MAKE_NONMOVABLE(StyleInputScope);

public:
    StyleEngine& engine() const { return m_engine; }

private:
    friend class StyleEngine;

    explicit StyleInputScope(StyleEngine& engine)
        : m_engine(engine)
    {
    }

    StyleEngine& m_engine;
};

}
