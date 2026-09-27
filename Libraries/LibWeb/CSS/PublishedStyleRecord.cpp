/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/PublishedStyleRecord.h>

namespace Web::CSS {

RefPtr<PublishedStyleRecord const> PublishedStyleRecord::adopt(void const* handle)
{
    if (!handle)
        return nullptr;
    return adopt_ref(*new PublishedStyleRecord(handle, StyleEngineFFI::published_style_record_read(handle)));
}

PublishedStyleRecord::~PublishedStyleRecord()
{
    StyleEngineFFI::published_style_record_release(m_handle);
}

}
