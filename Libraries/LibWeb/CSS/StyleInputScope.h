/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Noncopyable.h>
#include <AK/SourceLocation.h>

namespace Web::CSS {

class StyleEngine;

// Proof that the host publishes a style input between passes: while no batch a pass published is
// waiting for the host or being drained. An input published while a pass is in flight changes what
// that pass answers for under it, so such an input has to queue until the pass has drained.
class StyleInputScope {
    AK_MAKE_NONCOPYABLE(StyleInputScope);
    AK_MAKE_NONMOVABLE(StyleInputScope);

public:
    // FIXME: An input published while a pass is in flight does not queue yet. The style seal counts
    //        each place that publishes one, and each is a host step still to be moved between passes.
    static StyleInputScope between_passes(StyleEngine&, SourceLocation = SourceLocation::current());

    StyleEngine& engine() const { return m_engine; }

private:
    explicit StyleInputScope(StyleEngine& engine)
        : m_engine(engine)
    {
    }

    StyleEngine& m_engine;
};

}
