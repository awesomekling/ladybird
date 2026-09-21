/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibTest/TestCase.h>
#include <dlfcn.h>

// `libgfx_rust` is compiled into two libraries: LibGfx links it, and LibWeb's Rust crate depends on
// it. A linker with a flat namespace, as Linux has, binds every call to one of the two copies and
// the duplication never shows; one with two-level namespaces, as macOS has, gives each library its
// own copy of every `static` in the crate, and a list one library pushes to is not the list the
// other drains. That is how the wanted-pending-face handover broke on macOS and nowhere else.
//
// This test makes Linux answer the macOS question. It reaches each library's own copy of the crate
// by name, checks that they really are two, and then checks that the state they share is one.
TEST_CASE(the_graphics_crate_is_compiled_twice_and_its_process_state_once)
{
    auto* gfx = dlopen("liblagom-gfx.so", RTLD_NOW | RTLD_LOCAL);
    auto* web = dlopen("liblagom-web.so", RTLD_NOW | RTLD_LOCAL);
    EXPECT(gfx);
    EXPECT(web);

    // Each library carries its own copy of the crate's code. If these two ever became one, the
    // test below would pass for the wrong reason, so fail loudly instead.
    auto* register_through_gfx = reinterpret_cast<void (*)()>(dlsym(gfx, "ladybird_gfx_register_rust_crate_copy"));
    auto* register_through_web = reinterpret_cast<void (*)()>(dlsym(web, "ladybird_gfx_register_rust_crate_copy"));
    EXPECT(register_through_gfx);
    EXPECT(register_through_web);
    EXPECT_NE(reinterpret_cast<void*>(register_through_gfx), reinterpret_cast<void*>(register_through_web));

    // The store is LibGfx's alone, so asking for it through either library is asking one store.
    auto* copies_seen_through_gfx = dlsym(gfx, "ladybird_gfx_process_crate_copies_seen");
    auto* copies_seen_through_web = dlsym(web, "ladybird_gfx_process_crate_copies_seen");
    EXPECT(copies_seen_through_gfx);
    EXPECT_EQ(copies_seen_through_gfx, copies_seen_through_web);

    // And both copies of the crate write into it. Before the crate's process-wide state moved to
    // C++, registering through LibWeb's copy landed in a second store and this said one.
    register_through_gfx();
    register_through_web();
    EXPECT_EQ(reinterpret_cast<size_t (*)()>(copies_seen_through_gfx)(), 2u);

    dlclose(web);
    dlclose(gfx);
}
