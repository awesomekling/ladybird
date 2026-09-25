/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <Compositor/RenderClockConnection.h>

namespace Compositor {

RenderClockConnection::RenderClockConnection(NonnullOwnPtr<IPC::Transport> transport, int client_id)
    : IPC::ConnectionFromClient<RenderClockClientEndpoint, RenderClockServerEndpoint>(*this, move(transport), client_id)
{
}

void RenderClockConnection::die()
{
    on_request_clock_tick = nullptr;
    if (auto on_death = move(this->on_death))
        on_death();
}

void RenderClockConnection::request_clock_tick(Compositing::CompositorContextId context_id, double maximum_frames_per_second)
{
    if (on_request_clock_tick)
        on_request_clock_tick(context_id, maximum_frames_per_second);
}

}
