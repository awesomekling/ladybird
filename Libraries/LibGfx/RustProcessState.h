/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>

namespace Gfx {

// The process-wide state the Rust graphics crate needs exactly one copy of.
//
// `libgfx_rust` is compiled into two libraries: LibGfx links it, and LibWeb's own Rust crate
// depends on it, so its code and every `static` in it end up in both. A dynamic linker with a
// flat namespace binds all the calls to one copy and the duplication stays invisible; one with
// two-level namespaces, as macOS has, gives each library its own copy of every `static`. A list
// one library pushes to is then not the list the other drains, and nothing about the Rust says so.
//
// So the crate keeps no process-wide state of its own. What has to be shared lives here, in a
// library there is only one of, and the crate reaches it through `extern "C"` entry points.
// `Meta/Linters/check_rust_process_wide_state.py` keeps it that way.

// How many copies of the Rust graphics crate have reported themselves: one where the linker binds
// them together, two on macOS. Nothing may depend on the answer; it exists so that a process can
// say how many it is running, and so a test can prove the shared state is not one of them.
[[nodiscard]] size_t rust_crate_copies_seen();

}
