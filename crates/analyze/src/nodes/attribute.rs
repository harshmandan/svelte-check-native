//! Attribute analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Attribute.ts`.

use svn_parser::Attribute;

/// Return the literal string value of a plain attribute `name="LITERAL"`,
/// or None if the attribute is absent, quoted with an expression
/// interpolation, or bound via `name={expr}`. Used for context-aware
/// bind dispatch (`<input type="number" bind:value={...}>`).
pub fn literal_attr_value<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    for attr in attrs {
        let Attribute::Plain(p) = attr else {
            continue;
        };
        if p.name.as_str() != name {
            continue;
        }
        let value = p.value.as_ref()?;
        // Require a single text part — reject interpolated values like
        // `type="my-{x}"` where we can't statically resolve the type.
        let [svn_parser::AttrValuePart::Text { content, .. }] = value.parts.as_slice() else {
            return None;
        };
        return Some(content.as_str());
    }
    None
}
