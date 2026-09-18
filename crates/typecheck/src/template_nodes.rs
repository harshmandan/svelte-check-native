//! Questions the diagnostic filters ask about a `.svelte` source file.
//!
//! svelte-check decides some of its drops by looking at the Svelte
//! source rather than at the generated TypeScript: whether a mapped
//! diagnostic starts on an element's attribute name, whether the markup
//! is written in pug, and which names the instance script exports. The
//! answers come from the parsed source here, so the filters never guess
//! from the generated text.
//!
//! The file is parsed on demand. Only a handful of diagnostic codes ask
//! the attribute and export questions, and the pug question is only
//! worth a parse when the source mentions `<template` at all.

use svn_core::Range;
use svn_parser::{AttrValuePart, Attribute, DirectiveKind, DirectiveValue, Fragment, Node};
use svn_parser::{SvelteElementKind, parse_all_template_runs, parse_sections};

/// Does source byte `offset` sit on an element attribute's own node —
/// its name, `=`, or quotes — rather than inside its value?
///
/// svelte-check asks the legacy Svelte AST for the deepest node whose
/// inclusive `start..=end` span contains the position, and treats the
/// position as an attribute name when that node is an `Attribute` or an
/// `EventHandler` (`on:` directive) whose parent is an element-like node:
/// a regular element, `<svelte:element>`, `<svelte:body>` or
/// `<svelte:window>`. Components, `<slot>`, `<title>` inside
/// `<svelte:head>` and every other special element have their own node
/// types there and never count. Value parts are child nodes, so a
/// position inside `{…}` or inside the quoted text resolves to them.
pub(crate) fn is_element_attribute_name(source: &str, offset: u32) -> bool {
    let (doc, _) = parse_sections(source);
    let (fragment, _) = parse_all_template_runs(source, &doc.template.text_runs);
    fragment_has_attribute_name_at(&fragment, source, offset, false)
}

fn fragment_has_attribute_name_at(
    fragment: &Fragment,
    source: &str,
    offset: u32,
    in_head: bool,
) -> bool {
    fragment
        .nodes
        .iter()
        .any(|node| node_has_attribute_name_at(node, source, offset, in_head))
}

fn node_has_attribute_name_at(node: &Node, source: &str, offset: u32, in_head: bool) -> bool {
    if !contains(node.range(), offset) {
        return false;
    }
    match node {
        Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => false,
        Node::Element(e) => {
            let element_like = !(e.name == "slot" || (in_head && e.name == "title"));
            (element_like && attributes_have_name_at(&e.attributes, source, offset))
                || fragment_has_attribute_name_at(&e.children, source, offset, false)
        }
        Node::Component(c) => fragment_has_attribute_name_at(&c.children, source, offset, false),
        Node::SvelteElement(s) => {
            let element_like = matches!(
                s.kind,
                SvelteElementKind::Element | SvelteElementKind::Body | SvelteElementKind::Window
            );
            (element_like && attributes_have_name_at(&s.attributes, source, offset))
                || fragment_has_attribute_name_at(
                    &s.children,
                    source,
                    offset,
                    s.kind == SvelteElementKind::Head,
                )
        }
        Node::IfBlock(b) => {
            fragment_has_attribute_name_at(&b.consequent, source, offset, false)
                || b.elseif_arms
                    .iter()
                    .any(|arm| fragment_has_attribute_name_at(&arm.body, source, offset, false))
                || b.alternate
                    .as_ref()
                    .is_some_and(|f| fragment_has_attribute_name_at(f, source, offset, false))
        }
        Node::EachBlock(b) => {
            fragment_has_attribute_name_at(&b.body, source, offset, false)
                || b.alternate
                    .as_ref()
                    .is_some_and(|f| fragment_has_attribute_name_at(f, source, offset, false))
        }
        Node::AwaitBlock(b) => {
            b.pending
                .as_ref()
                .is_some_and(|f| fragment_has_attribute_name_at(f, source, offset, false))
                || b.then_branch
                    .as_ref()
                    .is_some_and(|t| fragment_has_attribute_name_at(&t.body, source, offset, false))
                || b.catch_branch
                    .as_ref()
                    .is_some_and(|c| fragment_has_attribute_name_at(&c.body, source, offset, false))
        }
        Node::KeyBlock(b) => fragment_has_attribute_name_at(&b.body, source, offset, false),
        Node::SnippetBlock(b) => fragment_has_attribute_name_at(&b.body, source, offset, false),
    }
}

fn attributes_have_name_at(attributes: &[Attribute], source: &str, offset: u32) -> bool {
    attributes.iter().any(|attr| {
        let (range, children) = match attr {
            Attribute::Plain(p) => (
                p.range,
                p.value
                    .as_ref()
                    .map(|v| v.parts.iter().map(value_part_range).collect())
                    .unwrap_or_default(),
            ),
            // The value is one mustache tag, braces included.
            Attribute::Expression(e) => (e.range, vec![braced(source, e.expression_range)]),
            // `{name}` is an attribute whose value node is the name
            // inside the braces.
            Attribute::Shorthand(s) => (
                s.range,
                vec![Range::new(
                    s.range.start.saturating_add(1).min(s.range.end),
                    s.range.end.saturating_sub(1).max(s.range.start),
                )],
            ),
            // An `on:` directive's child is its expression itself.
            Attribute::Directive(d) if d.kind == DirectiveKind::On => (
                d.range,
                match &d.value {
                    None => Vec::new(),
                    Some(DirectiveValue::Expression {
                        expression_range, ..
                    }) => vec![*expression_range],
                    Some(DirectiveValue::BindPair { range, .. }) => vec![*range],
                    Some(DirectiveValue::Quoted(v)) => {
                        v.parts.iter().map(value_part_range).collect()
                    }
                },
            ),
            Attribute::Directive(_) | Attribute::Spread(_) | Attribute::Comment(_) => {
                return false;
            }
        };
        contains(range, offset) && !children.iter().any(|&c| contains(c, offset))
    })
}

fn value_part_range(part: &AttrValuePart) -> Range {
    match part {
        AttrValuePart::Text { range } => *range,
        AttrValuePart::Expression { range, .. } => *range,
    }
}

/// Widen an attribute expression's range to the `{`…`}` around it.
fn braced(source: &str, expression: Range) -> Range {
    let bytes = source.as_bytes();
    let start = bytes[..(expression.start as usize).min(bytes.len())]
        .iter()
        .rposition(|&b| b == b'{')
        .map_or(expression.start, |i| i as u32);
    let end = bytes
        .get(expression.end as usize..)
        .and_then(|tail| tail.iter().position(|&b| b == b'}'))
        .map_or(expression.end, |i| expression.end + i as u32 + 1);
    Range::new(start.min(expression.start), end.max(expression.end))
}

/// The legacy AST's containment test: both ends inclusive.
fn contains(range: Range, offset: u32) -> bool {
    range.start <= offset && offset <= range.end
}

/// The content range of the file's markup-language `<template>` tag when
/// that language is pug.
///
/// svelte-check takes the first top-level `<template>` tag — one that is
/// not inside a control-flow block or an `{@html}` expression — and reads
/// its language from `lang`, falling back to `type`, with a leading
/// `text/` stripped. A valueless attribute reads as its own name. The
/// tag's range is its content: from the end of the start tag to the start
/// of the end tag (the element's end when it is never closed).
pub(crate) fn pug_template_content(source: &str) -> Option<(u32, u32)> {
    if !source.contains("<template") {
        return None;
    }
    let (doc, _) = parse_sections(source);
    let (fragment, _) = parse_all_template_runs(source, &doc.template.text_runs);
    let template = fragment.nodes.iter().find_map(|node| match node {
        Node::Element(e) if e.name == "template" => Some(e),
        _ => None,
    })?;
    let attr_value = |name: &'static str| -> Option<&str> {
        template.attributes.iter().find_map(|attr| match attr {
            Attribute::Plain(p) if p.name == name => match &p.value {
                None => Some(name),
                Some(v) => source
                    .get(v.range.start as usize..v.range.end as usize)
                    .map(strip_outer_quotes),
            },
            _ => None,
        })
    };
    let lang = attr_value("lang")
        .filter(|v| !v.is_empty())
        .or_else(|| attr_value("type"))
        .unwrap_or("");
    if lang.strip_prefix("text/").unwrap_or(lang) != "pug" {
        return None;
    }
    let content = template.children.range;
    let start_tag_end = source
        .get(template.range.start as usize..)
        .and_then(|tail| tail.find('>'))
        .map_or(content.start, |i| template.range.start + i as u32 + 1);
    let end = source
        .get(start_tag_end as usize..template.range.end as usize)
        .and_then(|body| body.rfind("</template"))
        .map_or(template.range.end, |i| start_tag_end + i as u32);
    Some((start_tag_end, end))
}

/// Does the instance script export `name`?
///
/// svelte-check keys its exported-names map by the LOCAL name of every
/// top-level export of the instance script: each identifier bound by an
/// `export let` / `export const` / `export var` pattern, the name of an
/// `export function` / `export class`, and the local side of every
/// `export { … }` specifier (`a` in `export { a as b }`). Type-only
/// declarations (`export type`, `export interface`) are not entered.
pub(crate) fn instance_script_exports(source: &str, name: &str) -> bool {
    use oxc_ast::ast::{Declaration, Statement};

    let (doc, _) = parse_sections(source);
    let Some(script) = doc.instance_script.as_ref() else {
        return false;
    };
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, script.content, script.lang);
    let specifier_named = |specs: &[oxc_ast::ast::ExportSpecifier<'_>]| {
        specs.iter().any(|spec| spec.local.name() == name)
    };
    parsed.program.body.iter().any(|stmt| match stmt {
        Statement::ExportNamedDeclaration(export) => specifier_named(&export.specifiers),
        Statement::ExportFromDeclaration(export) => specifier_named(&export.specifiers),
        Statement::ExportDeclaration(export) => match &export.declaration {
            Declaration::VariableDeclaration(var) => var.declarations.iter().any(|d| {
                d.id.get_binding_identifiers()
                    .iter()
                    .any(|id| id.name == name)
            }),
            Declaration::FunctionDeclaration(f) => f.id.as_ref().is_some_and(|id| id.name == name),
            Declaration::ClassDeclaration(c) => c.id.as_ref().is_some_and(|id| id.name == name),
            _ => false,
        },
        _ => false,
    })
}

fn strip_outer_quotes(raw: &str) -> &str {
    for q in ['"', '\''] {
        if raw.len() >= 2 && raw.starts_with(q) && raw.ends_with(q) {
            return &raw[1..raw.len() - 1];
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(source: &str, needle: &str) -> u32 {
        source.find(needle).expect("needle present") as u32
    }

    #[test]
    fn attribute_names_count_but_values_do_not() {
        let src = r#"<div on:click={f} on:click title={({ "a": 1 })} class="x"></div>"#;
        assert!(is_element_attribute_name(src, at(src, "on:click")));
        assert!(is_element_attribute_name(src, at(src, "on:click title")));
        assert!(is_element_attribute_name(src, at(src, "class")));
        assert!(!is_element_attribute_name(src, at(src, "\"a\"")));
        assert!(!is_element_attribute_name(src, at(src, "x\"")));
        assert!(!is_element_attribute_name(src, at(src, "f}")));
    }

    #[test]
    fn components_and_mustaches_never_count() {
        let src = r#"<Comp title="x" /><div>{({ "a": 1 })}</div>"#;
        assert!(!is_element_attribute_name(src, at(src, "title")));
        assert!(!is_element_attribute_name(src, at(src, "\"a\"")));
    }

    #[test]
    fn nested_elements_and_special_elements() {
        let src = "{#if x}<svelte:window on:keydown /><span id=\"a\"></span>{/if}<svelte:head><title id=\"t\"></title></svelte:head>";
        assert!(is_element_attribute_name(src, at(src, "on:keydown")));
        assert!(is_element_attribute_name(src, at(src, "id=\"a\"")));
        assert!(!is_element_attribute_name(src, at(src, "id=\"t\"")));
    }

    #[test]
    fn instance_script_export_names() {
        let src = "<script context=\"module\">export let m = 1;</script>\n<script lang=\"ts\">\n  export let a: number;\n  export const { b, c: d } = o;\n  export function f() {}\n  let x = 1;\n  export { x as y };\n  export type T = 1;\n</script>";
        for name in ["a", "b", "d", "f", "x"] {
            assert!(instance_script_exports(src, name), "{name}");
        }
        for name in ["c", "y", "T", "m", "z"] {
            assert!(!instance_script_exports(src, name), "{name}");
        }
    }

    #[test]
    fn pug_template_language_and_content_range() {
        let src = "<script></script>\n<template type=\"text/pug\">\n  div\n</template>\n";
        let (start, end) = pug_template_content(src).expect("pug");
        assert_eq!(&src[start as usize..end as usize], "\n  div\n");
        assert!(pug_template_content("<template lang=\"pug\">p</template>").is_some());
        assert!(pug_template_content("<template lang='markup'>p</template>").is_none());
        assert!(pug_template_content("{@html '<template lang=\"pug\">'}<div></div>").is_none());
        assert!(pug_template_content("{#if a}<template lang=\"pug\">p</template>{/if}").is_none());
    }
}
