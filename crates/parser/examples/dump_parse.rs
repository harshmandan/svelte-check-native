//! Dump a JSON skeleton of our parse for one or more `.svelte` files.
//!
//! Consumed by `scripts/diff-parse.mjs`, which parses the same files with
//! the real `svelte/compiler` `parse()` and diffs the two skeletons. This
//! is an example target on purpose: zero impact on the shipped CLI
//! surface, built on demand via
//! `cargo build --release -p svn-parser --example dump_parse`.
//!
//! Output: one compact JSON object per input file, one per line (NDJSON):
//!
//! ```json
//! {"file":"...","module_script":{...}|null,"instance_script":{...}|null,
//!  "style":{...}|null,"errors":[{"code":"...","start":N,"end":N}],
//!  "template":[ ...nodes... ]}
//! ```
//!
//! The dump is FAITHFUL to our AST — no cross-parser normalization here.
//! All shape-mapping against the upstream modern AST lives in one place
//! (the .mjs side) so a normalization bug can't hide in two layers.
//! JSON is hand-written; the skeleton is flat enough that pulling serde
//! into the parser crate isn't warranted.

use std::fmt::Write as _;

use svn_parser::{
    Attribute, AwaitBlock, DirectiveValue, EachBlock, Fragment, IfBlock, InterpolationKind, Node,
    ScriptContext, ScriptLang, ScriptSection,
};

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: dump_parse <file.svelte>...");
        std::process::exit(2);
    }
    let mut out = String::new();
    for file in &files {
        out.clear();
        match std::fs::read_to_string(file) {
            Ok(source) => {
                dump_file(&mut out, file, &source);
                println!("{out}");
            }
            Err(err) => {
                println!(
                    "{{\"file\":{},\"read_error\":{}}}",
                    json_str(file),
                    json_str(&err.to_string())
                );
            }
        }
    }
}

fn dump_file(out: &mut String, file: &str, source: &str) {
    let (doc, mut errors) = svn_parser::parse_sections(source);
    let (fragment, template_errors) =
        svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
    errors.extend(template_errors);

    let _ = write!(out, "{{\"file\":{},", json_str(file));
    let _ = write!(out, "\"module_script\":");
    write_script(out, doc.module_script.as_ref());
    let _ = write!(out, ",\"instance_script\":");
    write_script(out, doc.instance_script.as_ref());
    match &doc.style {
        Some(s) => {
            let _ = write!(
                out,
                ",\"style\":{{\"start\":{},\"end\":{}}}",
                s.open_tag_range.start, s.close_tag_range.end
            );
        }
        None => {
            let _ = write!(out, ",\"style\":null");
        }
    }
    let _ = write!(out, ",\"errors\":[");
    for (i, e) in errors.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let r = e.range();
        let _ = write!(
            out,
            "{{\"code\":{},\"start\":{},\"end\":{}}}",
            json_str(e.code_slug()),
            r.start,
            r.end
        );
    }
    let _ = write!(out, "],\"template\":");
    write_fragment(out, &fragment);
    out.push('}');
}

fn write_script(out: &mut String, script: Option<&ScriptSection<'_>>) {
    match script {
        Some(s) => {
            let lang = match s.lang {
                ScriptLang::Js => "js",
                ScriptLang::Ts => "ts",
            };
            let context = match s.context {
                ScriptContext::Instance => "instance",
                ScriptContext::Module => "module",
            };
            let _ = write!(
                out,
                "{{\"start\":{},\"end\":{},\"lang\":\"{lang}\",\"context\":\"{context}\"}}",
                s.open_tag_range.start, s.close_tag_range.end
            );
        }
        None => {
            let _ = write!(out, "null");
        }
    }
}

fn write_fragment(out: &mut String, fragment: &Fragment) {
    out.push('[');
    for (i, node) in fragment.nodes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_node(out, node);
    }
    out.push(']');
}

fn write_opt_fragment(out: &mut String, fragment: Option<&Fragment>) {
    match fragment {
        Some(f) => write_fragment(out, f),
        None => {
            let _ = write!(out, "null");
        }
    }
}

fn write_node(out: &mut String, node: &Node) {
    match node {
        Node::Text(t) => {
            let _ = write!(
                out,
                "{{\"kind\":\"text\",\"start\":{},\"end\":{}}}",
                t.range.start, t.range.end
            );
        }
        Node::Interpolation(i) => {
            let tag = match i.kind {
                InterpolationKind::Expression => "expression",
                InterpolationKind::AtConst => "at_const",
                InterpolationKind::DeclConst => "decl_const",
                InterpolationKind::DeclLet => "decl_let",
                InterpolationKind::AtHtml => "at_html",
                InterpolationKind::AtRender => "at_render",
                InterpolationKind::AtDebug => "at_debug",
                InterpolationKind::AtTag => "at_tag",
            };
            let _ = write!(
                out,
                "{{\"kind\":\"interpolation\",\"tag\":\"{tag}\",\"start\":{},\"end\":{}}}",
                i.range.start, i.range.end
            );
        }
        Node::Comment(c) => {
            let _ = write!(
                out,
                "{{\"kind\":\"comment\",\"start\":{},\"end\":{}}}",
                c.range.start, c.range.end
            );
        }
        Node::Element(e) => {
            write_element(out, "element", &e.name, &e.attributes, &e.children, e.range);
        }
        Node::Component(c) => {
            write_element(
                out,
                "component",
                &c.name,
                &c.attributes,
                &c.children,
                c.range,
            );
        }
        Node::SvelteElement(se) => {
            let name = format!("svelte:{}", se.kind.as_str());
            write_element(
                out,
                "svelte_element",
                &name,
                &se.attributes,
                &se.children,
                se.range,
            );
        }
        Node::IfBlock(b) => write_if(out, b),
        Node::EachBlock(b) => write_each(out, b),
        Node::AwaitBlock(b) => write_await(out, b),
        Node::KeyBlock(b) => {
            let _ = write!(
                out,
                "{{\"kind\":\"key\",\"start\":{},\"end\":{},\"body\":",
                b.range.start, b.range.end
            );
            write_fragment(out, &b.body);
            out.push('}');
        }
        Node::SnippetBlock(b) => {
            let _ = write!(
                out,
                "{{\"kind\":\"snippet\",\"name\":{},\"start\":{},\"end\":{},\"body\":",
                json_str(&b.name),
                b.range.start,
                b.range.end
            );
            write_fragment(out, &b.body);
            out.push('}');
        }
    }
}

fn write_element(
    out: &mut String,
    kind: &str,
    name: &str,
    attributes: &[Attribute],
    children: &Fragment,
    range: svn_core::Range,
) {
    let _ = write!(
        out,
        "{{\"kind\":\"{kind}\",\"name\":{},\"start\":{},\"end\":{},\"attrs\":[",
        json_str(name),
        range.start,
        range.end
    );
    for (i, attr) in attributes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_attr(out, attr);
    }
    let _ = write!(out, "],\"children\":");
    write_fragment(out, children);
    out.push('}');
}

fn write_attr(out: &mut String, attr: &Attribute) {
    match attr {
        Attribute::Plain(a) => {
            let _ = write!(
                out,
                "{{\"kind\":\"attribute\",\"name\":{},\"start\":{},\"end\":{}}}",
                json_str(&a.name),
                a.range.start,
                a.range.end
            );
        }
        Attribute::Expression(a) => {
            let _ = write!(
                out,
                "{{\"kind\":\"attribute\",\"name\":{},\"start\":{},\"end\":{}}}",
                json_str(&a.name),
                a.range.start,
                a.range.end
            );
        }
        Attribute::Shorthand(a) => {
            let _ = write!(
                out,
                "{{\"kind\":\"attribute\",\"name\":{},\"start\":{},\"end\":{}}}",
                json_str(&a.name),
                a.range.start,
                a.range.end
            );
        }
        Attribute::Spread(a) => {
            let kind = if a.is_attach { "attach" } else { "spread" };
            let _ = write!(
                out,
                "{{\"kind\":\"{kind}\",\"start\":{},\"end\":{}}}",
                a.range.start, a.range.end
            );
        }
        Attribute::Directive(d) => {
            // `bind:foo={getter, setter}` is recorded so the normalizer
            // can decide how to compare against upstream's BindDirective.
            let pair = matches!(d.value, Some(DirectiveValue::BindPair { .. }));
            let _ = write!(
                out,
                "{{\"kind\":\"directive\",\"dir\":\"{}\",\"name\":{},\"pair\":{pair},\"start\":{},\"end\":{}}}",
                d.kind.as_str(),
                json_str(&d.name),
                d.range.start,
                d.range.end
            );
        }
        Attribute::Comment(c) => {
            let _ = write!(
                out,
                "{{\"kind\":\"comment\",\"start\":{},\"end\":{}}}",
                c.range.start, c.range.end
            );
        }
    }
}

fn write_if(out: &mut String, b: &IfBlock) {
    let _ = write!(
        out,
        "{{\"kind\":\"if\",\"start\":{},\"end\":{},\"consequent\":",
        b.range.start, b.range.end
    );
    write_fragment(out, &b.consequent);
    let _ = write!(out, ",\"elseif\":[");
    for (i, arm) in b.elseif_arms.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_fragment(out, &arm.body);
    }
    let _ = write!(out, "],\"alternate\":");
    write_opt_fragment(out, b.alternate.as_ref());
    out.push('}');
}

fn write_each(out: &mut String, b: &EachBlock) {
    let (has_context, has_index, has_key) = match &b.as_clause {
        Some(c) => (
            c.context_range.is_some(),
            c.index_range.is_some(),
            c.key_range.is_some(),
        ),
        None => (false, false, false),
    };
    let _ = write!(
        out,
        "{{\"kind\":\"each\",\"start\":{},\"end\":{},\"has_context\":{has_context},\
         \"has_index\":{has_index},\"has_key\":{has_key},\"body\":",
        b.range.start, b.range.end
    );
    write_fragment(out, &b.body);
    let _ = write!(out, ",\"alternate\":");
    write_opt_fragment(out, b.alternate.as_ref());
    out.push('}');
}

fn write_await(out: &mut String, b: &AwaitBlock) {
    let _ = write!(
        out,
        "{{\"kind\":\"await\",\"start\":{},\"end\":{},\"pending\":",
        b.range.start, b.range.end
    );
    write_opt_fragment(out, b.pending.as_ref());
    let _ = write!(out, ",\"then\":");
    match &b.then_branch {
        Some(t) => {
            let _ = write!(
                out,
                "{{\"has_context\":{},\"body\":",
                t.context_range.is_some()
            );
            write_fragment(out, &t.body);
            out.push('}');
        }
        None => {
            let _ = write!(out, "null");
        }
    }
    let _ = write!(out, ",\"catch\":");
    match &b.catch_branch {
        Some(c) => {
            let _ = write!(
                out,
                "{{\"has_context\":{},\"body\":",
                c.context_range.is_some()
            );
            write_fragment(out, &c.body);
            out.push('}');
        }
        None => {
            let _ = write!(out, "null");
        }
    }
    out.push('}');
}

/// Minimal JSON string escaping — quotes, backslashes, control chars.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
