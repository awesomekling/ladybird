/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <UI/Qt/HiddenPageSizing.h>

#include <QLayout>
#include <QWindow>

namespace Ladybird {

void size_hidden_page_like(QWidget& page, QWidget const& current)
{
    page.resize(current.size());

    // Resizing a hidden page updates its geometry, but not its children's: The page's layout runs only on its next
    // activation, which Qt defers until the page is shown. Invalidating first makes activate() lay it out now.
    if (auto* layout = page.layout()) {
        layout->invalidate();
        layout->activate();
    }

    // Qt leaves a hidden widget's native window at its old geometry. If it stays behind, the window later reports that
    // geometry back and undoes the layout, which kept a never-shown page's view at Qt's default 100x30 even after the
    // page was selected.
    move_native_descendants_along(page);
}

void move_native_descendants_along(QWidget& widget)
{
    for (auto* child : widget.findChildren<QWidget*>()) {
        if (child->isWindow() || !child->testAttribute(Qt::WA_NativeWindow))
            continue;
        auto* native_parent = child->nativeParentWidget();
        if (auto* window = child->windowHandle(); window && native_parent)
            window->setGeometry(QRect(child->mapTo(native_parent, QPoint()), child->size()));
    }
}

}
