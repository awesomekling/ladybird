// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

//! Constants for HTML, SVG, and MathML tag names and attribute names.
//!
//! These replace raw string literals in the parser and stack of open elements,
//! providing compile-time typo prevention. A misspelled constant like
//! `tags::TEMLPATE` is a compile error; a misspelled `"temlpate"` is a silent bug.
//!
//! Note: Some tag names still appear as string literals inside `matches!()` and
//! `match` arms, since Rust requires literal patterns in those positions.

/// HTML, SVG, and MathML tag name constants.
pub mod tags {
    // HTML tag names
    pub const A: &str = "a";
    pub const BODY: &str = "body";
    pub const BR: &str = "br";
    pub const BUTTON: &str = "button";
    pub const CAPTION: &str = "caption";
    pub const COL: &str = "col";
    pub const COLGROUP: &str = "colgroup";
    pub const DD: &str = "dd";
    pub const DT: &str = "dt";
    pub const FONT: &str = "font";
    pub const FORM: &str = "form";
    pub const FRAMESET: &str = "frameset";
    pub const H1: &str = "h1";
    pub const H2: &str = "h2";
    pub const H3: &str = "h3";
    pub const H4: &str = "h4";
    pub const H5: &str = "h5";
    pub const H6: &str = "h6";
    pub const HEAD: &str = "head";
    pub const HR: &str = "hr";
    pub const HTML: &str = "html";
    pub const IFRAME: &str = "iframe";
    pub const IMAGE: &str = "image";
    pub const IMG: &str = "img";
    pub const INPUT: &str = "input";
    pub const LI: &str = "li";
    pub const META: &str = "meta";
    pub const NOBR: &str = "nobr";
    pub const NOEMBED: &str = "noembed";
    pub const NOFRAMES: &str = "noframes";
    pub const NOSCRIPT: &str = "noscript";
    pub const OPTGROUP: &str = "optgroup";
    pub const OPTION: &str = "option";
    pub const P: &str = "p";
    pub const PLAINTEXT: &str = "plaintext";
    pub const RTC: &str = "rtc";
    pub const RUBY: &str = "ruby";
    pub const SCRIPT: &str = "script";
    pub const SELECT: &str = "select";
    pub const TABLE: &str = "table";
    pub const TBODY: &str = "tbody";
    pub const TD: &str = "td";
    pub const TEMPLATE: &str = "template";
    pub const TEXTAREA: &str = "textarea";
    pub const TFOOT: &str = "tfoot";
    pub const TH: &str = "th";
    pub const THEAD: &str = "thead";
    pub const TITLE: &str = "title";
    pub const TR: &str = "tr";
    pub const XMP: &str = "xmp";

    // SVG tag names
    pub const MATH: &str = "math";
    pub const SVG: &str = "svg";

    // MathML tag names
    pub const ANNOTATION_XML: &str = "annotation-xml";
    pub const MALIGNMARK: &str = "malignmark";
    pub const MGLYPH: &str = "mglyph";
}

/// Attribute name constants used in the parser.
pub mod attrs {
    pub const COLOR: &str = "color";
    pub const ENCODING: &str = "encoding";
    pub const FACE: &str = "face";
    pub const IS: &str = "is";
    pub const SHADOWROOTCLONABLE: &str = "shadowrootclonable";
    pub const SHADOWROOTDELEGATESFOCUS: &str = "shadowrootdelegatesfocus";
    pub const SHADOWROOTMODE: &str = "shadowrootmode";
    pub const SHADOWROOTSERIALIZABLE: &str = "shadowrootserializable";
    pub const SIZE: &str = "size";
    pub const TYPE: &str = "type";
}
