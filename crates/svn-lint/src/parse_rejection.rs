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
//! - the errors `crate::parse_errors` ports (duplicate attributes,
//!   misplaced `<svelte:*>` tags, invalid `<svelte:options>` values,
//!   malformed `{@…}` tags, …);
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
use svn_parser::ast::{AttrValuePart, Attribute, DirectiveValue, Fragment};
use svn_parser::{Component, Element, ScriptLang, SvelteElement};

/// True when the compiler's `parse()` would throw on this template for
/// one of the reasons in the module doc.
pub fn template_parse_rejected(fragment: &Fragment, source: &str, lang: ScriptLang) -> bool {
    if crate::parse_errors::first_template_parse_error(fragment, source, lang == ScriptLang::Ts)
        .is_some()
    {
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
    fn template_errors_of_the_compilers_parser_are_rejected() {
        for source in [
            "<div a=\"1\" a=\"2\"></div>",
            "<input value=\"1\" bind:value={v} />",
            "<div class:a class:a></div>",
            "<div style:color=\"red\" style:color|important=\"red\"></div>",
            "<div on:click=\"foo\"></div>",
            "<div on:click=\"\"></div>",
            "<svelte:element></svelte:element>",
            "<svelte:element this></svelte:element>",
            "<svelte:component></svelte:component>",
            "<svelte:component this=\"x\"></svelte:component>",
            "<div title=\"{@html a}\"></div>",
            "{@debug a.b}",
            "{@render a}",
            "{#if a}{@const b = 1, c = 2}{/if}",
            "{@foo x}",
            "{@attach x}",
            "<svelte:window /><svelte:window />",
            "{#if a}<svelte:window />{/if}",
            "<div><svelte:head></svelte:head></div>",
            "<svelte:options tag=\"my-el\" />",
            "<svelte:options foo />",
            "<svelte:options namespace=\"foo\" />",
            "<svelte:options css=\"external\" />",
            "<svelte:options immutable=\"yes\" />",
            "<svelte:options on:click={() => {}} />",
            "<svelte:options>hi</svelte:options>",
            "<svelte:options customElement />",
            "<svelte:options customElement={42} />",
            "<svelte:options customElement=\"font-face\" />",
            "<svelte:options customElement=\"Invalid\" />",
            "<div. ></div.>",
            "<p>{}</p>",
            "<div title={}></div>",
            "{#snippet s({ class })}x{/snippet}",
            "<div {this}></div>",
            "{#each items as x, default}{x}{/each}",
        ] {
            assert!(rejected(source), "{source}");
        }
    }

    #[test]
    fn templates_the_compilers_parser_accepts_are_kept() {
        for source in [
            "<script>let el;</script><svelte:element this=\"div\" bind:this={el}></svelte:element>",
            "<div class=\"a\" class:a style:color=\"red\" style:a=\"b\"></div>",
            "<div on:click={f} on:click={g}></div>",
            "<svelte:element this={tag}></svelte:element>",
            "<svelte:component this={C}></svelte:component>",
            "{#if a}{@const b = (1, 2)}{b}{/if}",
            "{@debug a, b}{@debug}",
            "{@render a?.()}",
            "<svelte:options customElement={null} />",
            "<svelte:options customElement={{ tag: \"my-el\", props: { a: { type: \"String\" } } }} />",
            "<svelte:options namespace=\"svg\" immutable accessors={false} css=\"injected\" />",
            "<enhanced:img src=\"x\" /><Foo.Bar /><my-element />",
            "<!DOCTYPE html>",
            "{#snippet s({ a, b = 1 }, ...rest)}x{/snippet}",
        ] {
            assert!(!rejected(source), "{source}");
        }
    }

    #[test]
    fn non_boolean_runes_option_is_rejected() {
        assert!(rejected("<svelte:options runes=\"true\" />"));
        assert!(rejected("<svelte:options runes={flag} />"));
        assert!(!rejected("<svelte:options runes={false} />"));
        assert!(!rejected("<svelte:options runes />"));
    }
}
