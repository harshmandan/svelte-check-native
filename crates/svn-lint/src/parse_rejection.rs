//! Template input the Svelte compiler's `parse()` rejects outright,
//! beyond the structural errors our own parser reports.
//!
//! `svelte-check --tsgo` converts every component with `svelte2tsx`
//! in strict mode (`emitOnTemplateError: false`), which calls the
//! compiler's `parse()`. When that throws, svelte-check drops the file
//! from the run entirely: no overlay, no TypeScript diagnostics, no
//! compiler diagnostics, and the file does not count toward `<N> FILES`.
//! Two such rejections are detected here:
//!
//! - a template expression that is not valid JavaScript / TypeScript
//!   (`js_parse_error`, e.g. `{a +}`);
//! - a `<svelte:options runes>` value that is not a static boolean
//!   (`svelte_options_invalid_attribute_value`, e.g. `runes="true"` or
//!   `runes={flag}`) — `read_options` runs inside `parse()`;
//! - in a TypeScript component, an `{#each a, b as item}` head: the
//!   compiler first reads `a, b as item` as one expression (a sequence
//!   ending in a type assertion), keeps only `a`, and then needs an
//!   index identifier after the comma (`expected_identifier`).

use oxc_allocator::Allocator;
use oxc_ast::ast::Expression;
use oxc_parser::Parser;
use oxc_span::SourceType;
use svn_analyze::template_scope::{TemplateScopeVisitor, walk_with_visitor};
use svn_core::Range;
use svn_parser::ast::{
    AttrValuePart, Attribute, DirectiveValue, Fragment, Node, SvelteElementKind,
};
use svn_parser::{Component, Element, ScriptLang, SvelteElement};

/// True when the compiler's `parse()` would throw on this template for
/// one of the reasons in the module doc.
pub fn template_parse_rejected(fragment: &Fragment, source: &str, lang: ScriptLang) -> bool {
    if has_invalid_runes_option(fragment, source) {
        return true;
    }
    let mut finder = InvalidExpressionFinder {
        source,
        lang,
        allocator: Allocator::default(),
        found: false,
    };
    walk_with_visitor(fragment, source, &mut finder);
    finder.found
}

/// `read_options`' `get_boolean_value`: the value must be absent (bare
/// attribute) or a single `{true}` / `{false}` literal.
fn has_invalid_runes_option(fragment: &Fragment, source: &str) -> bool {
    fragment.nodes.iter().any(|node| {
        let Node::SvelteElement(se) = node else {
            return false;
        };
        se.kind == SvelteElementKind::Options
            && se.attributes.iter().any(|attr| match attr {
                Attribute::Plain(p) if p.name == "runes" => match &p.value {
                    None => false,
                    Some(v) => match v.parts.as_slice() {
                        [
                            AttrValuePart::Expression {
                                expression_range, ..
                            },
                        ] => !is_boolean_literal(*expression_range, source),
                        _ => true,
                    },
                },
                Attribute::Expression(e) if e.name == "runes" => {
                    !is_boolean_literal(e.expression_range, source)
                }
                Attribute::Shorthand(s) => s.name == "runes",
                _ => false,
            })
    })
}

fn is_boolean_literal(range: Range, source: &str) -> bool {
    matches!(
        source
            .get(range.start as usize..range.end as usize)
            .map(str::trim),
        Some("true" | "false")
    )
}

struct InvalidExpressionFinder<'s> {
    source: &'s str,
    lang: ScriptLang,
    allocator: Allocator,
    found: bool,
}

impl InvalidExpressionFinder<'_> {
    fn check(&mut self, range: Range) {
        if self.found {
            return;
        }
        let Some(text) = self.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.lang == ScriptLang::Ts);
        self.allocator.reset();
        let parsed: Result<Expression<'_>, _> =
            Parser::new(&self.allocator, text, source_type).parse_expression();
        if parsed.is_err() {
            self.found = true;
        }
    }

    fn check_attributes(&mut self, attributes: &[Attribute]) {
        for attr in attributes {
            match attr {
                Attribute::Plain(p) => {
                    if let Some(v) = &p.value {
                        self.check_parts(&v.parts);
                    }
                }
                Attribute::Expression(e) => self.check(e.expression_range),
                Attribute::Spread(s) => self.check(s.expression_range),
                Attribute::Directive(d) => match &d.value {
                    Some(DirectiveValue::Expression {
                        expression_range, ..
                    }) => self.check(*expression_range),
                    Some(DirectiveValue::Quoted(v)) => self.check_parts(&v.parts),
                    Some(DirectiveValue::BindPair { .. }) | None => {}
                },
                Attribute::Shorthand(_) | Attribute::Comment(_) => {}
            }
        }
    }

    fn check_parts(&mut self, parts: &[AttrValuePart]) {
        for part in parts {
            if let AttrValuePart::Expression {
                expression_range, ..
            } = part
            {
                self.check(*expression_range);
            }
        }
    }
}

impl InvalidExpressionFinder<'_> {
    /// The each-head rejection described in the module doc.
    fn check_each_head(&mut self, block: &svn_parser::EachBlock) {
        if self.found || self.lang != ScriptLang::Ts {
            return;
        }
        let Some(clause) = &block.as_clause else {
            return;
        };
        let Some(context) = clause.context_range else {
            return;
        };
        // What follows the context (`, i`, `(key)`) cannot turn a
        // sequence back into a single expression, so the head up to the
        // context's end decides. A context that is no valid type
        // (`{ x = 1 }`) fails to read, and the compiler then backs up to
        // the `as` and reads the head normally.
        let Some(text) = self
            .source
            .get(block.expression_range.start as usize..context.end as usize)
        else {
            return;
        };
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(true);
        self.allocator.reset();
        let parsed = Parser::new(&self.allocator, text, source_type).parse_expression();
        if let Ok(Expression::SequenceExpression(_)) = parsed {
            self.found = true;
        }
    }
}

impl TemplateScopeVisitor for InvalidExpressionFinder<'_> {
    fn visit_each_block(&mut self, block: &svn_parser::EachBlock) {
        self.check_each_head(block);
    }

    fn visit_expr(&mut self, range: Range) {
        self.check(range);
    }

    fn visit_element(&mut self, element: &Element) {
        self.check_attributes(&element.attributes);
    }

    fn visit_component(&mut self, component: &Component) {
        self.check_attributes(&component.attributes);
    }

    fn visit_svelte_element(&mut self, element: &SvelteElement) {
        self.check_attributes(&element.attributes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(source: &str) -> bool {
        let (doc, _) = svn_parser::parse_sections(source);
        let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
        template_parse_rejected(&fragment, source, doc.script_lang())
    }

    #[test]
    fn invalid_template_expression_is_rejected() {
        assert!(rejected("<p>{a +}</p>"));
        assert!(rejected("<div title={a +}></div>"));
        assert!(rejected("{#if a +}x{/if}"));
    }

    #[test]
    fn valid_template_expressions_are_accepted() {
        assert!(!rejected(
            "<p>{a + b}</p><div {...rest} on:click={() => {}}></div>"
        ));
        assert!(!rejected("<script lang=\"ts\"></script>{a as number}"));
        assert!(!rejected("{@debug a, b}{#each xs as x (x.id)}{x}{/each}"));
    }

    #[test]
    fn each_head_read_as_a_sequence_is_rejected_in_typescript() {
        let ts = "<script lang=\"ts\"></script>";
        assert!(rejected(&format!(
            "{ts}{{#each true, [1] as item}}{{/each}}"
        )));
        assert!(rejected(&format!("{ts}{{#each a, b as item, i}}{{/each}}")));
        assert!(rejected(&format!(
            "{ts}{{#each a, b as item (item)}}{{/each}}"
        )));
        assert!(rejected(&format!("{ts}{{#each a, b as [x]}}{{/each}}")));
        assert!(!rejected(&format!(
            "{ts}{{#each a, b as {{x = 1}}}}{{/each}}"
        )));
        assert!(!rejected(&format!("{ts}{{#each f(a, b) as x}}{{/each}}")));
        assert!(!rejected(&format!("{ts}{{#each a, i}}{{/each}}")));
        assert!(!rejected(&format!("{ts}{{#each a as item, i}}{{/each}}")));
        assert!(!rejected("{#each true, [1] as item}{/each}"));
    }

    #[test]
    fn non_boolean_runes_option_is_rejected() {
        assert!(rejected("<svelte:options runes=\"true\" />"));
        assert!(rejected("<svelte:options runes={flag} />"));
        assert!(!rejected("<svelte:options runes={false} />"));
        assert!(!rejected("<svelte:options runes />"));
    }
}
