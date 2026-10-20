/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <LibWeb/CSS/InstalledStyle.h>

namespace Web::CSS {

namespace {

struct ViewKey {
    void const* engine { nullptr };
    StyleRecordID record;

    bool operator==(ViewKey const&) const = default;
};

}

}

template<>
struct AK::Traits<Web::CSS::ViewKey> : public AK::DefaultTraits<Web::CSS::ViewKey> {
    static unsigned hash(Web::CSS::ViewKey const& key) { return pair_int_hash(ptr_hash(key.engine), u64_hash(key.record.value())); }
};

namespace Web::CSS {

// The views elements installed, by the record they are of. The elements and their records live on the main thread.
static HashMap<ViewKey, InstalledStyleView const*>& installed_views()
{
    static HashMap<ViewKey, InstalledStyleView const*> views;
    return views;
}

static StyleEngineFFI::FfiStyleRecordView const s_missing_view {};

InstalledStyle::InstalledStyle(void const* engine, StyleRecordID record, StyleEngineFFI::FfiStyleRecordView const& view)
    : m_record(record)
{
    if (view.present)
        m_view = InstalledStyleView::intern(engine, record, view);
}

StyleEngineFFI::FfiStyleRecordView const& InstalledStyle::view() const
{
    return m_view ? m_view->view() : s_missing_view;
}

static bool views_are_equal(StyleEngineFFI::FfiStyleRecordView const& a, StyleEngineFFI::FfiStyleRecordView const& b)
{
    return a.payloads == b.payloads
        && a.base_payloads == b.base_payloads
        && a.longhand_table == b.longhand_table
        && a.animated_overlay == b.animated_overlay
        && a.payload_count == b.payload_count
        && a.pseudo_element_styles == b.pseudo_element_styles
        && a.counter_style_environment_identity == b.counter_style_environment_identity
        && a.animation_overlay_identity == b.animation_overlay_identity
        && a.dependency_flags == b.dependency_flags
        && a.present == b.present;
}

NonnullRefPtr<InstalledStyleView const> InstalledStyleView::intern(void const* engine, StyleRecordID record, StyleEngineFFI::FfiStyleRecordView const& view)
{
    auto& views = installed_views();
    // NB: A view is shared only where it is the same view: a document's engine may take the place of one whose
    //     elements still hold views of its records, with the same numbers for other records.
    if (auto existing = views.get({ engine, record }); existing.has_value() && views_are_equal((*existing)->m_view, view))
        return **existing;
    auto installed = adopt_ref(*new InstalledStyleView(engine, record, view));
    views.set({ engine, record }, installed.ptr());
    return installed;
}

InstalledStyleView::~InstalledStyleView()
{
    auto& views = installed_views();
    if (auto entry = views.get({ m_engine, m_record }); entry.has_value() && *entry == this)
        views.remove({ m_engine, m_record });
}

}
