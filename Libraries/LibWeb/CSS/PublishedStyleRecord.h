/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/RefCounted.h>
#include <AK/RefPtr.h>
#include <LibWeb/CSS/StyleRecordID.h>
#include <LibWeb/Export.h>
#include <LibWeb/StyleEngineRustFFI.h>

namespace Web::CSS {

enum class StyleRecordDependencyFlag : u8;

// A style record as the style engine published it (published_record.rs): immutable, and it owns everything a read of it
// reads, its group payloads, its base record's longhand table and its animated overlay among them. What the document
// thread reads of computed style it reads through one: an element holds the record the drain installed on it, and a
// style read demand answers with one. A read made through it reaches no style engine and no layout node arena.
class WEB_API PublishedStyleRecord : public RefCounted<PublishedStyleRecord> {
public:
    // Takes over the reference the engine handed out with `handle`; null for a null handle.
    static RefPtr<PublishedStyleRecord const> adopt(void const* handle);
    ~PublishedStyleRecord();

    // The engine's identity of the record, for comparing records and naming one to the engine.
    StyleRecordID identity() const { return StyleRecordID { m_read.style_record }; }
    // The record as Rust names it, for handing it on to what keeps a reference of its own.
    void const* handle() const { return m_handle; }
    StyleEngineFFI::FfiStyleRecordView const& view() const { return m_read.view; }
    void const* payloads() const { return m_read.view.payloads; }
    StyleRecordDependencyFlag dependency_flags() const { return static_cast<StyleRecordDependencyFlag>(m_read.view.dependency_flags); }
    u64 custom_property_environment() const { return m_read.custom_property_environment; }
    bool is_animation_overlay() const { return m_read.view.animation_overlay_identity != 0; }

private:
    PublishedStyleRecord(void const* handle, StyleEngineFFI::FfiPublishedStyleRecordRead const& read)
        : m_handle(handle)
        , m_read(read)
    {
    }

    void const* m_handle { nullptr };
    StyleEngineFFI::FfiPublishedStyleRecordRead m_read;
};

}
