//! Template identifier-reference collection.
//!
//! Walks every expression-bearing position in the template fragment —
//! interpolations, expression attributes, attribute-value parts, spreads,
//! directive values, control-flow conditions, each-iterables, key-block
//! expressions, await promises — and extracts the *root* identifiers
//! referenced.
//!
//! The component-tag name of every `<Component />` is also collected
//! (root identifier before any `.`), since a component invocation in the
//! template is a value reference to the imported component binding.
//!
//! ### Why this exists
//!
//! Without this pass, a script-level import or local that's only used in
//! the template (e.g. `<MyButton />` or `{count}`) is flagged as
//! TS6133 ("declared but never read"): our wrapper compiles the script
//! body inside `function $$render() { ... }` but doesn't spell out a
//! single use of those names. Voiding template references inside the
//! same function brings them back into the "used" set.
//!
//! ### Why the byte-scanner over a per-expression oxc parse
//!
//! Most templates have dozens to hundreds of expression sites. Spinning
//! up an oxc parse per site is the obvious approach but it's measurably
//! slow on large component sets (the benchmark project: ~1300 components, many
//! hundreds of expressions each).
//!
//! Here we use a simple JS tokenizer: skip strings, template literals,
//! regex literals, and comments; collect every identifier not preceded
//! by `.` or `?.` (so `obj.prop` only yields `obj`, not `prop`). The
//! scanner is intentionally lenient — it may collect a few false
//! positives (e.g. a property key in `{ key: value }`), but consumers
//! always intersect with the script's declared bindings, so a name not
//! actually in scope just gets dropped.
//!
//! Reserved words and the auto-subscribe `$store` syntax are filtered
//! out at the byte-scanner level (we only collect names that look like
//! identifiers; the `$store` form goes through [`crate::store`]).
//!
//! ### Why we don't try to track scope in the template
//!
//! `{#each items as item}{item}{/each}` introduces `item` as a local in
//! the body. A pedantic implementation would skip `item` when collecting
//! refs from the body. We don't — `item` is unlikely to also be a
//! script-level binding name, so the intersection step in emit drops it
//! naturally. If a user happens to name a local the same as an each-bind,
//! we may emit a redundant `void item;`, which is harmless.

use std::collections::HashSet;

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{
    Attribute, AttrValuePart, Directive, DirectiveKind, DirectiveValue, Fragment, Node,
};

/// Find every root identifier referenced in the template fragment.
///
/// Returns names in source order of first occurrence, deduplicated.
/// Filtering by which of these are actually script-declared is the
/// caller's responsibility.
pub fn find_template_refs(fragment: &Fragment, source: &str) -> Vec<SmolStr> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    walk_fragment(fragment, source, &mut seen, &mut out);
    out
}

fn walk_fragment(
    fragment: &Fragment,
    source: &str,
    seen: &mut HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    for node in &fragment.nodes {
        walk_node(node, source, seen, out);
    }
}

fn walk_node(
    node: &Node,
    source: &str,
    seen: &mut HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    match node {
        Node::Element(e) => {
            walk_attributes(&e.attributes, source, seen, out);
            walk_fragment(&e.children, source, seen, out);
        }
        Node::Component(c) => {
            // `<MyButton />` and `<ui.MyButton />` — the root identifier is
            // a value reference to the imported binding.
            push_ident(component_root(&c.name), seen, out);
            walk_attributes(&c.attributes, source, seen, out);
            walk_fragment(&c.children, source, seen, out);
        }
        Node::SvelteElement(s) => {
            walk_attributes(&s.attributes, source, seen, out);
            walk_fragment(&s.children, source, seen, out);
        }
        Node::Interpolation(i) => extract_idents(source, i.expression_range, seen, out),
        Node::IfBlock(b) => {
            extract_idents(source, b.condition_range, seen, out);
            walk_fragment(&b.consequent, source, seen, out);
            for arm in &b.elseif_arms {
                extract_idents(source, arm.condition_range, seen, out);
                walk_fragment(&arm.body, source, seen, out);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, source, seen, out);
            }
        }
        Node::EachBlock(b) => {
            extract_idents(source, b.expression_range, seen, out);
            if let Some(c) = &b.as_clause {
                if let Some(k) = c.key_range {
                    extract_idents(source, k, seen, out);
                }
            }
            walk_fragment(&b.body, source, seen, out);
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, source, seen, out);
            }
        }
        Node::AwaitBlock(b) => {
            extract_idents(source, b.expression_range, seen, out);
            if let Some(p) = &b.pending {
                walk_fragment(p, source, seen, out);
            }
            if let Some(t) = &b.then_branch {
                walk_fragment(&t.body, source, seen, out);
            }
            if let Some(c) = &b.catch_branch {
                walk_fragment(&c.body, source, seen, out);
            }
        }
        Node::KeyBlock(b) => {
            extract_idents(source, b.expression_range, seen, out);
            walk_fragment(&b.body, source, seen, out);
        }
        Node::SnippetBlock(b) => walk_fragment(&b.body, source, seen, out),
        Node::Text(_) | Node::Comment(_) => {}
    }
}

fn walk_attributes(
    attrs: &[Attribute],
    source: &str,
    seen: &mut HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    for attr in attrs {
        match attr {
            Attribute::Plain(p) => {
                if let Some(v) = &p.value {
                    for part in &v.parts {
                        if let AttrValuePart::Expression { expression_range, .. } = part {
                            extract_idents(source, *expression_range, seen, out);
                        }
                    }
                }
            }
            Attribute::Expression(e) => extract_idents(source, e.expression_range, seen, out),
            Attribute::Shorthand(s) => push_ident(&s.name, seen, out),
            Attribute::Spread(s) => extract_idents(source, s.expression_range, seen, out),
            Attribute::Directive(d) => walk_directive(d, source, seen, out),
        }
    }
}

fn walk_directive(
    d: &Directive,
    source: &str,
    seen: &mut HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    // For directives where the name itself is a value reference (action,
    // transition, animation), record it. For the others (`on:click`,
    // `class:active`, `style:left`) the name is an event/CSS-name and
    // never refers to a script binding.
    let name_is_ref = matches!(
        d.kind,
        DirectiveKind::Use
            | DirectiveKind::Transition
            | DirectiveKind::In
            | DirectiveKind::Out
            | DirectiveKind::Animate
    );
    if name_is_ref {
        push_ident(&d.name, seen, out);
    }

    match &d.value {
        Some(DirectiveValue::Expression { expression_range, .. }) => {
            extract_idents(source, *expression_range, seen, out);
        }
        Some(DirectiveValue::BindPair { getter_range, setter_range, .. }) => {
            extract_idents(source, *getter_range, seen, out);
            extract_idents(source, *setter_range, seen, out);
        }
        Some(DirectiveValue::Quoted(v)) => {
            for part in &v.parts {
                if let AttrValuePart::Expression { expression_range, .. } = part {
                    extract_idents(source, *expression_range, seen, out);
                }
            }
        }
        None => {
            // Shorthand expansions:
            //   - `bind:value`     → `bind:value={value}`     — value is local
            //   - `class:active`   → `class:active={active}`  — active is local
            //
            // For `on:click`, `style:left`, `let:foo` the name is an
            // event/CSS-name/new-binding, NOT a script-level reference.
            let bare_is_shorthand = matches!(d.kind, DirectiveKind::Bind | DirectiveKind::Class);
            if bare_is_shorthand {
                push_ident(&d.name, seen, out);
            }
        }
    }
}

fn component_root(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

fn push_ident(name: &str, seen: &mut HashSet<SmolStr>, out: &mut Vec<SmolStr>) {
    if !is_valid_ident(name) || is_keyword(name) {
        return;
    }
    let s = SmolStr::from(name);
    if seen.insert(s.clone()) {
        out.push(s);
    }
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    if !is_ident_start(first) {
        return false;
    }
    chars.all(is_ident_continue)
}

#[inline]
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

#[inline]
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// JS-ish reserved words and built-ins that should never be voided as
/// references. Excludes `this` etc. — those aren't valid identifiers
/// anyway, but explicit filtering keeps intent clear.
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "void"
            | "typeof"
            | "new"
            | "instanceof"
            | "in"
            | "of"
            | "as"
            | "let"
            | "const"
            | "var"
            | "function"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "return"
            | "yield"
            | "await"
            | "async"
            | "delete"
            | "throw"
            | "try"
            | "catch"
            | "finally"
            | "switch"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "class"
            | "extends"
            | "super"
            | "import"
            | "export"
            | "from"
            | "satisfies"
    )
}

/// Byte-scan an expression range, collecting root identifiers.
///
/// Skips string literals, template literals (recursing into `${...}`),
/// regex literals, and comments. Suppresses identifiers preceded by `.`
/// or `?.` so `obj.prop` only yields `obj`.
fn extract_idents(
    source: &str,
    range: Range,
    seen: &mut HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    let Some(slice) = source.get(range.start as usize..range.end as usize) else {
        return;
    };
    let bytes = slice.as_bytes();
    let mut i = 0;
    let mut after_dot = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Line comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
            continue;
        }
        // String literal.
        if b == b'"' || b == b'\'' {
            let q = b;
            i += 1;
            while i < bytes.len() && bytes[i] != q {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            after_dot = false;
            continue;
        }
        // Template literal.
        if b == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    i += 2;
                    let inner_start = i;
                    let mut depth = 1usize;
                    while i < bytes.len() {
                        match bytes[i] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let inner_range = Range::new(
                        range.start + inner_start as u32,
                        range.start + i as u32,
                    );
                    extract_idents(source, inner_range, seen, out);
                    if i < bytes.len() {
                        i += 1; // past `}`
                    }
                } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            after_dot = false;
            continue;
        }
        // Identifier-like start.
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
                    i += 1;
                } else {
                    break;
                }
            }
            let name = &slice[start..i];
            if !after_dot {
                push_ident(name, seen, out);
            }
            after_dot = false;
            continue;
        }
        // Member access — suppress the next ident.
        if b == b'.' {
            after_dot = true;
            i += 1;
            continue;
        }
        // Anything else clears member-access context (except whitespace).
        if !b.is_ascii_whitespace() {
            after_dot = false;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svn_parser::{parse_all_template_runs, parse_sections};

    fn refs_in(src: &str) -> Vec<String> {
        let (doc, errors) = parse_sections(src);
        assert!(errors.is_empty(), "section errors: {errors:?}");
        let (frag, errors) = parse_all_template_runs(src, &doc.template.text_runs);
        assert!(errors.is_empty(), "template errors: {errors:?}");
        find_template_refs(&frag, src)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn empty_template() {
        assert!(refs_in("").is_empty());
    }

    #[test]
    fn interpolation_collects_root_identifier() {
        assert_eq!(refs_in("<p>{count}</p>"), vec!["count"]);
    }

    #[test]
    fn member_access_only_collects_root() {
        assert_eq!(refs_in("<p>{user.name}</p>"), vec!["user"]);
    }

    #[test]
    fn optional_chaining_only_collects_root() {
        assert_eq!(refs_in("<p>{user?.name}</p>"), vec!["user"]);
    }

    #[test]
    fn component_tag_collected() {
        let r = refs_in("<MyButton />");
        assert!(r.contains(&"MyButton".to_string()));
    }

    #[test]
    fn dotted_component_collects_root() {
        let r = refs_in("<ui.Button />");
        assert!(r.contains(&"ui".to_string()));
        assert!(!r.contains(&"Button".to_string()));
    }

    #[test]
    fn shorthand_attribute_collected() {
        // `<div {foo} />` is shorthand for `foo={foo}`.
        let r = refs_in("<div {foo} />");
        assert!(r.contains(&"foo".to_string()));
    }

    #[test]
    fn expression_attribute_collected() {
        let r = refs_in("<div title={greeting} />");
        assert!(r.contains(&"greeting".to_string()));
    }

    #[test]
    fn spread_attribute_collected() {
        let r = refs_in("<div {...rest} />");
        assert!(r.contains(&"rest".to_string()));
    }

    #[test]
    fn quoted_attr_with_interpolation() {
        // The structural parser currently emits a single Text part for
        // quoted attribute values and doesn't lift `{...}` interpolations
        // inside them into expression parts. Once the attribute scanner
        // recognizes interpolations within quoted values, this test should
        // assert `r.contains("bar")`. For now the documented behavior is:
        // no idents collected from inside quoted strings.
        let r = refs_in(r#"<div class="foo {bar} baz" />"#);
        assert!(!r.contains(&"bar".to_string()));
    }

    #[test]
    fn directive_expression_collected() {
        let r = refs_in("<input bind:value={inputValue} />");
        assert!(r.contains(&"inputValue".to_string()));
    }

    #[test]
    fn bind_pair_collects_both_sides() {
        let r = refs_in("<input bind:value={() => g(), (v) => s(v)} />");
        assert!(r.contains(&"g".to_string()));
        assert!(r.contains(&"s".to_string()));
        assert!(r.contains(&"v".to_string()));
    }

    #[test]
    fn bare_directive_collects_name() {
        // `bind:value` (no `={...}`) is shorthand for `bind:value={value}`.
        let r = refs_in("<input bind:value />");
        assert!(r.contains(&"value".to_string()));
    }

    #[test]
    fn use_directive_with_arg() {
        let r = refs_in("<div use:tooltip={text} />");
        assert!(r.contains(&"tooltip".to_string()));
        assert!(r.contains(&"text".to_string()));
    }

    #[test]
    fn if_condition_collected() {
        let r = refs_in("{#if showThing}<p>x</p>{/if}");
        assert!(r.contains(&"showThing".to_string()));
    }

    #[test]
    fn elseif_condition_collected() {
        let r = refs_in("{#if a}<p/>{:else if b}<p/>{/if}");
        assert!(r.contains(&"a".to_string()));
        assert!(r.contains(&"b".to_string()));
    }

    #[test]
    fn each_iterable_collected() {
        let r = refs_in("{#each items as item}<p>{item}</p>{/each}");
        assert!(r.contains(&"items".to_string()));
    }

    #[test]
    fn await_promise_collected() {
        let r = refs_in("{#await fetchUser()}<p>...</p>{/await}");
        assert!(r.contains(&"fetchUser".to_string()));
    }

    #[test]
    fn key_block_expression_collected() {
        let r = refs_in("{#key trigger}<p />{/key}");
        assert!(r.contains(&"trigger".to_string()));
    }

    #[test]
    fn dedupe_preserves_first_occurrence_order() {
        let r = refs_in("<p>{a} {b} {a}</p>");
        assert_eq!(r, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn string_literal_contents_skipped() {
        let r = refs_in(r#"<p>{"foo bar baz"}</p>"#);
        assert!(r.is_empty());
    }

    #[test]
    fn template_literal_contents_skipped_but_substitutions_walked() {
        let r = refs_in("<p>{`hello ${name} world`}</p>");
        assert!(r.contains(&"name".to_string()));
        assert!(!r.contains(&"hello".to_string()));
    }

    #[test]
    fn comment_in_expression_skipped() {
        // The structural mustache scanner sees `{/` and treats it as a
        // closing block tag start, so we test the byte scanner directly
        // on a constructed range instead.
        let src = "/* commentedOut */ realRef";
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        extract_idents(src, Range::new(0, src.len() as u32), &mut seen, &mut out);
        let r: Vec<String> = out.into_iter().map(|s| s.to_string()).collect();
        assert!(r.contains(&"realRef".to_string()));
        assert!(!r.contains(&"commentedOut".to_string()));
    }

    #[test]
    fn keyword_not_collected() {
        let r = refs_in("<p>{typeof x}</p>");
        assert!(!r.contains(&"typeof".to_string()));
        assert!(r.contains(&"x".to_string()));
    }

    #[test]
    fn dollar_store_ref_collected() {
        // `$count` looks like an identifier-with-leading-dollar; we DO
        // collect it, the caller's intersect step decides whether the
        // store-alias declaration covers it.
        let r = refs_in("<p>{$count}</p>");
        assert!(r.contains(&"$count".to_string()));
    }

    #[test]
    fn object_literal_property_value_collected() {
        let r = refs_in("<p>{getThing({ key: someValue })}</p>");
        assert!(r.contains(&"getThing".to_string()));
        assert!(r.contains(&"someValue".to_string()));
    }

    #[test]
    fn nested_in_block_walked() {
        let r = refs_in("{#if cond}<MyButton onclick={handler} />{/if}");
        assert!(r.contains(&"cond".to_string()));
        assert!(r.contains(&"MyButton".to_string()));
        assert!(r.contains(&"handler".to_string()));
    }

    #[test]
    fn each_body_idents_collected() {
        let r = refs_in("{#each items as item}<p>{item.label}</p>{/each}");
        assert!(r.contains(&"items".to_string()));
        assert!(r.contains(&"item".to_string()));
    }
}
