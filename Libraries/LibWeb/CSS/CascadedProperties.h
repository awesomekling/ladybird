/*
 * Copyright (c) 2024, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/FixedBitmap.h>
#include <AK/RefCounted.h>
#include <LibWeb/CSS/CascadeOrigin.h>
#include <LibWeb/CSS/PropertyID.h>
#include <LibWeb/CSS/Selector.h>
#include <LibWeb/CSS/StyleProperty.h>
#include <LibWeb/CSS/StyleValues/StyleValue.h>

namespace Web::CSS {

class CascadedProperties final : public RefCounted<CascadedProperties> {
public:
    CascadedProperties();
    ~CascadedProperties();

    [[nodiscard]] RefPtr<StyleValue const> property(PropertyID) const;
    [[nodiscard]] Optional<StyleProperty> style_property(PropertyID) const;

    void set_property(PropertyID, NonnullRefPtr<StyleValue const>, Important, CascadeOrigin, Optional<FlyString> layer_name);
    void set_property_from_presentational_hint(PropertyID, NonnullRefPtr<StyleValue const>);

    void revert_property(PropertyID, Important, CascadeOrigin);
    void revert_layer_property(PropertyID, Important, Optional<FlyString> layer_name);

    void resolve_unresolved_properties(DOM::AbstractElement);

private:
    struct Entry {
        StyleProperty property;
        CascadeOrigin origin;
        Optional<FlyString> layer_name;
    };
    HashMap<PropertyID, Vector<Entry>> m_properties;
    AK::FixedBitmap<to_underlying(last_longhand_property_id) + 1> m_contained_properties_cache { false };
};

}
