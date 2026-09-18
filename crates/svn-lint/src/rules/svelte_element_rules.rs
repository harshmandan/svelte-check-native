//! Rules that fire on `<svelte:*>` special elements.

use svn_parser::ast::{AttrValuePart, Attribute, SvelteElement, SvelteElementKind};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::element_rules::{AttrParent, visit_attribute};

pub fn visit(se: &SvelteElement, ctx: &mut LintContext<'_>, ancestors: &[crate::walk::Ancestor]) {
    match se.kind {
        SvelteElementKind::Element => {
            crate::rules::element_rules::validate_element_errors(&se.attributes, true, ctx);
        }
        SvelteElementKind::Head => {
            if let Some(attr) = first_attribute(&se.attributes) {
                ctx.emit_error(
                    Code::svelte_head_illegal_attribute,
                    messages::svelte_head_illegal_attribute(),
                    attr.range(),
                );
            }
        }
        SvelteElementKind::Window | SvelteElementKind::Document | SvelteElementKind::Body => {
            disallow_children(se, ctx);
            let name = se.kind.as_str();
            for attr in &se.attributes {
                let illegal = match attr {
                    Attribute::Spread(s) => !s.is_attach,
                    Attribute::Plain(_) | Attribute::Expression(_) | Attribute::Shorthand(_) => {
                        !is_event_attribute(attr)
                    }
                    _ => false,
                };
                if !illegal {
                    continue;
                }
                if se.kind == SvelteElementKind::Body {
                    ctx.emit_error(
                        Code::svelte_body_illegal_attribute,
                        messages::svelte_body_illegal_attribute(),
                        attr.range(),
                    );
                } else {
                    ctx.emit_error(
                        Code::illegal_element_attribute,
                        messages::illegal_element_attribute(&format!("svelte:{name}")),
                        attr.range(),
                    );
                }
            }
        }
        SvelteElementKind::Fragment => {
            if !matches!(
                ctx.template_path.last(),
                Some(crate::walk::PathFrame::Component {
                    kind: crate::walk::ComponentKind::Component
                        | crate::walk::ComponentKind::SvelteComponent,
                    ..
                })
            ) {
                ctx.emit_error(
                    Code::svelte_fragment_invalid_placement,
                    messages::svelte_fragment_invalid_placement(),
                    se.range,
                );
            }
            for attr in &se.attributes {
                match attr {
                    Attribute::Plain(_) | Attribute::Expression(_) | Attribute::Shorthand(_) => {
                        crate::rules::element_rules::validate_fragment_slot_attribute(attr, ctx);
                    }
                    Attribute::Directive(d) if d.kind == svn_parser::ast::DirectiveKind::Let => {}
                    Attribute::Comment(_) => {}
                    Attribute::Directive(_) | Attribute::Spread(_) => ctx.emit_error(
                        Code::svelte_fragment_invalid_attribute,
                        messages::svelte_fragment_invalid_attribute(),
                        attr.range(),
                    ),
                }
            }
        }
        SvelteElementKind::Boundary => {
            for attr in &se.attributes {
                if matches!(attr, Attribute::Comment(_)) {
                    continue;
                }
                let valid_name = matches!(
                    attr,
                    Attribute::Plain(_) | Attribute::Expression(_) | Attribute::Shorthand(_)
                ) && matches!(
                    crate::rules::element_rules::plain_attribute_name(attr),
                    Some("onerror" | "failed" | "pending")
                );
                if !valid_name {
                    ctx.emit_error(
                        Code::svelte_boundary_invalid_attribute,
                        messages::svelte_boundary_invalid_attribute(),
                        attr.range(),
                    );
                }
                let invalid_value = match attr {
                    Attribute::Plain(p) => match &p.value {
                        None => true,
                        Some(v) => {
                            !matches!(v.parts.as_slice(), [AttrValuePart::Expression { .. }])
                        }
                    },
                    _ => false,
                };
                if invalid_value {
                    ctx.emit_error(
                        Code::svelte_boundary_invalid_attribute_value,
                        messages::svelte_boundary_invalid_attribute_value(),
                        attr.range(),
                    );
                }
            }
        }
        _ => {}
    }
    // svelte_self_invalid_placement (`SvelteSelf.js`): `<svelte:self>`
    // needs an `{#if}`, `{#each}`, `{#snippet}` or component ancestor.
    if se.kind == SvelteElementKind::SelfRef
        && !ctx.template_path.iter().any(|f| {
            matches!(
                f,
                crate::walk::PathFrame::IfBlock
                    | crate::walk::PathFrame::EachBlock { .. }
                    | crate::walk::PathFrame::SnippetBlock
                    | crate::walk::PathFrame::Component {
                        kind: crate::walk::ComponentKind::Component,
                        ..
                    }
            )
        })
    {
        ctx.emit_error(
            Code::svelte_self_invalid_placement,
            messages::svelte_self_invalid_placement(),
            se.range,
        );
    }

    // svelte_component_deprecated: `<svelte:component>` in runes mode.
    if ctx.runes && se.kind == SvelteElementKind::Component {
        let msg = messages::svelte_component_deprecated();
        ctx.emit(Code::svelte_component_deprecated, msg, se.range);
    }

    // svelte_self_deprecated: `<svelte:self>` in runes mode, naming the
    // component the way the compiler does from its file name.
    if ctx.runes && se.kind == SvelteElementKind::SelfRef {
        let (name, basename) = match &ctx.filename {
            Some(path) => (
                component_name(path, ctx.scope_tree.as_ref()),
                path.file_name()
                    .map(|b| b.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            None => ("Self".to_string(), "Self.svelte".to_string()),
        };
        let msg = messages::svelte_self_deprecated(&name, &basename);
        ctx.emit(Code::svelte_self_deprecated, msg, se.range);
    }

    // Attribute checks: which AttrParent shape to route through.
    let parent = match se.kind {
        SvelteElementKind::Component | SvelteElementKind::SelfRef => {
            AttrParent::SvelteComponentLike
        }
        SvelteElementKind::Element => AttrParent::SvelteElement,
        SvelteElementKind::Window => AttrParent::SvelteSpecial("svelte:window"),
        SvelteElementKind::Document => AttrParent::SvelteSpecial("svelte:document"),
        SvelteElementKind::Body => AttrParent::SvelteSpecial("svelte:body"),
        SvelteElementKind::Fragment => AttrParent::SvelteFragment,
        _ => AttrParent::OtherSvelte,
    };
    if matches!(parent, AttrParent::SvelteComponentLike) {
        crate::rules::component_rules::check_component_attributes(&se.attributes, ctx);
    }
    for attr in &se.attributes {
        visit_attribute(attr, &se.attributes, ctx, parent);
    }

    // Only route `<svelte:element>` through the a11y check — the
    // other svelte:* kinds (component/self/window/document/body/
    // head/options/fragment/boundary) aren't rendered elements.
    if se.kind == SvelteElementKind::Element {
        // svelte_element_invalid_this: `<svelte:element this="div">` (or
        // `this="h{n}"`) — `this` should be a single `{expression}`, not a
        // string / text-with-interpolation. Mirrors upstream's
        // `!is_expression_attribute(this)` warning (1-parse/state/element.js).
        if let Some(this_attr) = se.attributes.iter().find(|a| match a {
            Attribute::Plain(p) => p.name == "this",
            Attribute::Expression(e) => e.name == "this",
            Attribute::Shorthand(s) => s.name == "this",
            _ => false,
        }) {
            let is_expression = match this_attr {
                Attribute::Expression(_) | Attribute::Shorthand(_) => true,
                Attribute::Plain(p) => matches!(
                    p.value.as_ref(),
                    Some(v) if v.parts.len() == 1
                        && matches!(v.parts[0], AttrValuePart::Expression { .. })
                ),
                _ => false,
            };
            if !is_expression {
                let r = match this_attr {
                    Attribute::Plain(p) => p.range,
                    Attribute::Expression(e) => e.range,
                    Attribute::Shorthand(s) => s.range,
                    _ => se.range,
                };
                ctx.emit(
                    Code::svelte_element_invalid_this,
                    messages::svelte_element_invalid_this(),
                    r,
                );
            }
        }
        crate::rules::a11y_rules::visit_dynamic(se, ctx, ancestors);
    }
}

/// The compiler's component name: `get_component_name` (the file's
/// base name without `.svelte`, or its directory for an `index`
/// outside `src`, capitalised), then `scope.generate` (characters an
/// identifier can't hold become `_`, and a name the component already
/// uses gets a `_N` suffix).
fn component_name(path: &std::path::Path, tree: Option<&crate::scope::ScopeTree>) -> String {
    let basename = path
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut name = basename.replacen(".svelte", "", 1);
    if name == "index"
        && let Some(dir) = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|d| d.to_string_lossy().into_owned())
        && !dir.is_empty()
        && dir != "src"
    {
        name = dir;
    }
    let mut chars = name.chars();
    let name: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    let mut preferred: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if preferred.starts_with(|c: char| c.is_ascii_digit()) {
        preferred.replace_range(0..1, "_");
    }
    let taken = |candidate: &str| {
        tree.is_some_and(|t| {
            t.all_bindings().any(|(_, b)| b.name == candidate)
                || t.unresolved_refs.iter().any(|r| r.name == candidate)
        })
    };
    if !taken(&preferred) {
        return preferred;
    }
    (1..)
        .map(|n| format!("{preferred}_{n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(preferred)
}

/// The first attribute (in-tag comments are not attributes).
fn first_attribute(attributes: &[Attribute]) -> Option<&Attribute> {
    attributes
        .iter()
        .find(|a| !matches!(a, Attribute::Comment(_)))
}

/// The compiler's `is_event_attribute`: an `on*` attribute whose value
/// is a single expression.
fn is_event_attribute(attr: &Attribute) -> bool {
    match attr {
        Attribute::Plain(p) => {
            p.name.starts_with("on")
                && matches!(
                    p.value.as_ref().map(|v| v.parts.as_slice()),
                    Some([AttrValuePart::Expression { .. }])
                )
        }
        Attribute::Expression(e) => e.name.starts_with("on"),
        Attribute::Shorthand(s) => s.name.starts_with("on"),
        _ => false,
    }
}

/// `disallow_children` (`shared/special-element.js`): a
/// `<svelte:window>`, `<svelte:document>` or `<svelte:body>` has no
/// content, reported over the whole of it.
fn disallow_children(se: &SvelteElement, ctx: &mut LintContext<'_>) {
    if let (Some(first), Some(last)) = (se.children.nodes.first(), se.children.nodes.last()) {
        ctx.emit_error(
            Code::svelte_meta_invalid_content,
            messages::svelte_meta_invalid_content(&format!("svelte:{}", se.kind.as_str())),
            svn_core::Range::new(first.range().start, last.range().end),
        );
    }
}
