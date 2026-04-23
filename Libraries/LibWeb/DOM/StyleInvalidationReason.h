/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/EnumBits.h>
#include <AK/FlyString.h>
#include <AK/StringView.h>

namespace Web::DOM {

#define ENUMERATE_STYLE_INVALIDATION_REASONS(X)     \
    X(AdoptedStyleSheetsList)                       \
    X(BaseURLChanged)                               \
    X(CounterStyleCacheInvalidated)                 \
    X(CSSFontLoaded)                                \
    X(CSSImportRule)                                \
    X(CSSStylePropertiesRemoveProperty)             \
    X(CSSStylePropertiesSetProperty)                \
    X(CSSStylePropertiesSetPropertyStyleValue)      \
    X(CSSStylePropertiesTextChange)                 \
    X(CustomElementStateChange)                     \
    X(CustomStateSetChange)                         \
    X(EditingInsertion)                             \
    X(EditingDeletion)                              \
    X(ElementAttributeChange)                       \
    X(ElementSetShadowRoot)                         \
    X(ElementSetActive)                             \
    X(Fullscreen)                                   \
    X(HTMLDialogElementSetIsModal)                  \
    X(HTMLDetailsOrDialogOpenAttributeChange)       \
    X(HTMLHyperlinkElementHrefChange)               \
    X(HTMLIFrameElementGeometryChange)              \
    X(HTMLInputElementSetChecked)                   \
    X(HTMLInputElementSetIsOpen)                    \
    X(HTMLInputElementSetType)                      \
    X(HTMLObjectElementUpdateLayoutAndChildObjects) \
    X(HTMLOptionElementSelectedChange)              \
    X(HTMLSelectElementSetIsOpen)                   \
    X(MediaListSetMediaText)                        \
    X(MediaListAppendMedium)                        \
    X(MediaListDeleteMedium)                        \
    X(MediaQueryChangedMatchState)                  \
    X(NavigableSetViewportSize)                     \
    X(NodeInsertBefore)                             \
    X(NodeRemove)                                   \
    X(NodeSetTextContent)                           \
    X(Other)                                        \
    X(SetSelectorText)                              \
    X(SettingsChange)                               \
    X(StyleSheetDisabledStateChange)                \
    X(StyleSheetDeleteRule)                         \
    X(StyleSheetInsertRule)                         \
    X(StyleSheetListAddSheet)                       \
    X(StyleSheetListRemoveSheet)                    \
    X(StyleSheetReplace)

enum class StyleInvalidationReason {
#define __ENUMERATE_STYLE_INVALIDATION_REASON(reason) reason,
    ENUMERATE_STYLE_INVALIDATION_REASONS(__ENUMERATE_STYLE_INVALIDATION_REASON)
#undef __ENUMERATE_STYLE_INVALIDATION_REASON
};

struct StyleInvalidationOptions {
    bool invalidate_self { false };
};

enum class StyleDirtyingSource : u8 {
    None = 0,
    DirectSetNeedsStyleUpdate = 1 << 0,
    EntireSubtreeRoot = 1 << 1,
    EntireSubtreePreviousSibling = 1 << 2,
    EntireSubtreeNextSibling = 1 << 3,
    ChildrenChangedParent = 1 << 4,
};
AK_ENUM_BITWISE_OPERATORS(StyleDirtyingSource);

[[nodiscard]] FlyString style_dirtying_source_name(StyleDirtyingSource);
[[nodiscard]] FlyString style_dirtying_source_combination_name(StyleDirtyingSource);

}
