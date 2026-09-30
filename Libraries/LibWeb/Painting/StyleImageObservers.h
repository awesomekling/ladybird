/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/OwnPtr.h>
#include <AK/Vector.h>
#include <AK/Weakable.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/StyleValues/CursorStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/Export.h>
#include <LibWeb/Painting/BoxSlot.h>

namespace Web::Painting {

// Observes an <image> a row's style names, and pushes the row's layer image paint facts when the image updates.
class WEB_API StyleImageObserver final
    : public CSS::ImageStyleValue::Client
    , public Weakable<StyleImageObserver> {
public:
    AK_MAKE_NONCOPYABLE(StyleImageObserver);
    AK_ALLOC_WITH_KMALLOC;

    StyleImageObserver(DOM::Document&, Compositing::RustFFI::NodeSlotId, NonnullRefPtr<CSS::ImageStyleValue const> image);
    virtual ~StyleImageObserver() override;

    virtual void image_style_value_did_update(CSS::ImageStyleValue&) override;

private:
    Compositing::RustFFI::NodeSlotId m_slot;
    NonnullRefPtr<CSS::ImageStyleValue const> m_image;
};

// The observers a row's style asks for, with the style values they observe: the ones whose resources the row's style
// attach loaded. The set belongs to the arena, which deletes it with the row.
struct WEB_API StyleImageObserverSet {
    AK_ALLOC_WITH_KMALLOC;

    Vector<OwnPtr<StyleImageObserver>> background_layers;
    Vector<OwnPtr<StyleImageObserver>> mask_layers;
    Vector<OwnPtr<StyleImageObserver>> cursors;
    OwnPtr<StyleImageObserver> border_image_source;
    OwnPtr<StyleImageObserver> list_style_image;
    Vector<RefPtr<CSS::CursorStyleValue const>> cursor_style_values;
    Vector<CSS::BackgroundLayerData> background_layer_data;
    Vector<CSS::BackgroundLayerData> mask_layer_data;
    CSS::BorderImageData border_image;

    StyleImageObserver const* background_image_observer(size_t layer_index) const;
    StyleImageObserver const* mask_image_observer(size_t layer_index) const;
    StyleImageObserver const* cursor_image_observer(size_t cursor_index) const;
};

// The set the row holds, if its style asks for one.
WEB_API StyleImageObserverSet const* style_image_observers(BoxSlot const&);
// Installs the set the row holds, deleting the one it replaces.
WEB_API void replace_style_image_observers(DOM::Document&, Compositing::RustFFI::NodeSlotId, OwnPtr<StyleImageObserverSet>);

}
