/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibGC/Ptr.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>

namespace Web::HTML {

// What a frame in flight references, and what keeps it alive until the frame is consumed. The collector only scans the
// stack of the thread it runs on, so neither the stage thread's stack nor a heap-owned ticket roots anything by itself.
//
// - The navigables whose frames the render side records and their documents: the frame scheduler traces its ticket,
//   and the event loop, which owns the scheduler, traces the scheduler. A document only references its navigable
//   weakly, so the ticket has to name the navigable itself.
// - Each document's page and window (whose client and callbacks the frame's tail uses): traced edges of the document.
// - Observers, observer targets and callbacks: none. Resize and intersection observations run on the main thread,
//   before the frame is submitted. A stage that comes to name one in its output has to put it in the ticket.
// - The layout arena, the recording, and the images and fonts the recording paints: not on the heap. The frame owns
//   the arena and the recording, and images and fonts are reference counted resources.
// - Shared style records, which the frame reads through the style payloads of the arena's rows: not on the heap. A
//   row's payloads are reference counted, and the frame holds a reference on each one its rows name, so what the style
//   engine reclaims beside the frame does not reach it.
//
// The frame scheduler calls hold_for_frame_in_flight() for each document whose frame it hands to the render side, and
// release_holds_for_frame_in_flight() once consume-commit has taken the frame in.
WEB_API void hold_for_frame_in_flight(DOM::Document&);
WEB_API void release_holds_for_frame_in_flight();

// The stage thread only reads what the frame references and never touches the heap itself. This makes that a debug
// assertion (and a collection there a verification failure) for whatever C++ code the stages call. Call it before the
// first stage runs.
void forbid_heap_access_on_the_stage_thread();

// Whether a frame in flight holds the document.
WEB_API bool frame_in_flight_holds(DOM::Document const&);
// Whether each document a frame in flight holds is alive, and so are its navigable, page and window.
WEB_API bool frame_in_flight_references_are_alive();

}
