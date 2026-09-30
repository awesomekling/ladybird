/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Noncopyable.h>

namespace Web::CSS {

// Proof that code runs inside the style stage, the one run_stage() of a style update. The stage is
// the engine's own code, so nothing on the host side makes one: an engine computation that takes
// one belongs to the stage, and each host call of it is a computation still to be moved there.
class StyleStageScope {
    AK_MAKE_NONCOPYABLE(StyleStageScope);
    AK_MAKE_NONMOVABLE(StyleStageScope);

private:
    StyleStageScope() = default;
};

}
