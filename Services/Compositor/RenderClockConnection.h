/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <Compositor/RenderClockClientEndpoint.h>
#include <Compositor/RenderClockServerEndpoint.h>
#include <LibCompositing/Types.h>
#include <LibIPC/ConnectionFromClient.h>

namespace Compositor {

// The Compositor end of a WebContent process's render clock channel: a side channel that delivers display ticks to
// WebContent's RenderClock thread without passing through its main thread. It is owned by, and forwards its requests
// to, the ConnectionFromWebContent the channel was offered on.
class RenderClockConnection final
    : public IPC::ConnectionFromClient<RenderClockClientEndpoint, RenderClockServerEndpoint> {
    C_OBJECT(RenderClockConnection);

public:
    virtual ~RenderClockConnection() override = default;

    Function<void(Compositing::CompositorContextId, double maximum_frames_per_second)> on_request_clock_tick;
    Function<void()> on_death;

private:
    RenderClockConnection(NonnullOwnPtr<IPC::Transport>, int client_id);

    virtual void die() override;
    virtual void request_clock_tick(Compositing::CompositorContextId, double maximum_frames_per_second) override;
};

}
