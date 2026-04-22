//! Shared helpers for rule modules.

use svn_parser::ast::{AttrValuePart, Attribute};

/// Void HTML elements (no closing tag needed).
///
/// Reference: https://html.spec.whatwg.org/multipage/syntax.html#void-elements
pub fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// SVG element names (for the self-closing-tag rule).
///
/// Transcribed from upstream `utils.js::is_svg` — deliberately
/// narrower than the full SVG element set. Upstream EXCLUDES tags
/// like `<a>`, `<audio>`, `<canvas>`, `<iframe>`, `<image>`,
/// `<script>`, `<style>`, `<title>`, `<video>` which can appear
/// inside SVG but are typically HTML. The self-closing-tag rule
/// flags `<video … />` because the user most likely meant the HTML
/// form, not the SVG context.
pub fn is_svg_element(tag: &str) -> bool {
    matches!(
        tag,
        "altGlyph"
            | "altGlyphDef"
            | "altGlyphItem"
            | "animate"
            | "animateColor"
            | "animateMotion"
            | "animateTransform"
            | "circle"
            | "clipPath"
            | "color-profile"
            | "cursor"
            | "defs"
            | "desc"
            | "discard"
            | "ellipse"
            | "feBlend"
            | "feColorMatrix"
            | "feComponentTransfer"
            | "feComposite"
            | "feConvolveMatrix"
            | "feDiffuseLighting"
            | "feDisplacementMap"
            | "feDistantLight"
            | "feDropShadow"
            | "feFlood"
            | "feFuncA"
            | "feFuncB"
            | "feFuncG"
            | "feFuncR"
            | "feGaussianBlur"
            | "feImage"
            | "feMerge"
            | "feMergeNode"
            | "feMorphology"
            | "feOffset"
            | "fePointLight"
            | "feSpecularLighting"
            | "feSpotLight"
            | "feTile"
            | "feTurbulence"
            | "filter"
            | "font"
            | "font-face"
            | "font-face-format"
            | "font-face-name"
            | "font-face-src"
            | "font-face-uri"
            | "foreignObject"
            | "g"
            | "glyph"
            | "glyphRef"
            | "hatch"
            | "hatchpath"
            | "hkern"
            | "image"
            | "line"
            | "linearGradient"
            | "marker"
            | "mask"
            | "metadata"
            | "missing-glyph"
            | "mpath"
            | "path"
            | "pattern"
            | "polygon"
            | "polyline"
            | "radialGradient"
            | "rect"
            | "set"
            | "solidcolor"
            | "stop"
            | "svg"
            | "switch"
            | "symbol"
            | "text"
            | "textPath"
            | "tref"
            | "tspan"
            | "unknown"
            | "use"
            | "view"
            | "vkern"
    )
}

/// MathML element names.
pub fn is_mathml_element(tag: &str) -> bool {
    matches!(
        tag,
        "annotation"
            | "annotation-xml"
            | "maction"
            | "maligngroup"
            | "malignmark"
            | "math"
            | "menclose"
            | "merror"
            | "mfenced"
            | "mfrac"
            | "mglyph"
            | "mi"
            | "mlabeledtr"
            | "mlongdiv"
            | "mmultiscripts"
            | "mn"
            | "mo"
            | "mover"
            | "mpadded"
            | "mphantom"
            | "mprescripts"
            | "mroot"
            | "mrow"
            | "ms"
            | "mscarries"
            | "mscarry"
            | "msgroup"
            | "msline"
            | "mspace"
            | "msqrt"
            | "msrow"
            | "mstack"
            | "mstyle"
            | "msub"
            | "msubsup"
            | "msup"
            | "mtable"
            | "mtd"
            | "mtext"
            | "mtr"
            | "munder"
            | "munderover"
            | "none"
            | "semantics"
            | "square"
    )
}

/// Find a plain (name="value") attribute by name on an element.
pub fn find_plain_attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|a| match a {
        Attribute::Plain(p) => p.name.as_str() == name,
        Attribute::Expression(e) => e.name.as_str() == name,
        Attribute::Shorthand(s) => s.name.as_str() == name,
        _ => false,
    })
}

/// Get the literal-string value of a `name="value"` attribute, when
/// the value has exactly one text part and no interpolation.
pub fn plain_attr_text<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    for a in attrs {
        if let Attribute::Plain(p) = a
            && p.name.as_str() == name
        {
            let Some(v) = &p.value else {
                return None;
            };
            if v.parts.len() == 1
                && let AttrValuePart::Text { content, .. } = &v.parts[0]
            {
                return Some(content.as_str());
            }
        }
    }
    None
}
