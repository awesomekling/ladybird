/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibTest/TestCase.h>
#include <LibWebView/FontService.h>
#include <sys/mman.h>

TEST_CASE(font_catalog_is_shared_read_only)
{
    auto font_service = WebView::FontService::create({});
    auto catalog = MUST(font_service->clone_catalog());
    VERIFY(catalog.size > 0);

    // Every helper gets this descriptor. None of them may change the catalog that the others read. On Linux the
    // descriptor stays read-write and seals refuse writable mappings, elsewhere it is opened read-only.
    auto fd = catalog.file.fd();
    EXPECT_EQ(mmap(nullptr, catalog.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0), MAP_FAILED);

    auto* mapping = mmap(nullptr, catalog.size, PROT_READ, MAP_SHARED, fd, 0);
    EXPECT_NE(mapping, MAP_FAILED);
}

TEST_CASE(code_points_one_face_covers_share_its_face)
{
    auto font_service = WebView::FontService::create({});

    // A page in a script no primary font covers asks for each of its characters. The fallback face
    // that covers them is brokered once, not once per character.
    auto first = font_service->match_font_for_code_point('A', 400, 5, 0, false);
    auto second = font_service->match_font_for_code_point('B', 400, 5, 0, false);
    EXPECT_EQ(first.face_id, second.face_id);
}
