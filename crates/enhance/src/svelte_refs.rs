//! Every place a file references a `.svelte` module, found by walking
//! real ASTs.
//!
//! The compiler resolves a module wherever it is named: an `import` /
//! `export … from` / `import()` / `import x = require()`, an `import()`
//! type, and — in JavaScript only — the import types and `@import` tags
//! of JSDoc comments. In a component the template counts too: every
//! expression, attribute value, `{@const}` and snippet signature ends up
//! in the code svelte2tsx generates. The default svelte-check engine
//! reports `TS2307` at each one when the module is missing, so each one
//! is collected here, at the exact span of its string literal.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration, ImportExpression,
    Program, StringLiteral, TSExternalModuleReference, TSImportType,
};
use oxc_ast_visit::{Visit, walk};
use oxc_jsdoc::JSDoc;
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use svn_analyze::template_scope::{TemplateScopeVisitor, walk_with_visitor};
use svn_core::Range;
use svn_parser::{AttrValue, AttrValuePart, Attribute, DirectiveValue, Document, Fragment};

/// A `.svelte` module specifier, with the byte span of its string
/// literal — quotes included — in the original file.
pub(crate) struct SvelteImportRef {
    pub(crate) specifier: String,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// Where a parsed piece of source sits in the original file: a span `s`
/// of the parsed text is at `base + (s - prefix)`, `prefix` being the
/// wrapper text added in front to make the piece parse.
#[derive(Clone, Copy)]
struct Origin {
    base: u32,
    prefix: u32,
}

impl Origin {
    fn place(self, span: Span) -> (u32, u32) {
        (
            self.base + span.start.saturating_sub(self.prefix),
            self.base + span.end.saturating_sub(self.prefix),
        )
    }
}

/// The references in a whole TS/JS file. `js` reads JSDoc too.
pub(crate) fn collect_module(program: &Program<'_>, js: bool) -> Vec<SvelteImportRef> {
    let mut out = Vec::new();
    collect_program(program, Origin { base: 0, prefix: 0 }, js, &mut out);
    dedup(out)
}

/// The references in a component: its instance and module scripts and
/// every expression of its template. `is_ts` is the component's
/// language (`<script lang="ts">` on either script); JSDoc counts only
/// in a JavaScript component.
pub(crate) fn collect_component(
    doc: &Document<'_>,
    fragment: &Fragment,
    source: &str,
    is_ts: bool,
) -> Vec<SvelteImportRef> {
    let mut out = Vec::new();
    for section in [doc.instance_script.as_ref(), doc.module_script.as_ref()]
        .into_iter()
        .flatten()
    {
        let allocator = Allocator::default();
        let parsed = svn_parser::parse_script_body(&allocator, section.content, section.lang);
        if parsed.panicked {
            // A syntactically broken script yields a garbage AST; the
            // caller reports the parse error separately. Skip.
            continue;
        }
        let origin = Origin {
            base: section.content_range.start,
            prefix: 0,
        };
        collect_program(&parsed.program, origin, !is_ts, &mut out);
    }
    let mut sites = TemplateSites {
        source,
        sites: Vec::new(),
    };
    walk_with_visitor(fragment, source, &mut sites);
    for (range, wrap) in sites.sites {
        let (prefix, suffix) = wrap.text();
        if let Some(body) = source.get(range.start as usize..range.end as usize) {
            collect_wrapped(body, range.start, prefix, suffix, is_ts, !is_ts, &mut out);
        }
    }
    dedup(out)
}

/// The same literal can be reached twice (a snippet parameter's default
/// is both part of the parameter list and an expression of its own).
fn dedup(mut refs: Vec<SvelteImportRef>) -> Vec<SvelteImportRef> {
    refs.sort_by_key(|r| (r.start, r.end));
    refs.dedup_by_key(|r| (r.start, r.end));
    refs
}

fn collect_program(
    program: &Program<'_>,
    origin: Origin,
    js: bool,
    out: &mut Vec<SvelteImportRef>,
) {
    ImportCollector { origin, out }.visit_program(program);
    if js {
        collect_jsdoc(program, origin, out);
    }
}

/// Parse `prefix + body + suffix` and collect from it, placing what it
/// finds back relative to `base`, where `body` starts in the original
/// file.
fn collect_wrapped(
    body: &str,
    base: u32,
    prefix: &str,
    suffix: &str,
    ts: bool,
    js: bool,
    out: &mut Vec<SvelteImportRef>,
) {
    if !body.contains(".svelte") {
        return;
    }
    let text = format!("{prefix}{body}{suffix}");
    let allocator = Allocator::default();
    let source_type = if ts {
        SourceType::ts()
    } else {
        SourceType::mjs()
    };
    let parsed = Parser::new(&allocator, &text, source_type).parse();
    if parsed.fatal_error {
        return;
    }
    let origin = Origin {
        base,
        prefix: prefix.len() as u32,
    };
    collect_program(&parsed.program, origin, js, out);
}

/// JSDoc import types (`@type {import('./X.svelte').default}`, any tag's
/// leading `{type}`) and `@import` tags. Each type or tag body is parsed
/// as TypeScript on its own. The parser recovers from JSDoc-only prefix
/// syntax such as `?T`; a body it cannot parse at all contributes
/// nothing.
fn collect_jsdoc(program: &Program<'_>, origin: Origin, out: &mut Vec<SvelteImportRef>) {
    let text = program.source_text;
    for comment in program.comments.iter().filter(|c| c.is_jsdoc()) {
        let content = comment.content_span();
        // `content_span` starts after `/*`; JSDoc text starts after `/**`.
        let span = Span::new(content.start + 1, content.end);
        let jsdoc = JSDoc::new(span.source_text(text), span);
        for tag in jsdoc.tags() {
            let (part, prefix, suffix) = if tag.kind.parsed() == "import" {
                (tag.comment().span, "import ", "\n;")
            } else if let Some(ty) = tag.r#type() {
                (
                    Span::new(ty.span.start + 1, ty.span.end - 1),
                    "type __svn_jsdoc = ",
                    "\n;",
                )
            } else {
                continue;
            };
            let Some(body) = text.get(part.start as usize..part.end as usize) else {
                continue;
            };
            let (start, _) = origin.place(part);
            collect_wrapped(
                &blank_line_markers(body),
                start,
                prefix,
                suffix,
                true,
                false,
                out,
            );
        }
    }
}

/// A JSDoc body that runs over several lines carries each line's
/// leading `*`; the compiler's JSDoc scanner skips it. Blank it (same
/// byte length, so spans stay put) so the body parses as plain code.
fn blank_line_markers(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for (i, line) in body.split_inclusive('\n').enumerate() {
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        if i > 0 && line[indent..].starts_with('*') {
            out.push_str(&line[..indent]);
            out.push(' ');
            out.push_str(&line[indent + 1..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// AST visitor that records every `.svelte` module specifier: static
/// and dynamic imports, re-exports, import types and `import =
/// require()`.
struct ImportCollector<'o> {
    origin: Origin,
    out: &'o mut Vec<SvelteImportRef>,
}

impl ImportCollector<'_> {
    fn record(&mut self, lit: &StringLiteral<'_>) {
        let Some(spec) = lit.value.as_str() else {
            return;
        };
        if !is_svelte_specifier(spec) {
            return;
        }
        let (start, end) = self.origin.place(lit.span);
        self.out.push(SvelteImportRef {
            specifier: spec.to_string(),
            start,
            end,
        });
    }
}

impl<'a> Visit<'a> for ImportCollector<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.record(&it.source);
    }

    /// `export { Foo } from './Foo.svelte'`. This is its own node kind;
    /// a specifier list with no `from` clause (`ExportNamedDeclaration`)
    /// names no module and needs no visitor, and the declaration form
    /// (`ExportDeclaration`, e.g. `export const x =
    /// import('./y.svelte')`) is reached by the default walk.
    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        self.record(&it.source);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        self.record(&it.source);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        if let Expression::StringLiteral(lit) = &it.source {
            self.record(lit);
        }
        walk::walk_import_expression(self, it);
    }

    /// `typeof import('./X.svelte')`, `import('./X.svelte').Props<T>`.
    fn visit_ts_import_type(&mut self, it: &TSImportType<'a>) {
        self.record(&it.source);
        walk::walk_ts_import_type(self, it);
    }

    /// `import X = require('./X.svelte')`.
    fn visit_ts_external_module_reference(&mut self, it: &TSExternalModuleReference<'a>) {
        self.record(&it.expression);
    }
}

/// A specifier that names a Svelte component file: ends in `.svelte`.
/// Excludes `.svelte.ts` / `.svelte.js` runes-module specifiers (those end
/// in `.ts` / `.js`). Query-suffixed forms (`x.svelte?raw`) are excluded —
/// they don't end in `.svelte` and aren't TS's concern.
fn is_svelte_specifier(spec: &str) -> bool {
    spec.ends_with(".svelte")
}

/// How a template site's text is wrapped to parse as a program.
#[derive(Clone, Copy)]
enum Wrap {
    /// `{expr}`, a block's expression, an attribute value.
    Expression,
    /// `{@const x = expr}` / `{const …}` / `{let …}`: the body after the
    /// keyword.
    Declaration,
    /// A snippet's parameter list.
    Parameters,
    /// A snippet's type-parameter list.
    TypeParameters,
}

impl Wrap {
    /// Prefix and suffix. Each suffix starts on a new line so a trailing
    /// line comment in the site cannot swallow it.
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Wrap::Expression => ("(", "\n);"),
            Wrap::Declaration => ("let ", "\n;"),
            Wrap::Parameters => ("function __svn(", "\n) {}"),
            Wrap::TypeParameters => ("function __svn<", "\n>() {}"),
        }
    }
}

/// Every expression-bearing site of a template, from the shared template
/// walk.
struct TemplateSites<'s> {
    source: &'s str,
    sites: Vec<(Range, Wrap)>,
}

impl TemplateSites<'_> {
    fn attributes(&mut self, attributes: &[Attribute]) {
        for attribute in attributes {
            match attribute {
                Attribute::Plain(a) => {
                    if let Some(value) = &a.value {
                        self.value(value);
                    }
                }
                Attribute::Expression(a) => self.expression(a.expression_range),
                Attribute::Spread(a) => self.expression(a.expression_range),
                Attribute::Directive(d) => match &d.value {
                    Some(DirectiveValue::Expression {
                        expression_range, ..
                    }) => self.expression(*expression_range),
                    Some(DirectiveValue::BindPair {
                        getter_range,
                        setter_range,
                        ..
                    }) => {
                        self.expression(*getter_range);
                        self.expression(*setter_range);
                    }
                    Some(DirectiveValue::Quoted(value)) => self.value(value),
                    None => {}
                },
                // A bare identifier or a comment: no module reference.
                Attribute::Shorthand(_) | Attribute::Comment(_) => {}
            }
        }
    }

    fn value(&mut self, value: &AttrValue) {
        for part in &value.parts {
            if let AttrValuePart::Expression {
                expression_range, ..
            } = part
            {
                self.expression(*expression_range);
            }
        }
    }

    fn expression(&mut self, range: Range) {
        self.push(range, Wrap::Expression);
    }

    fn push(&mut self, range: Range, wrap: Wrap) {
        // Most sites name no module at all; skip them before parsing.
        if self
            .source
            .get(range.start as usize..range.end as usize)
            .is_some_and(|s| s.contains(".svelte"))
        {
            self.sites.push((range, wrap));
        }
    }
}

impl TemplateScopeVisitor for TemplateSites<'_> {
    fn visit_expr(&mut self, range: Range) {
        self.push(range, Wrap::Expression);
    }

    fn visit_at_const(&mut self, _bound_names: &[smol_str::SmolStr], expr_range: Range) {
        self.push(expr_range, Wrap::Declaration);
    }

    fn visit_element(&mut self, element: &svn_parser::Element) {
        self.attributes(&element.attributes);
    }

    fn visit_component(&mut self, component: &svn_parser::Component) {
        self.attributes(&component.attributes);
    }

    fn visit_svelte_element(&mut self, element: &svn_parser::SvelteElement) {
        self.attributes(&element.attributes);
    }

    fn visit_snippet_block(&mut self, block: &svn_parser::SnippetBlock) {
        if block.parameters_range.start < block.parameters_range.end {
            self.push(block.parameters_range, Wrap::Parameters);
        }
        if let Some(generics) = block.generics_range {
            self.push(generics, Wrap::TypeParameters);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(src: &str) -> Vec<(String, String)> {
        let (doc, _) = svn_parser::parse_sections(src);
        let (fragment, _) = svn_parser::parse_all_template_runs(src, &doc.template.text_runs);
        let is_ts = svn_parser::is_ts_svelte(src);
        collect_component(&doc, &fragment, src, is_ts)
            .into_iter()
            .map(|r| {
                (
                    r.specifier,
                    src[r.start as usize..r.end as usize].to_string(),
                )
            })
            .collect()
    }

    fn module(src: &str, js: bool) -> Vec<(String, String)> {
        let allocator = Allocator::default();
        let source_type = if js {
            SourceType::mjs()
        } else {
            SourceType::ts()
        };
        let parsed = Parser::new(&allocator, src, source_type).parse();
        collect_module(&parsed.program, js)
            .into_iter()
            .map(|r| {
                (
                    r.specifier,
                    src[r.start as usize..r.end as usize].to_string(),
                )
            })
            .collect()
    }

    fn specs(found: &[(String, String)]) -> Vec<&str> {
        found.iter().map(|(s, _)| s.as_str()).collect()
    }

    #[test]
    fn module_references_land_on_their_literal() {
        let src = "import A from './A.svelte';\n\
            import { 'a-b' as B } from './B.svelte';\n\
            export * from './C.svelte';\n\
            export { default as D } from './D.svelte';\n\
            type E = typeof import('./E.svelte');\n\
            type F = import('./F.svelte').P<import('./G.svelte').Q>;\n\
            import H = require('./H.svelte');\n\
            const i = () => import('./I.svelte');\n\
            import J from './J.svelte.ts';\n";
        let found = module(src, false);
        assert_eq!(
            specs(&found),
            [
                "./A.svelte",
                "./B.svelte",
                "./C.svelte",
                "./D.svelte",
                "./E.svelte",
                "./F.svelte",
                "./G.svelte",
                "./H.svelte",
                "./I.svelte"
            ]
        );
        assert!(found.iter().all(|(s, lit)| *lit == format!("'{s}'")));
    }

    #[test]
    fn jsdoc_counts_only_in_javascript() {
        let src = "/** @import { P } from './At.svelte' */\n\
            /**\n * @typedef {{\n *   a: import('./Multi.svelte').A\n * }} M\n */\n\
            /** @param {import('./Param.svelte').X} x */\n\
            export function f(x) { return x; }\n\
            /** @type {?import('./Nullable.svelte')} */\n\
            export let y;\n\
            /* @type {import('./NotJsDoc.svelte')} */\n";
        let found = module(src, true);
        assert_eq!(
            specs(&found),
            [
                "./At.svelte",
                "./Multi.svelte",
                "./Param.svelte",
                "./Nullable.svelte"
            ]
        );
        assert!(found.iter().all(|(s, lit)| *lit == format!("'{s}'")));
        assert!(module(src, false).is_empty());
    }

    #[test]
    fn component_template_sites() {
        let src = "<script lang=\"ts\">\nimport S from './Script.svelte';\n</script>\n\
            {#await import('./Await.svelte') then m}<m.default />{/await}\n\
            <div data-x={import('./Attr.svelte')} title=\"a{import('./Part.svelte')}\"></div>\n\
            <Comp {...import('./Spread.svelte')} bind:value={() => import('./Get.svelte'), (v) => v} />\n\
            {#snippet s(p: import('./Param.svelte').P = import('./Default.svelte'))}{p}{/snippet}\n\
            {#snippet g<T extends import('./Generic.svelte').G>(t: T)}{t}{/snippet}\n\
            {#if true}{@const L = import('./Const.svelte')}{L}{/if}\n\
            {#each [] as { a = import('./EachDefault.svelte') }}{a}{/each}\n\
            <p>{typeof import('./Interp.svelte')}</p>\n";
        let found = component(src);
        let mut got = specs(&found);
        got.sort();
        assert_eq!(
            got,
            [
                "./Attr.svelte",
                "./Await.svelte",
                "./Const.svelte",
                "./Default.svelte",
                "./EachDefault.svelte",
                "./Generic.svelte",
                "./Get.svelte",
                "./Interp.svelte",
                "./Param.svelte",
                "./Part.svelte",
                "./Script.svelte",
                "./Spread.svelte"
            ]
        );
        assert!(found.iter().all(|(s, lit)| *lit == format!("'{s}'")));
    }

    #[test]
    fn component_jsdoc_follows_the_component_language() {
        let js = "<script>\n/** @type {import('./Doc.svelte').default} */\nlet c;\n</script>\n\
            {/** @type {import('./Cast.svelte')} */ (c)}\n";
        assert_eq!(specs(&component(js)), ["./Doc.svelte", "./Cast.svelte"]);
        let ts = js.replace("<script>", "<script lang=\"ts\">");
        assert!(component(&ts).is_empty());
    }

    #[test]
    fn line_markers_blank_to_the_same_length() {
        let body = "{\n *   a: 1\n   * }";
        let blanked = blank_line_markers(body);
        assert_eq!(blanked, "{\n     a: 1\n     }");
        assert_eq!(blanked.len(), body.len());
    }
}
