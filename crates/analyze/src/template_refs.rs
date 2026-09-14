//! Template expression sites.
//!
//! Walks every expression-bearing position in the template fragment —
//! interpolations, expression attributes, attribute-value parts, spreads,
//! directive values, control-flow conditions, each-iterables, key-block
//! expressions, await promises — and hands each one to a sink as a byte
//! range (or as a bare name for component tags, directive names and
//! shorthand attributes).
//!
//! Consumers parse the ranges with oxc. Nothing here reads the
//! expression text itself: identifiers, store subscriptions and rune
//! calls all come from the parsed expression, the way upstream
//! svelte2tsx walks the template AST.

use svn_core::Range;
use svn_parser::{AttrValuePart, Attribute, Directive, DirectiveValue, Fragment, Node};

/// Every expression-bearing byte range in the template fragment —
/// the positions `find_template_refs` reads identifiers from — in
/// source order. A declaration payload (`{@const a = b}`, `{let a =
/// b}`) is flagged: its text is a declarator list, not an expression.
pub fn template_expression_ranges(fragment: &Fragment) -> Vec<TemplateExpression> {
    let mut out = Vec::new();
    walk_fragment(fragment, &mut |site| match site {
        TemplateSite::Expression(range) => out.push(TemplateExpression {
            range,
            is_declaration: false,
        }),
        TemplateSite::Declaration(range) => out.push(TemplateExpression {
            range,
            is_declaration: true,
        }),
        TemplateSite::DirectiveName(_) => {}
    });
    out
}

/// Every directive name in the template fragment, in source order.
/// Upstream's `Stores.handleDirective` treats a directive whose name
/// is a store (`use:$action`, `transition:$fly`) as a store reference.
pub fn template_directive_names(fragment: &Fragment) -> Vec<smol_str::SmolStr> {
    let mut out = Vec::new();
    walk_fragment(fragment, &mut |site| {
        if let TemplateSite::DirectiveName(name) = site {
            out.push(smol_str::SmolStr::from(name));
        }
    });
    out
}

/// One expression-bearing template range — see
/// [`template_expression_ranges`].
#[derive(Debug, Clone, Copy)]
pub struct TemplateExpression {
    pub range: Range,
    /// The payload after `@const` / `const` / `let`: `NAME = EXPR`,
    /// to be read as a declarator list.
    pub is_declaration: bool,
}

/// One template position of interest: an expression slice, a
/// declaration-tag payload, or a directive's name.
enum TemplateSite<'a> {
    Expression(Range),
    Declaration(Range),
    DirectiveName(&'a str),
}

fn walk_fragment(fragment: &Fragment, sink: &mut dyn FnMut(TemplateSite<'_>)) {
    for node in &fragment.nodes {
        walk_node(node, sink);
    }
}

fn walk_node(node: &Node, sink: &mut dyn FnMut(TemplateSite<'_>)) {
    match node {
        Node::Element(e) => {
            walk_attributes(&e.attributes, sink);
            walk_fragment(&e.children, sink);
        }
        Node::Component(c) => {
            // `<MyButton />` and `<ui.MyButton />` — the root identifier is
            // a value reference to the imported binding.);
            walk_attributes(&c.attributes, sink);
            walk_fragment(&c.children, sink);
        }
        Node::SvelteElement(s) => {
            walk_attributes(&s.attributes, sink);
            walk_fragment(&s.children, sink);
        }
        Node::Interpolation(i) => {
            use svn_parser::InterpolationKind;
            if matches!(
                i.kind,
                InterpolationKind::AtConst
                    | InterpolationKind::DeclConst
                    | InterpolationKind::DeclLet
            ) {
                sink(TemplateSite::Declaration(i.expression_range))
            } else {
                sink(TemplateSite::Expression(i.expression_range))
            }
        }
        Node::IfBlock(b) => {
            sink(TemplateSite::Expression(b.condition_range));
            walk_fragment(&b.consequent, sink);
            for arm in &b.elseif_arms {
                sink(TemplateSite::Expression(arm.condition_range));
                walk_fragment(&arm.body, sink);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, sink);
            }
        }
        Node::EachBlock(b) => {
            sink(TemplateSite::Expression(b.expression_range));
            if let Some(c) = &b.as_clause {
                if let Some(k) = c.key_range {
                    sink(TemplateSite::Expression(k));
                }
            }
            walk_fragment(&b.body, sink);
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, sink);
            }
        }
        Node::AwaitBlock(b) => {
            sink(TemplateSite::Expression(b.expression_range));
            if let Some(p) = &b.pending {
                walk_fragment(p, sink);
            }
            if let Some(t) = &b.then_branch {
                walk_fragment(&t.body, sink);
            }
            if let Some(c) = &b.catch_branch {
                walk_fragment(&c.body, sink);
            }
        }
        Node::KeyBlock(b) => {
            sink(TemplateSite::Expression(b.expression_range));
            walk_fragment(&b.body, sink);
        }
        Node::SnippetBlock(b) => walk_fragment(&b.body, sink),
        Node::Text(_) | Node::Comment(_) => {}
    }
}

fn walk_attributes(attrs: &[Attribute], sink: &mut dyn FnMut(TemplateSite<'_>)) {
    for attr in attrs {
        match attr {
            Attribute::Plain(p) => {
                if let Some(v) = &p.value {
                    for part in &v.parts {
                        if let AttrValuePart::Expression {
                            expression_range, ..
                        } = part
                        {
                            sink(TemplateSite::Expression(*expression_range));
                        }
                    }
                }
            }
            Attribute::Expression(e) => sink(TemplateSite::Expression(e.expression_range)),
            Attribute::Shorthand(_) => {}
            Attribute::Spread(s) => sink(TemplateSite::Expression(s.expression_range)),
            Attribute::Directive(d) => walk_directive(d, sink),
            Attribute::Comment(_) => {}
        }
    }
}

fn walk_directive(d: &Directive, sink: &mut dyn FnMut(TemplateSite<'_>)) {
    sink(TemplateSite::DirectiveName(&d.name));
    match &d.value {
        Some(DirectiveValue::Expression {
            expression_range, ..
        }) => {
            sink(TemplateSite::Expression(*expression_range));
        }
        Some(DirectiveValue::BindPair {
            getter_range,
            setter_range,
            ..
        }) => {
            sink(TemplateSite::Expression(*getter_range));
            sink(TemplateSite::Expression(*setter_range));
        }
        Some(DirectiveValue::Quoted(v)) => {
            for part in &v.parts {
                if let AttrValuePart::Expression {
                    expression_range, ..
                } = part
                {
                    sink(TemplateSite::Expression(*expression_range));
                }
            }
        }
        // A bare directive (`bind:value`, `class:active`) carries no
        // expression of its own.
        None => {}
    }
}
