/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

/// Smoke-test function to verify Rust-C++ linkage works.
/// Returns 42.
#[no_mangle]
pub extern "C" fn rust_parser_smoke_test() -> i32 {
    42
}
