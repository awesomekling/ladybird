/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2022-2023, MacDue <macdue@dueutil.tech>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/CSSStyleSheet.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/DOMURL/DOMURL.h>
#include <LibWeb/HTML/Scripting/Environments.h>

namespace Web::CSS {

ValueComparingNonnullRefPtr<ImageStyleValue const> ImageStyleValue::create(URL const& url)
{
    return adopt_ref(*new (nothrow) ImageStyleValue(url));
}

ValueComparingNonnullRefPtr<ImageStyleValue const> ImageStyleValue::create(::URL::URL const& url)
{
    return adopt_ref(*new (nothrow) ImageStyleValue(URL { url.to_string() }));
}

ImageStyleValue::ImageStyleValue(URL const& url)
    : AbstractImageStyleValue(Type::Image)
    , m_url(url)
{
}

void ImageStyleValue::serialize(StringBuilder& builder, SerializationMode) const
{
    builder.append(m_url.to_string());
}

bool ImageStyleValue::equals(StyleValue const& other) const
{
    if (type() != other.type())
        return false;
    return m_url == other.as_image().m_url;
}

void ImageStyleValue::set_style_sheet(GC::Ptr<CSSStyleSheet> style_sheet)
{
    Base::set_style_sheet(style_sheet);
    if (style_sheet) {
        m_base_url = style_sheet->base_url()
                         .value_or_lazy_evaluated_optional([&]() { return style_sheet->location(); })
                         .value_or_lazy_evaluated_optional([&]() { return HTML::relevant_settings_object(*style_sheet).api_base_url(); });
    }
}

ValueComparingNonnullRefPtr<StyleValue const> ImageStyleValue::absolutized(ComputationContext const&) const
{
    if (m_url.url().is_empty())
        return *this;

    if (m_base_url.has_value()) {
        if (auto resolved_url = DOMURL::parse(m_url.url(), *m_base_url); resolved_url.has_value())
            return ImageStyleValue::create(*resolved_url);
    }

    return *this;
}

}
