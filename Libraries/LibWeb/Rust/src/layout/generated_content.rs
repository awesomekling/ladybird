/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the layout tree build puts inside a pseudo-element's box: its `content` resolved against the
//! counters sets and the quote depth, or the marker string of a list marker whose `content` is
//! `normal`.

use super::counters::{CounterName, CounterOwner, LIST_ITEM_COUNTER_NAME, style_of};
use super::layout_node_arena::LayoutNodeArena;
use crate::css::counter_representation::{CounterStyle, generate_a_counter_representation};
use crate::css::css_string::CssString;
use crate::css::style::fast_hash::FastMap as HashMap;
use crate::css::style::tree::StyleNodeID;
use crate::css::style_compute::keyword;
use crate::css::style_value::StyleValueData;
use std::sync::Arc;

/// The counter styles a pseudo-element's `content` names, as the host resolved them when it built the
/// box: one per `counter()` or `counters()`, in the order they appear in `content` and then in its
/// alt text. A style the name does not resolve to is `None`, which is `decimal`.
pub(crate) struct ContentCounterStyles {
    pub(crate) tree_scope: u32,
    pub(crate) counter_styles: Vec<Option<Arc<CounterStyle>>>,
}

/// What a list marker whose `content` is `normal` shows, as the host resolved its `list-style-type` and
/// `list-style-image` when it built the marker box.
pub(crate) enum MarkerContent {
    Image,
    String(Vec<u16>),
    /// A counter style, or `None` for a name that resolves to none.
    CounterStyle(Option<Arc<CounterStyle>>),
}

pub(crate) struct MarkerContentStyles {
    pub(crate) tree_scope: u32,
    pub(crate) content: MarkerContent,
    pub(crate) text_depends_on_list_item_counter: bool,
}

/// What the host resolved for the generated content of each pseudo-element it built a box for, until
/// the tree build resolves that content, and the text each one's content resolved to.
#[derive(Default)]
pub(crate) struct GeneratedContent {
    content_counter_styles: HashMap<CounterOwner, ContentCounterStyles>,
    marker_content_styles: HashMap<CounterOwner, MarkerContentStyles>,
    accessible_texts: HashMap<CounterOwner, Vec<u16>>,
}

impl GeneratedContent {
    pub(crate) fn set_content_counter_styles(&mut self, owner: CounterOwner, styles: ContentCounterStyles) {
        self.content_counter_styles.insert(owner, styles);
    }

    pub(crate) fn set_marker_content_styles(&mut self, owner: CounterOwner, styles: MarkerContentStyles) {
        self.marker_content_styles.insert(owner, styles);
    }

    /// The text the content of `owner` last resolved to, the way accessibility reads it: the alt text
    /// when there is one, otherwise every string in order.
    pub(crate) fn accessible_text(&self, owner: CounterOwner) -> &[u16] {
        self.accessible_texts.get(&owner).map_or(&[], Vec::as_slice)
    }

    /// Drops everything kept for an element's pseudo-elements, once its identity is retired.
    pub(crate) fn forget(&mut self, element: StyleNodeID) {
        let belongs_to_element = |owner: &CounterOwner| owner.element != element;
        self.content_counter_styles.retain(|owner, _| belongs_to_element(owner));
        self.marker_content_styles.retain(|owner, _| belongs_to_element(owner));
        self.accessible_texts.retain(|owner, _| belongs_to_element(owner));
    }
}

/// One thing to put in a pseudo-element's box, in order.
pub(crate) enum ContentItem {
    Text(Vec<u16>),
    /// The `<image>` at this index of the `content` list.
    Image(usize),
    /// The marker box's `list-style-image`.
    ListStyleImage,
}

pub(crate) struct ResolvedContent {
    pub(crate) items: Vec<ContentItem>,
    /// Whether the content is a list of items, as opposed to `normal` or `none`.
    pub(crate) is_list: bool,
    pub(crate) final_quote_nesting_level: u32,
    /// Whether the content shows the value of the `list-item` counter, which the document tracks to
    /// know when renumbering a list changed what is rendered.
    pub(crate) renders_list_item_counter_value: bool,
}

fn representation(
    arena: &LayoutNodeArena,
    tree_scope: u32,
    counter_style: Option<Arc<CounterStyle>>,
    value: i32,
) -> Vec<u16> {
    arena.with_counter_style_registry(|registry| {
        generate_a_counter_representation(registry, tree_scope, counter_style, value)
    })
}

// https://drafts.csswg.org/css-lists-3/#text-markers
// "<counter-style>: Specifies the element's marker string as the value of the list-item counter
// represented using the specified <counter-style>. Specifically, the marker string is the result of
// generating a counter representation of the list-item counter value using the specified
// <counter-style>, prefixed by the prefix of the <counter-style>, and followed by the suffix of the
// <counter-style>. If the specified <counter-style> does not exist, decimal is assumed.
// <string>: The element's marker string is the specified <string>."
fn resolve_normal_marker_content(
    arena: &LayoutNodeArena,
    element: CounterOwner,
    styles: MarkerContentStyles,
) -> ContentItem {
    let counter_style = match styles.content {
        MarkerContent::Image => return ContentItem::ListStyleImage,
        MarkerContent::String(string) => {
            // NB: The value is used even when the marker shows none of it, as it instantiates the
            //     counter when it is missing.
            arena
                .counters_sets()
                .borrow_mut()
                .counter_value_for_use(element, &CounterName::Units(&LIST_ITEM_COUNTER_NAME));
            return ContentItem::Text(string);
        }
        MarkerContent::CounterStyle(counter_style) => counter_style,
    };
    let counter_value = arena
        .counters_sets()
        .borrow_mut()
        .counter_value_for_use(element, &CounterName::Units(&LIST_ITEM_COUNTER_NAME));
    let counter_representation = representation(arena, styles.tree_scope, counter_style.clone(), counter_value);
    let text = match counter_style {
        Some(counter_style) => {
            let mut text = counter_style.prefix.to_vec();
            text.extend_from_slice(&counter_representation);
            text.extend_from_slice(&counter_style.suffix);
            text
        }
        None => {
            let mut text = counter_representation;
            text.extend_from_slice(&[b'.' as u16, b' ' as u16]);
            text
        }
    };
    ContentItem::Text(text)
}

fn marker_renders_list_item_counter_value(styles: &MarkerContentStyles) -> bool {
    // NB: A marker showing its list-style-image shows no text at all.
    !matches!(styles.content, MarkerContent::Image) && styles.text_depends_on_list_item_counter
}

/// The marker string of a list marker a list-item pseudo-element nests, from what the host resolved
/// when it built the marker box.
pub(crate) fn resolve_nested_marker_content(arena: &LayoutNodeArena, element: CounterOwner) -> ResolvedContent {
    let styles = arena
        .generated_content()
        .borrow_mut()
        .marker_content_styles
        .remove(&element)
        .expect("a nested list marker has its content styles");
    let renders_list_item_counter_value = marker_renders_list_item_counter_value(&styles);
    ResolvedContent {
        items: vec![resolve_normal_marker_content(arena, element, styles)],
        is_list: true,
        final_quote_nesting_level: 0,
        renders_list_item_counter_value,
    }
}

enum QuotesData<'a> {
    None,
    Auto,
    Specified(&'a [crate::css::style_value::RetainedStyleValueData]),
}

fn quotes_data(value: Option<&StyleValueData>) -> QuotesData<'_> {
    match value {
        Some(StyleValueData::Keyword { keyword }) if *keyword == keyword::NONE => QuotesData::None,
        Some(StyleValueData::ValueList { values, .. }) => {
            let values = values.as_slice();
            assert!(values.len().is_multiple_of(2));
            QuotesData::Specified(values)
        }
        _ => QuotesData::Auto,
    }
}

fn quote_string<'a>(quotes: &QuotesData<'a>, open: bool, depth: u32) -> &'a [u16] {
    match quotes {
        QuotesData::None => &[],
        // FIXME: "A typographically appropriate used value for quotes is automatically chosen by the UA
        //        based on the content language of the element and/or its parent."
        QuotesData::Auto => match (open, depth) {
            (true, 0) => &[0x201C],
            (true, _) => &[0x2018],
            (false, 0) => &[0x201D],
            (false, _) => &[0x2019],
        },
        QuotesData::Specified(values) => {
            // If the depth is greater than the number of pairs, the last pair is repeated.
            let level = (depth as usize).min(values.len() / 2 - 1);
            let quote = &values[level * 2 + usize::from(!open)];
            match quote.optional_data() {
                Some(StyleValueData::String { string, .. }) => string.units(),
                _ => panic!("a quote is a string"),
            }
        }
    }
}

struct CounterItemResolver<'a> {
    arena: &'a LayoutNodeArena,
    element: CounterOwner,
    styles: ContentCounterStyles,
    next_counter_style: usize,
    renders_list_item_counter_value: bool,
}

impl CounterItemResolver<'_> {
    // counter( <counter-name>, <counter-style>? )
    // counters( <counter-name>, <string>, <counter-style>? )
    fn resolve(&mut self, function: u8, counter_name: &CssString, join_string: &CssString) -> Vec<u16> {
        if counter_name.units() == LIST_ITEM_COUNTER_NAME {
            self.renders_list_item_counter_value = true;
        }
        let counter_style = self.styles.counter_styles[self.next_counter_style].clone();
        self.next_counter_style += 1;

        // "If no counter named <counter-name> exists on an element where counter() or counters() is used,
        // one is first instantiated with a starting value of 0."
        let name = CounterName::Css(counter_name);

        // "Represents the value of the innermost counter in the element’s CSS counters set named <counter-name>
        // using the counter style named <counter-style>."
        if function == 0 {
            let value = self
                .arena
                .counters_sets()
                .borrow_mut()
                .counter_value_for_use(self.element, &name);
            return representation(self.arena, self.styles.tree_scope, counter_style, value);
        }

        // "Represents the values of all the counters in the element’s CSS counters set named <counter-name>
        // using the counter style named <counter-style>, sorted in outermost-first to innermost-last order
        // and joined by the specified <string>."
        let values = self
            .arena
            .counters_sets()
            .borrow_mut()
            .counter_values_for_use(self.element, &name);
        let mut result = Vec::new();
        for value in values {
            let counter_string = representation(self.arena, self.styles.tree_scope, counter_style.clone(), value);
            if !result.is_empty() {
                result.extend_from_slice(join_string.units());
            }
            result.extend_from_slice(&counter_string);
        }
        result
    }
}

fn value_list(value: Option<&StyleValueData>) -> &[crate::css::style_value::RetainedStyleValueData] {
    match value {
        Some(StyleValueData::ValueList { values, .. }) => values.as_slice(),
        _ => &[],
    }
}

/// Resolves the content of the pseudo-element `element` names, whose box the host has just built. A
/// list marker box whose `content` is `normal` shows its marker string instead.
pub(crate) fn resolve_content(
    arena: &LayoutNodeArena,
    element: CounterOwner,
    box_is_list_item_marker: bool,
    initial_quote_nesting_level: u32,
) -> ResolvedContent {
    arena.with_style_store(|engine| {
        let style = style_of(arena, engine, element).expect("a pseudo-element with a box has a style");
        let content = style.content_value();

        if box_is_list_item_marker
            && matches!(content, Some(StyleValueData::Keyword { keyword }) if *keyword == keyword::NORMAL)
        {
            let styles = arena
                .generated_content()
                .borrow_mut()
                .marker_content_styles
                .remove(&element)
                .expect("a list marker has its content styles");
            let renders_list_item_counter_value = marker_renders_list_item_counter_value(&styles);
            return ResolvedContent {
                items: vec![resolve_normal_marker_content(arena, element, styles)],
                is_list: true,
                final_quote_nesting_level: initial_quote_nesting_level,
                renders_list_item_counter_value,
            };
        }

        let Some(StyleValueData::Content {
            content: content_list,
            alt_text,
        }) = content
        else {
            arena.generated_content().borrow_mut().accessible_texts.remove(&element);
            return ResolvedContent {
                items: Vec::new(),
                is_list: false,
                final_quote_nesting_level: initial_quote_nesting_level,
                renders_list_item_counter_value: false,
            };
        };

        let styles = arena
            .generated_content()
            .borrow_mut()
            .content_counter_styles
            .remove(&element)
            .expect("a pseudo-element with content has its counter styles");
        let mut counters = CounterItemResolver {
            arena,
            element,
            styles,
            next_counter_style: 0,
            renders_list_item_counter_value: false,
        };
        let quotes = quotes_data(style.quotes_value());
        let mut quote_nesting_level = initial_quote_nesting_level;

        let mut items = Vec::new();
        let mut pending_text: Option<Vec<u16>> = None;
        let flush_pending_text = |items: &mut Vec<ContentItem>, pending_text: &mut Option<Vec<u16>>| {
            if let Some(text) = pending_text.take() {
                items.push(ContentItem::Text(text));
            }
        };

        for (index, item) in value_list(content_list.optional_data()).iter().enumerate() {
            match item.optional_data() {
                Some(StyleValueData::String { string, .. }) => {
                    pending_text.get_or_insert_default().extend_from_slice(string.units());
                }
                Some(StyleValueData::Keyword { keyword }) => match *keyword {
                    keyword::OPEN_QUOTE => {
                        let quote = quote_string(&quotes, true, quote_nesting_level);
                        quote_nesting_level += 1;
                        pending_text.get_or_insert_default().extend_from_slice(quote);
                    }
                    keyword::CLOSE_QUOTE => {
                        // A 'close-quote' or 'no-close-quote' that would make the depth negative is in error and is ignored
                        // (at rendering time): the depth stays at 0 and no quote mark is rendered (although the rest of the
                        // 'content' property's value is still inserted).
                        // - https://www.w3.org/TR/CSS21/generate.html#quotes-insert
                        // (This is missing from the CONTENT-3 spec.)
                        if quote_nesting_level > 0 {
                            quote_nesting_level -= 1;
                            let quote = quote_string(&quotes, false, quote_nesting_level);
                            pending_text.get_or_insert_default().extend_from_slice(quote);
                        }
                    }
                    keyword::NO_OPEN_QUOTE => quote_nesting_level += 1,
                    keyword::NO_CLOSE_QUOTE => {
                        // NOTE: See CloseQuote
                        quote_nesting_level = quote_nesting_level.saturating_sub(1);
                    }
                    _ => {}
                },
                Some(StyleValueData::Counter {
                    function,
                    counter_name,
                    join_string,
                    ..
                }) => {
                    flush_pending_text(&mut items, &mut pending_text);
                    items.push(ContentItem::Text(counters.resolve(
                        *function,
                        counter_name,
                        join_string,
                    )));
                }
                // https://drafts.csswg.org/css-content-3/#typedef-content-list
                // https://drafts.csswg.org/css-images-4/#typedef-image
                // <content-list> accepts <image>, and image-set() is an <image>.
                Some(StyleValueData::Image { .. } | StyleValueData::ImageSet { .. }) => {
                    flush_pending_text(&mut items, &mut pending_text);
                    items.push(ContentItem::Image(index));
                }
                // TODO: Implement images, and other things.
                _ => {}
            }
        }
        flush_pending_text(&mut items, &mut pending_text);

        let mut accessible_text = Vec::new();
        if let Some(alt_text) = alt_text.optional_data() {
            for item in value_list(Some(alt_text)) {
                match item.optional_data() {
                    Some(StyleValueData::String { string, .. }) => accessible_text.extend_from_slice(string.units()),
                    Some(StyleValueData::Counter {
                        function,
                        counter_name,
                        join_string,
                        ..
                    }) => accessible_text.extend(counters.resolve(*function, counter_name, join_string)),
                    _ => {}
                }
            }
        } else {
            for item in &items {
                if let ContentItem::Text(text) = item {
                    accessible_text.extend_from_slice(text);
                }
            }
        }
        arena
            .generated_content()
            .borrow_mut()
            .accessible_texts
            .insert(element, accessible_text);

        ResolvedContent {
            items,
            is_list: true,
            final_quote_nesting_level: quote_nesting_level,
            renders_list_item_counter_value: counters.renders_list_item_counter_value,
        }
    })
}
