/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NumericLimits.h>
#include <AK/Optional.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/DOM/Document.h>

namespace Web::CSS {

// A style read that has to answer synchronously, as a CSSOM read does: the record of an element or
// one of its pseudo-elements that no style update installs.
struct StyleReadDemand {
    StyleNodeID node;
    Optional<u8> pseudo_kind {};
    bool exclude_inline_style { false };
    bool targeted { false };
    // A demand that is not read-only settles the engine's published match answer as it answers.
    bool read_only { true };
    // The record of the highlight pseudo-element a highlight pseudo-element inherits from.
    StyleRecordID parent_highlight {};
};

// The read holds the JoinScope it joined the frame in flight under, and the engine answers it in a
// style stage run of its own: the host computes nothing, and only reads the published answer.
inline StyleEngineFFI::FfiRecordDemandAnswer answer_style_read_demand(DOM::Document::JoinScope const&, StyleEngine& engine, StyleReadDemand const& demand)
{
    return StyleEngineFFI::style_engine_answer_read_demand(engine.rust_handle(), demand.node.value(),
        demand.pseudo_kind.value_or(NumericLimits<u8>::max()), demand.exclude_inline_style, demand.targeted,
        demand.read_only, demand.parent_highlight.value());
}

}
