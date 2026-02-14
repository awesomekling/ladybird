/*
 * Copyright (c) 2020, the SerenityOS developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/HTML/HTMLElement.h>

namespace Web::HTML {

class HTMLEmbedElement final : public HTMLElement {
    WEB_PLATFORM_OBJECT(HTMLEmbedElement, HTMLElement);
    GC_DECLARE_ALLOCATOR(HTMLEmbedElement);

public:
    virtual ~HTMLEmbedElement() override;

private:
    HTMLEmbedElement(DOM::Document&, DOM::QualifiedName);

    virtual bool is_html_embed_element() const override { return true; }
    virtual void initialize(JS::Realm&) override;
    virtual bool is_presentational_hint(FlyString const&) const override;
    virtual void apply_presentational_hints(CSS::CascadedProperties&) const override;
    virtual bool disallows_display_contents() const override { return true; }
};

}

namespace Web::DOM {

template<>
inline bool Node::fast_is<HTML::HTMLEmbedElement>() const { return is_html_embed_element(); }

}
