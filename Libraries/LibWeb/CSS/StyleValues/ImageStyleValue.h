/*
 * Copyright (c) 2018-2020, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2022-2023, MacDue <macdue@dueutil.tech>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibURL/URL.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/CSS/URL.h>
#include <LibWeb/Forward.h>

namespace Web::CSS {

class ImageStyleValue final : public AbstractImageStyleValue {

    using Base = AbstractImageStyleValue;

public:
    static ValueComparingNonnullRefPtr<ImageStyleValue const> create(URL const&);
    static ValueComparingNonnullRefPtr<ImageStyleValue const> create(::URL::URL const&);
    virtual ~ImageStyleValue() override = default;

    virtual void serialize(StringBuilder&, SerializationMode) const override;
    virtual bool equals(StyleValue const& other) const override;

    virtual bool is_paintable() const override { return false; }
    void paint(DisplayListRecordingContext&, DevicePixelRect const&, CSS::ImageRendering) const override { }

    URL const& url() const { return m_url; }

private:
    ImageStyleValue(URL const&);

    virtual void set_style_sheet(GC::Ptr<CSSStyleSheet>) override;
    virtual ValueComparingNonnullRefPtr<StyleValue const> absolutized(ComputationContext const&) const override;

    URL m_url;
    Optional<::URL::URL> m_base_url;
};

}
