/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibTest/TestCase.h>
#include <LibWeb/Painting/DisplayListResourceStorage.h>

using namespace Web::Painting;

TEST_CASE(resource_storages_have_unique_cache_ids)
{
    DisplayListResourceStorage first;
    DisplayListResourceStorage second;

    EXPECT_NE(first.cache_id(), second.cache_id());
}

TEST_CASE(resource_storage_cache_id_moves_with_resources)
{
    DisplayListResourceStorage original;
    auto original_cache_id = original.cache_id();

    DisplayListResourceStorage move_constructed { move(original) };
    EXPECT_EQ(move_constructed.cache_id(), original_cache_id);
    EXPECT_NE(original.cache_id(), original_cache_id);

    DisplayListResourceStorage move_assigned;
    auto moved_from_cache_id = move_constructed.cache_id();
    move_assigned = move(move_constructed);

    EXPECT_EQ(move_assigned.cache_id(), moved_from_cache_id);
    EXPECT_NE(move_constructed.cache_id(), moved_from_cache_id);
}
